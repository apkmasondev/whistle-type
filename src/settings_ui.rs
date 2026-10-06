//! Settings window: small, native, one column. Every change is applied and saved immediately.

use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows::Win32::System::SystemServices::SS_OWNERDRAW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{with_app, App, DownloadItem, EngineState, WhisperState};
use crate::audio::{Purpose, Recorder};
use crate::i18n::{fmt, t, UI_LANGUAGES};
use crate::models::{self, WHISPER_MODELS};
use crate::settings::{InsertMethod, Mode, OverlayPosition, ENGINE_MODES, LANGUAGES};
use crate::ui::{self, Font, Form};
use crate::{autostart, hotkey, model, util};

const ID_MIC: i32 = 101;
const ID_TEST: i32 = 102;
const ID_METER: i32 = 103;
const ID_LANG: i32 = 104;
const ID_HOTKEY: i32 = 105;
const ID_HOTKEY_HINT: i32 = 106;
const ID_MODE: i32 = 107;
const ID_AUTOPASTE: i32 = 108;
const ID_METHOD: i32 = 109;
const ID_RESTORE: i32 = 110;
const ID_RAW: i32 = 111;
const ID_SPACE: i32 = 112;
const ID_VOCAB: i32 = 113;
const ID_VOCAB_INFO: i32 = 114;
const ID_AUTOSTART: i32 = 115;
const ID_OVERLAY: i32 = 116;
const ID_MODEL_STATUS: i32 = 117;
const ID_MODEL_BTN: i32 = 118;
const ID_OFFLINE: i32 = 119;
const ID_LOGS: i32 = 120;
const ID_CLOSE: i32 = 121;
const ID_SUBTITLE: i32 = 122;
const ID_TEST_HINT: i32 = 123;
const ID_SECTION: i32 = 130;
const ID_LABEL: i32 = 140;
const ID_UILANG: i32 = 124;
const ID_ENGINE: i32 = 125;
const ID_ACC_MODEL: i32 = 126;
const ID_MODELS: i32 = 127;
const ID_GPU: i32 = 128;
const ID_ACC_STATUS: i32 = 129;

const MUTED: &[i32] = &[ID_SUBTITLE, ID_VOCAB_INFO, ID_OFFLINE, ID_TEST_HINT, ID_SECTION, ID_ACC_STATUS];
const TIMER_METER: usize = 1;

static METER: AtomicU32 = AtomicU32::new(0);

const METHODS: &[InsertMethod] = &[InsertMethod::CtrlV, InsertMethod::ShiftInsert, InsertMethod::CtrlShiftV, InsertMethod::Type];

fn method_label(m: InsertMethod) -> &'static str {
    match m {
        InsertMethod::CtrlV => t().method_ctrlv,
        InsertMethod::ShiftInsert => t().method_shiftins,
        InsertMethod::CtrlShiftV => t().method_ctrlshiftv,
        InsertMethod::Type => t().method_type,
    }
}

fn engine_labels() -> [&'static str; 3] {
    // same order as settings::ENGINE_MODES
    [t().engine_auto, t().engine_fast, t().engine_accurate]
}

/// Speech-language names; the two automatic modes are localised, languages are named in themselves.
fn language_names() -> Vec<String> {
    LANGUAGES
        .iter()
        .map(|(code, name)| match *code {
            "auto_pl_en" => t().lang_auto_pl_en.to_string(),
            "auto" => t().lang_auto.to_string(),
            _ => name.to_string(),
        })
        .collect()
}

pub struct SettingsUi {
    form: Form,
    mic: HWND,
    test: HWND,
    meter: HWND,
    test_hint: HWND,
    lang: HWND,
    engine: HWND,
    acc_model: HWND,
    gpu: HWND,
    acc_status: HWND,
    ui_lang: HWND,
    hotkey_btn: HWND,
    hotkey_hint: HWND,
    mode: HWND,
    autopaste: HWND,
    method: HWND,
    restore: HWND,
    raw: HWND,
    space: HWND,
    vocab_info: HWND,
    autostart: HWND,
    overlay: HWND,
    model_status: HWND,
    model_btn: HWND,
    offline: HWND,
    /// device ids in combo order (index 0 = system default)
    mic_ids: Vec<String>,
    hint_warn: bool,
}

