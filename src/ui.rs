//! Minimal DPI-aware helpers for native Win32 forms (Common Controls v6 via the manifest).
//! Controls are positioned in DIPs and re-laid out on WM_DPICHANGED.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, SystemParametersInfoForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::System::SystemServices::{SS_LEFT, SS_NOPREFIX};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::{wide, WStr};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    Normal,
    Bold,
    Title,
    Small,
}

struct Item {
    hwnd: HWND,
    rect: (f32, f32, f32, f32),
    font: Font,
}

pub struct Form {
    pub hwnd: HWND,
    pub dpi: u32,
    fonts: [HFONT; 4],
    items: Vec<Item>,
    /// vertical scroll position in pixels (0 when the form fits)
    scroll: i32,
    /// (width, height) of the content in DIPs, set by [`Form::fit`]
    content: (f32, f32),
}

fn make_fonts(dpi: u32) -> [HFONT; 4] {
    unsafe {
        let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as *mut _), 0, dpi);
        let base = ncm.lfMessageFont;
        let mut bold = base;
        bold.lfWeight = 600;
        let mut title = base;
        title.lfHeight = (base.lfHeight as f32 * 1.75) as i32;
        title.lfWeight = 600;
        let mut small = base;
        small.lfHeight = (base.lfHeight as f32 * 0.92) as i32;
        [CreateFontIndirectW(&base), CreateFontIndirectW(&bold), CreateFontIndirectW(&title), CreateFontIndirectW(&small)]
    }
}

impl Form {
    pub fn new(hwnd: HWND) -> Form {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        Form { hwnd, dpi, fonts: make_fonts(dpi), items: Vec::new(), scroll: 0, content: (0.0, 0.0) }
    }

