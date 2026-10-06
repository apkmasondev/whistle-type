//! "Start with Windows" via HKCU\Software\Microsoft\Windows\CurrentVersion\Run (per user, no admin).

use windows::core::w;
use windows::Win32::System::Registry::*;

use crate::util::{from_wide, WStr};

const VALUE: &str = "WhistleType";

fn command_line() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("\"{}\" --background", exe.display())
}

fn open(write: bool) -> Option<HKEY> {
    let mut key = HKEY::default();
    let access = if write { KEY_READ | KEY_SET_VALUE } else { KEY_READ };
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), Some(0), access, &mut key) };
    r.is_ok().then_some(key)
}

fn read_value() -> Option<String> {
    let key = open(false)?;
    let name = WStr::new(VALUE);
    let mut buf = vec![0u16; 2048];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegQueryValueExW(key, name.pcwstr(), None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut len)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r.is_ok().then(|| from_wide(&buf[..(len as usize / 2).min(buf.len())]))
}

/// True when the Run entry exists (whatever path it points to).
pub fn is_enabled() -> bool {
    read_value().is_some()
}

/// True when the Run entry points at this executable.
pub fn points_here() -> bool {
    read_value().is_some_and(|v| v.eq_ignore_ascii_case(&command_line()))
}

pub fn set(enabled: bool) -> Result<(), String> {
    let key = open(true).ok_or("cannot open the Run key")?;
    let name = WStr::new(VALUE);
    let r = if enabled {
        let cmd: Vec<u16> = command_line().encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2) };
        unsafe { RegSetValueExW(key, name.pcwstr(), Some(0), REG_SZ, Some(bytes)) }
    } else {
        let r = unsafe { RegDeleteValueW(key, name.pcwstr()) };
        if r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            windows::Win32::Foundation::ERROR_SUCCESS
        } else {
            r
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if r.is_ok() {
        Ok(())
    } else {
        Err(format!("registry error {}", r.0))
    }
}