impl SettingsUi {
    pub fn hwnd(&self) -> HWND {
        self.form.hwnd
    }

    pub fn create(app: &mut App) -> Option<SettingsUi> {
        ui::register_class(w!("WhistleType.Settings"), Some(wndproc));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = ui::create_centered(w!("WhistleType.Settings"), "WhistleType", 540.0, 716.0, style, WINDOW_EX_STYLE(0))
            .map_err(|e| crate::log_error!("ui: cannot create the settings window: {}", e.message()))
            .ok()?;
        let mut f = Form::new(hwnd);
        let tr = t();
        let lx = 24.0;
        let cx = 214.0;
        let cw = 340.0;
        let lw = 182.0;
        let width = cx + cw + 24.0;
        let mut y = 16.0;
        f.label("WhistleType", (lx, y, 400.0, 34.0), Font::Title, ID_LABEL);
        y += 34.0;
        f.label(
            &fmt(tr.subtitle, &[("ver", &env!("CARGO_PKG_VERSION"))]),
            (lx, y, width - 2.0 * lx, 20.0),
            Font::Small,
            ID_SUBTITLE,
        );
        y += 30.0;

        let row = |f: &mut Form, text: &str, y: f32| {
            f.label(text, (lx, y + 4.0, lw, 22.0), Font::Normal, ID_LABEL);
        };
        let section = |f: &mut Form, text: &str, y: f32| {
            f.label(text, (lx, y, 300.0, 20.0), Font::Bold, ID_SECTION + 1);
        };

        section(&mut f, tr.sec_input, y);
        y += 26.0;
        row(&mut f, tr.lbl_microphone, y);
        let mic = f.combo(ID_MIC, (cx, y, cw, 26.0), &[]);
        y += 32.0;
        let meter = f.add(w!("STATIC"), "", WINDOW_STYLE(SS_OWNERDRAW.0), WINDOW_EX_STYLE(0), ID_METER, (cx, y + 8.0, cw - 98.0, 10.0), Font::Normal);
        let test = f.button(tr.btn_test, ID_TEST, (cx + cw - 88.0, y, 88.0, 26.0), false);
        y += 28.0;
        let test_hint = f.label("", (cx, y, cw, 18.0), Font::Small, ID_TEST_HINT);
        y += 20.0;
        row(&mut f, tr.lbl_shortcut, y);
        let hotkey_btn = f.button("", ID_HOTKEY, (cx, y, 150.0, 26.0), false);
        let hotkey_hint = f.label(tr.hint_shortcut, (cx + 160.0, y - 2.0, cw - 160.0, 34.0), Font::Small, ID_HOTKEY_HINT);
        y += 36.0;
        row(&mut f, tr.lbl_mode, y);
        let mode = f.combo(ID_MODE, (cx, y, cw, 26.0), &[tr.mode_hold, tr.mode_toggle]);
        y += 40.0;

        section(&mut f, tr.sec_recognition, y);
        y += 26.0;
        row(&mut f, tr.lbl_language, y);
        let names = language_names();
        let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let lang = f.combo(ID_LANG, (cx, y, cw, 26.0), &names);
        y += 32.0;
        row(&mut f, tr.lbl_engine, y);
        let engine = f.combo(ID_ENGINE, (cx, y, cw, 26.0), &engine_labels());
        y += 32.0;
        row(&mut f, tr.lbl_accurate_model, y);
        let acc_model = f.combo(ID_ACC_MODEL, (cx, y, cw - 110.0, 26.0), &[]);
        f.button(tr.btn_models, ID_MODELS, (cx + cw - 102.0, y, 102.0, 26.0), false);
        y += 32.0;
        row(&mut f, tr.lbl_gpu, y);
        let gpu = f.check(tr.chk_gpu, ID_GPU, (cx, y + 2.0, cw, 22.0));
        y += 28.0;
        let acc_status = f.label("", (cx, y, cw, 34.0), Font::Small, ID_ACC_STATUS);
        y += 46.0;

        section(&mut f, tr.sec_output, y);
        y += 26.0;
        row(&mut f, tr.lbl_autopaste, y);
        let autopaste = f.check(tr.chk_autopaste, ID_AUTOPASTE, (cx, y + 2.0, cw, 22.0));
        y += 30.0;
        row(&mut f, tr.lbl_insert_using, y);
        let labels: Vec<&str> = METHODS.iter().map(|m| method_label(*m)).collect();
        let method = f.combo(ID_METHOD, (cx, y, cw, 26.0), &labels);
        y += 30.0;
        let restore = f.check(tr.chk_restore, ID_RESTORE, (cx, y + 2.0, cw, 22.0));
        y += 30.0;
        row(&mut f, tr.lbl_raw, y);
        let raw = f.check(tr.chk_raw, ID_RAW, (cx, y + 2.0, cw, 22.0));
        y += 26.0;
        let space = f.check(tr.chk_space, ID_SPACE, (cx, y + 2.0, cw, 22.0));
        y += 30.0;
        row(&mut f, tr.lbl_vocab, y);
        f.button(tr.btn_manage, ID_VOCAB, (cx, y, 110.0, 26.0), false);
        let vocab_info = f.label("", (cx + 120.0, y + 4.0, cw - 120.0, 20.0), Font::Small, ID_VOCAB_INFO);
        y += 40.0;

        section(&mut f, tr.sec_general, y);
        y += 26.0;
        row(&mut f, tr.lbl_ui_language, y);
        let ui_names: Vec<&str> = UI_LANGUAGES.iter().map(|(_, n)| *n).collect();
        let ui_lang = f.combo(ID_UILANG, (cx, y, cw, 26.0), &ui_names);
        y += 32.0;
        row(&mut f, tr.lbl_autostart, y);
        let autostart = f.check(tr.chk_autostart, ID_AUTOSTART, (cx, y + 2.0, cw, 22.0));
        y += 30.0;
        row(&mut f, tr.lbl_overlay, y);
        let overlay = f.combo(ID_OVERLAY, (cx, y, cw, 26.0), &[tr.overlay_bottom, tr.overlay_top, tr.overlay_off]);
        y += 40.0;

        section(&mut f, tr.sec_status, y);
        y += 26.0;
        row(&mut f, tr.lbl_fast_model, y);
        let model_status = f.label("", (cx, y + 4.0, cw - 114.0, 40.0), Font::Normal, ID_MODEL_STATUS);
        let model_btn = f.button(tr.btn_setup, ID_MODEL_BTN, (cx + cw - 106.0, y, 106.0, 26.0), false);
        y += 40.0;
        row(&mut f, tr.lbl_offline, y);
        let offline = f.label("", (cx, y + 4.0, cw, 36.0), Font::Small, ID_OFFLINE);
        y += 50.0;
        f.button(tr.btn_logs, ID_LOGS, (lx, y, 150.0, 28.0), false);
        f.button(tr.btn_close, ID_CLOSE, (width - 24.0 - 110.0, y, 110.0, 28.0), true);
        y += 28.0 + 18.0;
        f.fit(width, y);

        let mut s = SettingsUi {
            form: f,
            mic,
            test,
            meter,
            test_hint,
            lang,
            engine,
            acc_model,
            gpu,
            acc_status,
            ui_lang,
            hotkey_btn,
            hotkey_hint,
            mode,
            autopaste,
            method,
            restore,
            raw,
            space,
            vocab_info,
            autostart,
            overlay,
            model_status,
            model_btn,
            offline,
            mic_ids: Vec::new(),
            hint_warn: false,
        };
        s.load(app);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        Some(s)
    }

