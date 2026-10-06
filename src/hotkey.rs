//! Global push-to-talk hotkey.
//!
//! A `WH_KEYBOARD_LL` hook on its own thread sees key-down *and* key-up of the hotkey anywhere in the
//! session and swallows them, so the focused application never receives e.g. F8. The hook callback does
//! nothing but compare a few atomics and post a message: it can never exceed `LowLevelHooksTimeout`.
//!
//! Limits (Windows UIPI): while an elevated (administrator) window is focused, a non-elevated hook is not
//! called. The main window therefore also registers the same combination with `RegisterHotKey`, which keeps
//! working; it only fires when the hook did not swallow the key (see `app.rs`).

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentThreadId, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::msg::{self, HK_CANCEL, HK_DOWN, HK_UP};
use crate::settings::Hotkey;
use crate::{log_info, log_warn};

/// Unassigned virtual key used to stop a lone Alt/Win release from opening a menu (same trick as AutoHotkey).
pub const MASK_VK: u16 = 0xE8;

static HOTKEY: AtomicU64 = AtomicU64::new(0);
static HELD: AtomicBool = AtomicBool::new(false);
static CAPTURE: AtomicBool = AtomicBool::new(false);
static ESC_ACTIVE: AtomicBool = AtomicBool::new(false);
static ESC_SWALLOWED: AtomicBool = AtomicBool::new(false);
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
/// Last time (GetTickCount64) the hook callback ran - used to detect a hook Windows removed silently.
static LAST_CALL_TICK: AtomicU64 = AtomicU64::new(0);

const WM_HOOK_REINSTALL: u32 = WM_APP + 100;
const WM_HOOK_QUIT: u32 = WM_APP + 101;
const WM_HOOK_MASK: u32 = WM_APP + 102;

