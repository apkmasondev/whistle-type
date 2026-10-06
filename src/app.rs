//! Main controller: owns all state on the UI thread and reacts to window messages.
//!
//! Threads: UI (this), keyboard hook, audio capture (only while recording), engine (Whistle), inserter
//! (clipboard/SendInput), downloader (only while downloading). Other threads talk to the UI thread only by
//! posting messages to the main window; no state is shared except a few atomics.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::audio::{self, AudioError, AudioEvent, Device, Purpose, Recorder};
use crate::download::DownloadError;
use crate::engine::{EngineCmd, EngineError, EngineEvent, EngineService, JobResult, TranscribeJob, WhisperRequest, WhisperStatus};
use crate::models::{self, WhisperModel};
use crate::stt::EngineKind;
use crate::insert::{CopyReason, InsertJob, InsertOutcome, InsertResult, Inserter};
use crate::msg::{self, HK_CANCEL, HK_DOWN, HK_UP, WM_WT_EVENT};
use crate::overlay::{MsgKind, Overlay};
use crate::settings::{Hotkey, Mode, Settings};
use crate::tray::{Tray, TrayState};
use crate::vad::Verdict;
use crate::i18n::{fmt, t};
use crate::{autostart, hotkey, model, pipeline, settings_ui, setup_ui, ui, util, vocab_ui};
use crate::{log_error, log_info, log_warn};

pub enum AppEvent {
    Engine(EngineEvent),
    Audio { seq: u64, ev: AudioEvent },
    Insert(InsertResult),
    DownloadProgress { done: u64, total: u64 },
    DownloadDone(Result<(), DownloadError>),
    /// Run a closure on the UI thread outside of any App borrow (modal dialogs etc.).
    Deferred(Box<dyn FnOnce()>),
}

/// What the (single) download slot is fetching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadItem {
    Whistle,
    Whisper(&'static str),
    CudaPack,
}

/// Helper for the first-run window, which only deals with the FAST model.
pub trait WhistleDownload {
    fn is_none_or_whistle(&self) -> bool;
}

impl WhistleDownload for Option<DownloadItem> {
    fn is_none_or_whistle(&self) -> bool {
        matches!(self, None | Some(DownloadItem::Whistle))
    }
}