    pub fn scroll_into_view(&mut self, h: HWND) {
        // the focused control or its parent (the edit part of a combo)
        self.form.scroll_into_view(h);
        if let Ok(p) = unsafe { GetParent(h) } {
            self.form.scroll_into_view(p);
        }
    }

    fn load(&mut self, app: &App) {
        let st = &app.settings;
        ui::combo_set(self.lang, LANGUAGES.iter().position(|(c, _)| *c == st.language).unwrap_or(0) as i32);
        ui::set_text(self.hotkey_btn, &hotkey::display(&st.hotkey));
        ui::combo_set(self.mode, if st.mode == Mode::Hold { 0 } else { 1 });
        ui::check_set(self.autopaste, st.auto_paste);
        ui::combo_set(self.method, METHODS.iter().position(|m| *m == st.insert_method).unwrap_or(0) as i32);
        ui::combo_set(self.ui_lang, UI_LANGUAGES.iter().position(|(c, _)| *c == st.ui_language).unwrap_or(0) as i32);
        ui::check_set(self.restore, st.restore_clipboard);
        ui::check_set(self.raw, st.raw_transcription);
        ui::check_set(self.space, st.append_space);
        ui::check_set(self.autostart, autostart::is_enabled());
        let ov = if !st.show_overlay {
            2
        } else if st.overlay_position == OverlayPosition::Top {
            1
        } else {
            0
        };
        ui::combo_set(self.overlay, ov);
        self.update_enabled(app);
        self.update_vocab_info(app);
        self.set_hotkey_hint(app);
    }

