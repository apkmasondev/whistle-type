//! First-run window: explains the one-time model download, shows the source, progress and verification.

use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows::Win32::UI::Controls::{PBM_SETPOS, PBM_SETRANGE32, PBS_SMOOTH, PROGRESS_CLASSW};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{defer, with_app, App, EngineState, WhistleDownload};
use crate::i18n::{fmt, t, t as t_};
use crate::ui::{self, Font, Form};
use crate::{model, util};

const ID_TEXT: i32 = 301;
const ID_SOURCE: i32 = 302;
const ID_PROGRESS: i32 = 303;
const ID_STATUS: i32 = 304;
const ID_DOWNLOAD: i32 = 305;
const ID_IMPORT: i32 = 306;
const ID_CLOSE: i32 = 307;
const ID_TITLE: i32 = 308;
const ID_PAGE: i32 = 309;

pub struct SetupUi {
    form: Form,
    progress: HWND,
    status: HWND,
    download: HWND,
    import: HWND,
    close: HWND,
}

impl SetupUi {
    pub fn hwnd(&self) -> HWND {
        self.form.hwnd
    }

    pub fn create(app: &mut App) -> Option<SetupUi> {
        ui::register_class(w!("WhistleType.Setup"), Some(wndproc));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let hwnd = ui::create_centered(w!("WhistleType.Setup"), t().setup_window, 540.0, 412.0, style, WINDOW_EX_STYLE(0))
            .map_err(|e| crate::log_error!("ui: cannot create the setup window: {}", e.message()))
            .ok()?;
        let mut f = Form::new(hwnd);
        let tr = t();
        f.label(tr.setup_title, (24.0, 18.0, 492.0, 34.0), Font::Title, ID_TITLE);
        f.label(
            &fmt(tr.setup_text, &[("model", &format!("{} {}", model::MODEL_NAME, model::MODEL_VERSION)), ("size", &util::format_bytes(model::MODEL_SIZE))]),
            (24.0, 60.0, 492.0, 84.0),
            Font::Normal,
            ID_TEXT,
        );
        f.label(
            &fmt(
                tr.setup_source,
                &[
                    ("url", &model::model_page_url()),
                    ("rev", &&model::MODEL_REVISION[..10]),
                    ("lic", &model::MODEL_LICENSE),
                    ("sha", &&model::MODEL_SHA256[..16]),
                    ("dir", &model::model_dir().display()),
                ],
            ),
            (24.0, 150.0, 492.0, 70.0),
            Font::Small,
            ID_SOURCE,
        );
        let page = f.add(w!("BUTTON"), t().btn_model_page, WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32), WINDOW_EX_STYLE(0), ID_PAGE, (24.0, 228.0, 190.0, 26.0), Font::Normal);
        let _ = page;
        let progress = f.add(PROGRESS_CLASSW, "", WINDOW_STYLE(PBS_SMOOTH), WINDOW_EX_STYLE(0), ID_PROGRESS, (24.0, 272.0, 492.0, 14.0), Font::Normal);
        unsafe {
            SendMessageW(progress, PBM_SETRANGE32, Some(WPARAM(0)), Some(LPARAM(1000)));
        }
        let status = f.label("", (24.0, 294.0, 492.0, 40.0), Font::Normal, ID_STATUS);
        let download = f.button(t().btn_download, ID_DOWNLOAD, (24.0, 360.0, 140.0, 30.0), true);
        let import = f.button(t().btn_import, ID_IMPORT, (174.0, 360.0, 140.0, 30.0), false);
        let close = f.button(t().btn_not_now, ID_CLOSE, (540.0 - 24.0 - 120.0, 360.0, 120.0, 30.0), false);
        let s = SetupUi { form: f, progress, status, download, import, close };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        app.setup_ui = Some(s);
        refresh(app);
        app.setup_ui.take()
    }
}