/// State of the ACCURATE engine (Whisper).
#[derive(Debug, Clone, PartialEq)]
pub enum WhisperState {
    /// Not needed (FAST mode, or no Whisper model installed).
    Off,
    Loading { model_id: String },
    Ready(WhisperStatus),
    Failed { model_id: String, error: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineState {
    Starting,
    NoModel,
    Loading,
    Ready { load_ms: f64 },
    ModelError(String),
    RuntimeError(String),
}

const TIMER_TAIL: usize = 10;
const TIMER_POLL: usize = 11;
/// Audio captured after the key is released, so the last syllable is not cut off.
const RELEASE_TAIL_MS: u32 = 120;
const FALLBACK_HOTKEY_ID: i32 = 1;

pub const CMD_TOGGLE: usize = 1001;
pub const CMD_SETTINGS: usize = 1002;
pub const CMD_COPY_LAST: usize = 1003;
pub const CMD_MODEL: usize = 1004;
pub const CMD_EXIT: usize = 1005;
pub const CMD_MODELS: usize = 1006;
pub const CMD_ENGINE_BASE: usize = 1010;
pub const CMD_MIC_DEFAULT: usize = 1100;
pub const CMD_MIC_BASE: usize = 1101;

struct Active {
    seq: u64,
    recorder: Recorder,
    started: Instant,
    stopping: bool,
    /// Started through the RegisterHotKey fallback (elevated foreground): release is detected by polling.
    poll_seen_down: bool,
}

struct Pending {
    released: Instant,
    audio_s: f32,
    engine_ms: f64,
    /// The requested engine was not available and the other one transcribed this dictation.
    substituted: bool,
}

pub struct App {
    pub hwnd: HWND,
    pub settings: Settings,
    pub settings_path: PathBuf,
    pub tray: Tray,
    engine: Option<EngineService>,
    pub engine_state: EngineState,
    pub whisper_state: WhisperState,
    /// (model id, use GPU, GPU only) of the last LoadWhisper sent, to avoid reloading the same model.
    whisper_requested: Option<(String, bool, bool)>,
    inserter: Option<Inserter>,
    active: Option<Active>,
    pending: BTreeMap<u64, Pending>,
    seq: u64,
    last_text: Option<String>,
    pub devices: Vec<Device>,
    pub default_device: Option<Device>,
    _notifications: Option<audio::DeviceNotifications>,
    pub fallback_registered: bool,
    pub hook_ok: bool,
    pub download_cancel: Option<Arc<AtomicBool>>,
    pub download_progress: Option<(u64, u64)>,
    pub download_error: Option<String>,
    /// The item of the running (or last failed) download.
    pub download_item: Option<DownloadItem>,
    pub models_ui: Option<crate::models_ui::ModelsUi>,
    pub mic_test: Option<Recorder>,
    pub settings_ui: Option<settings_ui::SettingsUi>,
    pub vocab_ui: Option<vocab_ui::VocabUi>,
    pub setup_ui: Option<setup_ui::SetupUi>,
    tray_prev_fg: HWND,
    pub mic_warning: Option<String>,
    /// Started by autostart: do not pop up windows by ourselves.
    pub background: bool,
}

thread_local! {
    static OVERLAY_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// Runs `f` with the app state. Returns None when the state is already borrowed (re-entrant message) or not
/// initialised yet.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

pub fn with_overlay(f: impl FnOnce(&mut Overlay)) {
    OVERLAY.with(|o| {
        if let Ok(mut g) = o.try_borrow_mut() {
            if let Some(ov) = g.as_mut() {
                f(ov);
            }
        }
    });
}

/// Schedules `f` to run on the UI thread after the current message, outside of any borrow.
pub fn defer(f: impl FnOnce() + 'static) {
    let b: Box<dyn FnOnce()> = Box::new(f);
    msg::post_boxed(WM_WT_EVENT, AppEvent::Deferred(b));
}

fn post_event(ev: AppEvent) {
    msg::post_boxed(WM_WT_EVENT, ev);
}

pub fn install(app: App, overlay: Option<Overlay>) {
    OVERLAY.with(|o| *o.borrow_mut() = overlay);
    APP.with(|a| *a.borrow_mut() = Some(app));
}

pub fn uninstall() {
    let app = APP.with(|a| a.borrow_mut().take());
    if let Some(app) = app {
        app.shutdown();
    }
    OVERLAY.with(|o| o.borrow_mut().take());
}

fn overlay_listening(limit: u32, src: Option<Box<dyn Fn() -> f32>>) {
    with_overlay(|o| {
        o.set_level_source(src);
        o.listening(limit);
    });
}

/// Shows a short message. With the overlay set to "Off", only warnings are shown.
fn overlay_message(text: &str, kind: MsgKind, secs: f32) {
    if kind != MsgKind::Warning && !OVERLAY_ENABLED.with(|e| e.get()) {
        with_overlay(|o| o.hide());
        return;
    }
    with_overlay(|o| {
        o.set_level_source(None);
        o.message(text, kind, secs);
    });
}

impl App {
    pub fn new(hwnd: HWND, settings: Settings, settings_path: PathBuf) -> App {
        App {
            hwnd,
            settings,
            settings_path,
            tray: Tray::new(hwnd),
            engine: None,
            engine_state: EngineState::Starting,
            whisper_state: WhisperState::Off,
            whisper_requested: None,
            inserter: None,
            active: None,
            pending: BTreeMap::new(),
            seq: 0,
            last_text: None,
            devices: Vec::new(),
            default_device: None,
            _notifications: None,
            fallback_registered: false,
            hook_ok: false,
            download_cancel: None,
            download_progress: None,
            download_error: None,
            download_item: None,
            models_ui: None,
            mic_test: None,
            settings_ui: None,
            vocab_ui: None,
            setup_ui: None,
            tray_prev_fg: HWND::default(),
            mic_warning: None,
            background: false,
        }
    }

    /// Everything that can fail is started here; failures are reported, never fatal.
    pub fn start(&mut self) {
        self.apply_overlay_settings();
        self.tray.add();
        match hotkey::start(self.settings.hotkey) {
            Ok(()) => self.hook_ok = true,
            Err(e) => log_error!("hotkey: keyboard hook failed: {e}"),
        }
        #[cfg(feature = "test-hooks")]
        if std::env::var_os("WHISTLETYPE_TEST_WAV").is_some() {
            log_warn!("test-hooks: audio comes from a WAV file, not the microphone");
        }
        self.register_fallback_hotkey();

        self.engine = Some(EngineService::start(
            crate::engine::default_dll_path(),
            Box::new(|ev| post_event(AppEvent::Engine(ev))),
        ));
        self.load_model_if_present();
        self.ensure_whisper();

        self.inserter = Some(Inserter::start(Arc::new(|r| post_event(AppEvent::Insert(r)))));

        match audio::DeviceNotifications::register() {
            Ok(n) => self._notifications = Some(n),
            Err(e) => log_warn!("audio: device notifications unavailable: {e}"),
        }
        self.refresh_devices();
        // keep the registry entry in sync with the setting (e.g. after the app was moved)
        if self.settings.start_with_windows && !autostart::points_here() {
            if let Err(e) = autostart::set(true) {
                log_warn!("autostart: {e}");
            }
        }
        self.update_tray();
    }

    /// Switches the interface language and rebuilds the open windows in it.
    pub fn set_ui_language(&mut self, code: &str) {
        let Some(lang) = crate::i18n::UiLang::from_code(code) else { return };
        self.settings.ui_language = code.to_string();
        crate::i18n::set(lang);
        self.save_settings();
        self.update_tray();
        let reopen_vocab = self.vocab_ui.is_some();
        let reopen_setup = self.setup_ui.is_some();
        let reopen_models = self.models_ui.is_some();
        for h in [
            self.settings_ui.as_ref().map(|s| s.hwnd()),
            self.vocab_ui.as_ref().map(|v| v.hwnd()),
            self.setup_ui.as_ref().map(|s| s.hwnd()),
            self.models_ui.as_ref().map(|m| m.hwnd()),
        ]
            .into_iter()
            .flatten()
        {
            unsafe {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        // posted after the WM_CLOSEs, so the old windows are gone when this runs
        defer(move || {
            with_app(|a| {
                a.open_settings();
                if reopen_vocab {
                    a.open_vocab();
                }
                if reopen_setup {
                    a.open_setup();
                }
                if reopen_models {
                    a.open_models();
                }
            });
        });
    }

    pub fn apply_overlay_settings(&mut self) {
        let (on, pos) = (self.settings.show_overlay, self.settings.overlay_position);
        OVERLAY_ENABLED.with(|e| e.set(on));
        with_overlay(|o| {
            o.set_position(pos);
            if !on {
                o.hide();
            }
        });
    }

    pub fn load_model_if_present(&mut self) {
        match model::file_state() {
            model::ModelFileState::Present => {
                self.engine_state = EngineState::Loading;
                if let Some(e) = &self.engine {
                    e.send(EngineCmd::LoadModel { path: model::model_path(), sha256: Some(model::MODEL_SHA256.to_string()) });
                }
            }
            model::ModelFileState::Missing => self.engine_state = EngineState::NoModel,
            model::ModelFileState::WrongSize(n) => {
                log_warn!("model: wrong size {n}");
                self.engine_state = EngineState::ModelError(t().err_model_incomplete.into());
            }
        }
        self.refresh_status_ui();
    }

    /// The Whisper model ACCURATE/AUTO should use now, or None when Whisper is not needed.
    /// AUTO only uses Whisper on an NVIDIA GPU (on a CPU, Whistle is far faster for dictation).
    pub fn wanted_whisper(&self) -> Option<&'static WhisperModel> {
        let mode = self.settings.engine_mode.as_str();
        if mode == "fast" {
            return None;
        }
        if mode == "auto" && !(self.settings.use_gpu && models::CudaPack::installed() && models::nvidia_gpu_cached().is_some()) {
            return None;
        }
        let installed = models::installed_whisper_models();
        if let Some(m) = models::whisper_model(&self.settings.whisper_model) {
            if installed.contains(&m) {
                return Some(m);
            }
        }
        installed.into_iter().max_by_key(|m| m.polish)
    }

    /// Loads, switches or frees the Whisper model so that it matches the settings.
    pub fn ensure_whisper(&mut self) {
        let Some(engine) = &self.engine else { return };
        match self.wanted_whisper() {
            None => {
                if self.whisper_requested.take().is_some() {
                    engine.send(EngineCmd::UnloadWhisper);
                }
                self.whisper_state = WhisperState::Off;
            }
            Some(m) => {
                let gpu_only = self.settings.engine_mode == "auto";
                let key = (m.id.to_string(), self.settings.use_gpu, gpu_only);
                if self.whisper_requested.as_ref() == Some(&key) && !matches!(self.whisper_state, WhisperState::Failed { .. }) {
                    return;
                }
                engine.send(EngineCmd::LoadWhisper(WhisperRequest {
                    model_id: m.id.to_string(),
                    model_path: models::whisper_model_path(m),
                    label: m.name.to_string(),
                    runtime_dirs: models::runtime_dirs(self.settings.use_gpu),
                    use_gpu: self.settings.use_gpu,
                    gpu_only,
                }));
                self.whisper_requested = Some(key);
                self.whisper_state = WhisperState::Loading { model_id: m.id.to_string() };
            }
        }
        self.refresh_status_ui();
        self.update_tray();
    }

    /// Engine for the next dictation. FAST: Whistle. ACCURATE: Whisper. AUTO: Whisper when it is ready on a GPU.
    fn engine_for_dictation(&self) -> EngineKind {
        match self.settings.engine_mode.as_str() {
            "fast" => EngineKind::Whistle,
            "accurate" => EngineKind::Whisper,
            _ => match &self.whisper_state {
                WhisperState::Ready(s) if s.device.is_gpu() => EngineKind::Whisper,
                _ => EngineKind::Whistle,
            },
        }
    }

    pub fn needs_model_setup(&self) -> bool {
        matches!(self.engine_state, EngineState::NoModel | EngineState::ModelError(_))
    }

    fn register_fallback_hotkey(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), FALLBACK_HOTKEY_ID);
        }
        let h = self.settings.hotkey;
        let mut mods = MOD_NOREPEAT;
        if h.ctrl {
            mods |= MOD_CONTROL;
        }
        if h.alt {
            mods |= MOD_ALT;
        }
        if h.shift {
            mods |= MOD_SHIFT;
        }
        if h.win {
            mods |= MOD_WIN;
        }
        let r = unsafe { RegisterHotKey(Some(self.hwnd), FALLBACK_HOTKEY_ID, HOT_KEY_MODIFIERS(mods.0), h.vk) };
        self.fallback_registered = r.is_ok();
        if let Err(e) = r {
            log_warn!("hotkey: {} is registered by another application ({}); the keyboard hook still handles it", hotkey::display(&h), e.message());
        }
    }

    pub fn apply_hotkey(&mut self, h: Hotkey) {
        self.settings.hotkey = h;
        hotkey::set_hotkey(h);
        self.register_fallback_hotkey();
        self.save_settings();
        self.update_tray();
    }

    pub fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            log_error!("settings: save failed: {e}");
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    pub fn update_tray(&mut self) {
        let hk = hotkey::display(&self.settings.hotkey);
        let (state, tip) = if self.active.is_some() {
            (TrayState::Recording, t().tip_listening.to_string())
        } else if !self.pending.is_empty() {
            (TrayState::Busy, t().tip_transcribing.to_string())
        } else {
            match &self.engine_state {
                EngineState::Ready { .. } => {
                    let tpl = if self.settings.mode == Mode::Hold { t().tip_ready_hold } else { t().tip_ready_press };
                    (TrayState::Idle, fmt(tpl, &[("key", &hk)]))
                }
                EngineState::Starting | EngineState::Loading => (TrayState::Busy, t().tip_loading.into()),
                EngineState::NoModel => (TrayState::Warning, t().tip_no_model.into()),
                EngineState::ModelError(_) => (TrayState::Warning, t().tip_model_problem.into()),
                EngineState::RuntimeError(_) => (TrayState::Warning, t().tip_engine_problem.into()),
            }
        };
        self.tray.set(state, &tip);
    }

    pub fn refresh_status_ui(&mut self) {
        if self.settings_ui.is_some() {
            settings_ui::refresh_status(self);
        }
        if self.setup_ui.is_some() {
            setup_ui::refresh(self);
        }
    }

    pub fn refresh_devices(&mut self) {
        match audio::list_devices() {
            Ok(d) => self.devices = d,
            Err(e) => {
                log_warn!("audio: listing devices failed: {e}");
                self.devices.clear();
            }
        }
        self.default_device = audio::default_device();
        if self.settings_ui.is_some() {
            settings_ui::refresh_devices(self);
        }
    }

    // --------------------------------------------------------------------------------------------
    // Hotkey → recording
    // --------------------------------------------------------------------------------------------

    pub fn on_hotkey(&mut self, kind: usize) {
        match kind {
            HK_DOWN => match self.settings.mode {
                Mode::Hold => {
                    if let Some(a) = &self.active {
                        if a.stopping {
                            // pressed again during the release tail: finish this one now, start a new one
                            self.finish_recording(false);
                            self.begin_recording(false);
                        }
                    } else {
                        self.begin_recording(false);
                    }
                }
                Mode::Toggle => {
                    if self.active.is_some() {
                        self.end_recording();
                    } else {
                        self.begin_recording(false);
                    }
                }
            },
            HK_UP => {
                if self.settings.mode == Mode::Hold && self.active.is_some() {
                    self.end_recording();
                }
            }
            HK_CANCEL
                if self.active.is_some() => {
                    self.finish_recording(true);
                }
            _ => {}
        }
    }

    /// WM_HOTKEY: only arrives when the keyboard hook did *not* swallow the key - either an elevated window
    /// has the focus (UIPI) or Windows removed our hook.
    pub fn on_fallback_hotkey(&mut self) {
        let fg = unsafe { GetForegroundWindow() };
        let elevated = !fg.0.is_null() && crate::insert::target_is_elevated(fg);
        if !elevated {
            log_warn!("hotkey: fallback fired while the hook should have handled it - reinstalling the hook");
            hotkey::reinstall();
        }
        match self.settings.mode {
            Mode::Toggle => {
                if self.active.is_some() {
                    self.end_recording();
                } else {
                    self.begin_recording(true);
                }
            }
            Mode::Hold => {
                if self.active.is_none() {
                    self.begin_recording(true);
                }
            }
        }
    }

    fn on_poll_timer(&mut self) {
        let vk = self.settings.hotkey.vk as i32;
        let down = unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
        let Some(a) = self.active.as_mut() else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
            }
            return;
        };
        if down {
            a.poll_seen_down = true;
        } else if a.poll_seen_down || a.started.elapsed().as_millis() > 400 {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
            }
            if self.settings.mode == Mode::Hold {
                self.end_recording();
            }
        }
    }

    pub fn toggle_from_ui(&mut self) {
        if self.active.is_some() {
            self.end_recording();
        } else {
            self.begin_recording(false);
        }
    }

    fn begin_recording(&mut self, poll_release: bool) {
        if self.active.is_some() {
            return;
        }
        // Whisper (ACCURATE/AUTO) can take the dictation even when the FAST model is missing or broken
        let whisper_usable = self.settings.engine_mode != "fast" && matches!(self.whisper_state, WhisperState::Ready(_) | WhisperState::Loading { .. });
        match &self.engine_state {
            _ if whisper_usable => {}
            EngineState::NoModel | EngineState::ModelError(_) => {
                overlay_message(t().model_missing_opening_setup, MsgKind::Warning, 3.0);
                defer(|| {
                    with_app(|a| a.open_setup());
                });
                return;
            }
            EngineState::RuntimeError(e) => {
                let e = e.clone();
                overlay_message(t().engine_unavailable_see_settings, MsgKind::Warning, 3.5);
                self.tray.notify("WhistleType", &e, true);
                return;
            }
            _ => {}
        }
        if let Some(t) = self.mic_test.take() {
            drop(t.stop());
            if self.settings_ui.is_some() {
                settings_ui::mic_test_stopped(self);
            }
        }
        let device = self.resolve_device();
        self.seq += 1;
        let seq = self.seq;
        let max_samples = self.settings.max_recording_secs as usize * audio::SAMPLE_RATE as usize;
        let notify: Arc<dyn Fn(AudioEvent) + Send + Sync> = Arc::new(move |ev| post_event(AppEvent::Audio { seq, ev }));
        match Recorder::start(device, Purpose::Dictation { max_samples }, notify) {
            Ok(rec) => {
                let src = rec.level_source();
                self.active = Some(Active { seq, recorder: rec, started: Instant::now(), stopping: false, poll_seen_down: false });
                hotkey::set_escape_cancel(true);
                if self.settings.show_overlay {
                    overlay_listening(self.settings.max_recording_secs, Some(src));
                }
                if poll_release {
                    unsafe { SetTimer(Some(self.hwnd), TIMER_POLL, 30, None) };
                }
                if let Some(w) = self.mic_warning.take() {
                    log_info!("audio: {w}");
                }
                log_info!("dictation #{seq}: recording");
            }
            Err(e) => {
                log_error!("audio: cannot start: {e}");
                self.audio_problem(&e);
            }
        }
        self.update_tray();
    }

    fn resolve_device(&mut self) -> Option<String> {
        let id = self.settings.microphone_id.clone();
        if id.is_empty() {
            return None;
        }
        if self.devices.iter().any(|d| d.id == id) {
            Some(id)
        } else {
            self.mic_warning = Some(format!("\"{}\" is not connected - using the default microphone", self.settings.microphone_name));
            if self.settings.show_overlay {
                // shown briefly before "Listening" replaces it; the tray tooltip/log keep the detail
            }
            None
        }
    }

    fn end_recording(&mut self) {
        let Some(a) = self.active.as_mut() else { return };
        if a.stopping {
            return;
        }
        a.stopping = true;
        unsafe { SetTimer(Some(self.hwnd), TIMER_TAIL, RELEASE_TAIL_MS, None) };
    }

    fn finish_recording(&mut self, cancel: bool) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TAIL);
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
        }
        hotkey::set_escape_cancel(false);
        let Some(a) = self.active.take() else { return };
        let seq = a.seq;
        let held_ms = a.started.elapsed().as_millis();
        let captured = a.recorder.stop();
        let released = Instant::now();
        with_overlay(|o| o.set_level_source(None));
        if cancel {
            log_info!("dictation #{seq}: cancelled");
            overlay_message(t().cancelled, MsgKind::Info, 1.0);
            self.update_tray();
            return;
        }
        if let Some(err) = &captured.error {
            if captured.samples.len() < audio::SAMPLE_RATE as usize / 4 {
                self.audio_problem(err);
                self.update_tray();
                return;
            }
            log_warn!("dictation #{seq}: capture ended with {err}; transcribing what was recorded");
        }
        let audio_s = captured.samples.len() as f32 / audio::SAMPLE_RATE as f32;
        let prepared = pipeline::prepare(captured.samples);
        let st = prepared.stats;
        log_info!(
            "dictation #{seq}: held {held_ms} ms, audio {audio_s:.2} s, peak {:.0} dBFS, floor {:.0} dBFS, voiced {:.2} s -> {:?}",
            st.peak_dbfs, st.noise_floor_dbfs, st.voiced_s, prepared.verdict
        );
        match prepared.verdict {
            Verdict::TooShort => {
                let tpl = if self.settings.mode == Mode::Hold { t().too_short_hold } else { t().too_short_press };
                overlay_message(&fmt(tpl, &[("key", &hotkey::display(&self.settings.hotkey))]), MsgKind::Info, 1.8);
            }
            Verdict::Silence => overlay_message(t().no_sound, MsgKind::Warning, 2.2),
            Verdict::NoSpeech => overlay_message(t().no_speech, MsgKind::Info, 1.6),
            Verdict::Speech => {
                let engine = self.engine_for_dictation();
                let job = TranscribeJob {
                    id: seq,
                    audio: prepared.audio,
                    engine,
                    language: self.settings.language_plan(),
                    vocabulary: self.settings.engine_vocabulary(),
                };
                if self.engine.as_ref().is_some_and(|e| e.send(EngineCmd::Transcribe(job))) {
                    self.pending.insert(seq, Pending { released, audio_s, engine_ms: 0.0, substituted: false });
                    if self.settings.show_overlay && self.active.is_none() {
                        let detail = self.engine_detail(engine);
                        with_overlay(|o| {
                            o.set_detail(detail);
                            o.transcribing(None)
                        });
                    }
                } else {
                    overlay_message(t().engine_not_running, MsgKind::Warning, 2.5);
                }
            }
        }
        self.update_tray();
    }

    /// Short text for the overlay, e.g. "Whisper · GPU" (None for the default FAST engine).
    fn engine_detail(&self, engine: EngineKind) -> Option<String> {
        match (engine, &self.whisper_state) {
            (EngineKind::Whisper, WhisperState::Ready(s)) => Some(format!("Whisper · {}", s.device.short())),
            (EngineKind::Whisper, _) => Some("Whisper".into()),
            (EngineKind::Whistle, _) if self.settings.engine_mode != "fast" => Some("Whistle".into()),
            _ => None,
        }
    }

    fn audio_problem(&mut self, e: &AudioError) {
        let text = e.to_string();
        overlay_message(&text, MsgKind::Warning, 3.5);
        if *e == AudioError::AccessDenied {
            self.tray.notify(t().mic_blocked_title, t().mic_blocked_text, true);
        }
    }

    // --------------------------------------------------------------------------------------------
    // Events from worker threads
    // --------------------------------------------------------------------------------------------

    pub fn on_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Engine(e) => self.on_engine(e),
            AppEvent::Audio { seq, ev } => self.on_audio(seq, ev),
            AppEvent::Insert(r) => self.on_insert(r),
            AppEvent::DownloadProgress { done, total } => {
                self.download_progress = Some((done, total));
                if self.setup_ui.is_some() {
                    setup_ui::refresh(self);
                }
                if self.models_ui.is_some() {
                    crate::models_ui::refresh(self);
                }
            }
            AppEvent::DownloadDone(r) => self.on_download_done(r),
            AppEvent::Deferred(_) => unreachable!("handled in the window procedure"),
        }
    }

    fn on_engine(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::RuntimeFailed(e) => {
                self.engine_state = EngineState::RuntimeError(e.to_string());
                self.tray.notify(t().engine_start_failed_title, &e.to_string(), true);
            }
            EngineEvent::ModelLoaded { load_ms, warmup_ms } => {
                self.engine_state = EngineState::Ready { load_ms };
                log_info!("model ready (load {load_ms:.0} ms, warm-up {warmup_ms:.0} ms)");
            }
            EngineEvent::ModelFailed(e) => {
                self.engine_state = match &e {
                    EngineError::ModelMissing(_) => EngineState::NoModel,
                    EngineError::RuntimeMissing(_) | EngineError::RuntimeCorrupt(_) | EngineError::RuntimeLoad(_) => EngineState::RuntimeError(e.to_string()),
                    _ => EngineState::ModelError(e.to_string()),
                };
                if matches!(self.engine_state, EngineState::ModelError(_) | EngineState::NoModel) {
                    if self.background {
                        self.tray.notify("WhistleType", &e.to_string(), true);
                    } else {
                        defer(|| {
                            with_app(|a| a.open_setup());
                        });
                    }
                }
            }
            EngineEvent::WhisperLoaded(s) => {
                log_info!("whisper ready: {} on {:?} (load {:.0} ms)", s.label, s.device, s.load_ms);
                if s.restart_for_gpu {
                    self.tray.notify("WhistleType", t().restart_for_gpu, false);
                }
                // asked for the GPU, got the CPU although a GPU exists: tell the user once
                if self.settings.use_gpu && !s.device.is_gpu() && !s.gpus.is_empty() {
                    self.tray.notify("WhistleType", t().gpu_fallback_cpu, true);
                }
                self.whisper_state = WhisperState::Ready(s);
            }
            EngineEvent::WhisperFailed { model_id, error } => {
                log_warn!("whisper: {model_id} failed: {error}");
                if self.settings.engine_mode == "accurate" {
                    self.tray.notify("WhistleType", &fmt(t().accurate_unavailable, &[("e", &error)]), true);
                }
                self.whisper_state = WhisperState::Failed { model_id, error: error.to_string() };
            }
            EngineEvent::WhisperUnloaded => self.whisper_state = WhisperState::Off,
            EngineEvent::Busy { id, segment, of } => {
                if self.active.is_none() && self.settings.show_overlay && self.pending.contains_key(&id) {
                    with_overlay(|o| o.transcribing(Some((segment, of))));
                }
            }
            EngineEvent::Done(r) => self.on_transcribed(r),
        }
        self.update_tray();
        self.refresh_status_ui();
    }

    fn on_transcribed(&mut self, r: JobResult) {
        let Some(p) = self.pending.get_mut(&r.id) else { return };
        p.engine_ms = r.engine_ms;
        p.substituted = r.substituted;
        let latency = p.released.elapsed().as_secs_f64() * 1000.0;
        match r.parts {
            Err(e) => {
                self.pending.remove(&r.id);
                log_error!("dictation #{}: {e}", r.id);
                if self.active.is_none() {
                    overlay_message(&e.to_string(), MsgKind::Warning, 3.5);
                }
            }
            Ok(parts) => {
                let fin = pipeline::finish(&parts, self.settings.raw_transcription, self.settings.append_space);
                log_info!(
                    "dictation #{}: {} on {} ({}{}), {} segment(s), {:.2} s audio, engine {:.0} ms, release->text {:.0} ms, lang '{}', {} chars",
                    r.id,
                    r.label,
                    r.device.short(),
                    match r.engine { EngineKind::Whistle => "fast", EngineKind::Whisper => "accurate" },
                    if r.substituted { ", substituted" } else { "" },
                    parts.len(),
                    r.audio_s,
                    r.engine_ms,
                    latency,
                    fin.language,
                    fin.text.chars().count()
                );
                if self.settings.log_transcripts {
                    log_info!("dictation #{}: text: {}", r.id, fin.text);
                }
                if fin.text.is_empty() {
                    self.pending.remove(&r.id);
                    if self.active.is_none() {
                        overlay_message(t().no_speech_recognized, MsgKind::Info, 1.6);
                    }
                    return;
                }
                self.last_text = Some(fin.text.clone());
                if let Some(ins) = &self.inserter {
                    ins.submit(InsertJob {
                        id: r.id,
                        text: fin.text,
                        method: self.settings.insert_method,
                        auto_paste: self.settings.auto_paste,
                        restore_clipboard: self.settings.restore_clipboard,
                    });
                }
            }
        }
    }

    fn on_insert(&mut self, r: InsertResult) {
        let p = self.pending.remove(&r.id);
        if let Some(p) = &p {
            log_info!(
                "dictation #{}: inserted {:?}; release->inserted {:.0} ms (engine {:.0} ms, insert {:.0} ms, audio {:.2} s)",
                r.id, r.outcome, p.released.elapsed().as_secs_f64() * 1000.0, p.engine_ms, r.elapsed_ms, p.audio_s
            );
        }
        if self.active.is_some() {
            self.update_tray();
            return; // the overlay is showing "Listening" for the next dictation
        }
        let paste = hotkey::display(&Hotkey { vk: 0x56, ctrl: true, ..Default::default() });
        match r.outcome {
            InsertOutcome::Pasted { .. } | InsertOutcome::Typed => {
                if p.as_ref().is_some_and(|p| p.substituted) && self.settings.engine_mode == "accurate" {
                    overlay_message(t().used_fast_instead, MsgKind::Info, 2.5);
                } else if self.pending.is_empty() {
                    with_overlay(|o| o.hide());
                }
            }
            InsertOutcome::NotPasted => {
                overlay_message(t().not_pasted, MsgKind::Warning, 4.0)
            }
            InsertOutcome::Copied(CopyReason::AutoPasteOff) => overlay_message(t().copied, MsgKind::Success, 1.5),
            InsertOutcome::Copied(CopyReason::NoTarget) => overlay_message(t().copied, MsgKind::Success, 1.8),
            InsertOutcome::Copied(CopyReason::ElevatedTarget) => {
                overlay_message(&fmt(t().admin_window, &[("key", &paste)]), MsgKind::Warning, 4.0)
            }
            InsertOutcome::Failed(e) => overlay_message(&fmt(t().insert_failed, &[("e", &e)]), MsgKind::Warning, 3.5),
        }
        self.update_tray();
    }

    fn on_audio(&mut self, seq: u64, ev: AudioEvent) {
        let current = self.active.as_ref().map(|a| a.seq) == Some(seq);
        match ev {
            AudioEvent::Started { .. } => {}
            AudioEvent::LimitReached if current => {
                log_info!("dictation #{seq}: reached the {} s limit", self.settings.max_recording_secs);
                self.finish_recording(false);
            }
            AudioEvent::Failed(e) if current => {
                log_warn!("dictation #{seq}: {e}");
                // finish with whatever was captured (the error is reported there if nothing was). The key
                // stays "held" in the hook so auto-repeat does not start a new (failing) recording.
                self.finish_recording(false);
                if e == AudioError::Disconnected {
                    overlay_message(t().mic_disconnected, MsgKind::Warning, 3.0);
                }
            }
            _ => {}
        }
    }

    pub fn on_devices_changed(&mut self, default_changed: bool) {
        self.refresh_devices();
        if default_changed {
            log_info!("audio: default input is now {:?}", self.default_device.as_ref().map(|d| &d.name));
        }
    }

    // --------------------------------------------------------------------------------------------
    // Model download / import
    // --------------------------------------------------------------------------------------------

    /// Downloads the FAST model (first-run window).
    pub fn start_download(&mut self) {
        self.start_item_download(DownloadItem::Whistle);
    }

    /// One download at a time (Whistle, a Whisper model or the CUDA pack): pinned URL, resume, SHA-256.
    pub fn start_item_download(&mut self, item: DownloadItem) {
        if self.download_cancel.is_some() {
            return;
        }
        let (host, path, dest, size, sha): (&'static str, String, std::path::PathBuf, u64, &'static str) = match item {
            DownloadItem::Whistle => (model::MODEL_HOST, model::model_url_path(), model::model_path(), model::MODEL_SIZE, model::MODEL_SHA256),
            DownloadItem::Whisper(id) => {
                let Some(m) = models::whisper_model(id) else { return };
                ("huggingface.co", models::whisper_url_path(m), models::whisper_model_path(m), m.size, m.sha256)
            }
            DownloadItem::CudaPack => (
                models::CudaPack::HOST,
                models::CudaPack::PATH.to_string(),
                models::CudaPack::dir().with_file_name("whisper-cuda-12.4-b5130.zip"),
                models::CudaPack::SIZE,
                models::CudaPack::SHA256,
            ),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel = Some(cancel.clone());
        self.download_progress = Some((0, size));
        self.download_error = None;
        self.download_item = Some(item);
        log_info!("download: https://{host}{path} (user confirmed)");
        std::thread::Builder::new()
            .name("download".into())
            .spawn(move || {
                let mut last_pct = u64::MAX;
                let mut progress = |done: u64, total: u64| {
                    let pct = done * 100 / total.max(1);
                    if pct != last_pct {
                        last_pct = pct;
                        post_event(AppEvent::DownloadProgress { done, total });
                    }
                };
                let mut r = crate::download::download(host, &path, &dest, size, sha, &mut progress, &cancel);
                if r.is_ok() && item == DownloadItem::CudaPack {
                    // unpack + verify every DLL, then drop the archive
                    r = models::CudaPack::install_from_zip(&dest).map_err(DownloadError::Integrity);
                    let _ = std::fs::remove_file(&dest);
                }
                post_event(AppEvent::DownloadDone(r));
            })
            .ok();
        self.refresh_status_ui();
    }

    pub fn cancel_download(&mut self) {
        if let Some(c) = &self.download_cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    fn on_download_done(&mut self, r: Result<(), DownloadError>) {
        self.download_cancel = None;
        let item = self.download_item;
        let ok = r.is_ok();
        match r {
            Ok(()) => {
                self.download_progress = None;
                match item {
                    Some(DownloadItem::Whistle) | None => self.load_model_if_present(),
                    Some(DownloadItem::Whisper(id)) => {
                        if models::whisper_model(&self.settings.whisper_model).map_or(true, |m| !models::whisper_installed(m)) {
                            self.settings.whisper_model = id.to_string();
                            self.save_settings();
                        }
                        self.ensure_whisper();
                    }
                    Some(DownloadItem::CudaPack) => {
                        // the CPU runtime is already in this process: the GPU needs a restart
                        let cpu_loaded = matches!(&self.whisper_state, WhisperState::Ready(s) if s.runtime_dir == models::cpu_runtime_dir());
                        if cpu_loaded {
                            self.tray.notify("WhistleType", t().restart_for_gpu, false);
                        } else {
                            self.whisper_requested = None;
                            self.ensure_whisper();
                        }
                    }
                }
            }
            Err(e) => {
                log_warn!("download: {e}");
                self.download_error = Some(e.to_string());
            }
        }
        self.refresh_status_ui();
        if let (true, Some(item)) = (self.models_ui.is_some(), item) {
            crate::models_ui::on_download_done(self, item, ok);
        }
        self.update_tray();
    }

    /// Deletes a Whisper model or the CUDA pack (the Model Manager's Delete button).
    pub fn delete_item(&mut self, item: DownloadItem) -> Result<(), String> {
        let r = match item {
            DownloadItem::Whistle => Err("the FAST model cannot be removed".into()),
            DownloadItem::Whisper(id) => {
                let Some(m) = models::whisper_model(id) else { return Ok(()) };
                if matches!(&self.whisper_state, WhisperState::Ready(s) if s.model_id == id) || self.whisper_requested.as_ref().is_some_and(|r| r.0 == id) {
                    if let Some(e) = &self.engine {
                        e.send(EngineCmd::UnloadWhisper);
                    }
                    self.whisper_requested = None;
                    self.whisper_state = WhisperState::Off;
                    // whisper.cpp reads the file into memory and closes it, so it can be deleted right away
                }
                models::delete_whisper(m)
            }
            DownloadItem::CudaPack => {
                let loaded = matches!(&self.whisper_state, WhisperState::Ready(s) if s.runtime_dir == models::CudaPack::dir());
                if loaded {
                    models::CudaPack::schedule_removal().map(|_| ())?;
                    Err(t().restart_to_remove.to_string())
                } else {
                    models::CudaPack::remove()
                }
            }
        };
        self.ensure_whisper();
        if self.models_ui.is_some() {
            crate::models_ui::refresh(self);
        }
        r
    }

    /// A key moved the focus inside one of our forms: keep the focused control visible in a scrolled form.
    pub fn focus_moved(&mut self) {
        let focus = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
        if let Some(s) = self.settings_ui.as_mut() {
            s.scroll_into_view(focus);
        }
    }

    pub fn open_models(&mut self) {
        if let Some(m) = &self.models_ui {
            ui::bring_to_front(m.hwnd());
            return;
        }
        self.models_ui = crate::models_ui::ModelsUi::create(self);
        crate::models_ui::refresh(self);
    }

    pub fn import_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        model::import_from(path)?;
        log_info!("model: imported from a local file");
        self.load_model_if_present();
        self.update_tray();
        Ok(())
    }

    // --------------------------------------------------------------------------------------------
    // Windows
    // --------------------------------------------------------------------------------------------

    pub fn open_settings(&mut self) {
        if let Some(s) = &self.settings_ui {
            ui::bring_to_front(s.hwnd());
            return;
        }
        self.settings_ui = settings_ui::SettingsUi::create(self);
        settings_ui::refresh_devices(self);
        settings_ui::refresh_status(self);
    }

    pub fn open_vocab(&mut self) {
        if let Some(v) = &self.vocab_ui {
            ui::bring_to_front(v.hwnd());
            return;
        }
        self.vocab_ui = vocab_ui::VocabUi::create(self);
    }

    pub fn open_setup(&mut self) {
        if let Some(s) = &self.setup_ui {
            ui::bring_to_front(s.hwnd());
            return;
        }
        self.setup_ui = setup_ui::SetupUi::create(self);
    }

    pub fn copy_last(&mut self) {
        if let Some(t) = self.last_text.clone() {
            if let Some(ins) = &self.inserter {
                ins.submit(InsertJob { id: 0, text: t, method: self.settings.insert_method, auto_paste: false, restore_clipboard: false });
            }
        }
    }

    /// Builds the tray menu. The modal TrackPopupMenu runs outside the App borrow (see `handle_main`).
    pub fn build_tray_menu(&mut self) -> Option<HMENU> {
        unsafe {
            self.tray_prev_fg = GetForegroundWindow();
            let menu = CreatePopupMenu().ok()?;
            let rec = if self.active.is_some() { t().menu_stop } else { t().menu_start };
            let whisper_usable = self.settings.engine_mode != "fast" && matches!(self.whisper_state, WhisperState::Ready(_) | WhisperState::Loading { .. });
            let ready = whisper_usable || matches!(self.engine_state, EngineState::Ready { .. } | EngineState::Loading);
            let flag = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };
            let _ = AppendMenuW(menu, flag(ready), CMD_TOGGLE, crate::util::WStr::new(rec).pcwstr());
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            if let Ok(mics) = CreatePopupMenu() {
                let def_name = self.default_device.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| t().menu_none.into());
                let checked = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(mics, checked(self.settings.microphone_id.is_empty()), CMD_MIC_DEFAULT, crate::util::WStr::new(&fmt(t().menu_system_default, &[("name", &def_name)])).pcwstr());
                for (i, d) in self.devices.iter().enumerate().take(50) {
                    let _ = AppendMenuW(mics, checked(d.id == self.settings.microphone_id), CMD_MIC_BASE + i, crate::util::WStr::new(&d.name).pcwstr());
                }
                let _ = AppendMenuW(menu, MF_POPUP, mics.0 as usize, util::WStr::new(t().menu_mic).pcwstr());
            }
            let status = match &self.engine_state {
                EngineState::Ready { .. } => fmt(t().menu_model_ready, &[("name", &format!("{} {}", model::MODEL_NAME, model::MODEL_VERSION))]),
                EngineState::Loading | EngineState::Starting => t().menu_model_loading.into(),
                EngineState::NoModel => t().menu_model_missing.into(),
                EngineState::ModelError(_) => t().menu_model_problem.into(),
                EngineState::RuntimeError(_) => t().menu_engine_na.into(),
            };
            let _ = AppendMenuW(menu, flag(self.needs_model_setup()), CMD_MODEL, crate::util::WStr::new(&status).pcwstr());
            if let Ok(eng) = CreatePopupMenu() {
                let labels = [t().engine_auto, t().engine_fast, t().engine_accurate];
                for (i, (mode, label)) in crate::settings::ENGINE_MODES.iter().zip(labels).enumerate() {
                    let f = if *mode == self.settings.engine_mode { MF_STRING | MF_CHECKED } else { MF_STRING };
                    let _ = AppendMenuW(eng, f, CMD_ENGINE_BASE + i, util::WStr::new(label).pcwstr());
                }
                let _ = AppendMenuW(eng, MF_SEPARATOR, 0, None);
                let _ = AppendMenuW(eng, MF_STRING, CMD_MODELS, util::WStr::new(t().menu_models).pcwstr());
                let _ = AppendMenuW(menu, MF_POPUP, eng.0 as usize, util::WStr::new(t().menu_engine).pcwstr());
            }
            let _ = AppendMenuW(menu, flag(self.last_text.is_some()), CMD_COPY_LAST, util::WStr::new(t().menu_copy_last).pcwstr());
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, util::WStr::new(t().menu_settings).pcwstr());
            let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, util::WStr::new(t().menu_exit).pcwstr());
            let _ = SetMenuDefaultItem(menu, CMD_SETTINGS as u32, 0);
            Some(menu)
        }
    }

    pub fn on_command(&mut self, id: usize) {
        match id {
            CMD_TOGGLE => {
                // give the focus back to the app the user was in before opening the menu
                if !self.tray_prev_fg.0.is_null() {
                    unsafe {
                        let _ = SetForegroundWindow(self.tray_prev_fg);
                    }
                }
                self.toggle_from_ui();
            }
            CMD_SETTINGS => self.open_settings(),
            CMD_COPY_LAST => self.copy_last(),
            CMD_MODEL => self.open_setup(),
            CMD_MODELS => self.open_models(),
            id if (CMD_ENGINE_BASE..CMD_ENGINE_BASE + crate::settings::ENGINE_MODES.len()).contains(&id) => {
                self.settings.engine_mode = crate::settings::ENGINE_MODES[id - CMD_ENGINE_BASE].to_string();
                self.save_settings();
                self.ensure_whisper();
                if self.settings_ui.is_some() {
                    settings_ui::refresh_engine(self);
                }
            }
            CMD_EXIT => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            CMD_MIC_DEFAULT => self.select_device(None),
            id if (CMD_MIC_BASE..CMD_MIC_BASE + 50).contains(&id) => {
                let i = id - CMD_MIC_BASE;
                if let Some(d) = self.devices.get(i).cloned() {
                    self.select_device(Some(d));
                }
            }
            _ => {}
        }
    }

    pub fn select_device(&mut self, d: Option<Device>) {
        match d {
            Some(d) => {
                self.settings.microphone_id = d.id;
                self.settings.microphone_name = d.name;
            }
            None => {
                self.settings.microphone_id.clear();
                self.settings.microphone_name.clear();
            }
        }
        self.save_settings();
        if self.settings_ui.is_some() {
            settings_ui::refresh_devices(self);
        }
    }

    pub fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_TAIL => self.finish_recording(false),
            TIMER_POLL => self.on_poll_timer(),
            _ => {}
        }
    }

    fn shutdown(mut self) {
        log_info!("shutting down");
        hotkey::set_capture(false);
        hotkey::stop();
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), FALLBACK_HOTKEY_ID);
        }
        if let Some(a) = self.active.take() {
            drop(a.recorder.stop());
        }
        if let Some(t) = self.mic_test.take() {
            drop(t.stop());
        }
        self.cancel_download();
        if let Some(i) = self.inserter.take() {
            i.shutdown(); // restores the clipboard if a paste is in flight
        }
        if let Some(e) = self.engine.take() {
            e.shutdown(std::time::Duration::from_secs(2));
        }
        self.save_settings();
        self.tray.remove();
    }
}