    pub fn px(&self, dip: f32) -> i32 {
        (dip * self.dpi as f32 / 96.0).round() as i32
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(&mut self, class: PCWSTR, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, id: i32, rect: (f32, f32, f32, f32), font: Font) -> HWND {
        let t = WStr::new(text);
        let hwnd = unsafe {
            CreateWindowExW(
                ex,
                class,
                t.pcwstr(),
                WS_CHILD | WS_VISIBLE | style,
                self.px(rect.0),
                self.px(rect.1) - self.scroll,
                self.px(rect.2),
                self.px(rect.3),
                Some(self.hwnd),
                Some(HMENU(id as isize as *mut _)),
                GetModuleHandleW(None).ok().map(|h| h.into()),
                None,
            )
        }
        .unwrap_or_default();
        unsafe {
            SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(self.fonts[font as usize].0 as usize)), Some(LPARAM(1)));
        }
        self.items.push(Item { hwnd, rect, font });
        hwnd
    }

    pub fn label(&mut self, text: &str, rect: (f32, f32, f32, f32), font: Font, id: i32) -> HWND {
        self.add(w!("STATIC"), text, WINDOW_STYLE(SS_LEFT.0 | SS_NOPREFIX.0), WINDOW_EX_STYLE(0), id, rect, font)
    }

    pub fn button(&mut self, text: &str, id: i32, rect: (f32, f32, f32, f32), default: bool) -> HWND {
        let style = if default { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON };
        self.add(w!("BUTTON"), text, WS_TABSTOP | WINDOW_STYLE(style as u32), WINDOW_EX_STYLE(0), id, rect, Font::Normal)
    }

    pub fn check(&mut self, text: &str, id: i32, rect: (f32, f32, f32, f32)) -> HWND {
        self.add(w!("BUTTON"), text, WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32), WINDOW_EX_STYLE(0), id, rect, Font::Normal)
    }

    pub fn combo(&mut self, id: i32, rect: (f32, f32, f32, f32), items: &[&str]) -> HWND {
        // height includes the drop-down list
        let r = (rect.0, rect.1, rect.2, rect.3 + 200.0);
        let h = self.add(w!("COMBOBOX"), "", WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32), WINDOW_EX_STYLE(0), id, r, Font::Normal);
        for it in items {
            combo_add(h, it);
        }
        h
    }

    pub fn relayout(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        self.scroll = 0;
        let old = std::mem::replace(&mut self.fonts, make_fonts(self.dpi));
        for it in &self.items {
            unsafe {
                let _ = MoveWindow(it.hwnd, self.px(it.rect.0), self.px(it.rect.1), self.px(it.rect.2), self.px(it.rect.3), true);
                SendMessageW(it.hwnd, WM_SETFONT, Some(WPARAM(self.fonts[it.font as usize].0 as usize)), Some(LPARAM(1)));
            }
        }
        for f in old {
            unsafe {
                let _ = DeleteObject(f.into());
            }
        }
    }

    pub fn font(&self, f: Font) -> HFONT {
        self.fonts[f as usize]
    }

    /// Sizes the window to `w`x`h` DIPs of content. When that is taller than the monitor's work area the window
    /// gets a vertical scroll bar (mouse wheel / scroll bar, see [`Form::on_vscroll`] and [`Form::on_wheel`]).
    pub fn fit(&mut self, w: f32, h: f32) {
        self.content = (w, h);
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd).max(96);
            let style = WINDOW_STYLE(GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32 & !WS_VSCROLL.0);
            let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32);
            // same rounding as the controls (px), so content that fits never shows a scroll bar
            let mut r = RECT { left: 0, top: 0, right: self.px(w), bottom: self.px(h) };
            let _ = AdjustWindowRectExForDpi(&mut r, style, false, ex, dpi);
            let mut cur = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut cur);
            let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let work_h = mi.rcWork.bottom - mi.rcWork.top;
            let mut ww = r.right - r.left;
            let wh = (r.bottom - r.top).min(work_h);
            if r.bottom - r.top > work_h {
                ww += GetSystemMetricsForDpi(SM_CXVSCROLL, dpi);
            }
            let y = cur.top.min(mi.rcWork.bottom - wh).max(mi.rcWork.top);
            let _ = SetWindowPos(self.hwnd, None, cur.left, y, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.update_scrollbar();
    }

    fn page(&self) -> i32 {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        rc.bottom - rc.top
    }

    /// Re-applies the scroll range (after [`Form::fit`], a size or a DPI change).
    pub fn update_scrollbar(&mut self) {
        let total = self.px(self.content.1);
        unsafe {
            if total <= self.page() + 2 {
                self.scroll_to(0);
                let _ = ShowScrollBar(self.hwnd, SB_VERT, false);
                return;
            }
            let _ = ShowScrollBar(self.hwnd, SB_VERT, true);
            let si = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: total - 1,
                nPage: self.page() as u32,
                nPos: self.scroll,
                nTrackPos: 0,
            };
            SetScrollInfo(self.hwnd, SB_VERT, &si, true);
        }
        self.scroll_to(self.scroll);
    }

    fn scroll_to(&mut self, pos: i32) {
        let max = (self.px(self.content.1) - self.page()).max(0);
        let pos = pos.clamp(0, max);
        let dy = self.scroll - pos;
        if dy == 0 {
            return;
        }
        self.scroll = pos;
        unsafe {
            let _ = ScrollWindowEx(self.hwnd, 0, dy, None, None, None, None, SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE);
            let si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_POS, nPos: pos, ..Default::default() };
            SetScrollInfo(self.hwnd, SB_VERT, &si, true);
        }
    }

    /// WM_VSCROLL
    pub fn on_vscroll(&mut self, wparam: WPARAM) {
        let line = self.px(32.0);
        let page = self.page();
        let pos = match SCROLLBAR_COMMAND((wparam.0 & 0xFFFF) as i32) {
            SB_LINEUP => self.scroll - line,
            SB_LINEDOWN => self.scroll + line,
            SB_PAGEUP => self.scroll - page,
            SB_PAGEDOWN => self.scroll + page,
            SB_TOP => 0,
            SB_BOTTOM => i32::MAX / 2,
            SB_THUMBTRACK | SB_THUMBPOSITION => unsafe {
                let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, ..Default::default() };
                let _ = GetScrollInfo(self.hwnd, SB_VERT, &mut si);
                si.nTrackPos
            },
            _ => return,
        };
        self.scroll_to(pos);
    }

    /// WM_MOUSEWHEEL; returns false when the form does not scroll.
    pub fn on_wheel(&mut self, wparam: WPARAM) -> bool {
        let style = unsafe { GetWindowLongPtrW(self.hwnd, GWL_STYLE) } as u32;
        if style & WS_VSCROLL.0 == 0 {
            return false;
        }
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        self.scroll_to(self.scroll - delta * self.px(96.0) / 120);
        true
    }

    /// Scrolls so that the control `h` (which just got the keyboard focus) is visible.
    pub fn scroll_into_view(&mut self, h: HWND) {
        let Some(it) = self.items.iter().find(|i| i.hwnd == h) else { return };
        // combo rects include the drop-down list
        let (top, bottom) = (self.px(it.rect.1), self.px(it.rect.1 + it.rect.3.min(40.0)));
        let page = self.page();
        if top < self.scroll {
            self.scroll_to(top - self.px(8.0));
        } else if bottom > self.scroll + page {
            self.scroll_to(bottom - page + self.px(8.0));
        }
    }
}

impl Drop for Form {
    fn drop(&mut self) {
        for f in self.fonts {
            unsafe {
                let _ = DeleteObject(f.into());
            }
        }
    }
}

pub fn set_text(h: HWND, s: &str) {
    let w = WStr::new(s);
    unsafe {
        let _ = SetWindowTextW(h, w.pcwstr());
    }
}

pub fn get_text(h: HWND) -> String {
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn combo_add(h: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        SendMessageW(h, CB_ADDSTRING, None, Some(LPARAM(w.as_ptr() as isize)));
    }
}

