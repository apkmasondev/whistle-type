//! Clipboard snapshot / restore and privacy-aware text placement.
//!
//! All functions must be called on the thread that owns `owner` (the inserter thread).

use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{CopyEnhMetaFileW, DeleteEnhMetaFile, HENHMETAFILE};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_ENHMETAFILE, CF_UNICODETEXT};

use crate::log_warn;

/// Snapshots larger than this are not kept in memory (huge images / spreadsheets).
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

pub enum Item {
    Global { format: u32, data: Vec<u8> },
    EnhMeta(HENHMETAFILE),
}

pub struct Snapshot {
    pub items: Vec<Item>,
    /// Some formats could not be saved (too large, GDI-only, unreadable).
    pub incomplete: bool,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        for it in self.items.drain(..) {
            if let Item::EnhMeta(h) = it {
                unsafe {
                    let _ = DeleteEnhMetaFile(Some(h));
                }
            }
        }
    }
}

/// Holds the clipboard open; closes it on drop.
pub struct Open;

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Opens the clipboard, retrying while another process holds it.
pub fn open(owner: HWND) -> Result<Open, String> {
    let mut last = String::new();
    for _ in 0..40 {
        match unsafe { OpenClipboard(Some(owner)) } {
            Ok(()) => return Ok(Open),
            Err(e) => last = e.message(),
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Err(format!("clipboard is busy ({last})"))
}

fn registered(name: &str) -> u32 {
    let w = crate::util::wide(name);
    unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) }
}

fn is_restorable(format: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_OWNERDISPLAY: u32 = 0x80;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8E;
    !matches!(
        format,
        CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_OWNERDISPLAY | CF_DSPBITMAP | CF_DSPMETAFILEPICT | CF_DSPENHMETAFILE
    ) && !(0x0200..=0x03FF).contains(&format) // CF_PRIVATEFIRST..CF_GDIOBJLAST: not HGLOBAL
}

/// Copies every restorable format currently on the clipboard.
pub fn snapshot(owner: HWND) -> Result<Snapshot, String> {
    let _open = open(owner)?;
    let mut snap = Snapshot { items: Vec::new(), incomplete: false };
    let mut total = 0usize;
    let mut fmt = 0u32;
    loop {
        fmt = unsafe { EnumClipboardFormats(fmt) };
        if fmt == 0 {
            break;
        }
        if fmt == CF_ENHMETAFILE.0 as u32 {
            if let Ok(h) = unsafe { GetClipboardData(fmt) } {
                let copy = unsafe { CopyEnhMetaFileW(HENHMETAFILE(h.0), PCWSTR::null()) };
                if !copy.is_invalid() {
                    snap.items.push(Item::EnhMeta(copy));
                }
            }
            continue;
        }
        if !is_restorable(fmt) {
            snap.incomplete = true;
            continue;
        }
        let Ok(h) = (unsafe { GetClipboardData(fmt) }) else {
            snap.incomplete = true;
            continue;
        };
        let hg = HGLOBAL(h.0);
        let size = unsafe { GlobalSize(hg) };
        if size == 0 {
            // e.g. a marker format with no data
            snap.items.push(Item::Global { format: fmt, data: Vec::new() });
            continue;
        }
        if total + size > MAX_SNAPSHOT_BYTES {
            snap.incomplete = true;
            continue;
        }
        let p = unsafe { GlobalLock(hg) };
        if p.is_null() {
            snap.incomplete = true;
            continue;
        }
        let data = unsafe { std::slice::from_raw_parts(p as *const u8, size) }.to_vec();
        unsafe {
            let _ = GlobalUnlock(hg);
        }
        total += size;
        snap.items.push(Item::Global { format: fmt, data });
    }
    Ok(snap)
}

fn alloc_global(bytes: &[u8]) -> Result<HGLOBAL, String> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(|e| e.message())?;
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

fn set_global(format: u32, bytes: &[u8]) -> Result<(), String> {
    let h = alloc_global(bytes)?;
    match unsafe { SetClipboardData(format, Some(HANDLE(h.0))) } {
        Ok(_) => Ok(()), // the system owns the memory now
        Err(e) => {
            unsafe {
                let _ = GlobalFree(Some(h));
            }
            Err(e.message())
        }
    }
}