// ------------------------------------------------------------------------------------------------
// Main window procedure
// ------------------------------------------------------------------------------------------------

static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn register_taskbar_created() {
    let m = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    TASKBAR_CREATED.store(m, Ordering::SeqCst);
}

/// Window procedure of the hidden main window.
///
/// # Safety
/// Called by Windows with the arguments of a window message for the window created in `main`.
pub unsafe extern "system" fn main_wndproc(hwnd: HWND, m: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_main(hwnd, m, wparam, lparam)));
    match r {
        Ok(Some(res)) => res,
        Ok(None) => unsafe { DefWindowProcW(hwnd, m, wparam, lparam) },
        Err(_) => {
            log_error!("panic while handling message 0x{m:04X} - continuing");
            LRESULT(0)
        }
    }
}

fn handle_main(hwnd: HWND, m: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let tb = TASKBAR_CREATED.load(Ordering::Relaxed);
    if tb != 0 && m == tb {
        with_app(|a| a.tray.add());
        return Some(LRESULT(0));
    }
    match m {
        msg::WM_WT_HOTKEY => {
            if with_app(|a| a.on_hotkey(wparam.0)).is_none() {
                // state busy (re-entrant): retry after the current message
                unsafe {
                    let _ = PostMessageW(Some(hwnd), m, wparam, lparam);
                }
            }
            Some(LRESULT(0))
        }
        WM_HOTKEY if wparam.0 as i32 == FALLBACK_HOTKEY_ID => {
            with_app(|a| a.on_fallback_hotkey());
            Some(LRESULT(0))
        }
        msg::WM_WT_CAPTURE => {
            with_app(|a| settings_ui::on_capture(a, wparam.0 as u64));
            Some(LRESULT(0))
        }
        msg::WM_WT_EVENT => {
            let ev = unsafe { msg::take_boxed::<AppEvent>(lparam) };
            match *ev {
                AppEvent::Deferred(f) => f(),
                other => {
                    // must never be lost: if the state is borrowed, re-post it
                    let mut slot = Some(other);
                    let done = with_app(|a| a.on_event(slot.take().unwrap()));
                    if done.is_none() {
                        if let Some(ev) = slot.take() {
                            msg::post_boxed(WM_WT_EVENT, ev);
                        }
                    }
                }
            }
            Some(LRESULT(0))
        }
        msg::WM_WT_TRAY => {
            let event = (lparam.0 as u32) & 0xFFFF;
            match event {
                WM_CONTEXTMENU | WM_RBUTTONUP => {
                    if let Some(menu) = with_app(|a| a.build_tray_menu()).flatten() {
                        unsafe {
                            let mut pt = POINT::default();
                            let _ = GetCursorPos(&mut pt);
                            let _ = SetForegroundWindow(hwnd);
                            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
                            let _ = DestroyMenu(menu);
                            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                            if cmd.0 != 0 {
                                with_app(|a| a.on_command(cmd.0 as usize));
                            }
                        }
                    }
                }
                NIN_SELECT | NIN_KEYSELECT | WM_LBUTTONUP => {
                    with_app(|a| {
                        if a.needs_model_setup() {
                            a.open_setup()
                        } else {
                            a.open_settings()
                        }
                    });
                }
                _ => {}
            }
            Some(LRESULT(0))
        }
        msg::WM_WT_ACTIVATE => {
            with_app(|a| {
                if a.needs_model_setup() {
                    a.open_setup()
                } else {
                    a.open_settings()
                }
            });
            Some(LRESULT(0))
        }
        msg::WM_WT_DEVICES => {
            with_app(|a| a.on_devices_changed(wparam.0 == 1));
            Some(LRESULT(0))
        }
        WM_TIMER => {
            with_app(|a| a.on_timer(wparam.0));
            Some(LRESULT(0))
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                log_info!("power: resumed - reinstalling the keyboard hook");
                hotkey::reinstall();
            }
            Some(LRESULT(1))
        }
        WM_QUERYENDSESSION => Some(LRESULT(1)),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                with_app(|a| a.save_settings());
                crate::log::flush();
            }
            Some(LRESULT(0))
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }
        _ => None,
    }
}

const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;