    fn update_enabled(&self, app: &App) {
        let st = &app.settings;
        ui::enable(self.method, st.auto_paste);
        ui::enable(self.restore, st.auto_paste && st.insert_method != InsertMethod::Type);
    }

    pub fn update_vocab_info(&self, app: &App) {
        let n = app.settings.vocabulary.len();
        let tpl = if app.settings.use_vocabulary { t().vocab_info_on } else { t().vocab_info_off };
        let text = fmt(tpl, &[("n", &n)]);
        ui::set_text(self.vocab_info, &text);
    }

    fn set_hotkey_hint(&mut self, app: &App) {
        self.hint_warn = false;
        let text = if !app.hook_ok {
            self.hint_warn = true;
            t().hint_hook_unavailable
        } else if !app.fallback_registered {
            t().hint_also_used
        } else {
            t().hint_shortcut
        };
        ui::set_text(self.hotkey_hint, text);
    }
}

pub fn refresh_devices(app: &mut App) {
    let Some(s) = app.settings_ui.as_mut() else { return };
    ui::combo_clear(s.mic);
    s.mic_ids.clear();
    let def = app.default_device.as_ref().map(|d| d.name.as_str()).unwrap_or(t().mic_none_found);
    ui::combo_add(s.mic, &fmt(t().mic_system_default, &[("name", &def)]));
    s.mic_ids.push(String::new());
    let mut sel = 0;
    for d in &app.devices {
        if d.id == app.settings.microphone_id {
            sel = s.mic_ids.len();
        }
        ui::combo_add(s.mic, &d.name);
        s.mic_ids.push(d.id.clone());
    }
    if !app.settings.microphone_id.is_empty() && sel == 0 {
        let name = if app.settings.microphone_name.is_empty() { t().mic_selected } else { &app.settings.microphone_name };
        ui::combo_add(s.mic, &fmt(t().mic_not_connected, &[("name", &name)]));
        sel = s.mic_ids.len();
        s.mic_ids.push(app.settings.microphone_id.clone());
    }
    ui::combo_set(s.mic, sel as i32);
    if app.devices.is_empty() {
        ui::set_text(s.test_hint, t().mic_no_devices);
    }
}

