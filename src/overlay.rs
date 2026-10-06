//! Small status pill shown while dictating.
//!
//! A layered, click-through, never-activated tool window (`WS_EX_NOACTIVATE | WS_EX_TRANSPARENT`), rendered
//! with Direct2D (software target - no GPU device is created) into a 32-bit DIB with per-pixel alpha.
//! The state is conveyed by text, an icon and the pill's shape/animation - never by colour alone:
//!   Listening     - microphone icon, "Listening…", elapsed time, live level bars
//!   Transcribing  - spinning dots, "Transcribing…", narrower pill
//!   Message       - info/check/warning icon + text, disappears by itself
//! The animation timer only runs while the pill is visible.

use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::core::BOOL;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::settings::OverlayPosition;
use crate::util::wide;

const TIMER_ID: usize = 1;
const FRAME_MS: u32 = 40;
const HEIGHT: f32 = 44.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Info,
    Success,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Hidden,
    Listening { since: Instant, limit_s: u32 },
    Transcribing { since: Instant, part: Option<(usize, usize)> },
    Message { text: String, kind: MsgKind, until: Instant },
}

pub struct Overlay {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: Option<ID2D1DCRenderTarget>,
    text_font: Vec<u16>,
    icon_font: Option<Vec<u16>>,
    state: State,
    position: OverlayPosition,
    level: f32,
    level_source: Option<Box<dyn Fn() -> f32>>,
    /// Engine shown next to "Transcribing…", e.g. "Whisper · GPU".
    detail: Option<String>,
    bars: [f32; 5],
    reduce_motion: bool,
    anim_start: Instant,
    placed: Option<(i32, i32, i32, i32, u32)>, // x, y, w, h, dpi
    dib: Option<(HDC, HBITMAP, HGDIOBJ, i32, i32)>,
    timer_on: bool,
    /// DirectWrite text formats, created once per (size, weight, icon font).
    formats: std::cell::RefCell<Vec<(FormatKey, IDWriteTextFormat)>>,
}