pub fn combo_clear(h: HWND) {
    unsafe {
        SendMessageW(h, CB_RESETCONTENT, None, None);
    }
}

pub fn combo_set(h: HWND, idx: i32) {
    unsafe {
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(idx as usize)), None);
    }
}

pub fn combo_get(h: HWND) -> i32 {
    unsafe { SendMessageW(h, CB_GETCURSEL, None, None).0 as i32 }
}

pub fn check_set(h: HWND, on: bool) {
    unsafe {
        SendMessageW(h, BM_SETCHECK, Some(WPARAM(if on { 1 } else { 0 })), None);
    }
}

pub fn check_get(h: HWND) -> bool {
    unsafe { SendMessageW(h, BM_GETCHECK, None, None).0 == 1 }
}

pub fn enable(h: HWND, on: bool) {
    unsafe {
        let _ = EnableWindow(h, on);
    }
}

pub fn show(h: HWND, on: bool) {
    unsafe {
        let _ = ShowWindow(h, if on { SW_SHOW } else { SW_HIDE });
    }
}

pub fn list_add(h: HWND, s: &str) {
    let w = wide(s);
    unsafe {
        SendMessageW(h, LB_ADDSTRING, None, Some(LPARAM(w.as_ptr() as isize)));
    }
}

pub fn list_clear(h: HWND) {
    unsafe {
        SendMessageW(h, LB_RESETCONTENT, None, None);
    }
}

pub fn list_get(h: HWND) -> i32 {
    unsafe { SendMessageW(h, LB_GETCURSEL, None, None).0 as i32 }
}

pub fn list_set(h: HWND, idx: i32) {
    unsafe {
        SendMessageW(h, LB_SETCURSEL, Some(WPARAM(idx as isize as usize)), None);
    }
}

pub fn shell_open(target: &str) {
    let t = WStr::new(target);
    unsafe {
        ShellExecuteW(None, w!("open"), t.pcwstr(), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn message_box(owner: Option<HWND>, text: &str, title: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = WStr::new(text);
    let c = WStr::new(title);
    unsafe { MessageBoxW(owner, t.pcwstr(), c.pcwstr(), style) }
}

/// Creates a top-level window whose *client* area is `w`×`h` DIPs on the monitor under the cursor, centred.
pub fn create_centered(class: PCWSTR, title: &str, w: f32, h: f32, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE) -> windows::core::Result<HWND> {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let mut r = RECT { left: 0, top: 0, right: (w * s) as i32, bottom: (h * s) as i32 };
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, ex, dx);
        let ww = r.right - r.left;
        let wh = r.bottom - r.top;
        let work = mi.rcWork;
        let x = work.left + ((work.right - work.left) - ww).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - wh).max(0) / 2;
        let t = WStr::new(title);
        CreateWindowExW(ex, class, t.pcwstr(), style, x, y, ww, wh, None, None, GetModuleHandleW(None).ok().map(|h| h.into()), None)
    }
}

/// Resizes a window so that its client area is `w`×`h` DIPs at its current DPI.
pub fn resize_client(hwnd: HWND, w: f32, h: f32) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = dpi as f32 / 96.0;
        let style = WINDOW_STYLE(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        let mut r = RECT { left: 0, top: 0, right: (w * s) as i32, bottom: (h * s) as i32 };
        let _ = AdjustWindowRectExForDpi(&mut r, style, false, ex, dpi);
        let mut cur = RECT::default();
        let _ = GetWindowRect(hwnd, &mut cur);
        // keep it on the work area of its monitor
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let ww = r.right - r.left;
        let wh = (r.bottom - r.top).min(mi.rcWork.bottom - mi.rcWork.top);
        let y = cur.top.min(mi.rcWork.bottom - wh).max(mi.rcWork.top);
        let _ = SetWindowPos(hwnd, None, cur.left, y, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn register_class(name: PCWSTR, proc: WNDPROC) {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: proc,
            hInstance: hinst.into(),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1 as *const u16)).unwrap_or_default(),
            hIconSm: LoadIconW(Some(hinst.into()), PCWSTR(1 as *const u16)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: name,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

/// Standard handling for static/checkbox colours on a white form. `muted` ids are drawn grey.
pub fn ctl_color_static(wparam: WPARAM, lparam: LPARAM, muted: &[i32], warn: &[i32]) -> isize {
    unsafe {
        let hdc = HDC(wparam.0 as *mut _);
        let id = GetDlgCtrlID(HWND(lparam.0 as *mut _));
        SetBkMode(hdc, TRANSPARENT);
        if warn.contains(&id) {
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x0000_3CB4)); // dark orange-red (BGR)
        } else if muted.contains(&id) {
            SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x0070_6A66)); // grey (BGR)
        }
        GetSysColorBrush(COLOR_WINDOW).0 as isize
    }
}

pub fn init_common_controls() {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS | ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
    }
}

pub fn bring_to_front(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
    }
}