/// The Recognition section: engine mode, accurate model, GPU and the state of the ACCURATE engine.
pub fn refresh_engine(app: &mut App) {
    let Some(s) = app.settings_ui.as_ref() else { return };
    let tr = t();
    let st = &app.settings;
    ui::combo_set(s.engine, ENGINE_MODES.iter().position(|m| *m == st.engine_mode).unwrap_or(0) as i32);
    ui::combo_clear(s.acc_model);
    let wanted = app.wanted_whisper();
    // nothing chosen yet: show the recommended model for this PC
    let shown = if st.whisper_model.is_empty() {
        wanted.map(|w| w.id).unwrap_or(if models::nvidia_gpu_cached().is_some() { models::DEFAULT_WHISPER_GPU } else { models::DEFAULT_WHISPER_CPU })
    } else {
        st.whisper_model.as_str()
    };
    let mut sel = -1;
    for (i, m) in WHISPER_MODELS.iter().enumerate() {
        // the row label already says "Accurate model": "large-v3-turbo" instead of "Whisper large-v3-turbo"
        let short = m.name.trim_start_matches("Whisper ");
        let name = if models::whisper_installed(m) { short.to_string() } else { fmt(tr.model_not_downloaded, &[("name", &short)]) };
        ui::combo_add(s.acc_model, &name);
        if m.id == shown {
            sel = i as i32;
        }
    }
    ui::combo_set(s.acc_model, sel);
    ui::check_set(s.gpu, st.use_gpu);
    ui::enable(s.gpu, models::nvidia_gpu_cached().is_some());
    let downloading = match (app.download_cancel.is_some(), app.download_item, app.download_progress) {
        (true, Some(item @ (DownloadItem::Whisper(_) | DownloadItem::CudaPack)), Some((d, total))) => Some((item, d * 100 / total.max(1))),
        _ => None,
    };
    let mut text = match (&app.whisper_state, wanted) {
        _ if st.engine_mode == "fast" => tr.acc_fast_only.to_string(),
        (_, None) if st.engine_mode == "auto" && models::installed_whisper_models().is_empty() => tr.acc_no_model.to_string(),
        (_, None) if st.engine_mode == "auto" => tr.acc_auto_no_gpu.to_string(),
        (_, None) => tr.acc_no_model.to_string(),
        (WhisperState::Ready(w), Some(_)) => {
            let device = match &w.device {
                crate::stt::ComputeDevice::Gpu(name) => format!("GPU ({name})"),
                crate::stt::ComputeDevice::Cpu => "CPU".to_string(),
            };
            let mut s = fmt(tr.acc_ready, &[("name", &w.label), ("device", &device), ("ms", &format!("{:.0}", w.load_ms))]);
            if w.restart_for_gpu {
                s.push_str(tr.acc_restart_gpu);
            }
            s
        }
        // AUTO only loads Whisper on the GPU; if that is not possible it simply dictates with FAST
        (WhisperState::Failed { error, .. }, Some(_)) if st.engine_mode == "auto" => format!("{} ({error})", tr.acc_auto_no_gpu),
        (WhisperState::Failed { error, .. }, Some(m)) => fmt(tr.acc_failed, &[("name", &m.name), ("e", error)]),
        (_, Some(m)) => fmt(tr.acc_loading, &[("name", &m.name)]),
    };
    if let (Some(chosen), Some(m)) = (models::whisper_model(&st.whisper_model), wanted) {
        if chosen.id != m.id && st.engine_mode != "fast" {
            text = format!("{} · {text}", fmt(tr.acc_using_other, &[("wanted", &chosen.name), ("name", &m.name)]));
        }
    }
    if let Some((item, pct)) = downloading {
        let name = match item {
            DownloadItem::Whisper(id) => models::whisper_model(id).map(|m| m.name).unwrap_or_default().to_string(),
            _ => tr.gpu_pack_name.to_string(),
        };
        text = format!("{name}: {}", fmt(tr.status_downloading, &[("pct", &pct)]));
    }
    ui::set_text(s.acc_status, &text);
}

