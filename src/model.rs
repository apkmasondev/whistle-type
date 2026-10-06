//! The Whistle model file: where it lives, what it should be, and whether it is installed.

use std::path::PathBuf;

pub const MODEL_NAME: &str = "Whistle";
pub const MODEL_VERSION: &str = "2.0.0";
pub const MODEL_FILE: &str = "whistle.cact";
pub const MODEL_SIZE: u64 = 16_919_407;
pub const MODEL_SHA256: &str = "b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb";
pub const MODEL_REPO: &str = "Cactus-Compute/whistle";
/// Immutable Hugging Face commit, so the downloaded bytes can never change under us.
pub const MODEL_REVISION: &str = "b358ddadd89b7a713b5aa131f23032d3cca1b251";
pub const MODEL_LICENSE: &str = "Apache-2.0";
pub const MODEL_HOST: &str = "huggingface.co";

pub fn model_url_path() -> String {
    format!("/{MODEL_REPO}/resolve/{MODEL_REVISION}/{MODEL_FILE}")
}

pub fn model_url() -> String {
    format!("https://{MODEL_HOST}{}", model_url_path())
}

pub fn model_page_url() -> String {
    format!("https://{MODEL_HOST}/{MODEL_REPO}")
}

pub fn model_dir() -> PathBuf {
    crate::paths::models_dir().join(format!("whistle-{MODEL_VERSION}"))
}

pub fn model_path() -> PathBuf {
    if let Some(p) = std::env::var_os("WHISTLETYPE_MODEL") {
        return PathBuf::from(p);
    }
    model_dir().join(MODEL_FILE)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelFileState {
    Missing,
    /// Present but wrong size (interrupted copy, wrong file).
    WrongSize(u64),
    /// Present with the right size; the SHA-256 is checked when the engine loads it.
    Present,
}

pub fn file_state() -> ModelFileState {
    match std::fs::metadata(model_path()) {
        Err(_) => ModelFileState::Missing,
        Ok(m) if m.len() != MODEL_SIZE => ModelFileState::WrongSize(m.len()),
        Ok(_) => ModelFileState::Present,
    }
}

/// Copies a user-selected `whistle.cact` into place after verifying it (offline installation).
pub fn import_from(src: &std::path::Path) -> Result<(), String> {
    let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
    if meta.len() != MODEL_SIZE {
        return Err(crate::i18n::fmt(
            crate::i18n::t().err_import_size,
            &[("ver", &MODEL_VERSION), ("exp", &MODEL_SIZE), ("got", &meta.len())],
        ));
    }
    let sha = crate::util::sha256_file(src).map_err(|e| e.to_string())?;
    if sha != MODEL_SHA256 {
        return Err(crate::i18n::fmt(crate::i18n::t().err_import_sha, &[("ver", &MODEL_VERSION)]));
    }
    let dest = model_path();
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("cact.part");
    std::fs::copy(src, &tmp).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(())
}
