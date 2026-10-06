//! Notification-area icon.

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{LoadIconMetric, LIM_SMALL};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::msg::WM_WT_TRAY;

pub const IDI_APP: u16 = 1;
pub const IDI_REC: u16 = 2;
pub const IDI_BUSY: u16 = 3;
pub const IDI_WARN: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Busy,
    Warning,
}

pub struct Tray {
    hwnd: HWND,
    icons: [HICON; 4],
    added: bool,
    state: TrayState,
    tip: String,
}

fn load_icon(id: u16) -> HICON {
    unsafe {
        let hinst = GetModuleHandleW(None).ok();
        LoadIconMetric(hinst.map(|h| h.into()), PCWSTR(id as usize as *const u16), LIM_SMALL)
            .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        Tray {
            hwnd,
            icons: [load_icon(IDI_APP), load_icon(IDI_REC), load_icon(IDI_BUSY), load_icon(IDI_WARN)],
            added: false,
            state: TrayState::Idle,
            tip: "WhistleType".into(),
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_WT_TRAY,
            hIcon: self.icons[self.state as usize],
            ..Default::default()
        };
        copy_wide(&mut d.szTip, &self.tip);
        d
    }

    /// Adds the icon (again after Explorer restarts).
    pub fn add(&mut self) {
        let mut d = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            self.added = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
            d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        }
    }

    pub fn set(&mut self, state: TrayState, tip: &str) {
        if state == self.state && tip == self.tip {
            return;
        }
        self.state = state;
        self.tip = tip.to_string();
        if self.added {
            let d = self.data();
            unsafe {
                if !Shell_NotifyIconW(NIM_MODIFY, &d).as_bool() {
                    self.add();
                }
            }
        }
    }

    /// Windows notification ("balloon") for problems that need attention.
    pub fn notify(&self, title: &str, text: &str, warning: bool) {
        let mut d = self.data();
        d.uFlags = NIF_INFO;
        copy_wide(&mut d.szInfoTitle, title);
        copy_wide(&mut d.szInfo, text);
        d.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO } | NIIF_RESPECT_QUIET_TIME;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}