/// (size, weight, icon font) of a cached text format.
type FormatKey = (u32, i32, bool);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER if wparam.0 == TIMER_ID => {
            crate::app::with_overlay(|o| o.tick());
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn family_exists(dwrite: &IDWriteFactory, name: &str) -> bool {
    unsafe {
        let mut col: Option<IDWriteFontCollection> = None;
        if dwrite.GetSystemFontCollection(&mut col, false).is_err() {
            return false;
        }
        let Some(col) = col else { return false };
        let w = wide(name);
        let mut idx = 0u32;
        let mut exists = BOOL(0);
        col.FindFamilyName(PCWSTR(w.as_ptr()), &mut idx, &mut exists).is_ok() && exists.as_bool()
    }
}

fn color(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

impl Overlay {
    pub fn create() -> windows::core::Result<Overlay> {
        unsafe {
            let hinst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst.into(),
                lpszClassName: w!("WhistleType.Overlay"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("WhistleType.Overlay"),
                w!("WhistleType"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            )?;
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text_font = if family_exists(&dwrite, "Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
            let icon_font = ["Segoe Fluent Icons", "Segoe MDL2 Assets"].into_iter().find(|f| family_exists(&dwrite, f)).map(wide);
            let mut anim = BOOL(1);
            let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut anim as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            Ok(Overlay {
                hwnd,
                d2d,
                dwrite,
                target: None,
                text_font: wide(text_font),
                icon_font,
                state: State::Hidden,
                position: OverlayPosition::Bottom,
                level: 0.0,
                level_source: None,
                detail: None,
                bars: [0.0; 5],
                reduce_motion: !anim.as_bool(),
                anim_start: Instant::now(),
                placed: None,
                dib: None,
                timer_on: false,
                formats: std::cell::RefCell::new(Vec::new()),
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn set_position(&mut self, p: OverlayPosition) {
        self.position = p;
    }

    /// Live microphone level for the bars (cleared when the recording ends).
    pub fn set_level_source(&mut self, src: Option<Box<dyn Fn() -> f32>>) {
        self.level_source = src;
        if self.level_source.is_none() {
            self.level = 0.0;
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_detail(&mut self, detail: Option<String>) {
        self.detail = detail;
    }

    pub fn listening(&mut self, limit_s: u32) {
        self.show(State::Listening { since: Instant::now(), limit_s });
    }

    pub fn transcribing(&mut self, part: Option<(usize, usize)>) {
        let since = match &self.state {
            State::Transcribing { since, .. } => *since,
            _ => Instant::now(),
        };
        self.show(State::Transcribing { since, part });
    }

    pub fn message(&mut self, text: &str, kind: MsgKind, secs: f32) {
        self.show(State::Message { text: text.to_string(), kind, until: Instant::now() + Duration::from_secs_f32(secs) });
    }

    pub fn hide(&mut self) {
        self.state = State::Hidden;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            if self.timer_on {
                let _ = KillTimer(Some(self.hwnd), TIMER_ID);
                self.timer_on = false;
            }
        }
        self.placed = None;
        self.free_dib();
    }

    fn show(&mut self, state: State) {
        let was_hidden = self.state == State::Hidden;
        self.state = state;
        if was_hidden {
            self.anim_start = Instant::now();
            self.bars = [0.0; 5];
            self.placed = None; // pick the monitor of the current foreground window
        }
        self.render();
        if !self.timer_on {
            unsafe { SetTimer(Some(self.hwnd), TIMER_ID, FRAME_MS, None) };
            self.timer_on = true;
        }
    }

    fn tick(&mut self) {
        if let State::Message { until, .. } = &self.state {
            if Instant::now() >= *until {
                self.hide();
                return;
            }
        }
        if let Some(f) = &self.level_source {
            self.level = f();
        }
        if self.state != State::Hidden {
            self.render();
        }
    }

    fn width_dip(&self) -> f32 {
        match &self.state {
            State::Hidden => 0.0,
            State::Listening { .. } => 236.0,
            State::Transcribing { part, .. } => {
                let base = if part.is_some() { 236.0 } else { 196.0 };
                match &self.detail {
                    Some(d) => base + self.measure(d, 12.0) + 14.0,
                    None => base,
                }
            }
            State::Message { text, .. } => {
                let w = self.measure(text, 14.0);
                (w + 50.0 + 26.0).clamp(160.0, 560.0)
            }
        }
    }

    fn measure(&self, text: &str, size: f32) -> f32 {
        unsafe {
            let Ok(fmt) = self.dwrite.CreateTextFormat(
                PCWSTR(self.text_font.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!(""),
            ) else {
                return 200.0;
            };
            let s: Vec<u16> = text.encode_utf16().collect();
            let Ok(layout) = self.dwrite.CreateTextLayout(&s, &fmt, 2000.0, 100.0) else { return 200.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            if layout.GetMetrics(&mut m).is_ok() {
                m.widthIncludingTrailingWhitespace
            } else {
                200.0
            }
        }
    }

    /// Chooses position/size on the monitor that holds the foreground window.
    fn place(&mut self, width_dip: f32) -> (i32, i32, i32, i32, u32) {
        unsafe {
            let fg = GetForegroundWindow();
            let mon = if fg.0.is_null() {
                MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY)
            } else {
                MonitorFromWindow(fg, MONITOR_DEFAULTTOPRIMARY)
            };
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let scale = dx as f32 / 96.0;
            let w = (width_dip * scale).ceil() as i32;
            let h = (HEIGHT * scale).ceil() as i32 + 2;
            let work = mi.rcWork;
            let x = work.left + (work.right - work.left - w) / 2;
            let margin = (28.0 * scale) as i32;
            let y = match self.position {
                OverlayPosition::Bottom => work.bottom - h - margin,
                OverlayPosition::Top => work.top + margin,
            };
            (x, y, w, h, dx)
        }
    }

    fn free_dib(&mut self) {
        if let Some((dc, bmp, old, _, _)) = self.dib.take() {
            unsafe {
                SelectObject(dc, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(dc);
            }
        }
    }

    fn ensure_dib(&mut self, w: i32, h: i32) -> Option<HDC> {
        if let Some((dc, _, _, dw, dh)) = self.dib {
            if dw == w && dh == h {
                return Some(dc);
            }
        }
        self.free_dib();
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(dc, bmp.into());
            self.dib = Some((dc, bmp, old, w, h));
            Some(dc)
        }
    }

    fn render_target(&mut self) -> Option<ID2D1DCRenderTarget> {
        if self.target.is_none() {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            self.target = unsafe { self.d2d.CreateDCRenderTarget(&props) }.ok();
        }
        self.target.clone()
    }

    fn render(&mut self) {
        if self.state == State::Hidden {
            return;
        }
        let width_dip = self.width_dip();
        let (x, y, w, h, dpi) = match self.placed {
            Some((x, y, _, h, dpi)) => {
                // keep the monitor, but adapt the width (centre stays)
                let scale = dpi as f32 / 96.0;
                let nw = (width_dip * scale).ceil() as i32;
                let (_, _, ow, _, _) = self.placed.unwrap();
                (x + (ow - nw) / 2, y, nw, h, dpi)
            }
            None => self.place(width_dip),
        };
        self.placed = Some((x, y, w, h, dpi));
        let Some(dc) = self.ensure_dib(w, h) else { return };
        let Some(rt) = self.render_target() else { return };
        let rect = RECT { left: 0, top: 0, right: w, bottom: h };
        unsafe {
            if rt.BindDC(dc, &rect).is_err() {
                self.target = None;
                return;
            }
            rt.SetDpi(dpi as f32, dpi as f32);
            rt.BeginDraw();
            rt.Clear(Some(&color(0, 0, 0, 0.0)));
            self.draw(&rt, width_dip);
            if rt.EndDraw(None, None).is_err() {
                self.target = None;
                return;
            }
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pos = POINT { x, y };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(self.hwnd, None, Some(&pos), Some(&size), Some(dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            if !IsWindowVisible(self.hwnd).as_bool() {
                // z-order is set once when the pill appears; re-asserting it every frame would churn the
                // window stack (and disturbed text input in the focused app)
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
        }
    }

    fn text_format(&self, size: f32, weight: DWRITE_FONT_WEIGHT, icon: bool) -> Option<IDWriteTextFormat> {
        let key = (size.to_bits(), weight.0, icon);
        if let Some((_, f)) = self.formats.borrow().iter().find(|(k, _)| *k == key) {
            return Some(f.clone());
        }
        let f = self.create_text_format(size, weight, icon)?;
        self.formats.borrow_mut().push((key, f.clone()));
        Some(f)
    }

    fn create_text_format(&self, size: f32, weight: DWRITE_FONT_WEIGHT, icon: bool) -> Option<IDWriteTextFormat> {
        let family = if icon { self.icon_font.as_ref()? } else { &self.text_font };
        unsafe {
            let f = self
                .dwrite
                .CreateTextFormat(PCWSTR(family.as_ptr()), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!(""))
                .ok()?;
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            Some(f)
        }
    }

    unsafe fn text(&self, rt: &ID2D1DCRenderTarget, s: &str, fmt: &IDWriteTextFormat, r: D2D_RECT_F, c: D2D1_COLOR_F, align: DWRITE_TEXT_ALIGNMENT) {
        unsafe {
            let _ = fmt.SetTextAlignment(align);
            if let Ok(b) = rt.CreateSolidColorBrush(&c, None) {
                let s: Vec<u16> = s.encode_utf16().collect();
                rt.DrawText(&s, fmt, &r, &b, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
            }
        }
    }

    unsafe fn draw(&mut self, rt: &ID2D1DCRenderTarget, width: f32) {
        let t = self.anim_start.elapsed().as_secs_f32();
        let h = HEIGHT;
        unsafe {
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            // pill background + hairline border
            let pill = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: 0.5, top: 0.5, right: width - 0.5, bottom: h - 0.5 }, radiusX: h / 2.0, radiusY: h / 2.0 };
            if let Ok(bg) = rt.CreateSolidColorBrush(&color(24, 24, 28, 0.94), None) {
                rt.FillRoundedRectangle(&pill, &bg);
            }
            let border_alpha = match &self.state {
                State::Listening { .. } if !self.reduce_motion => 0.18 + 0.12 * (t * 3.0).sin().abs(),
                _ => 0.16,
            };
            if let Ok(br) = rt.CreateSolidColorBrush(&color(255, 255, 255, border_alpha), None) {
                rt.DrawRoundedRectangle(&pill, &br, 1.0, None);
            }
            let Some(label) = self.text_format(14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false) else { return };
            let white = color(255, 255, 255, 0.96);
            let grey = color(255, 255, 255, 0.62);
            let cx = h / 2.0;
            let cy = h / 2.0;
            match self.state.clone() {
                State::Hidden => {}
                State::Listening { since, limit_s } => {
                    // red disc with a microphone glyph (shape + icon, not colour only)
                    if let Ok(red) = rt.CreateSolidColorBrush(&color(229, 72, 77, 1.0), None) {
                        let pulse = if self.reduce_motion { 0.0 } else { 1.5 * (t * 4.0).sin().abs() };
                        rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 14.0 + pulse, radiusY: 14.0 + pulse }, &red);
                    }
                    self.glyph(rt, '\u{E720}', "●", cx, cy, 15.0, white);
                    self.text(rt, crate::i18n::t().listening, &label, D2D_RECT_F { left: 50.0, top: 0.0, right: width - 90.0, bottom: h }, white, DWRITE_TEXT_ALIGNMENT_LEADING);
                    let secs = since.elapsed().as_secs();
                    let near_limit = limit_s > 0 && secs + 15 >= limit_s as u64;
                    let elapsed = format!("{}:{:02}", secs / 60, secs % 60);
                    if let Some(small) = self.text_format(12.0, DWRITE_FONT_WEIGHT_NORMAL, false) {
                        let c = if near_limit { color(255, 200, 80, 1.0) } else { grey };
                        self.text(rt, &elapsed, &small, D2D_RECT_F { left: width - 96.0, top: 0.0, right: width - 54.0, bottom: h }, c, DWRITE_TEXT_ALIGNMENT_TRAILING);
                    }
                    // level bars
                    let target = self.level;
                    for (i, b) in self.bars.iter_mut().enumerate() {
                        let shape = [0.55, 0.85, 1.0, 0.8, 0.6][i];
                        let wobble = if self.reduce_motion { 1.0 } else { 0.8 + 0.2 * (t * 9.0 + i as f32 * 1.3).sin() };
                        let goal = (target * shape * wobble).clamp(0.0, 1.0);
                        *b = if goal > *b { goal } else { *b * 0.82 + goal * 0.18 };
                    }
                    if let Ok(bar) = rt.CreateSolidColorBrush(&white, None) {
                        for (i, b) in self.bars.iter().enumerate() {
                            let bh = 4.0 + b * 18.0;
                            let x = width - 46.0 + i as f32 * 6.0;
                            let r = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: x, top: cy - bh / 2.0, right: x + 3.0, bottom: cy + bh / 2.0 }, radiusX: 1.5, radiusY: 1.5 };
                            rt.FillRoundedRectangle(&r, &bar);
                        }
                    }
                }
                State::Transcribing { part, .. } => {
                    // ring of dots, rotating
                    let n = 8;
                    let step = if self.reduce_motion { 0 } else { (t * 12.0) as usize };
                    for i in 0..n {
                        let a = i as f32 / n as f32 * std::f32::consts::TAU;
                        let fade = ((i + n - step % n) % n) as f32 / n as f32;
                        if let Ok(b) = rt.CreateSolidColorBrush(&color(120, 170, 255, 0.25 + 0.75 * fade), None) {
                            let p = Vector2 { X: cx + a.cos() * 9.0, Y: cy + a.sin() * 9.0 };
                            rt.FillEllipse(&D2D1_ELLIPSE { point: p, radiusX: 2.2, radiusY: 2.2 }, &b);
                        }
                    }
                    let label_text = match part {
                        Some((i, n)) => format!("{} {i}/{n}", crate::i18n::t().transcribing),
                        None => crate::i18n::t().transcribing.to_string(),
                    };
                    self.text(rt, &label_text, &label, D2D_RECT_F { left: 50.0, top: 0.0, right: width - 16.0, bottom: h }, white, DWRITE_TEXT_ALIGNMENT_LEADING);
                    if let (Some(d), Some(small)) = (self.detail.clone(), self.text_format(12.0, DWRITE_FONT_WEIGHT_NORMAL, false)) {
                        self.text(rt, &d, &small, D2D_RECT_F { left: 50.0, top: 0.0, right: width - 18.0, bottom: h }, grey, DWRITE_TEXT_ALIGNMENT_TRAILING);
                    }
                }
                State::Message { text, kind, .. } => {
                    let (c, glyph, fallback) = match kind {
                        MsgKind::Info => (color(90, 140, 250, 1.0), '\u{E946}', "i"),
                        MsgKind::Success => (color(46, 160, 67, 1.0), '\u{E73E}', "✓"),
                        MsgKind::Warning => (color(230, 160, 30, 1.0), '\u{E7BA}', "!"),
                    };
                    if let Ok(b) = rt.CreateSolidColorBrush(&c, None) {
                        if kind == MsgKind::Warning {
                            // diamond shape for warnings
                            let m = Matrix3x2::rotation_around(45.0, Vector2 { X: cx, Y: cy });
                            rt.SetTransform(&m);
                            let r = D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: cx - 11.0, top: cy - 11.0, right: cx + 11.0, bottom: cy + 11.0 }, radiusX: 3.0, radiusY: 3.0 };
                            rt.FillRoundedRectangle(&r, &b);
                            rt.SetTransform(&Matrix3x2::identity());
                        } else {
                            rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 13.0, radiusY: 13.0 }, &b);
                        }
                    }
                    self.glyph(rt, glyph, fallback, cx, cy, 13.0, white);
                    self.text(rt, &text, &label, D2D_RECT_F { left: 50.0, top: 0.0, right: width - 18.0, bottom: h }, white, DWRITE_TEXT_ALIGNMENT_LEADING);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn glyph(&self, rt: &ID2D1DCRenderTarget, glyph: char, fallback: &str, cx: f32, cy: f32, size: f32, c: D2D1_COLOR_F) {
        let r = D2D_RECT_F { left: cx - 14.0, top: cy - 14.0, right: cx + 14.0, bottom: cy + 14.0 };
        unsafe {
            if let Some(f) = self.text_format(size, DWRITE_FONT_WEIGHT_NORMAL, true) {
                self.text(rt, &glyph.to_string(), &f, r, c, DWRITE_TEXT_ALIGNMENT_CENTER);
            } else if let Some(f) = self.text_format(size, DWRITE_FONT_WEIGHT_BOLD, false) {
                self.text(rt, fallback, &f, r, c, DWRITE_TEXT_ALIGNMENT_CENTER);
            }
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.free_dib();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
