//! wt-bench - developer tool: runs WAV files through the exact WhistleType pipeline
//! (speech gate → engine (Whistle or Whisper) → post-processing) and prints results + timings as JSON lines.
//!
//! usage:
//!   wt-bench [--engine whistle|whisper] [--model <whisper id or .bin path>] [--cpu] [--runtime <dir>]
//!            [--lang pl|auto|auto_pl_en|..] [--no-vocab] [--raw] [--repeat N] file.wav...
//!   wt-bench --devices | --record <seconds>

use std::path::PathBuf;
use std::time::Instant;

use whistletype::engine::{default_dll_path, Engine, Engines, TranscribeJob, WhisperRequest};
use whistletype::settings::Settings;
use whistletype::stt::EngineKind;
use whistletype::{model, models, pipeline, vad, wav};

/// (private MB, working set MB, CPU seconds) of this process.
fn process_stats() -> (f64, f64, f64) {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut _, c.cb);
        let _ = GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u);
    }
    let ft = |f: FILETIME| (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 1e7;
    (c.PrivateUsage as f64 / 1e6, c.WorkingSetSize as f64 / 1e6, ft(k) + ft(u))
}

fn record_mode(secs: f32) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    println!("default: {:?}", whistletype::audio::default_device());
    println!("devices: {:?}", whistletype::audio::list_devices());
    if secs <= 0.0 {
        return;
    }
    let t = Instant::now();
    let rec = whistletype::audio::Recorder::start(
        None,
        whistletype::audio::Purpose::Dictation { max_samples: 16_000 * 60 },
        std::sync::Arc::new(|e| println!("event: {e:?}")),
    );
    match rec {
        Ok(r) => {
            while !r.is_started() && t.elapsed().as_secs() < 3 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let open_ms = t.elapsed().as_secs_f64() * 1000.0;
            std::thread::sleep(std::time::Duration::from_secs_f32(secs));
            let c = r.stop();
            println!(
                "stream running after {open_ms:.1} ms, captured {:.2} s from {:?}, error {:?}, stats {:?}",
                c.samples.len() as f32 / 16000.0,
                c.device,
                c.error,
                vad::analyze(&c.samples)
            );
        }
        Err(e) => println!("start failed: {e}"),
    }
}

