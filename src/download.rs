//! One-time model download over HTTPS with WinHTTP (system TLS, system proxy settings).
//!
//! The file is written to `<dest>.part`, hashed while it streams, resumed with an HTTP Range request after
//! a dropped connection, verified against the pinned size + SHA-256, and only then renamed into place.
//! This is the only network code in WhistleType and it only runs when the user clicks "Download".

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

use crate::util::{hex, WStr};
use crate::{log_info, log_warn};

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadError {
    Cancelled,
    Network(String),
    Http(u32),
    Integrity(String),
    Disk(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::{fmt, t};
        let s = match self {
            DownloadError::Cancelled => t().err_dl_cancelled.to_string(),
            DownloadError::Network(m) => fmt(t().err_dl_network, &[("m", m)]),
            DownloadError::Http(c) => fmt(t().err_dl_http, &[("c", c)]),
            DownloadError::Integrity(m) => fmt(t().err_dl_integrity, &[("m", m)]),
            DownloadError::Disk(m) => fmt(t().err_dl_disk, &[("m", m)]),
        };
        f.write_str(&s)
    }
}

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn last_error(ctx: &str) -> DownloadError {
    let e = windows::core::Error::from_thread();
    DownloadError::Network(format!("{ctx}: {} (0x{:08X})", e.message().trim(), e.code().0 as u32))
}

/// One HTTP GET starting at byte `offset`; appends to `file`; returns when the body is complete.
#[allow(clippy::too_many_arguments)]
fn fetch_once(
    host: &str,
    path: &str,
    offset: u64,
    file: &mut std::fs::File,
    hasher: &mut Sha256,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<u64, DownloadError> {
    unsafe {
        let ua = WStr::new(&format!("WhistleType/{}", env!("CARGO_PKG_VERSION")));
        let session = Handle(WinHttpOpen(ua.pcwstr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let _ = WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 30_000);
        let h = WStr::new(host);
        let conn = Handle(WinHttpConnect(session.0, h.pcwstr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if conn.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let p = WStr::new(path);
        let req = Handle(WinHttpOpenRequest(conn.0, w!("GET"), p.pcwstr(), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        let headers = if offset > 0 { format!("Range: bytes={offset}-\r\n") } else { String::new() };
        let hw: Vec<u16> = headers.encode_utf16().collect();
        WinHttpSendRequest(req.0, if hw.is_empty() { None } else { Some(&hw) }, None, 0, 0, 0).map_err(|_| last_error("send"))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|_| last_error("receive"))?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|_| last_error("status"))?;
        let mut done = offset;
        match status {
            200 => {
                if offset > 0 {
                    // server ignored Range: start over
                    file.set_len(0).map_err(|e| DownloadError::Disk(e.to_string()))?;
                    file.seek(SeekFrom::Start(0)).map_err(|e| DownloadError::Disk(e.to_string()))?;
                    *hasher = Sha256::new();
                    done = 0;
                }
            }
            206 if offset > 0 => {}
            416 => return Ok(done),
            c => return Err(DownloadError::Http(c)),
        }
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let mut read = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read).map_err(|_| last_error("read"))?;
            if read == 0 {
                break;
            }
            let chunk = &buf[..read as usize];
            file.write_all(chunk).map_err(|e| DownloadError::Disk(e.to_string()))?;
            hasher.update(chunk);
            done += read as u64;
            if done > total {
                return Err(DownloadError::Integrity("file is larger than expected".into()));
            }
            progress(done, total);
        }
        Ok(done)
    }
}

/// Downloads `https://{host}{path}` to `dest`, verifying `size` and `sha256`.
pub fn download(
    host: &str,
    path: &str,
    dest: &Path,
    size: u64,
    sha256: &str,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| DownloadError::Disk(e.to_string()))?;
    }
    let part = dest.with_file_name(format!("{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")));
    // Resume a previous partial download: re-hash what is already there.
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut f) = std::fs::File::open(&part) {
        let mut b = vec![0u8; 1 << 16];
        loop {
            match f.read(&mut b) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&b[..n]);
                    have += n as u64;
                }
                Err(_) => {
                    have = 0;
                    hasher = Sha256::new();
                    break;
                }
            }
        }
        if have > size {
            have = 0;
            hasher = Sha256::new();
        }
    }
    let mut file = OpenOptions::new().create(true).append(false).write(true).truncate(false).open(&part).map_err(|e| DownloadError::Disk(e.to_string()))?;
    file.set_len(have).map_err(|e| DownloadError::Disk(e.to_string()))?;
    file.seek(SeekFrom::End(0)).map_err(|e| DownloadError::Disk(e.to_string()))?;
    if have > 0 {
        log_info!("download: resuming at {have} bytes");
    }
    let mut attempt = 0;
    while have < size {
        attempt += 1;
        match fetch_once(host, path, have, &mut file, &mut hasher, size, progress, cancel) {
            Ok(n) => {
                have = n;
                if have < size {
                    log_warn!("download: connection closed early at {have}/{size}");
                }
            }
            Err(DownloadError::Cancelled) => return Err(DownloadError::Cancelled),
            Err(e @ DownloadError::Http(_)) | Err(e @ DownloadError::Integrity(_)) | Err(e @ DownloadError::Disk(_)) => {
                drop(file);
                if matches!(e, DownloadError::Integrity(_)) {
                    let _ = std::fs::remove_file(&part);
                }
                return Err(e);
            }
            Err(e) => {
                log_warn!("download: attempt {attempt} failed: {e}");
                // re-sync the hasher position with the file (the failed chunk was written+hashed fully)
                have = file.stream_position().unwrap_or(have);
            }
        }
        if have < size {
            if attempt >= 8 {
                return Err(DownloadError::Network("too many connection failures".into()));
            }
            for _ in 0..(attempt * 10) {
                if cancel.load(Ordering::Relaxed) {
                    return Err(DownloadError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    file.flush().map_err(|e| DownloadError::Disk(e.to_string()))?;
    drop(file);
    let got = hex(&hasher.finalize());
    if got != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(DownloadError::Integrity(format!("SHA-256 {}… does not match", &got[..12])));
    }
    std::fs::rename(&part, dest).map_err(|e| DownloadError::Disk(e.to_string()))?;
    log_info!("download: complete and verified ({size} bytes)");
    Ok(())
}
