//! Binding to whisper.cpp (OpenAI Whisper inference, MIT) using the official prebuilt Windows binaries.
//!
//! The DLLs are loaded at runtime from a *runtime pack* folder:
//! * `whisper-cpu`  - shipped with the app (CPU backends for every x64 CPU generation),
//! * `whisper-cuda` - optional download (adds `ggml-cuda.dll` + NVIDIA cuBLAS/cudart; works on CPU too).
//!
//! Only one pack can be loaded per process (both contain `whisper.dll`/`ggml.dll`), so the pack is chosen once.
//! `whisper_full_params` is passed by value, so its Rust mirror must match the C layout exactly: the layout is
//! verified at load time against the library's own defaults, and Whisper is disabled if they do not match.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use windows::core::{s, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryExW, SetDllDirectoryW, LOAD_WITH_ALTERED_SEARCH_PATH};

use crate::engine::{EngineError, RawTranscript};
use crate::stt::{ComputeDevice, SpeechEngine};
use crate::util::WStr;
use crate::{log_info, log_warn};

/// whisper.cpp build the runtime packs come from (release b5130 = v1.9.4, commit 927cfce).
pub const WHISPER_CPP_BUILD: &str = "b5130";

// ---- C layouts (whisper.h @ 927cfce) ----------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct ContextParams {
    use_gpu: bool,
    flash_attn: bool,
    gpu_device: i32,
    dtw_token_timestamps: bool,
    dtw_aheads_preset: i32,
    dtw_n_top: i32,
    dtw_aheads_n_heads: usize,
    dtw_aheads_heads: *const c_void,
    dtw_mem_size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VadParams {
    threshold: f32,
    min_speech_duration_ms: i32,
    min_silence_duration_ms: i32,
    max_speech_duration_s: f32,
    speech_pad_ms: i32,
    samples_overlap: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FullParams {
    strategy: i32,
    n_threads: i32,
    n_max_text_ctx: i32,
    offset_ms: i32,
    duration_ms: i32,
    translate: bool,
    no_context: bool,
    no_timestamps: bool,
    single_segment: bool,
    print_special: bool,
    print_progress: bool,
    print_realtime: bool,
    print_timestamps: bool,
    token_timestamps: bool,
    thold_pt: f32,
    thold_ptsum: f32,
    max_len: i32,
    split_on_word: bool,
    max_tokens: i32,
    debug_mode: bool,
    audio_ctx: i32,
    tdrz_enable: bool,
    suppress_regex: *const c_char,
    initial_prompt: *const c_char,
    carry_initial_prompt: bool,
    prompt_tokens: *const i32,
    prompt_n_tokens: i32,
    language: *const c_char,
    detect_language: bool,
    suppress_blank: bool,
    suppress_nst: bool,
    temperature: f32,
    max_initial_ts: f32,
    length_penalty: f32,
    temperature_inc: f32,
    entropy_thold: f32,
    logprob_thold: f32,
    no_speech_thold: f32,
    greedy_best_of: i32,
    beam_size: i32,
    beam_patience: f32,
    new_segment_callback: *const c_void,
    new_segment_callback_user_data: *mut c_void,
    progress_callback: *const c_void,
    progress_callback_user_data: *mut c_void,
    encoder_begin_callback: *const c_void,
    encoder_begin_callback_user_data: *mut c_void,
    abort_callback: *const c_void,
    abort_callback_user_data: *mut c_void,
    logits_filter_callback: *const c_void,
    logits_filter_callback_user_data: *mut c_void,
    grammar_rules: *const c_void,
    n_grammar_rules: usize,
    i_start_rule: usize,
    grammar_penalty: f32,
    vad: bool,
    vad_model_path: *const c_char,
    vad_params: VadParams,
}

const SAMPLING_GREEDY: i32 = 0;
const SAMPLING_BEAM_SEARCH: i32 = 1;
const GGML_BACKEND_DEVICE_TYPE_GPU: i32 = 1;

type FnCtxDefaultsByRef = unsafe extern "C" fn() -> *mut ContextParams;
type FnFreeCtxParams = unsafe extern "C" fn(*mut ContextParams);
type FnFullDefaultsByRef = unsafe extern "C" fn(i32) -> *mut FullParams;
type FnFreeParams = unsafe extern "C" fn(*mut FullParams);
type FnInitFromFile = unsafe extern "C" fn(*const c_char, ContextParams) -> *mut c_void;
type FnFree = unsafe extern "C" fn(*mut c_void);
type FnFull = unsafe extern "C" fn(*mut c_void, FullParams, *const f32, i32) -> i32;
type FnNSegments = unsafe extern "C" fn(*mut c_void) -> i32;
type FnSegmentText = unsafe extern "C" fn(*mut c_void, i32) -> *const c_char;
type FnLangId = unsafe extern "C" fn(*mut c_void) -> i32;
type FnLangStr = unsafe extern "C" fn(i32) -> *const c_char;
type FnVersion = unsafe extern "C" fn() -> *const c_char;
type LogCallback = unsafe extern "C" fn(i32, *const c_char, *mut c_void);
type FnLogSet = unsafe extern "C" fn(Option<LogCallback>, *mut c_void);
type FnLoadAllFromPath = unsafe extern "C" fn(*const c_char);
type FnDevCount = unsafe extern "C" fn() -> usize;
type FnDevGet = unsafe extern "C" fn(usize) -> *mut c_void;
type FnDevType = unsafe extern "C" fn(*mut c_void) -> i32;
type FnDevDescription = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type FnDevMemory = unsafe extern "C" fn(*mut c_void, *mut usize, *mut usize);

#[derive(Debug, Clone, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// The loaded whisper.cpp runtime (process-wide, loaded once).
pub struct WhisperRuntime {
    pub dir: PathBuf,
    pub version: String,
    /// GPUs the loaded backends can use (empty for the CPU pack or without an NVIDIA driver).
    pub gpus: Vec<GpuInfo>,
    ctx_defaults: FnCtxDefaultsByRef,
    free_ctx_params: FnFreeCtxParams,
    full_defaults: FnFullDefaultsByRef,
    free_params: FnFreeParams,
    init_from_file: FnInitFromFile,
    free: FnFree,
    full: FnFull,
    n_segments: FnNSegments,
    segment_text: FnSegmentText,
    lang_id: FnLangId,
    lang_str: FnLangStr,
}

unsafe fn sym<T>(m: HMODULE, name: PCSTR) -> Result<T, EngineError> {
    match unsafe { GetProcAddress(m, name) } {
        Some(f) => Ok(unsafe { std::mem::transmute_copy(&f) }),
        None => Err(EngineError::RuntimeLoad(format!("missing export {}", unsafe { name.display() }))),
    }
}

/// whisper.cpp / ggml log lines go to our log (warnings and errors only; the info chatter is dropped).
unsafe extern "C" fn log_callback(level: i32, text: *const c_char, _user: *mut c_void) {
    if text.is_null() || level < 3 {
        return;
    }
    let s = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let s = s.trim_end();
    if !s.is_empty() {
        log_warn!("whisper.cpp: {s}");
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

impl WhisperRuntime {
    /// Loads `whisper.dll` and all ggml backends found in `dir`, then checks the struct layouts.
    pub fn open(dir: &Path) -> Result<WhisperRuntime, EngineError> {
        // LOAD_WITH_ALTERED_SEARCH_PATH needs an absolute path
        let dir = &std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        let dll = dir.join("whisper.dll");
        if !dll.exists() {
            return Err(EngineError::RuntimeMissing(dll));
        }
        // An out-of-tree backend named by this variable would be loaded into our process - never honour it.
        std::env::remove_var("GGML_BACKEND_PATH");
        unsafe {
            // Dependencies (ggml*.dll, cuBLAS, VC runtime) are resolved from the pack folder: ggml loads its
            // backends with plain LoadLibraryW, which honours SetDllDirectory.
            let wdir = WStr::from_path(dir);
            SetDllDirectoryW(wdir.pcwstr()).map_err(|e| EngineError::RuntimeLoad(e.message()))?;
            let wdll = WStr::from_path(&dll);
            let module = LoadLibraryExW(wdll.pcwstr(), None, LOAD_WITH_ALTERED_SEARCH_PATH)
                .map_err(|e| EngineError::RuntimeLoad(format!("whisper.dll: {}", e.message())))?;
            let ggml_path = WStr::from_path(&dir.join("ggml.dll"));
            let ggml = GetModuleHandleW(ggml_path.pcwstr()).map_err(|e| EngineError::RuntimeLoad(format!("ggml.dll: {}", e.message())))?;
            let base_path = WStr::from_path(&dir.join("ggml-base.dll"));
            let base = GetModuleHandleW(base_path.pcwstr()).map_err(|e| EngineError::RuntimeLoad(format!("ggml-base.dll: {}", e.message())))?;

            let log_set: FnLogSet = sym(module, s!("whisper_log_set"))?;
            log_set(Some(log_callback), std::ptr::null_mut());
            let version: FnVersion = sym(module, s!("whisper_version"))?;
            let load_all: FnLoadAllFromPath = sym(ggml, s!("ggml_backend_load_all_from_path"))?;
            let dir_c = CString::new(dir.to_string_lossy().as_bytes()).map_err(|e| EngineError::RuntimeLoad(e.to_string()))?;
            load_all(dir_c.as_ptr());

            let dev_count: FnDevCount = sym(ggml, s!("ggml_backend_dev_count"))?;
            let dev_get: FnDevGet = sym(ggml, s!("ggml_backend_dev_get"))?;
            let dev_type: FnDevType = sym(base, s!("ggml_backend_dev_type"))?;
            let dev_desc: FnDevDescription = sym(base, s!("ggml_backend_dev_description"))?;
            let dev_mem: FnDevMemory = sym(base, s!("ggml_backend_dev_memory"))?;
            let mut gpus = Vec::new();
            for i in 0..dev_count() {
                let d = dev_get(i);
                if !d.is_null() && dev_type(d) == GGML_BACKEND_DEVICE_TYPE_GPU {
                    let (mut free, mut total) = (0usize, 0usize);
                    dev_mem(d, &mut free, &mut total);
                    gpus.push(GpuInfo { name: cstr(dev_desc(d)), total_bytes: total as u64, free_bytes: free as u64 });
                }
            }
            let rt = WhisperRuntime {
                dir: dir.to_path_buf(),
                version: cstr(version()),
                gpus,
                ctx_defaults: sym(module, s!("whisper_context_default_params_by_ref"))?,
                free_ctx_params: sym(module, s!("whisper_free_context_params"))?,
                full_defaults: sym(module, s!("whisper_full_default_params_by_ref"))?,
                free_params: sym(module, s!("whisper_free_params"))?,
                init_from_file: sym(module, s!("whisper_init_from_file_with_params"))?,
                free: sym(module, s!("whisper_free"))?,
                full: sym(module, s!("whisper_full"))?,
                n_segments: sym(module, s!("whisper_full_n_segments"))?,
                segment_text: sym(module, s!("whisper_full_get_segment_text"))?,
                lang_id: sym(module, s!("whisper_full_lang_id"))?,
                lang_str: sym(module, s!("whisper_lang_str"))?,
            };
            rt.check_layout()?;
            log_info!(
                "whisper: runtime {} from {} (GPUs: {})",
                rt.version,
                dir.display(),
                if rt.gpus.is_empty() { "none".to_string() } else { rt.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ") }
            );
            Ok(rt)
        }
    }

    /// Verifies that our struct mirrors match the DLL by comparing with the library's own default values,
    /// including the last fields - a size or order mismatch would shift them.
    fn check_layout(&self) -> Result<(), EngineError> {
        unsafe {
            let c = (self.ctx_defaults)();
            if c.is_null() {
                return Err(EngineError::RuntimeLoad("whisper_context_default_params_by_ref returned NULL".into()));
            }
            let cp = std::ptr::read(c);
            (self.free_ctx_params)(c);
            let ctx_ok = cp.use_gpu && cp.gpu_device == 0 && cp.dtw_n_top == -1 && cp.dtw_aheads_n_heads == 0 && cp.dtw_mem_size == 128 * 1024 * 1024;
            let p = (self.full_defaults)(SAMPLING_BEAM_SEARCH);
            if p.is_null() {
                return Err(EngineError::RuntimeLoad("whisper_full_default_params_by_ref returned NULL".into()));
            }
            let fp = std::ptr::read(p);
            (self.free_params)(p);
            let lang = cstr(fp.language);
            let full_ok = fp.strategy == SAMPLING_BEAM_SEARCH
                && fp.n_max_text_ctx == 16384
                && fp.no_context
                && (fp.thold_pt - 0.01).abs() < 1e-6
                && lang == "en"
                && (fp.temperature_inc - 0.2).abs() < 1e-6
                && (fp.entropy_thold - 2.4).abs() < 1e-6
                && (fp.no_speech_thold - 0.6).abs() < 1e-6
                && fp.beam_size == 5
                && (fp.grammar_penalty - 100.0).abs() < 1e-3
                && !fp.vad
                && (fp.vad_params.threshold - 0.5).abs() < 1e-6
                && fp.vad_params.min_speech_duration_ms == 250
                && fp.vad_params.min_silence_duration_ms == 100
                && fp.vad_params.speech_pad_ms == 30
                && (fp.vad_params.samples_overlap - 0.1).abs() < 1e-6;
            if ctx_ok && full_ok {
                Ok(())
            } else {
                Err(EngineError::RuntimeLoad(format!(
                    "whisper.cpp ABI does not match this WhistleType build (context params ok: {ctx_ok}, full params ok: {full_ok})"
                )))
            }
        }
    }

    pub fn best_gpu(&self) -> Option<&GpuInfo> {
        self.gpus.iter().max_by_key(|g| g.total_bytes)
    }
}

/// A loaded Whisper model.
pub struct WhisperEngine {
    rt: std::rc::Rc<WhisperRuntime>,
    ctx: *mut c_void,
    label: String,
    device: ComputeDevice,
    threads: i32,
    beam: bool,
}

impl Drop for WhisperEngine {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { (self.rt.free)(self.ctx) };
        }
    }
}

/// Physical CPU cores (whisper.cpp scales poorly onto SMT siblings).
fn physical_cores() -> i32 {
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) as i32;
    (logical / 2).clamp(2, 8)
}

impl WhisperEngine {
    /// Loads `model` (ggml .bin). With `use_gpu` and a GPU present it runs there; if GPU initialisation fails
    /// (e.g. out of VRAM) it falls back to the CPU automatically.
    /// With `gpu_only` a failed GPU initialisation is an error instead of a CPU fallback.
    pub fn load(rt: std::rc::Rc<WhisperRuntime>, model: &Path, label: &str, use_gpu: bool, gpu_only: bool) -> Result<WhisperEngine, EngineError> {
        if !model.exists() {
            return Err(EngineError::ModelMissing(model.to_path_buf()));
        }
        let path = CString::new(model.to_string_lossy().as_bytes()).map_err(|e| EngineError::ModelLoad(e.to_string()))?;
        let gpu = if use_gpu { rt.best_gpu().cloned() } else { None };
        let ctx_for = |gpu_on: bool| unsafe {
            let c = (rt.ctx_defaults)();
            if c.is_null() {
                return std::ptr::null_mut();
            }
            let mut cp = std::ptr::read(c);
            (rt.free_ctx_params)(c);
            cp.use_gpu = gpu_on;
            cp.flash_attn = gpu_on;
            (rt.init_from_file)(path.as_ptr(), cp)
        };
        let mut device = ComputeDevice::Cpu;
        let mut ctx = std::ptr::null_mut();
        if let Some(g) = &gpu {
            ctx = ctx_for(true);
            if ctx.is_null() {
                log_warn!("whisper: GPU initialisation failed on {}; falling back to the CPU", g.name);
            } else {
                device = ComputeDevice::Gpu(g.name.clone());
            }
        }
        if ctx.is_null() && gpu_only {
            return Err(EngineError::ModelLoad(format!("{label}: the GPU could not be used")));
        }
        if ctx.is_null() {
            ctx = ctx_for(false);
        }
        if ctx.is_null() {
            return Err(EngineError::ModelLoad(format!("whisper.cpp could not load {}", model.display())));
        }
        let gpu_on = device.is_gpu();
        Ok(WhisperEngine {
            rt,
            ctx,
            label: label.to_string(),
            device,
            // GPU: a few CPU threads are enough for the host side. CPU: physical cores.
            threads: if gpu_on { 4 } else { physical_cores() },
            // Beam search (5) on the GPU for quality; greedy on the CPU for speed.
            beam: gpu_on,
        })
    }

    /// Builds the Whisper prompt from the custom vocabulary (Whisper has no keyword biasing; a prompt with the
    /// terms is the documented way to steer spelling of names and jargon).
    pub fn vocabulary_prompt(vocabulary: &[String]) -> Option<String> {
        if vocabulary.is_empty() {
            return None;
        }
        let mut s = String::new();
        for v in vocabulary {
            if s.len() + v.len() + 2 > 600 {
                break; // Whisper uses at most ~224 prompt tokens
            }
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(v);
        }
        s.push('.');
        Some(s)
    }
}

impl SpeechEngine for WhisperEngine {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn device(&self) -> ComputeDevice {
        self.device.clone()
    }

    fn max_samples(&self) -> Option<usize> {
        None // whisper.cpp windows long audio itself
    }

    fn transcribe(&mut self, pcm: &[f32], language: Option<&str>, vocabulary: &[String]) -> Result<RawTranscript, EngineError> {
        if pcm.is_empty() {
            return Ok(RawTranscript::default());
        }
        let lang = CString::new(language.unwrap_or("auto")).unwrap_or_default();
        let prompt = Self::vocabulary_prompt(vocabulary).and_then(|p| CString::new(p.replace('\0', "")).ok());
        let rc = unsafe {
            let p = (self.rt.full_defaults)(if self.beam { SAMPLING_BEAM_SEARCH } else { SAMPLING_GREEDY });
            let mut fp = std::ptr::read(p);
            (self.rt.free_params)(p);
            fp.n_threads = self.threads;
            fp.no_context = true;
            fp.no_timestamps = true;
            fp.print_special = false;
            fp.print_progress = false;
            fp.print_realtime = false;
            fp.print_timestamps = false;
            fp.translate = false;
            fp.language = lang.as_ptr();
            fp.detect_language = false;
            fp.suppress_nst = true; // no "[MUSIC]", "(applause)" and similar non-speech tokens
            fp.initial_prompt = prompt.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
            fp.carry_initial_prompt = false;
            (self.rt.full)(self.ctx, fp, pcm.as_ptr(), pcm.len() as i32)
        };
        if rc != 0 {
            return Err(EngineError::Transcribe(format!("whisper_full returned {rc}")));
        }
        let mut text = String::new();
        let n = unsafe { (self.rt.n_segments)(self.ctx) };
        for i in 0..n {
            text.push_str(&cstr(unsafe { (self.rt.segment_text)(self.ctx, i) }));
        }
        let text = strip_subtitle_credits(&text);
        let lang_id = unsafe { (self.rt.lang_id)(self.ctx) };
        let detected = if lang_id >= 0 { cstr(unsafe { (self.rt.lang_str)(lang_id) }) } else { String::new() };
        Ok(RawTranscript {
            text: text.trim().to_string(),
            language: if text.trim().is_empty() { String::new() } else { detected },
            ttft_ms: 0.0,
            decode_tps: 0.0,
        })
    }
}

/// Whisper was trained on subtitles and sometimes "hears" their credits in quiet tails
/// ("Napisy stworzone przez społeczność Amara.org", "Subtitles by ..."). Sentences that contain such a credit
/// are removed; nothing else is touched.
pub fn strip_subtitle_credits(text: &str) -> String {
    const MARKERS: &[&str] = &["amara.org", "napisy stworzone przez", "napisy wykonane przez", "subtitles by", "transcribed by"];
    // sentences end at . ! ? followed by whitespace (so "Amara.org" stays one piece)
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let next_ws = chars.get(k + 1).map_or(true, |&(_, n)| n.is_whitespace());
        if matches!(c, '.' | '!' | '?') && next_ws {
            sentences.push(&text[start..i + c.len_utf8()]);
            start = i + c.len_utf8();
        }
    }
    sentences.push(&text[start..]);
    let kept: Vec<&str> = sentences
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !MARKERS.iter().any(|m| s.to_lowercase().contains(m)))
        .collect();
    kept.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_credits_are_removed() {
        assert_eq!(strip_subtitle_credits("Zrób commit. Napisy stworzone przez społeczność Amara.org"), "Zrób commit.");
        assert_eq!(strip_subtitle_credits("Napisy stworzone przez społeczność Amara.org"), "");
        assert_eq!(strip_subtitle_credits("Otwórz plik README. Dziękuję."), "Otwórz plik README. Dziękuję.");
    }

    #[test]
    fn struct_sizes_match_x64_c_layout() {
        // whisper_context_params: 3 bools/ints, the aheads pair and a size_t -> 48 bytes on x64
        assert_eq!(std::mem::size_of::<ContextParams>(), 48);
        assert_eq!(std::mem::size_of::<VadParams>(), 24);
        // sizeof / offsetof printed by MSVC from the real whisper.h @ 927cfce
        assert_eq!(std::mem::size_of::<FullParams>(), 304);
        assert_eq!(std::mem::offset_of!(FullParams, initial_prompt), 72);
        assert_eq!(std::mem::offset_of!(FullParams, language), 104);
        assert_eq!(std::mem::offset_of!(FullParams, greedy_best_of), 144);
        assert_eq!(std::mem::offset_of!(FullParams, beam_size), 148);
        assert_eq!(std::mem::offset_of!(FullParams, new_segment_callback), 160);
        assert_eq!(std::mem::offset_of!(FullParams, grammar_penalty), 264);
        assert_eq!(std::mem::offset_of!(FullParams, vad), 268);
        assert_eq!(std::mem::offset_of!(FullParams, vad_params), 280);
    }

    #[test]
    fn prompt_from_vocabulary() {
        assert_eq!(WhisperEngine::vocabulary_prompt(&[]), None);
        let v: Vec<String> = ["Claude Code", "Gradle"].iter().map(|s| s.to_string()).collect();
        assert_eq!(WhisperEngine::vocabulary_prompt(&v).unwrap(), "Claude Code, Gradle.");
        let long: Vec<String> = (0..300).map(|i| format!("Word{i}")).collect();
        assert!(WhisperEngine::vocabulary_prompt(&long).unwrap().len() <= 602);
    }
}