fn main() {
    whistletype::log::mirror_to_stderr(std::env::var_os("WT_VERBOSE").is_some());
    let mut args = std::env::args().skip(1);
    let mut settings = Settings { language: "pl".into(), ..Settings::default() };
    let mut files = Vec::new();
    let mut repeat = 1usize;
    let mut engine_kind = EngineKind::Whistle;
    let mut whisper_model: Option<String> = None;
    let mut use_gpu = true;
    let mut runtime: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--devices" => return record_mode(0.0),
            "--record" => return record_mode(args.next().and_then(|v| v.parse().ok()).unwrap_or(2.0)),
            "--engine" => {
                engine_kind = match args.next().as_deref() {
                    Some("whisper") => EngineKind::Whisper,
                    _ => EngineKind::Whistle,
                }
            }
            "--model" => whisper_model = args.next(),
            "--cpu" => use_gpu = false,
            "--runtime" => runtime = args.next().map(PathBuf::from),
            "--lang" => settings.language = args.next().expect("--lang value"),
            "--no-vocab" => settings.use_vocabulary = false,
            "--raw" => settings.raw_transcription = true,
            "--repeat" => repeat = args.next().and_then(|v| v.parse().ok()).expect("--repeat N"),
            _ => files.push(PathBuf::from(a)),
        }
    }

    let t = Instant::now();
    let before = process_stats();
    let mut whistle = match Engine::open(&default_dll_path(), true) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine: {e}");
            std::process::exit(2)
        }
    };
    if engine_kind == EngineKind::Whistle {
        if let Err(e) = whistle.load_model(&model::model_path(), Some(model::MODEL_SHA256)) {
            eprintln!("model: {e}");
            std::process::exit(3)
        }
    }
    let mut engines = Engines::new(Some(whistle));
    let events = std::cell::RefCell::new(Vec::new());
    let notify = |e: whistletype::engine::EngineEvent| events.borrow_mut().push(format!("{e:?}"));
    if engine_kind == EngineKind::Whisper {
        let id = whisper_model.clone().unwrap_or_else(|| models::DEFAULT_WHISPER_GPU.into());
        let (path, label) = match models::whisper_model(&id) {
            Some(m) => {
                // dev tree first, then the app's model folder
                let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("third_party").join("whisper-models").join(m.file);
                (if dev.exists() { dev } else { models::whisper_model_path(m) }, m.name.to_string())
            }
            None => (PathBuf::from(&id), id.clone()),
        };
        let runtime_dirs = match &runtime {
            Some(r) => vec![r.clone()],
            None => models::runtime_dirs(use_gpu),
        };
        engines.load_whisper(&WhisperRequest { model_id: id.clone(), model_path: path, label, runtime_dirs, use_gpu, gpu_only: false }, &notify);
    }
    let after_load = process_stats();
    println!(
        "{}",
        serde_json::json!({
            "event": "loaded",
            "engine": format!("{engine_kind:?}"),
            "load_ms": t.elapsed().as_secs_f64() * 1000.0,
            "private_mb": after_load.0,
            "load_private_mb": after_load.0 - before.0,
            "load_cpu_s": after_load.2 - before.2,
            "events": events.borrow().clone(),
        })
    );

    for f in &files {
        let w = match wav::read(f) {
            Ok(w) => w,
            Err(e) => {
                println!("{}", serde_json::json!({"file": f.display().to_string(), "error": e.to_string()}));
                continue;
            }
        };
        let audio = w.to_whistle_input();
        for run in 0..repeat {
            let t = Instant::now();
            let s0 = process_stats();
            let prepared = pipeline::prepare(audio.clone());
            let prep_ms = t.elapsed().as_secs_f64() * 1000.0;
            let mut text = String::new();
            let mut language = String::new();
            let mut error = None;
            let mut device = String::new();
            let mut engine_ms = 0.0;
            let mut segments = 0usize;
            if prepared.verdict == vad::Verdict::Speech {
                let job = TranscribeJob {
                    id: 0,
                    audio: prepared.audio.clone(),
                    engine: engine_kind,
                    language: settings.language_plan(),
                    vocabulary: settings.engine_vocabulary(),
                };
                let r = engines.run(&job, &notify);
                engine_ms = r.engine_ms;
                device = r.device.short().to_string();
                match r.parts {
                    Ok(parts) => {
                        segments = parts.len();
                        let fin = pipeline::finish(&parts, settings.raw_transcription, false);
                        text = fin.text;
                        language = fin.language;
                    }
                    Err(e) => error = Some(e.to_string()),
                }
            }
            let s1 = process_stats();
            let total_ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "{}",
                serde_json::json!({
                    "file": f.file_name().unwrap().to_string_lossy(),
                    "run": run,
                    "audio_s": audio.len() as f64 / 16000.0,
                    "verdict": format!("{:?}", prepared.verdict),
                    "device": device,
                    "prep_ms": prep_ms,
                    "engine_ms": engine_ms,
                    "segments": segments,
                    "total_ms": total_ms,
                    "cpu_s": s1.2 - s0.2,
                    "private_mb": s1.0,
                    "working_set_mb": s1.1,
                    "language": language,
                    "text": text,
                    "error": error,
                    "stats": {
                        "peak_dbfs": prepared.stats.peak_dbfs,
                        "floor_dbfs": prepared.stats.noise_floor_dbfs,
                        "p95_dbfs": prepared.stats.p95_dbfs,
                        "active_s": prepared.stats.active_s,
                        "voiced_s": prepared.stats.voiced_s
                    }
                })
            );
        }
    }
}
