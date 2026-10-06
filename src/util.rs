//! Small helpers shared by the Win32 modules.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use windows::core::PCWSTR;

/// NUL-terminated UTF-16 string that keeps its buffer alive while a `PCWSTR` is in use.
pub struct WStr(Vec<u16>);

impl WStr {
    pub fn new(s: &str) -> Self {
        WStr(s.encode_utf16().chain(std::iter::once(0)).collect())
    }
    pub fn from_path(p: &Path) -> Self {
        use std::os::windows::ffi::OsStrExt;
        WStr(p.as_os_str().encode_wide().chain(std::iter::once(0)).collect())
    }
    pub fn pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer (possibly NUL-terminated) to String.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_bytes(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Pack two i16 into an LPARAM-like value (used for posting small payloads).
pub fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}
pub fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

/// Decimal units, matching how the model size is published ("16.9 MB"; "16,9 MB" in the Polish UI).
pub fn format_bytes(n: u64) -> String {
    if n >= 1_000_000_000 {
        let s = format!("{:.2} GB", n as f64 / 1e9);
        if crate::i18n::current() == crate::i18n::UiLang::Pl {
            s.replace('.', ",")
        } else {
            s
        }
    } else if n >= 1_000_000 {
        let s = format!("{:.1} MB", n as f64 / 1e6);
        if crate::i18n::current() == crate::i18n::UiLang::Pl {
            s.replace('.', ",")
        } else {
            s
        }
    } else if n >= 1_000 {
        format!("{:.0} KB", n as f64 / 1e3)
    } else {
        format!("{n} B")
    }
}