pub fn refresh_status(app: &mut App) {
    refresh_engine(app);
    let Some(s) = app.settings_ui.as_ref() else { return };
    let tr = t();
    let size = util::format_bytes(model::MODEL_SIZE);
    let (status, button, offline) = match &app.engine_state {
        EngineState::Ready { load_ms } => (
            fmt(tr.status_ready, &[("model", &format!("{} {}", model::MODEL_NAME, model::MODEL_VERSION)), ("size", &size), ("ms", &format!("{load_ms:.0}"))]),
            None,
            tr.offline_ready,
        ),
        EngineState::Starting | EngineState::Loading => (tr.status_loading.to_string(), None, tr.offline_loading),
        EngineState::NoModel => (
            match &app.download_progress {
                Some((d, total)) if app.download_cancel.is_some() && app.download_item == Some(crate::app::DownloadItem::Whistle) => fmt(tr.status_downloading, &[("pct", &(d * 100 / (*total).max(1)))]),
                _ => fmt(tr.status_not_installed, &[("size", &size)]),
            },
            Some(tr.btn_setup),
            tr.offline_need_download,
        ),
        EngineState::ModelError(e) => (format!("⚠ {e}"), Some(tr.btn_repair), tr.offline_repair),
        EngineState::RuntimeError(e) => (format!("⚠ {e}"), None, tr.offline_unavailable),
    };
    ui::set_text(s.model_status, &status);
    ui::show(s.model_btn, button.is_some());
    if let Some(b) = button {
        ui::set_text(s.model_btn, b);
    }
    ui::set_text(s.offline, offline);
}

pub fn mic_test_stopped(app: &mut App) {
    if let Some(s) = app.settings_ui.as_ref() {
        ui::set_text(s.test, t().btn_test);
        ui::set_text(s.test_hint, "");
        unsafe {
            let _ = KillTimer(Some(s.form.hwnd), TIMER_METER);
        }
        METER.store(0, Ordering::Relaxed);
        unsafe {
            let _ = InvalidateRect(Some(s.meter), None, true);
        }
    }
}

fn toggle_mic_test(app: &mut App) {
    if let Some(t) = app.mic_test.take() {
        let c = t.stop();
        if let Some(e) = c.error {
            if let Some(s) = app.settings_ui.as_ref() {
                ui::set_text(s.test_hint, &e.to_string());
            }
        }
        mic_test_stopped(app);
        return;
    }
    if app.is_recording() {
        return;
    }
    let id = if app.settings.microphone_id.is_empty() || !app.devices.iter().any(|d| d.id == app.settings.microphone_id) {
        None
    } else {
        Some(app.settings.microphone_id.clone())
    };
    let hwnd = app.settings_ui.as_ref().map(|s| s.form.hwnd).unwrap_or_default();
    match Recorder::start(id, Purpose::Monitor, std::sync::Arc::new(|_| {})) {
        Ok(r) => {
            app.mic_test = Some(r);
            if let Some(s) = app.settings_ui.as_ref() {
                ui::set_text(s.test, t().btn_stop);
                ui::set_text(s.test_hint, t().mic_test_hint);
            }
            unsafe { SetTimer(Some(hwnd), TIMER_METER, 40, None) };
        }
        Err(e) => {
            if let Some(s) = app.settings_ui.as_ref() {
                ui::set_text(s.test_hint, &e.to_string());
            }
        }
    }
}

/// WM_WT_CAPTURE from the keyboard hook.
pub fn on_capture(app: &mut App, packed: u64) {
    let Some(s) = app.settings_ui.as_mut() else { return };
    if packed == 0 {
        ui::set_text(s.hotkey_btn, &hotkey::display(&app.settings.hotkey));
        ui::set_text(s.hotkey_hint, t().hint_unchanged);
        return;
    }
    let h = crate::settings::Hotkey::unpack(packed);
    match hotkey::validate(&h) {
        Err(e) => {
            ui::set_text(s.hotkey_btn, &hotkey::display(&app.settings.hotkey));
            s.hint_warn = true;
            ui::set_text(s.hotkey_hint, &e);
        }
        Ok(()) => {
            app.apply_hotkey(h);
            let st = &app.settings;
            let txt = hotkey::display(&st.hotkey);
            let fallback = app.fallback_registered;
            let hook = app.hook_ok;
            if let Some(s) = app.settings_ui.as_mut() {
                ui::set_text(s.hotkey_btn, &txt);
                s.hint_warn = false;
                let hint = if !hook {
                    t().hint_hook_unavailable
                } else if !fallback {
                    t().hint_also_used
                } else {
                    t().hint_saved
                };
                ui::set_text(s.hotkey_hint, hint);
            }
        }
    }
}