/// Tells clipboard history (Win+V), cloud clipboard and well-behaved clipboard managers to ignore this
/// content. Must be called while the clipboard is open, after EmptyClipboard.
fn mark_private() {
    let zero = 0u32.to_le_bytes();
    let _ = set_global(registered("ExcludeClipboardContentFromMonitorProcessing"), &zero);
    let _ = set_global(registered("CanIncludeInClipboardHistory"), &zero);
    let _ = set_global(registered("CanUploadToCloudClipboard"), &zero);
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect()
}

/// Puts `text` on the clipboard with delayed rendering: the data is produced in `render_text` when an
/// application actually reads it - this is how WhistleType knows the paste happened.
pub fn set_text_delayed(owner: HWND) -> Result<(), String> {
    let _open = open(owner)?;
    unsafe { EmptyClipboard() }.map_err(|e| e.message())?;
    // Delayed rendering: a NULL handle is the documented success value, which the `windows` crate reports
    // as an error with code 0. Only a real error code is a failure.
    if let Err(e) = unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, None) } {
        if e.code().is_err() {
            return Err(e.message());
        }
    }
    mark_private();
    Ok(())
}

/// Called from WM_RENDERFORMAT (clipboard already opened by the reader).
pub fn render_text(text: &str) -> Result<(), String> {
    set_global(CF_UNICODETEXT.0 as u32, &utf16z(text))
}

/// Called from WM_RENDERALLFORMATS (we must open the clipboard ourselves).
pub fn render_all(owner: HWND, text: &str) {
    if let Ok(_open) = open(owner) {
        if unsafe { GetClipboardOwner() }.ok() == Some(owner) {
            let _ = render_text(text);
        }
    }
}

/// Puts `text` on the clipboard immediately (used when the user should paste manually).
pub fn set_text_now(owner: HWND, text: &str, private: bool) -> Result<(), String> {
    let _open = open(owner)?;
    unsafe { EmptyClipboard() }.map_err(|e| e.message())?;
    set_global(CF_UNICODETEXT.0 as u32, &utf16z(text))?;
    if private {
        mark_private();
    }
    Ok(())
}

pub fn we_own_clipboard(owner: HWND) -> bool {
    unsafe { GetClipboardOwner() }.ok() == Some(owner)
}

/// Puts the snapshot back. The restored copy is kept out of clipboard history so it does not show up twice.
pub fn restore(owner: HWND, snap: &Snapshot) -> Result<(), String> {
    let _open = open(owner)?;
    unsafe { EmptyClipboard() }.map_err(|e| e.message())?;
    for it in &snap.items {
        match it {
            Item::Global { format, data } => {
                if let Err(e) = set_global(*format, data) {
                    log_warn!("clipboard: restoring format {format} failed: {e}");
                }
            }
            Item::EnhMeta(h) => unsafe {
                let copy = CopyEnhMetaFileW(*h, PCWSTR::null());
                if !copy.is_invalid() && SetClipboardData(CF_ENHMETAFILE.0 as u32, Some(HANDLE(copy.0))).is_err() {
                    let _ = DeleteEnhMetaFile(Some(copy));
                }
            },
        }
    }
    if !snap.items.is_empty() {
        let history = registered("CanIncludeInClipboardHistory");
        if !snap.items.iter().any(|i| matches!(i, Item::Global { format, .. } if *format == history)) {
            let _ = set_global(history, &0u32.to_le_bytes());
        }
    }
    Ok(())
}

/// Reads the clipboard as text (tests and diagnostics).
pub fn read_text(owner: HWND) -> Option<String> {
    let _open = open(owner).ok()?;
    let h = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
    let hg = HGLOBAL(h.0);
    let p = unsafe { GlobalLock(hg) } as *const u16;
    if p.is_null() {
        return None;
    }
    let n = unsafe { GlobalSize(hg) } / 2;
    let s = crate::util::from_wide(unsafe { std::slice::from_raw_parts(p, n) });
    unsafe {
        let _ = GlobalUnlock(hg);
    }
    Some(s)
}