pub fn refresh(app: &mut App) {
    let Some(s) = app.setup_ui.as_ref() else { return };
    let mine = app.download_item.is_none_or_whistle();
    let downloading = app.download_cancel.is_some() && mine;
    // a Whisper model or the GPU pack is downloading (Speech models window): one download at a time
    let other_busy = app.download_cancel.is_some() && !mine;
    let (text, pos, done) = match (&app.engine_state, downloading) {
        (_, true) => {
            let (d, t) = app.download_progress.unwrap_or((0, model::MODEL_SIZE));
            (
                fmt(t_().setup_downloading, &[("done", &util::format_bytes(d)), ("total", &util::format_bytes(t)), ("pct", &(d * 100 / t.max(1)))]),
                (d * 1000 / t.max(1)) as usize,
                false,
            )
        }
        (EngineState::Ready { .. }, _) => (t_().setup_ready.to_string(), 1000, true),
        (EngineState::Loading, _) => (t_().setup_loading.to_string(), 1000, false),
        (EngineState::ModelError(e), _) => (format!("⚠ {e}"), 0, false),
        (EngineState::RuntimeError(e), _) => (format!("⚠ {e}"), 0, false),
        _ if other_busy => (t_().models_busy.to_string(), 0, false),
        _ => match &app.download_error {
            Some(e) if mine => (fmt(t_().setup_error_retry, &[("e", e)]), 0, false),
            _ => (t_().setup_idle.to_string(), 0, false),
        },
    };
    ui::set_text(s.status, &text);
    unsafe {
        SendMessageW(s.progress, PBM_SETPOS, Some(WPARAM(pos)), None);
    }
    ui::enable(s.download, !downloading && !done && !other_busy);
    ui::enable(s.import, !downloading && !done);
    ui::set_text(s.download, if app.download_error.is_some() && mine { t_().btn_try_again } else { t_().btn_download });
    ui::set_text(s.close, if downloading { t_().btn_cancel } else if done { t_().btn_done } else { t_().btn_not_now });
}

fn pick_file(owner: HWND) -> Option<std::path::PathBuf> {
    let mut buf = vec![0u16; 1024];
    let filter: Vec<u16> = format!("{} (*.cact)\0*.cact\0{}\0*.*\0\0", t().file_filter_model.trim_end_matches(" (*.cact)"), t().file_filter_all)
        .encode_utf16()
        .collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        Some(std::path::PathBuf::from(util::from_wide(&buf)))
    } else {
        None
    }
}

fn on_command(app: &mut App, id: i32, code: u32) {
    if code != BN_CLICKED {
        return;
    }
    let Some(s) = app.setup_ui.as_ref() else { return };
    let hwnd = s.form.hwnd;
    match id {
        ID_DOWNLOAD => app.start_download(),
        ID_PAGE => ui::shell_open(&model::model_page_url()),
        ID_IMPORT => defer(move || {
            // modal file dialog outside the App borrow
            if let Some(p) = pick_file(hwnd) {
                let r = with_app(|a| a.import_model(&p)).unwrap_or(Ok(()));
                if let Err(e) = r {
                    ui::message_box(Some(hwnd), &e, "WhistleType", MB_OK | MB_ICONWARNING);
                }
                with_app(refresh);
            }
        }),
        ID_CLOSE => {
            if app.download_cancel.is_some() && app.download_item.is_none_or_whistle() {
                app.cancel_download();
            } else {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, m: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match m {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            with_app(|a| on_command(a, id, code));
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => LRESULT(ui::ctl_color_static(wparam, lparam, &[ID_SOURCE], &[])),
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let r = unsafe { &*(lparam.0 as *const RECT) };
            with_app(|a| {
                if let Some(s) = a.setup_ui.as_mut() {
                    s.form.relayout(dpi);
                }
            });
            unsafe {
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app(|a| {
                a.setup_ui = None;
                // closing the window keeps a running download going in the background (progress in Settings)
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, m, wparam, lparam) },
    }
}