fn on_command(app: &mut App, id: i32, code: u32) {
    let Some(s) = app.settings_ui.as_ref() else { return };
    let changed = code == CBN_SELCHANGE || code == BN_CLICKED;
    match id {
        ID_MIC if code == CBN_SELCHANGE => {
            let idx = ui::combo_get(s.mic).max(0) as usize;
            let id = s.mic_ids.get(idx).cloned().unwrap_or_default();
            let dev = app.devices.iter().find(|d| d.id == id).cloned();
            if idx == 0 {
                app.select_device(None);
            } else if let Some(d) = dev {
                app.select_device(Some(d));
            }
            if app.mic_test.is_some() {
                toggle_mic_test(app);
                toggle_mic_test(app);
            }
        }
        ID_TEST if changed => toggle_mic_test(app),
        ID_LANG if code == CBN_SELCHANGE => {
            let i = ui::combo_get(s.lang).max(0) as usize;
            app.settings.language = LANGUAGES[i.min(LANGUAGES.len() - 1)].0.to_string();
            app.save_settings();
        }
        ID_HOTKEY if changed => {
            ui::set_text(s.hotkey_btn, t().hint_press_keys);
            ui::set_text(s.hotkey_hint, t().hint_esc_cancels);
            hotkey::set_capture(true);
        }
        ID_MODE if code == CBN_SELCHANGE => {
            app.settings.mode = if ui::combo_get(s.mode) == 1 { Mode::Toggle } else { Mode::Hold };
            app.save_settings();
            app.update_tray();
        }
        ID_AUTOPASTE if changed => {
            app.settings.auto_paste = ui::check_get(s.autopaste);
            s.update_enabled(app);
            app.save_settings();
        }
        ID_METHOD if code == CBN_SELCHANGE => {
            let i = ui::combo_get(s.method).max(0) as usize;
            app.settings.insert_method = METHODS[i.min(METHODS.len() - 1)];
            s.update_enabled(app);
            app.save_settings();
        }
        ID_RESTORE if changed => {
            app.settings.restore_clipboard = ui::check_get(s.restore);
            app.save_settings();
        }
        ID_RAW if changed => {
            app.settings.raw_transcription = ui::check_get(s.raw);
            app.save_settings();
        }
        ID_SPACE if changed => {
            app.settings.append_space = ui::check_get(s.space);
            app.save_settings();
        }
        ID_VOCAB if changed => app.open_vocab(),
        ID_ENGINE if code == CBN_SELCHANGE => {
            let i = ui::combo_get(s.engine).max(0) as usize;
            app.settings.engine_mode = ENGINE_MODES[i.min(ENGINE_MODES.len() - 1)].to_string();
            app.save_settings();
            app.ensure_whisper();
            refresh_engine(app);
        }
        ID_ACC_MODEL if code == CBN_SELCHANGE => {
            let i = ui::combo_get(s.acc_model);
            if let Some(m) = usize::try_from(i).ok().and_then(|i| WHISPER_MODELS.get(i)) {
                app.settings.whisper_model = m.id.to_string();
                app.save_settings();
                app.ensure_whisper();
                refresh_engine(app);
                if !models::whisper_installed(m) {
                    app.open_models();
                }
                if app.models_ui.is_some() {
                    crate::models_ui::refresh(app);
                }
            }
        }
        ID_GPU if changed => {
            app.settings.use_gpu = ui::check_get(s.gpu);
            app.save_settings();
            app.ensure_whisper();
            refresh_engine(app);
        }
        ID_MODELS if changed => app.open_models(),
        ID_AUTOSTART if changed => {
            let on = ui::check_get(s.autostart);
            match autostart::set(on) {
                Ok(()) => {
                    app.settings.start_with_windows = on;
                    app.save_settings();
                }
                Err(e) => {
                    ui::check_set(s.autostart, !on);
                    crate::log_warn!("autostart: {e}");
                }
            }
        }
        ID_OVERLAY if code == CBN_SELCHANGE => {
            match ui::combo_get(s.overlay) {
                1 => {
                    app.settings.show_overlay = true;
                    app.settings.overlay_position = OverlayPosition::Top;
                }
                2 => app.settings.show_overlay = false,
                _ => {
                    app.settings.show_overlay = true;
                    app.settings.overlay_position = OverlayPosition::Bottom;
                }
            }
            app.apply_overlay_settings();
            app.save_settings();
        }
        ID_MODEL_BTN if changed => app.open_setup(),
        ID_UILANG if code == CBN_SELCHANGE => {
            let i = ui::combo_get(s.ui_lang).max(0) as usize;
            let code = UI_LANGUAGES[i.min(UI_LANGUAGES.len() - 1)].0;
            if code != app.settings.ui_language {
                app.set_ui_language(code);
            }
        }
        ID_LOGS if changed => ui::shell_open(&crate::paths::logs_dir().display().to_string()),
        ID_CLOSE if changed => unsafe {
            let _ = PostMessageW(Some(s.form.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
}

fn draw_meter(dis: &DRAWITEMSTRUCT) {
    unsafe {
        let r = dis.rcItem;
        let bg = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00E6_E2DF));
        FillRect(dis.hDC, &r, bg);
        let _ = DeleteObject(bg.into());
        let level = f32::from_bits(METER.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        if level > 0.0 {
            let fill = RECT { right: r.left + ((r.right - r.left) as f32 * level) as i32, ..r };
            let c = if level > 0.92 { 0x0030_50D0 } else { 0x0050_A02E }; // red-ish when clipping, else green (BGR)
            let br = CreateSolidBrush(windows::Win32::Foundation::COLORREF(c));
            FillRect(dis.hDC, &fill, br);
            let _ = DeleteObject(br.into());
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, m: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match m {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            with_app(|a| on_command(a, id, code));
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let warn = with_app(|a| a.settings_ui.as_ref().map(|s| s.hint_warn)).flatten().unwrap_or(false);
            let warn_ids: &[i32] = if warn { &[ID_HOTKEY_HINT] } else { &[] };
            LRESULT(ui::ctl_color_static(wparam, lparam, &[MUTED, &[ID_HOTKEY_HINT]].concat(), warn_ids))
        }
        WM_DRAWITEM => {
            let dis = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
            if dis.CtlID as i32 == ID_METER {
                draw_meter(dis);
            }
            LRESULT(1)
        }
        WM_TIMER if wparam.0 == TIMER_METER => {
            let lvl = with_app(|a| a.mic_test.as_ref().map(|t| t.level())).flatten();
            match lvl {
                Some(l) => {
                    let prev = f32::from_bits(METER.load(Ordering::Relaxed));
                    let v = if l > prev { l } else { prev * 0.85 + l * 0.15 };
                    METER.store(v.to_bits(), Ordering::Relaxed);
                    if let Ok(meter) = unsafe { GetDlgItem(Some(hwnd), ID_METER) } {
                        unsafe {
                            let _ = InvalidateRect(Some(meter), None, false);
                        }
                    }
                }
                None => unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_METER);
                },
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            // leaving the window while capturing a shortcut cancels the capture
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE && hotkey::is_capturing() {
                hotkey::set_capture(false);
                with_app(|a| on_capture(a, 0));
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let r = unsafe { &*(lparam.0 as *const RECT) };
            with_app(|a| {
                if let Some(s) = a.settings_ui.as_mut() {
                    s.form.relayout(dpi);
                }
            });
            unsafe {
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            with_app(|a| {
                if let Some(s) = a.settings_ui.as_mut() {
                    s.form.update_scrollbar();
                }
            });
            LRESULT(0)
        }
        WM_VSCROLL => {
            with_app(|a| {
                if let Some(s) = a.settings_ui.as_mut() {
                    s.form.on_vscroll(wparam);
                }
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let used = with_app(|a| a.settings_ui.as_mut().map(|s| s.form.on_wheel(wparam))).flatten().unwrap_or(false);
            if used {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, m, wparam, lparam) }
            }
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            hotkey::set_capture(false);
            with_app(|a| {
                if let Some(t) = a.mic_test.take() {
                    drop(t.stop());
                }
                a.settings_ui = None;
            });
            METER.store(0, Ordering::Relaxed);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, m, wparam, lparam) },
    }
}
