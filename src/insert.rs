//! Inserts text into the focused application.
//!
//! Default: clipboard + paste shortcut, with the user's clipboard saved before and restored after. The text
//! is offered with delayed rendering, so we know exactly when the target app read it and restore only then.
//! Alternative: typing via `SendInput(KEYEVENTF_UNICODE)` (no clipboard).
//!
//! Runs on its own thread with a message-only window (the clipboard owner), processing jobs in order.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcess, OpenProcessToken, SetEvent, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::clipboard;
use crate::settings::InsertMethod;
use crate::{log_info, log_warn};

pub struct InsertJob {
    pub id: u64,
    pub text: String,
    pub method: InsertMethod,
    pub auto_paste: bool,
    pub restore_clipboard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyReason {
    /// "Automatic paste" is off.
    AutoPasteOff,
    /// The focused window runs as administrator; Windows blocks input from a normal process.
    ElevatedTarget,
    /// No window has the keyboard focus.
    NoTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// Pasted; `restored` tells whether the previous clipboard was put back.
    Pasted { restored: bool },
    /// Paste shortcut was sent but no application read the clipboard.
    NotPasted,
    Typed,
    /// Left on the clipboard for the user to paste.
    Copied(CopyReason),
    Failed(String),
}

#[derive(Debug)]
pub struct InsertResult {
    pub id: u64,
    pub outcome: InsertOutcome,
    pub elapsed_ms: f64,
}

struct Pending {
    text: String,
    /// Set right before the paste keystroke is injected. Reads before that come from clipboard
    /// monitors (clipboard history, sync tools); they are declined, so the text stays unrendered and
    /// the first read after the keystroke is a reliable "the target app pasted" signal.
    paste_sent: bool,
    rendered_at: Option<Instant>,
    declined: u32,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

pub struct Inserter {
    tx: Sender<InsertJob>,
    wake: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Inserter {
    pub fn start(notify: Arc<dyn Fn(InsertResult) + Send + Sync>) -> Inserter {
        let (tx, rx) = mpsc::channel::<InsertJob>();
        let wake = unsafe { CreateEventW(None, false, false, None) }.expect("CreateEvent");
        let wake_raw = wake.0 as isize;
        let thread = std::thread::Builder::new()
            .name("inserter".into())
            .spawn(move || inserter_thread(rx, HANDLE(wake_raw as *mut _), notify))
            .expect("spawn inserter");
        Inserter { tx, wake: wake_raw, thread: Some(thread) }
    }

    pub fn submit(&self, job: InsertJob) {
        if self.tx.send(job).is_ok() {
            unsafe {
                let _ = SetEvent(HANDLE(self.wake as *mut _));
            }
        }
    }

    /// Finishes queued work (restoring the clipboard) and stops the thread.
    pub fn shutdown(mut self) {
        let (dummy_tx, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.tx, dummy_tx));
        unsafe {
            let _ = SetEvent(HANDLE(self.wake as *mut _));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(HANDLE(self.wake as *mut _));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_RENDERFORMAT => {
            PENDING.with(|p| {
                if let Some(p) = p.borrow_mut().as_mut() {
                    if !p.paste_sent {
                        p.declined += 1;
                        if p.declined == 1 {
                            log_info!("insert: declined an early clipboard read by {} (monitor)", clipboard_reader());
                        }
                        return;
                    }
                    if let Err(e) = clipboard::render_text(&p.text) {
                        log_warn!("insert: render failed: {e}");
                    }
                    if p.rendered_at.is_none() {
                        p.rendered_at = Some(Instant::now());
                    }
                }
            });
            LRESULT(0)
        }
        WM_RENDERALLFORMATS => {
            PENDING.with(|p| {
                if let Some(p) = p.borrow().as_ref() {
                    clipboard::render_all(hwnd, &p.text);
                }
            });
            LRESULT(0)
        }
        WM_DESTROYCLIPBOARD => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_window() -> Result<HWND, String> {
    unsafe {
        let hinst = GetModuleHandleW(None).map_err(|e| e.message())?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("WhistleType.Clipboard"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("WhistleType.Clipboard"),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .map_err(|e| e.message())
    }
}

/// Pumps this thread's messages until `until` returns true or the timeout elapses.
fn pump_until(timeout: Duration, mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        if until() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let ms = (deadline - now).as_millis().min(20) as u32;
        unsafe {
            MsgWaitForMultipleObjects(None, false, ms.max(1), QS_ALLINPUT);
        }
    }
}

fn inserter_thread(rx: Receiver<InsertJob>, wake: HANDLE, notify: Arc<dyn Fn(InsertResult) + Send + Sync>) {
    let hwnd = match create_window() {
        Ok(h) => h,
        Err(e) => {
            log_warn!("insert: cannot create clipboard window: {e}");
            // still drain jobs so callers get an answer
            while let Ok(job) = rx.recv() {
                notify(InsertResult { id: job.id, outcome: InsertOutcome::Failed(e.clone()), elapsed_ms: 0.0 });
            }
            return;
        }
    };
    loop {
        // wait for a job while keeping the message queue alive
        let job = loop {
            match rx.try_recv() {
                Ok(j) => break Some(j),
                Err(mpsc::TryRecvError::Disconnected) => break None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let handles = [wake];
            let r = unsafe { MsgWaitForMultipleObjects(Some(&handles), false, INFINITE_MS, QS_ALLINPUT) };
            if r != WAIT_OBJECT_0 {
                pump_until(Duration::ZERO, || true);
            }
        };
        let Some(job) = job else { break };
        let t = Instant::now();
        let outcome = run_job(hwnd, &job);
        let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
        log_info!("insert: {:?} ({} chars, {:?}) in {:.0} ms", outcome, job.text.chars().count(), job.method, elapsed_ms);
        notify(InsertResult { id: job.id, outcome, elapsed_ms });
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

const INFINITE_MS: u32 = u32::MAX;

fn run_job(hwnd: HWND, job: &InsertJob) -> InsertOutcome {
    if job.text.is_empty() {
        return InsertOutcome::Failed("empty text".into());
    }
    if !job.auto_paste {
        return copy_only(hwnd, &job.text, CopyReason::AutoPasteOff);
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return copy_only(hwnd, &job.text, CopyReason::NoTarget);
    }
    log_info!("insert: target {}", process_name(fg));
    if target_is_elevated(fg) {
        log_info!("insert: foreground window is elevated, leaving text on the clipboard");
        return copy_only(hwnd, &job.text, CopyReason::ElevatedTarget);
    }
    release_modifiers();
    match job.method {
        InsertMethod::Type => type_text(fg, &job.text),
        m => paste(hwnd, &job.text, m, job.restore_clipboard),
    }
}

fn copy_only(hwnd: HWND, text: &str, reason: CopyReason) -> InsertOutcome {
    // Text the user is meant to paste must be visible to them, but still kept out of the cloud clipboard.
    match clipboard::set_text_now(hwnd, text, false) {
        Ok(()) => InsertOutcome::Copied(reason),
        Err(e) => InsertOutcome::Failed(format!("clipboard: {e}")),
    }
}

fn key(vk: VIRTUAL_KEY, up: bool, extended: bool) -> INPUT {
    let mut flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: crate::hotkey::INJECT_TAG } },
    }
}

fn send(inputs: &[INPUT]) -> bool {
    let n = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    n as usize == inputs.len()
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

/// Waits (briefly) for the user to let go of Ctrl/Alt/Shift/Win so our Ctrl+V is not Ctrl+Shift+V.
/// If they are still held after 600 ms, synthetic key-ups are sent for them.
fn release_modifiers() {
    let mods = [VK_LCONTROL, VK_RCONTROL, VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN];
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        if !mods.iter().any(|&m| down(m)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let held: Vec<INPUT> = mods
        .iter()
        .filter(|&&m| down(m))
        .map(|&m| key(m, true, matches!(m, VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN)))
        .collect();
    if !held.is_empty() {
        log_info!("insert: releasing {} held modifier(s)", held.len());
        let _ = send(&held);
    }
}

fn paste_keys(method: InsertMethod) -> Vec<INPUT> {
    match method {
        InsertMethod::ShiftInsert => vec![
            key(VK_SHIFT, false, false),
            key(VK_INSERT, false, true),
            key(VK_INSERT, true, true),
            key(VK_SHIFT, true, false),
        ],
        InsertMethod::CtrlShiftV => vec![
            key(VK_CONTROL, false, false),
            key(VK_SHIFT, false, false),
            key(VK_V, false, false),
            key(VK_V, true, false),
            key(VK_SHIFT, true, false),
            key(VK_CONTROL, true, false),
        ],
        _ => vec![key(VK_CONTROL, false, false), key(VK_V, false, false), key(VK_V, true, false), key(VK_CONTROL, true, false)],
    }
}

fn paste(hwnd: HWND, text: &str, method: InsertMethod, restore: bool) -> InsertOutcome {
    let snapshot = if restore {
        match clipboard::snapshot(hwnd) {
            Ok(s) => Some(s),
            Err(e) => {
                log_warn!("insert: could not save the clipboard ({e}); it will not be restored");
                None
            }
        }
    } else {
        None
    };
    if let Some(s) = &snapshot {
        if s.incomplete {
            log_info!("insert: clipboard snapshot is partial ({} formats saved)", s.items.len());
        }
    }
    PENDING.with(|p| *p.borrow_mut() = Some(Pending { text: text.to_string(), paste_sent: false, rendered_at: None, declined: 0 }));
    if let Err(e) = clipboard::set_text_delayed(hwnd) {
        if let Some(snap) = &snapshot {
            let _ = clipboard::restore(hwnd, snap);
        }
        PENDING.with(|p| *p.borrow_mut() = None);
        return InsertOutcome::Failed(format!("clipboard: {e}"));
    }
    // let clipboard monitors react (and be declined) before the paste keystroke
    pump_until(Duration::from_millis(10), || false);
    PENDING.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            p.paste_sent = true;
        }
    });
    let sent_at = Instant::now();
    if !send(&paste_keys(method)) {
        log_warn!("insert: SendInput was blocked");
    }
    let pasted = pump_until(Duration::from_millis(3000), || PENDING.with(|p| p.borrow().as_ref().and_then(|p| p.rendered_at).is_some()));
    if pasted {
        let read_ms = PENDING.with(|p| p.borrow().as_ref().and_then(|p| p.rendered_at)).map(|t| (t - sent_at).as_secs_f64() * 1000.0).unwrap_or(0.0);
        log_info!("insert: target read the text {read_ms:.0} ms after the paste keystroke");
        // the app may read the clipboard more than once while handling the paste
        pump_until(Duration::from_millis(200), || false);
    }
    if snapshot.is_none() && clipboard::we_own_clipboard(hwnd) {
        // not restoring: make sure the text stays available after this job ends
        clipboard::render_all(hwnd, text);
    }
    let mut restored = false;
    if !pasted {
        // Nobody read the text. Do NOT restore the old clipboard: an app that reads late would paste the
        // user's previous clipboard (possibly a password) instead of the dictation. Leave the text instead.
        if clipboard::we_own_clipboard(hwnd) {
            clipboard::render_all(hwnd, text);
        }
        if snapshot.is_some() {
            log_info!("insert: no paste detected within 3 s; leaving the dictated text on the clipboard");
        }
        PENDING.with(|p| *p.borrow_mut() = None);
        return InsertOutcome::NotPasted;
    }
    if let Some(snap) = snapshot {
        if clipboard::we_own_clipboard(hwnd) {
            match clipboard::restore(hwnd, &snap) {
                Ok(()) => restored = true,
                Err(e) => log_warn!("insert: restoring the clipboard failed: {e}"),
            }
        } else {
            log_info!("insert: clipboard changed by another app meanwhile; not restoring");
        }
    }
    PENDING.with(|p| *p.borrow_mut() = None);
    if pasted {
        InsertOutcome::Pasted { restored }
    } else {
        InsertOutcome::NotPasted
    }
}

fn unicode_key(u: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: u,
                dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: crate::hotkey::INJECT_TAG,
            },
        },
    }
}

/// Keystrokes that produce `ch` on keyboard layout `hkl`: the plain key or Shift+key. Characters that need
/// Ctrl/Alt/AltGr (ą, ł, ż… on Polish layouts) return None on purpose: synthetic Ctrl+Alt combinations are
/// treated as shortcuts by apps (measured: Win11 Notepad opened new tabs and a sign-in dialog).
fn layout_keys(ch: char, hkl: HKL, caps_lock: bool) -> Option<Vec<INPUT>> {
    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    if units.len() != 1 {
        return None;
    }
    // With Caps Lock on, letters would come out in the wrong case; let VK_PACKET handle them.
    if caps_lock && ch.is_alphabetic() {
        return None;
    }
    let r = unsafe { VkKeyScanExW(units[0], hkl) };
    if r == -1 {
        return None;
    }
    let vk = VIRTUAL_KEY((r & 0xFF) as u16);
    let shift_state = ((r >> 8) & 0xFF) as u8;
    if shift_state & !0x01 != 0 {
        return None; // needs Ctrl/Alt/AltGr or a special shift state
    }
    let mods: Vec<VIRTUAL_KEY> = if shift_state & 1 != 0 { vec![VK_SHIFT] } else { Vec::new() };
    let mut v = Vec::with_capacity(mods.len() * 2 + 2);
    for &m in &mods {
        v.push(key(m, false, false));
    }
    v.push(key(vk, false, false));
    v.push(key(vk, true, false));
    for &m in mods.iter().rev() {
        v.push(key(m, true, false));
    }
    Some(v)
}

/// Pause after each typed character. Real typing is ~10 chars/s; much faster synthetic input overruns
/// slow editors: Win11 Notepad 11.2607 dropped letters at 6 ms per character (round 4), 12 ms is reliable.
const TYPE_PAUSE_MS: u64 = 12;

fn type_text(target: HWND, text: &str) -> InsertOutcome {
    // Characters the target's keyboard layout can produce are sent as real key presses: each WM_KEYDOWN
    // carries its own key state, so a busy app cannot mix them up. Other characters fall back to
    // VK_PACKET, which slow apps mistranslate when several are queued (measured in Win11 Notepad under
    // load: "wwwwwą"), hence one character per SendInput call and a short pause.
    let hkl = unsafe { GetKeyboardLayout(GetWindowThreadProcessId(target, None)) };
    let caps_lock = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
    let mut packets = 0usize;
    for ch in text.chars() {
        let inputs = match layout_keys(ch, hkl, caps_lock) {
            Some(v) => v,
            None => {
                packets += 1;
                let mut buf = [0u16; 2];
                ch.encode_utf16(&mut buf).iter().flat_map(|&u| [unicode_key(u, false), unicode_key(u, true)]).collect()
            }
        };
        if !send(&inputs) {
            return InsertOutcome::Failed("typing was blocked by Windows".into());
        }
        std::thread::sleep(Duration::from_millis(TYPE_PAUSE_MS));
    }
    if packets > 0 {
        log_info!("insert: {packets} character(s) typed as unicode packets");
    }
    InsertOutcome::Typed
}

/// Executable name of the process that currently has the clipboard open (diagnostics only).
fn clipboard_reader() -> String {
    unsafe {
        let Ok(w) = windows::Win32::System::DataExchange::GetOpenClipboardWindow() else { return "unknown".into() };
        process_name(w)
    }
}

pub fn process_name(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return format!("pid {pid}") };
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = windows::Win32::System::Threading::QueryFullProcessImageNameW(h, windows::Win32::System::Threading::PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return format!("pid {pid}");
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or(&full).to_string()
    }
}

fn integrity_level(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let r = GetTokenInformation(token, TokenIntegrityLevel, Some(buf.as_mut_ptr() as *mut _), len, &mut len);
        let _ = CloseHandle(token);
        r.ok()?;
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, count as u32 - 1))
    }
}

/// True when the window belongs to a process with a higher integrity level than ours (e.g. "Run as
/// administrator"); Windows UIPI then silently drops our SendInput.
pub fn target_is_elevated(hwnd: HWND) -> bool {
    let ours = integrity_level(unsafe { GetCurrentProcess() }).unwrap_or(0x2000);
    if ours >= 0x3000 {
        return false; // we are elevated ourselves
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return false;
    }
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let theirs = integrity_level(h);
    unsafe {
        let _ = CloseHandle(h);
    }
    match theirs {
        Some(il) => il > ours,
        None => true, // token not readable from a normal process: an elevated process
    }
}