/// Queues the Alt/Win masking key on the hook thread, to be sent after the hook callback has returned
/// (calling SendInput from inside a low-level hook callback would re-enter the hook chain).
fn queue_mask_key() {
    let tid = HOOK_THREAD.load(Ordering::Relaxed);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_HOOK_MASK, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn set_hotkey(h: Hotkey) {
    HOTKEY.store(h.pack(), Ordering::SeqCst);
    HELD.store(false, Ordering::SeqCst);
}

pub fn hotkey() -> Hotkey {
    Hotkey::unpack(HOTKEY.load(Ordering::SeqCst))
}

/// While capturing, the next non-modifier key (with modifiers) is reported via `WM_WT_CAPTURE`.
pub fn set_capture(on: bool) {
    CAPTURE.store(on, Ordering::SeqCst);
}

pub fn is_capturing() -> bool {
    CAPTURE.load(Ordering::SeqCst)
}

/// While recording, Esc cancels the dictation (and is swallowed).
pub fn set_escape_cancel(on: bool) {
    ESC_ACTIVE.store(on, Ordering::SeqCst);
}

/// Marker in `dwExtraInfo` of every keystroke WhistleType injects itself (paste, typing, Alt/Win mask).
/// The hook ignores only these; keys injected by other software (mouse/macro-key utilities, AutoHotkey,
/// remote tools) can trigger the shortcut like a physical key.
pub const INJECT_TAG: usize = 0x5754_4B59; // "WTKY"

pub fn is_held() -> bool {
    HELD.load(Ordering::SeqCst)
}


pub fn last_hook_call_tick() -> u64 {
    LAST_CALL_TICK.load(Ordering::Relaxed)
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

pub fn current_modifiers() -> (bool, bool, bool, bool) {
    (
        key_down(VK_CONTROL),
        key_down(VK_MENU),
        key_down(VK_SHIFT),
        key_down(VK_LWIN) || key_down(VK_RWIN),
    )
}

pub fn is_modifier(vk: u32) -> bool {
    matches!(
        VIRTUAL_KEY(vk as u16),
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU
            | VK_LWIN | VK_RWIN
    )
}

fn send_mask_key() {
    let mk = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(MASK_VK), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: INJECT_TAG },
        },
    };
    let inputs = [mk(KEYBD_EVENT_FLAGS(0)), mk(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    LAST_CALL_TICK.store(unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }, Ordering::Relaxed);
    let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let injected = kb.flags.0 & LLKHF_INJECTED.0 != 0;
    let vk = kb.vkCode;
    if vk == MASK_VK as u32 || (injected && kb.dwExtraInfo == INJECT_TAG) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let m = wparam.0 as u32;
    let is_down = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
    let is_up = m == WM_KEYUP || m == WM_SYSKEYUP;

    // Settings: "press the new shortcut"
    if CAPTURE.load(Ordering::Relaxed) {
        if is_modifier(vk) {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        if is_down {
            if vk == VK_ESCAPE.0 as u32 {
                CAPTURE.store(false, Ordering::SeqCst);
                msg::post(msg::WM_WT_CAPTURE, 0, 0);
            } else {
                let (ctrl, alt, shift, win) = current_modifiers();
                let h = Hotkey { vk, ctrl, alt, shift, win };
                CAPTURE.store(false, Ordering::SeqCst);
                if alt || win {
                    queue_mask_key();
                }
                msg::post(msg::WM_WT_CAPTURE, h.pack() as usize, 0);
            }
        }
        return LRESULT(1);
    }

    let hk = Hotkey::unpack(HOTKEY.load(Ordering::Relaxed));
    if vk == hk.vk && hk.vk != 0 {
        if is_down {
            if HELD.load(Ordering::Relaxed) {
                return LRESULT(1); // auto-repeat while held
            }
            let (ctrl, alt, shift, win) = current_modifiers();
            if ctrl == hk.ctrl && alt == hk.alt && shift == hk.shift && win == hk.win {
                HELD.store(true, Ordering::SeqCst);
                if hk.alt || hk.win {
                    queue_mask_key();
                }
                msg::post(msg::WM_WT_HOTKEY, HK_DOWN, 0);
                return LRESULT(1);
            }
        } else if is_up && HELD.swap(false, Ordering::SeqCst) {
            msg::post(msg::WM_WT_HOTKEY, HK_UP, 0);
            return LRESULT(1);
        }
    } else if vk == VK_ESCAPE.0 as u32 {
        if is_down && ESC_ACTIVE.load(Ordering::Relaxed) {
            ESC_SWALLOWED.store(true, Ordering::SeqCst);
            msg::post(msg::WM_WT_HOTKEY, HK_CANCEL, 0);
            return LRESULT(1);
        }
        if is_up && ESC_SWALLOWED.swap(false, Ordering::SeqCst) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn install() -> windows::core::Result<HHOOK> {
    let hmod = unsafe { GetModuleHandleW(None)? };
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(HINSTANCE(hmod.0)), 0) }
}

/// Starts the hook thread. Returns once the hook is installed (or failed).
pub fn start(initial: Hotkey) -> Result<(), String> {
    set_hotkey(initial);
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            // make sure this thread has a message queue before reporting success
            let mut m = MSG::default();
            let _ = PeekMessageW(&mut m, None, 0, 0, PM_NOREMOVE);
            match install() {
                Ok(h) => {
                    HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
                    let _ = tx.send(Ok(()));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.message()));
                    return;
                }
            }
            while GetMessageW(&mut m, None, 0, 0).as_bool() {
                match m.message {
                    WM_HOOK_REINSTALL => {
                        let old = HHOOK(HOOK_HANDLE.load(Ordering::SeqCst) as *mut _);
                        let _ = UnhookWindowsHookEx(old);
                        match install() {
                            Ok(h) => {
                                HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
                                log_info!("hotkey: keyboard hook reinstalled");
                            }
                            Err(e) => log_warn!("hotkey: reinstall failed: {}", e.message()),
                        }
                    }
                    WM_HOOK_QUIT => break,
                    WM_HOOK_MASK => send_mask_key(),
                    _ => {
                        let _ = TranslateMessage(&m);
                        DispatchMessageW(&m);
                    }
                }
            }
            let h = HHOOK(HOOK_HANDLE.swap(0, Ordering::SeqCst) as *mut _);
            if !h.0.is_null() {
                let _ = UnhookWindowsHookEx(h);
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// Re-creates the hook (Windows may remove a low-level hook without notice).
pub fn reinstall() {
    let tid = HOOK_THREAD.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_HOOK_REINSTALL, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn stop() {
    let tid = HOOK_THREAD.swap(0, Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_HOOK_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

/// Human-readable name, e.g. "Ctrl+Shift+Space".
pub fn display(h: &Hotkey) -> String {
    let mut parts: Vec<String> = Vec::new();
    if h.ctrl {
        parts.push("Ctrl".into());
    }
    if h.alt {
        parts.push("Alt".into());
    }
    if h.shift {
        parts.push("Shift".into());
    }
    if h.win {
        parts.push("Win".into());
    }
    parts.push(key_name(h.vk));
    parts.join("+")
}

pub fn key_name(vk: u32) -> String {
    let v = VIRTUAL_KEY(vk as u16);
    let fixed = match v {
        VK_SPACE => Some("Space"),
        VK_RETURN => Some("Enter"),
        VK_TAB => Some("Tab"),
        VK_BACK => Some("Backspace"),
        VK_INSERT => Some("Insert"),
        VK_DELETE => Some("Delete"),
        VK_HOME => Some("Home"),
        VK_END => Some("End"),
        VK_PRIOR => Some("Page Up"),
        VK_NEXT => Some("Page Down"),
        VK_LEFT => Some("Left"),
        VK_RIGHT => Some("Right"),
        VK_UP => Some("Up"),
        VK_DOWN => Some("Down"),
        VK_PAUSE => Some("Pause"),
        VK_SCROLL => Some("Scroll Lock"),
        VK_CAPITAL => Some("Caps Lock"),
        VK_NUMLOCK => Some("Num Lock"),
        VK_SNAPSHOT => Some("Print Screen"),
        VK_APPS => Some("Menu"),
        VK_ESCAPE => Some("Esc"),
        VK_MEDIA_PLAY_PAUSE => Some("Play/Pause"),
        VK_BROWSER_BACK => Some("Browser Back"),
        VK_BROWSER_FORWARD => Some("Browser Forward"),
        VK_LAUNCH_APP1 => Some("Launch App 1"),
        VK_LAUNCH_APP2 => Some("Launch App 2"),
        _ => None,
    };
    if let Some(s) = fixed {
        return s.into();
    }
    if (VK_F1.0..=VK_F24.0).contains(&v.0) {
        return format!("F{}", v.0 - VK_F1.0 + 1);
    }
    if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&v.0) {
        return format!("Num {}", v.0 - VK_NUMPAD0.0);
    }
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from_u32(vk).unwrap().to_string();
    }
    // Fall back to the keyboard layout's name for the key.
    unsafe {
        let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
        let mut buf = [0u16; 64];
        let n = GetKeyNameTextW((scan << 16) as i32, &mut buf);
        if n > 0 {
            return String::from_utf16_lossy(&buf[..n as usize]);
        }
    }
    format!("Key 0x{vk:02X}")
}

/// Rejects shortcuts that would break normal typing.
pub fn validate(h: &Hotkey) -> Result<(), String> {
    let v = VIRTUAL_KEY(h.vk as u16);
    if is_modifier(h.vk) {
        return Err(crate::i18n::t().hk_modifier.into());
    }
    if v == VK_ESCAPE {
        return Err(crate::i18n::t().hk_esc.into());
    }
    let typing = (0x30..=0x5A).contains(&h.vk)
        || v == VK_SPACE
        || v == VK_RETURN
        || v == VK_TAB
        || v == VK_BACK
        || (0xBA..=0xE2).contains(&h.vk); // OEM punctuation keys
    if typing && !(h.ctrl || h.alt || h.win) {
        return Err(crate::i18n::fmt(crate::i18n::t().hk_typing, &[("key", &display(h))]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_validation() {
        assert_eq!(display(&Hotkey::default()), "F8");
        let h = Hotkey { vk: 0x20, ctrl: true, alt: false, shift: true, win: false };
        assert_eq!(display(&h), "Ctrl+Shift+Space");
        assert!(validate(&Hotkey::default()).is_ok());
        assert!(validate(&Hotkey { vk: 0x41, ..Default::default() }).is_err()); // 'A'
        assert!(validate(&Hotkey { vk: 0x41, ctrl: true, ..Default::default() }).is_ok());
        assert!(validate(&Hotkey { vk: 0x20, shift: true, ..Default::default() }).is_err()); // Shift+Space types
        assert!(validate(&Hotkey { vk: VK_ESCAPE.0 as u32, ..Default::default() }).is_err());
        assert!(validate(&Hotkey { vk: VK_LCONTROL.0 as u32, ..Default::default() }).is_err());
        assert_eq!(key_name(VK_F13.0 as u32), "F13");
    }
}
