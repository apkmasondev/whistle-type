//! Where WhistleType keeps its files.
//!
//! * settings:  %APPDATA%\WhistleType\settings.json            (roams with the user profile)
//! * model:     %LOCALAPPDATA%\WhistleType\models\whistle-2.0.0\whistle.cact
//! * logs:      %LOCALAPPDATA%\WhistleType\logs\
//!
//! Portable mode: if a file named `WhistleType.portable` sits next to the executable, everything lives in
//! `<exe dir>\data\` instead. `WHISTLETYPE_DATA_DIR` overrides both (used by tests).

use std::path::PathBuf;

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn override_dir() -> Option<PathBuf> {
    if let Some(d) = env_dir("WHISTLETYPE_DATA_DIR") {
        return Some(d);
    }
    let exe = exe_dir();
    if exe.join("WhistleType.portable").exists() {
        return Some(exe.join("data"));
    }
    None
}

pub fn is_portable() -> bool {
    override_dir().is_some()
}

/// Roaming data (settings).
pub fn config_dir() -> PathBuf {
    override_dir()
        .or_else(|| env_dir("APPDATA").map(|d| d.join("WhistleType")))
        .unwrap_or_else(|| exe_dir().join("data"))
}

/// Machine-local data (model, logs).
pub fn local_dir() -> PathBuf {
    override_dir()
        .or_else(|| env_dir("LOCALAPPDATA").map(|d| d.join("WhistleType")))
        .unwrap_or_else(|| exe_dir().join("data"))
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn logs_dir() -> PathBuf {
    local_dir().join("logs")
}

pub fn models_dir() -> PathBuf {
    local_dir().join("models")
}
