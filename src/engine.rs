//! Binding to the official Cactus Compute Needle 3 engine (`libneedle3.dll`), which runs Whistle.
//!
//! The engine holds one process-global, non-thread-safe model, so [`Engine`] is `!Sync` and the app only
//! ever touches it from one dedicated thread ([`EngineService`]).

use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::Deserialize;
use windows::core::{s, PCSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use std::rc::Rc;

use crate::stt::{split_for, transcribe_planned, ComputeDevice, EngineKind, SpeechEngine};
use crate::util::{sha256_file, WStr};
use crate::whisper::{GpuInfo, WhisperEngine, WhisperRuntime};
use crate::{log_error, log_info, log_warn};

pub const ENGINE_DLL: &str = "libneedle3.dll";
/// SHA-256 of `needle/libneedle3.dll` inside `cactus_needle-3.1.0-py3-none-win_amd64.whl`
/// (Hugging Face `Cactus-Compute/needle3` @ c7c415a3d1b3d929014bc6e866d51ebb971f7089).
pub const ENGINE_DLL_SHA256: &str = "de2e2c39cd311fbd9971fad4736abc329ed970653674c203e149c4ef27fd1c62";
pub const ENGINE_VERSION: &str = "3.1.0";

/// Whistle's documented hard limit: 30 s of 16 kHz audio.
pub const MAX_SAMPLES: usize = 480_000;
const NEEDLE_SPEECH: i32 = 2;
const OUT_CAPACITY: usize = 1 << 18;

type LoadFn = unsafe extern "C" fn(*const u8, u64) -> i32;
type ModelsFn = unsafe extern "C" fn() -> i32;
type LastErrorFn = unsafe extern "C" fn() -> *const c_char;
type TranscribeFn =
    unsafe extern "C" fn(*const f32, i32, *const c_char, *const c_char, i32, *mut c_char, i32) -> i32;

#[derive(Debug, Clone, PartialEq)]
pub enum EngineError {
    /// libneedle3.dll is not where it should be.
    RuntimeMissing(PathBuf),
    /// The DLL exists but is not the pinned, verified build.
    RuntimeCorrupt(String),
    /// LoadLibrary / GetProcAddress failed.
    RuntimeLoad(String),
    ModelMissing(PathBuf),
    ModelCorrupt(String),
    ModelLoad(String),
    NotLoaded,
    TooLong(usize),
    Transcribe(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::{fmt, t};
        let s = match self {
            EngineError::RuntimeMissing(p) => fmt(t().err_engine_missing, &[("path", &p.display())]),
            EngineError::RuntimeCorrupt(m) => fmt(t().err_engine_corrupt, &[("m", m)]),
            EngineError::RuntimeLoad(m) => fmt(t().err_engine_load, &[("m", m)]),
            EngineError::ModelMissing(_) => t().err_model_missing.to_string(),
            EngineError::ModelCorrupt(m) => fmt(t().err_model_corrupt, &[("m", m)]),
            EngineError::ModelLoad(m) => fmt(t().err_model_load, &[("m", m)]),
            EngineError::NotLoaded => t().err_not_loaded.to_string(),
            EngineError::TooLong(n) => fmt(t().err_too_long, &[("s", &format!("{:.1}", *n as f32 / 16000.0))]),
            EngineError::Transcribe(m) => fmt(t().err_transcribe, &[("m", m)]),
        };
        f.write_str(&s)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawTranscript {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub ttft_ms: f64,
    #[serde(default)]
    pub decode_tps: f64,
}

struct Api {
    module: HMODULE,
    load: LoadFn,
    models: ModelsFn,
    last_error: LastErrorFn,
    transcribe: TranscribeFn,
}

impl Drop for Api {
    fn drop(&mut self) {
        // The engine keeps worker threads; unloading a DLL with live threads is unsafe. We only free the
        // module when the process is not shutting down it - in practice Api lives for the process lifetime.
        let _ = self.module;
    }
}

pub struct Engine {
    api: Api,
    out: Vec<u8>,
    loaded_model: Option<PathBuf>,
    _not_sync: std::marker::PhantomData<*const ()>,
}

/// Default location of the engine DLL: next to the executable.
pub fn default_dll_path() -> PathBuf {
    // developer override, not available in release builds without test hooks
    #[cfg(any(debug_assertions, feature = "test-hooks"))]
    if let Some(p) = std::env::var_os("WHISTLETYPE_ENGINE_DLL") {
        return PathBuf::from(p);
    }
    crate::paths::exe_dir().join(ENGINE_DLL)
}

impl Engine {
    /// Loads and verifies the engine DLL. `verify` checks the pinned SHA-256 first.
    pub fn open(dll: &Path, verify: bool) -> Result<Engine, EngineError> {
        if !dll.exists() {
            return Err(EngineError::RuntimeMissing(dll.to_path_buf()));
        }
        if verify {
            let got = sha256_file(dll).map_err(|e| EngineError::RuntimeLoad(e.to_string()))?;
            if got != ENGINE_DLL_SHA256 {
                return Err(EngineError::RuntimeCorrupt(format!("SHA-256 {got}")));
            }
        }
        let wpath = WStr::from_path(dll);
        // Absolute path + restricted search: dependencies only from the DLL's folder and System32.
        let module = unsafe {
            LoadLibraryExW(wpath.pcwstr(), None, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
        }
        .map_err(|e| EngineError::RuntimeLoad(e.message()))?;

        unsafe fn sym<T>(m: HMODULE, name: PCSTR) -> Result<T, EngineError> {
            match GetProcAddress(m, name) {
                Some(f) => Ok(std::mem::transmute_copy(&f)),
                None => Err(EngineError::RuntimeLoad(format!("missing export {}", name.display()))),
            }
        }
        let api = unsafe {
            let r = (|| {
                Ok(Api {
                    module,
                    load: sym(module, s!("needle_load"))?,
                    models: sym(module, s!("needle_models"))?,
                    last_error: sym(module, s!("needle_last_error"))?,
                    transcribe: sym(module, s!("needle_transcribe"))?,
                })
            })();
            if r.is_err() {
                let _ = FreeLibrary(module);
            }
            r?
        };
        Ok(Engine { api, out: vec![0u8; OUT_CAPACITY], loaded_model: None, _not_sync: Default::default() })
    }

    fn last_error(&self) -> String {
        unsafe {
            let p = (self.api.last_error)();
            if p.is_null() {
                "unknown engine error".into()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded_model.is_some()
    }

    /// Reads, verifies (optional SHA-256) and loads a Whistle `.cact`. The engine copies the blob, so the
    /// buffer is released right after (verified: the engine works after the buffer is zeroed and freed).
    pub fn load_model(&mut self, path: &Path, expected_sha256: Option<&str>) -> Result<(), EngineError> {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(EngineError::ModelMissing(path.to_path_buf())),
            Err(e) => return Err(EngineError::ModelLoad(e.to_string())),
        };
        if let Some(want) = expected_sha256 {
            let got = crate::util::sha256_bytes(&data);
            if got != want {
                return Err(EngineError::ModelCorrupt(format!("SHA-256 mismatch, got {}", &got[..16])));
            }
        }
        let rc = unsafe { (self.api.load)(data.as_ptr(), data.len() as u64) };
        drop(data);
        if rc < 0 {
            return Err(EngineError::ModelLoad(self.last_error()));
        }
        let models = unsafe { (self.api.models)() };
        if models & NEEDLE_SPEECH == 0 {
            return Err(EngineError::ModelLoad("file is not a Whistle speech model".into()));
        }
        self.loaded_model = Some(path.to_path_buf());
        Ok(())
    }

    /// Transcribes at most 30 s of 16 kHz mono float audio in [-1, 1].
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        language: Option<&str>,
        keywords: Option<&str>,
    ) -> Result<RawTranscript, EngineError> {
        if self.loaded_model.is_none() {
            return Err(EngineError::NotLoaded);
        }
        if pcm.len() > MAX_SAMPLES {
            return Err(EngineError::TooLong(pcm.len()));
        }
        if pcm.is_empty() {
            return Ok(RawTranscript::default());
        }
        let lang = language.and_then(|l| CString::new(l).ok());
        let kw = keywords.filter(|k| !k.is_empty()).and_then(|k| CString::new(k.replace('\0', "")).ok());
        self.out[0] = 0;
        let rc = unsafe {
            (self.api.transcribe)(
                pcm.as_ptr(),
                pcm.len() as i32,
                lang.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                kw.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                0,
                self.out.as_mut_ptr() as *mut c_char,
                self.out.len() as i32,
            )
        };
        if rc < 0 {
            return Err(EngineError::Transcribe(self.last_error()));
        }
        let end = self.out.iter().position(|&b| b == 0).unwrap_or(self.out.len());
        let json = String::from_utf8_lossy(&self.out[..end]);
        serde_json::from_str::<RawTranscript>(&json)
            .map_err(|e| EngineError::Transcribe(format!("unexpected engine output ({e})")))
    }
}

// ------------------------------------------------------------------------------------------------
// Engine service: owns the Engine on one thread, processes jobs in order.
// ------------------------------------------------------------------------------------------------

/// Which language to ask Whistle for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePlan {
    /// Forced language code, or None to let Whistle detect it.
    pub language: Option<String>,
    /// When detecting: detected languages accepted as they are (empty = accept any).
    pub accept: Vec<String>,
    /// Language for a second pass when the detected language is not in `accept`.
    pub fallback: Option<String>,
}

impl LanguagePlan {
    pub fn detect() -> Self {
        LanguagePlan { language: None, accept: Vec::new(), fallback: None }
    }
    pub fn forced(code: &str) -> Self {
        LanguagePlan { language: Some(code.to_string()), accept: Vec::new(), fallback: None }
    }
    /// True when a transcript detected as `detected` must be redone with the fallback language.
    pub fn needs_fallback(&self, detected: &str) -> bool {
        self.language.is_none()
            && self.fallback.is_some()
            && !detected.is_empty() // "" = no speech
            && !self.accept.is_empty()
            && !self.accept.iter().any(|a| a == detected)
    }
}

impl SpeechEngine for Engine {
    fn label(&self) -> String {
        format!("{} {}", crate::model::MODEL_NAME, crate::model::MODEL_VERSION)
    }

    fn device(&self) -> ComputeDevice {
        ComputeDevice::Cpu
    }

    fn max_samples(&self) -> Option<usize> {
        Some(MAX_SAMPLES)
    }

    fn transcribe(&mut self, pcm: &[f32], language: Option<&str>, vocabulary: &[String]) -> Result<RawTranscript, EngineError> {
        // Whistle's native keyword biasing: newline-separated words and phrases
        let kw = if vocabulary.is_empty() { None } else { Some(vocabulary.join("\n")) };
        Engine::transcribe(self, pcm, language, kw.as_deref())
    }
}

// ------------------------------------------------------------------------------------------------
// Engine service: owns both engines on one thread and processes jobs in order.
// ------------------------------------------------------------------------------------------------

pub struct TranscribeJob {
    pub id: u64,
    /// The whole (trimmed) recording; it is split here if the engine needs it (Whistle: 30 s per call).
    pub audio: Vec<f32>,
    pub engine: EngineKind,
    pub language: LanguagePlan,
    pub vocabulary: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WhisperRequest {
    pub model_id: String,
    pub model_path: PathBuf,
    pub label: String,
    /// Runtime pack folders in order of preference (CUDA pack first when the GPU should be used).
    pub runtime_dirs: Vec<PathBuf>,
    pub use_gpu: bool,
    /// AUTO mode: Whisper is only worth keeping on the GPU. If the GPU cannot be used, fail instead of loading the
    /// model on the CPU (seconds per sentence, ~1 GB RAM) - the app then dictates with Whistle.
    pub gpu_only: bool,
}

pub enum EngineCmd {
    /// Load the Whistle model (FAST engine).
    LoadModel { path: PathBuf, sha256: Option<String> },
    /// Load (or switch to) a Whisper model (ACCURATE engine). Frees the previous Whisper model first.
    LoadWhisper(WhisperRequest),
    /// Free the Whisper model (VRAM/RAM). The runtime DLLs stay loaded.
    UnloadWhisper,
    Transcribe(TranscribeJob),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhisperStatus {
    pub model_id: String,
    pub label: String,
    pub device: ComputeDevice,
    pub load_ms: f64,
    pub runtime_dir: PathBuf,
    pub gpus: Vec<GpuInfo>,
    /// The CUDA pack is preferred but another runtime is already loaded: a restart is needed to use the GPU.
    pub restart_for_gpu: bool,
}

#[derive(Debug)]
pub struct JobResult {
    pub id: u64,
    /// One transcript per piece of audio, in order.
    pub parts: Result<Vec<RawTranscript>, EngineError>,
    pub engine_ms: f64,
    pub audio_s: f64,
    pub engine: EngineKind,
    pub label: String,
    pub device: ComputeDevice,
    /// Set when the requested engine could not be used and the other one transcribed instead.
    pub substituted: bool,
}

#[derive(Debug)]
pub enum EngineEvent {
    RuntimeFailed(EngineError),
    ModelLoaded { load_ms: f64, warmup_ms: f64 },
    ModelFailed(EngineError),
    WhisperLoaded(WhisperStatus),
    WhisperFailed { model_id: String, error: EngineError },
    WhisperUnloaded,
    Busy { id: u64, segment: usize, of: usize },
    Done(JobResult),
}

pub struct EngineService {
    tx: Sender<EngineCmd>,
    thread: Option<JoinHandle<()>>,
}

impl EngineService {
    pub fn start(dll: PathBuf, notify: Box<dyn Fn(EngineEvent) + Send>) -> EngineService {
        let (tx, rx) = mpsc::channel::<EngineCmd>();
        let thread = std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || engine_thread(dll, rx, notify))
            .expect("spawn engine thread");
        EngineService { tx, thread: Some(thread) }
    }

    pub fn send(&self, cmd: EngineCmd) -> bool {
        self.tx.send(cmd).is_ok()
    }

    /// Asks the thread to stop and waits up to `timeout` (a transcription in progress cannot be interrupted).
    pub fn shutdown(mut self, timeout: std::time::Duration) {
        let _ = self.tx.send(EngineCmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let start = Instant::now();
            while !t.is_finished() && start.elapsed() < timeout {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}

/// All engines, owned by the engine thread.
pub struct Engines {
    pub whistle: Option<Engine>,
    whisper_rt: Option<Rc<WhisperRuntime>>,
    whisper: Option<WhisperEngine>,
    whisper_req: Option<WhisperRequest>,
}

impl Engines {
    pub fn new(whistle: Option<Engine>) -> Engines {
        Engines { whistle, whisper_rt: None, whisper: None, whisper_req: None }
    }

    /// Loads the whisper.cpp runtime once (the first pack in the list that loads).
    fn runtime(&mut self, dirs: &[PathBuf]) -> Result<Rc<WhisperRuntime>, EngineError> {
        if let Some(rt) = &self.whisper_rt {
            return Ok(rt.clone());
        }
        let mut last = EngineError::RuntimeMissing(dirs.first().cloned().unwrap_or_default());
        for d in dirs {
            match WhisperRuntime::open(d) {
                Ok(rt) => {
                    let rt = Rc::new(rt);
                    self.whisper_rt = Some(rt.clone());
                    return Ok(rt);
                }
                Err(e) => {
                    log_warn!("whisper: runtime in {} not usable: {e}", d.display());
                    last = e;
                }
            }
        }
        Err(last)
    }

    pub fn load_whisper(&mut self, req: &WhisperRequest, notify: &dyn Fn(EngineEvent)) {
        self.whisper = None; // free the previous model (VRAM) before loading the next one
        let t = Instant::now();
        let rt = match self.runtime(&req.runtime_dirs) {
            Ok(rt) => rt,
            Err(error) => {
                notify(EngineEvent::WhisperFailed { model_id: req.model_id.clone(), error });
                return;
            }
        };
        if req.gpu_only && rt.gpus.is_empty() {
            log_info!("whisper: no usable GPU in {} - not loading {} on the CPU (AUTO)", rt.dir.display(), req.label);
            notify(EngineEvent::WhisperFailed { model_id: req.model_id.clone(), error: EngineError::ModelLoad("no usable GPU".into()) });
            return;
        }
        match WhisperEngine::load(rt.clone(), &req.model_path, &req.label, req.use_gpu, req.gpu_only) {
            Ok(mut w) => {
                let load_ms = t.elapsed().as_secs_f64() * 1000.0;
                // Warm-up: the first call initialises kernels / allocates buffers.
                let t2 = Instant::now();
                let warm: Vec<f32> = (0..16_000).map(|i| ((i * 7919) % 13) as f32 * 1e-5).collect();
                if let Err(e) = w.transcribe(&warm, Some("en"), &[]) {
                    log_warn!("whisper: warm-up failed: {e}");
                }
                let restart_for_gpu = req.use_gpu && req.runtime_dirs.first().is_some_and(|d| d != &rt.dir) && rt.gpus.is_empty();
                log_info!(
                    "whisper: {} loaded on {:?} in {load_ms:.0} ms, warm-up {:.0} ms",
                    req.label,
                    w.device(),
                    t2.elapsed().as_secs_f64() * 1000.0
                );
                notify(EngineEvent::WhisperLoaded(WhisperStatus {
                    model_id: req.model_id.clone(),
                    label: req.label.clone(),
                    device: w.device(),
                    load_ms,
                    runtime_dir: rt.dir.clone(),
                    gpus: rt.gpus.clone(),
                    restart_for_gpu,
                }));
                self.whisper = Some(w);
                self.whisper_req = Some(req.clone());
            }
            Err(error) => {
                log_error!("whisper: loading {} failed: {error}", req.label);
                notify(EngineEvent::WhisperFailed { model_id: req.model_id.clone(), error });
            }
        }
    }

    fn available(&self, k: EngineKind) -> bool {
        match k {
            EngineKind::Whistle => self.whistle.as_ref().is_some_and(|w| w.is_loaded()),
            EngineKind::Whisper => self.whisper.is_some(),
        }
    }

    pub fn run(&mut self, job: &TranscribeJob, notify: &dyn Fn(EngineEvent)) -> JobResult {
        let t = Instant::now();
        let audio_s = job.audio.len() as f64 / 16_000.0;
        let mut kind = job.engine;
        let mut substituted = false;
        if !self.available(kind) {
            let other = if kind == EngineKind::Whisper { EngineKind::Whistle } else { EngineKind::Whisper };
            if self.available(other) {
                log_warn!("engine: {kind:?} is not available, using {other:?} for this dictation");
                kind = other;
                substituted = true;
            }
        }
        let mut result = self.run_on(kind, job, notify);
        // A GPU error at run time (driver reset, out of VRAM...): reload the model on the CPU and retry once -
        // or, in AUTO (gpu_only), drop Whisper and use Whistle for this and the following dictations.
        if kind == EngineKind::Whisper && result.0.is_err() && self.whisper.as_ref().is_some_and(|w| w.device().is_gpu()) {
            if let Some(req) = self.whisper_req.clone().filter(|r| r.gpu_only) {
                log_warn!("whisper: GPU transcription failed ({:?}); AUTO falls back to Whistle", result.0.as_ref().err());
                self.whisper = None;
                self.whisper_req = None;
                notify(EngineEvent::WhisperFailed { model_id: req.model_id, error: EngineError::Transcribe("GPU error".into()) });
                if self.available(EngineKind::Whistle) {
                    kind = EngineKind::Whistle;
                    substituted = true;
                    result = self.run_on(kind, job, notify);
                }
            } else if let Some(mut req) = self.whisper_req.clone() {
                log_warn!("whisper: GPU transcription failed ({:?}); reloading on the CPU", result.0.as_ref().err());
                req.use_gpu = false;
                self.load_whisper(&req, notify);
                result = self.run_on(kind, job, notify);
            }
        }
        let (parts, label, device) = result;
        JobResult { id: job.id, parts, engine_ms: t.elapsed().as_secs_f64() * 1000.0, audio_s, engine: kind, label, device, substituted }
    }

    fn run_on(
        &mut self,
        kind: EngineKind,
        job: &TranscribeJob,
        notify: &dyn Fn(EngineEvent),
    ) -> (Result<Vec<RawTranscript>, EngineError>, String, ComputeDevice) {
        let eng: &mut dyn SpeechEngine = match kind {
            EngineKind::Whistle => match self.whistle.as_mut().filter(|w| w.is_loaded()) {
                Some(w) => w,
                None => return (Err(EngineError::NotLoaded), String::new(), ComputeDevice::Cpu),
            },
            EngineKind::Whisper => match self.whisper.as_mut() {
                Some(w) => w,
                None => return (Err(EngineError::NotLoaded), String::new(), ComputeDevice::Cpu),
            },
        };
        let label = eng.label();
        let device = eng.device();
        let pieces = split_for(eng, &job.audio);
        let n = pieces.len();
        let mut out = Vec::with_capacity(n);
        for (i, r) in pieces.into_iter().enumerate() {
            if n > 1 {
                notify(EngineEvent::Busy { id: job.id, segment: i + 1, of: n });
            }
            match transcribe_planned(eng, &job.audio[r], &job.language, &job.vocabulary) {
                Ok((t, _)) => out.push(t),
                Err(e) => return (Err(e), label, device),
            }
        }
        (Ok(out), label, device)
    }
}

fn engine_thread(dll: PathBuf, rx: Receiver<EngineCmd>, notify: Box<dyn Fn(EngineEvent) + Send>) {
    let whistle = match Engine::open(&dll, true) {
        Ok(e) => {
            log_info!("engine: runtime {} loaded from {}", ENGINE_VERSION, dll.display());
            Some(e)
        }
        Err(e) => {
            log_error!("engine: runtime failed: {e}");
            notify(EngineEvent::RuntimeFailed(e));
            None
        }
    };
    let mut engines = Engines::new(whistle);
    while let Ok(cmd) = rx.recv() {
        match cmd {
            EngineCmd::Shutdown => break,
            EngineCmd::LoadModel { path, sha256 } => {
                let Some(eng) = engines.whistle.as_mut() else { continue };
                if eng.loaded_model.as_deref() == Some(path.as_path()) {
                    notify(EngineEvent::ModelLoaded { load_ms: 0.0, warmup_ms: 0.0 });
                    continue;
                }
                let t = Instant::now();
                match eng.load_model(&path, sha256.as_deref()) {
                    Ok(()) => {
                        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
                        // Warm-up: first call allocates caches. 1 s of near-silence returns "" quickly.
                        let t2 = Instant::now();
                        let warm: Vec<f32> = (0..16_000).map(|i| ((i * 7919) % 13) as f32 * 1e-5).collect();
                        if let Err(e) = eng.transcribe(&warm, Some("en"), None) {
                            log_warn!("engine: warm-up failed: {e}");
                        }
                        let warmup_ms = t2.elapsed().as_secs_f64() * 1000.0;
                        log_info!("engine: model loaded in {load_ms:.0} ms (incl. SHA-256), warm-up {warmup_ms:.0} ms");
                        notify(EngineEvent::ModelLoaded { load_ms, warmup_ms });
                    }
                    Err(e) => {
                        log_error!("engine: model load failed: {e}");
                        notify(EngineEvent::ModelFailed(e));
                    }
                }
            }
            EngineCmd::LoadWhisper(req) => {
                let same = engines.whisper.as_ref().is_some_and(|w| w.device().is_gpu() || !req.gpu_only)
                    && engines.whisper_req.as_ref().is_some_and(|r| r.model_path == req.model_path && r.use_gpu == req.use_gpu);
                if !same {
                    engines.load_whisper(&req, &*notify);
                } else if let (Some(w), Some(rt)) = (&engines.whisper, &engines.whisper_rt) {
                    notify(EngineEvent::WhisperLoaded(WhisperStatus {
                        model_id: req.model_id.clone(),
                        label: req.label.clone(),
                        device: w.device(),
                        load_ms: 0.0,
                        runtime_dir: rt.dir.clone(),
                        gpus: rt.gpus.clone(),
                        restart_for_gpu: false,
                    }));
                }
            }
            EngineCmd::UnloadWhisper => {
                if engines.whisper.take().is_some() {
                    engines.whisper_req = None;
                    log_info!("whisper: model unloaded");
                    notify(EngineEvent::WhisperUnloaded);
                }
            }
            EngineCmd::Transcribe(job) => {
                let r = engines.run(&job, &*notify);
                notify(EngineEvent::Done(r));
            }
        }
    }
    log_info!("engine: thread exit");
}
