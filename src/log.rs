//! Tiny local file logger with size-based rotation.
//!
//! Privacy: callers never pass audio or transcript text here unless the user enabled
//! `log_transcripts` (off by default). Only lengths, timings and error messages are logged.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1024 * 1024;

struct Logger {
    path: PathBuf,
    file: File,
    written: u64,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);
static STDERR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn init(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("whistletype.log");
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let written = file.metadata().map(|m| m.len()).unwrap_or(0);
    *LOGGER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Logger { path: path.clone(), file, written });
    Ok(path)
}

/// Also mirror log lines to stderr (console tools).
pub fn mirror_to_stderr(on: bool) {
    STDERR.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn timestamp() -> String {
    // Local wall-clock time with milliseconds.
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

pub fn write(level: &str, args: fmt::Arguments) {
    let line = format!("{} [{}] {}\r\n", timestamp(), level, args);
    if STDERR.load(std::sync::atomic::Ordering::Relaxed) {
        eprint!("{line}");
    }
    let mut guard = LOGGER.lock().unwrap_or_else(|e| e.into_inner());
    let Some(lg) = guard.as_mut() else { return };
    if lg.written + line.len() as u64 > MAX_BYTES {
        let old = lg.path.with_extension("1.log");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&lg.path, &old);
        if let Ok(f) = OpenOptions::new().create(true).write(true).truncate(true).open(&lg.path) {
            lg.file = f;
            lg.written = 0;
        }
    }
    if lg.file.write_all(line.as_bytes()).is_ok() {
        lg.written += line.len() as u64;
    }
}

pub fn flush() {
    if let Some(lg) = LOGGER.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = lg.file.flush();
    }
}

#[macro_export]
macro_rules! log_info { ($($t:tt)*) => { $crate::log::write("INFO", format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_warn { ($($t:tt)*) => { $crate::log::write("WARN", format_args!($($t)*)) } }
#[macro_export]
macro_rules! log_error { ($($t:tt)*) => { $crate::log::write("ERROR", format_args!($($t)*)) } }
