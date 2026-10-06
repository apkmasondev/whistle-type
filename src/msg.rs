//! Window messages used between WhistleType's threads and the main window.

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Hotkey events from the keyboard hook. wparam = HK_*
pub const WM_WT_HOTKEY: u32 = WM_APP + 1;
/// Hotkey capture in settings. wparam = packed Hotkey, or 0 when cancelled.
pub const WM_WT_CAPTURE: u32 = WM_APP + 2;
/// Tray icon callback.
pub const WM_WT_TRAY: u32 = WM_APP + 3;
/// Boxed [`crate::app::AppEvent`] in lparam.
pub const WM_WT_EVENT: u32 = WM_APP + 4;
/// Sent by a second instance: show the settings window.
pub const WM_WT_ACTIVATE: u32 = WM_APP + 5;
/// Audio device list / default device changed.
pub const WM_WT_DEVICES: u32 = WM_APP + 6;

pub const HK_DOWN: usize = 1;
pub const HK_UP: usize = 2;
pub const HK_CANCEL: usize = 3;

pub const MAIN_WINDOW_CLASS: &str = "WhistleType.Main";

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn set_main_hwnd(h: HWND) {
    MAIN_HWND.store(h.0 as isize, Ordering::SeqCst);
}

pub fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _)
}

pub fn post(msg: u32, wparam: usize, lparam: isize) -> bool {
    let h = main_hwnd();
    if h.0.is_null() {
        return false;
    }
    unsafe { PostMessageW(Some(h), msg, WPARAM(wparam), LPARAM(lparam)).is_ok() }
}

/// Posts a boxed value to the main window; the box is reclaimed if posting fails.
pub fn post_boxed<T>(msg: u32, value: T) -> bool {
    let ptr = Box::into_raw(Box::new(value));
    if post(msg, 0, ptr as isize) {
        true
    } else {
        drop(unsafe { Box::from_raw(ptr) });
        false
    }
}

/// Reclaims a value posted with [`post_boxed`].
///
/// # Safety
/// `lparam` must come from `post_boxed::<T>` and be taken exactly once.
pub unsafe fn take_boxed<T>(lparam: LPARAM) -> Box<T> {
    unsafe { Box::from_raw(lparam.0 as *mut T) }
}
