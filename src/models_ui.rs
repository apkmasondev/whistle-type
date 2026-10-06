//! Model Manager: the FAST model (Whistle), the ACCURATE models (Whisper) and the optional GPU pack.
//! Shows size, Polish quality, where each one runs and its state; downloads (on click only), deletes and selects.

use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{defer, with_app, App, DownloadItem, EngineState, WhisperState};
use crate::i18n::{fmt, t};
use crate::models::{self, CudaPack, Tier, WhisperModel, WHISPER_MODELS};
use crate::ui::{self, Font, Form};
use crate::util;
use crate::{log_info, model};

const ID_INFO: i32 = 401;
const ID_GPU: i32 = 402;
const ID_LIST: i32 = 403;
const ID_DOWNLOAD: i32 = 404;
const ID_DELETE: i32 = 405;
const ID_USE: i32 = 406;
const ID_PROGRESS: i32 = 407;
const ID_STATUS: i32 = 408;
const ID_LICENCES: i32 = 409;
const ID_CLOSE: i32 = 410;

/// Polish quality of Whistle on the same 1..4 scale as [`WhisperModel::polish`] (measured, see PERFORMANCE.md).
pub const WHISTLE_POLISH: u8 = 1;

/// Column widths in DIPs.
const COL_WIDTHS: [f32; 6] = [210.0, 82.0, 92.0, 88.0, 120.0, 122.0];

/// Rows of the list, in order.
fn rows() -> Vec<DownloadItem> {
    let mut v = vec![DownloadItem::Whistle];
    v.extend(WHISPER_MODELS.iter().map(|m| DownloadItem::Whisper(m.id)));
    v.push(DownloadItem::CudaPack);
    v
}

pub struct ModelsUi {
    form: Form,
    list: HWND,
    gpu: HWND,
    download: HWND,
    delete: HWND,
    use_btn: HWND,
    progress: HWND,
    status: HWND,
    /// result of the last action, shown until the next one
    message: Option<String>,
    gpu_text: String,
}

impl ModelsUi {
    pub fn hwnd(&self) -> HWND {
        self.form.hwnd
    }

    pub fn create(_app: &mut App) -> Option<ModelsUi> {
        ui::register_class(w!("WhistleType.Models"), Some(wndproc));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let (width, height) = (760.0, 470.0);
        let hwnd = ui::create_centered(w!("WhistleType.Models"), t().models_title, width, height, style, WINDOW_EX_STYLE(0))
            .map_err(|e| crate::log_error!("ui: cannot create the models window: {}", e.message()))
            .ok()?;
        let tr = t();
        let mut f = Form::new(hwnd);
        let (lx, cw) = (20.0, width - 40.0);
        f.label(tr.models_info, (lx, 14.0, cw, 36.0), Font::Small, ID_INFO);
        let gpu_text = match models::nvidia_gpu_cached() {
            Some(g) => fmt(
                tr.models_gpu,
                &[
                    ("name", &g.name),
                    ("vram", &util::format_bytes(g.vram_bytes)),
                    ("pack", &if CudaPack::installed() { tr.pack_installed } else { tr.pack_missing }),
                ],
            ),
            None => tr.models_no_gpu.to_string(),
        };
        let gpu = f.label(&gpu_text, (lx, 52.0, cw, 20.0), Font::Normal, ID_GPU);
        let list = f.add(
            WC_LISTVIEWW,
            "",
            WS_TABSTOP | WS_BORDER | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER),
            WINDOW_EX_STYLE(0),
            ID_LIST,
            (lx, 80.0, cw, 196.0),
            Font::Normal,
        );
        let names = [tr.col_model, tr.col_mode, tr.col_size, tr.col_polish, tr.col_runs_on, tr.col_status];
        let cols: Vec<(&str, f32)> = names.into_iter().zip(COL_WIDTHS).collect();
        unsafe {
            SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                Some(WPARAM(0)),
                Some(LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP) as isize)),
            );
            for (i, (name, w)) in cols.iter().enumerate() {
                let mut text = util::wide(name);
                let col = LVCOLUMNW {
                    mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
                    cx: f.px(*w),
                    pszText: PWSTR(text.as_mut_ptr()),
                    iSubItem: i as i32,
                    ..Default::default()
                };
                SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&col as *const _ as isize)));
            }
            for (i, _) in rows().iter().enumerate() {
                let mut empty = util::wide("");
                let item = LVITEMW { mask: LVIF_TEXT, iItem: i as i32, pszText: PWSTR(empty.as_mut_ptr()), ..Default::default() };
                SendMessageW(list, LVM_INSERTITEMW, Some(WPARAM(0)), Some(LPARAM(&item as *const _ as isize)));
            }
        }
        let y = 288.0;
        let download = f.button(tr.btn_download, ID_DOWNLOAD, (lx, y, 120.0, 28.0), false);
        let delete = f.button(tr.btn_delete, ID_DELETE, (lx + 128.0, y, 100.0, 28.0), false);
        let use_btn = f.button(tr.btn_use_accurate, ID_USE, (lx + 236.0, y, 170.0, 28.0), false);
        let progress = f.add(PROGRESS_CLASSW, "", WINDOW_STYLE(PBS_SMOOTH), WINDOW_EX_STYLE(0), ID_PROGRESS, (lx + 420.0, y + 7.0, cw - 420.0, 14.0), Font::Normal);
        unsafe {
            SendMessageW(progress, PBM_SETRANGE32, Some(WPARAM(0)), Some(LPARAM(1000)));
        }
        let status = f.label("", (lx, y + 40.0, cw, 54.0), Font::Normal, ID_STATUS);
        f.label(tr.models_licences, (lx, height - 64.0, cw, 18.0), Font::Small, ID_LICENCES);
        f.button(tr.btn_close, ID_CLOSE, (width - 20.0 - 110.0, height - 40.0, 110.0, 28.0), true);
        f.fit(width, height);
        let m = ModelsUi { form: f, list, gpu, download, delete, use_btn, progress, status, message: None, gpu_text };
        // start on the selected ACCURATE model (or the FAST row)
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(list));
        }
        Some(m)
    }

    fn selected(&self) -> Option<DownloadItem> {
        let i = unsafe { SendMessageW(self.list, LVM_GETNEXTITEM, Some(WPARAM(usize::MAX)), Some(LPARAM(LVNI_SELECTED as isize))) }.0;
        if i < 0 {
            return None;
        }
        rows().get(i as usize).copied()
    }

    fn select(&self, idx: usize) {
        let state = LVITEMW {
            stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
            state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
            ..Default::default()
        };
        unsafe {
            SendMessageW(self.list, LVM_SETITEMSTATE, Some(WPARAM(idx)), Some(LPARAM(&state as *const _ as isize)));
        }
    }

    fn set_cell(&self, row: usize, col: i32, text: &str) {
        let mut w = util::wide(text);
        let item = LVITEMW { iSubItem: col, pszText: PWSTR(w.as_mut_ptr()), ..Default::default() };
        unsafe {
            SendMessageW(self.list, LVM_SETITEMTEXTW, Some(WPARAM(row)), Some(LPARAM(&item as *const _ as isize)));
        }
    }
}

fn quality(q: u8) -> &'static str {
    match q {
        0 | 1 => t().q_1,
        2 => t().q_2,
        3 => t().q_3,
        _ => t().q_4,
    }
}

fn item_name(item: DownloadItem) -> String {
    match item {
        DownloadItem::Whistle => format!("{} {}", model::MODEL_NAME, model::MODEL_VERSION),
        DownloadItem::Whisper(id) => models::whisper_model(id).map(|m| m.name.to_string()).unwrap_or_default(),
        DownloadItem::CudaPack => t().gpu_pack_name.to_string(),
    }
}

fn item_size(item: DownloadItem) -> u64 {
    match item {
        DownloadItem::Whistle => model::MODEL_SIZE,
        DownloadItem::Whisper(id) => models::whisper_model(id).map(|m| m.size).unwrap_or(0),
        DownloadItem::CudaPack => CudaPack::SIZE,
    }
}

fn item_host(item: DownloadItem) -> &'static str {
    match item {
        DownloadItem::Whistle => model::MODEL_HOST,
        DownloadItem::Whisper(_) => "huggingface.co",
        DownloadItem::CudaPack => CudaPack::HOST,
    }
}

fn item_dir(item: DownloadItem) -> String {
    match item {
        DownloadItem::Whistle => model::model_dir(),
        DownloadItem::Whisper(_) => models::whisper_models_dir(),
        DownloadItem::CudaPack => CudaPack::dir(),
    }
    .display()
    .to_string()
}

pub fn installed(app: &App, item: DownloadItem) -> bool {
    match item {
        DownloadItem::Whistle => !matches!(app.engine_state, EngineState::NoModel),
        DownloadItem::Whisper(id) => models::whisper_model(id).is_some_and(models::whisper_installed),
        DownloadItem::CudaPack => CudaPack::installed(),
    }
}

fn downloading(app: &App, item: DownloadItem) -> Option<u64> {
    if app.download_cancel.is_some() && app.download_item == Some(item) {
        let (d, total) = app.download_progress.unwrap_or((0, 1));
        return Some(d * 1000 / total.max(1));
    }
    None
}

fn status_text(app: &App, item: DownloadItem) -> String {
    let tr = t();
    if let Some(permille) = downloading(app, item) {
        return if permille >= 1000 { tr.st_installing.to_string() } else { fmt(tr.st_downloading, &[("pct", &(permille / 10))]) };
    }
    if app.download_cancel.is_none() && app.download_item == Some(item) && app.download_error.is_some() {
        return tr.st_failed.to_string();
    }
    match item {
        DownloadItem::Whistle => match app.engine_state {
            EngineState::Ready { .. } => tr.st_in_use.to_string(),
            EngineState::NoModel => tr.st_not_downloaded.to_string(),
            EngineState::ModelError(_) | EngineState::RuntimeError(_) => tr.st_failed.to_string(),
            _ => tr.st_installed.to_string(),
        },
        DownloadItem::Whisper(id) => {
            let Some(m) = models::whisper_model(id) else { return String::new() };
            if !models::whisper_installed(m) {
                return tr.st_not_downloaded.to_string();
            }
            match &app.whisper_state {
                WhisperState::Ready(s) if s.model_id == id => format!("{} · {}", tr.st_in_use, s.device.short()),
                WhisperState::Loading { model_id } if model_id == id => tr.status_loading.to_string(),
                WhisperState::Failed { model_id, .. } if model_id == id => tr.st_failed.to_string(),
                _ if app.settings.whisper_model == id => tr.st_selected.to_string(),
                _ => tr.st_installed.to_string(),
            }
        }
        DownloadItem::CudaPack => {
            if CudaPack::removal_pending() {
                return tr.restart_to_remove.to_string();
            }
            if !CudaPack::installed() {
                return tr.st_not_downloaded.to_string();
            }
            match &app.whisper_state {
                WhisperState::Ready(s) if s.runtime_dir == CudaPack::dir() => tr.st_in_use.to_string(),
                _ => tr.st_installed.to_string(),
            }
        }
    }
}

fn runs_on(item: DownloadItem) -> &'static str {
    match item {
        DownloadItem::Whistle => t().runs_cpu,
        DownloadItem::Whisper(id) => match models::whisper_model(id).map(|m| m.tier) {
            Some(Tier::Gpu) => t().runs_gpu,
            _ => t().runs_cpu_gpu,
        },
        DownloadItem::CudaPack => t().runs_nvidia,
    }
}

/// Re-reads every row and the buttons from the app state.
pub fn refresh(app: &mut App) {
    let Some(m) = app.models_ui.as_ref() else { return };
    let tr = t();
    for (i, item) in rows().into_iter().enumerate() {
        let (mode, polish, size) = match item {
            DownloadItem::Whistle => ("FAST", quality(WHISTLE_POLISH).to_string(), util::format_bytes(model::MODEL_SIZE)),
            DownloadItem::Whisper(id) => {
                let wm: &WhisperModel = models::whisper_model(id).expect("catalogue");
                ("ACCURATE", quality(wm.polish).to_string(), util::format_bytes(wm.size))
            }
            DownloadItem::CudaPack => ("ACCURATE", "—".to_string(), util::format_bytes(CudaPack::SIZE)),
        };
        m.set_cell(i, 0, &item_name(item));
        m.set_cell(i, 1, mode);
        m.set_cell(i, 2, &size);
        m.set_cell(i, 3, &polish);
        m.set_cell(i, 4, runs_on(item));
        m.set_cell(i, 5, &status_text(app, item));
    }
    if m.selected().is_none() {
        let idx = rows().iter().position(|r| *r == DownloadItem::Whisper(models::DEFAULT_WHISPER_GPU)).unwrap_or(0);
        let idx = models::whisper_model(&app.settings.whisper_model)
            .and_then(|wm| rows().iter().position(|r| *r == DownloadItem::Whisper(wm.id)))
            .unwrap_or(idx);
        m.select(idx);
    }
    let sel = m.selected();
    let busy = app.download_cancel.is_some();
    let this_downloading = sel.is_some_and(|s| downloading(app, s).is_some());
    let inst = sel.is_some_and(|s| installed(app, s));
    ui::set_text(m.download, if this_downloading { tr.btn_cancel } else { tr.btn_download });
    ui::enable(m.download, this_downloading || (!busy && sel.is_some() && !inst));
    let removal_pending = sel == Some(DownloadItem::CudaPack) && CudaPack::removal_pending();
    let deletable = inst && !matches!(sel, Some(DownloadItem::Whistle));
    ui::enable(m.delete, deletable && !this_downloading && !removal_pending);
    let use_ok = matches!(sel, Some(DownloadItem::Whisper(id)) if inst && app.settings.whisper_model != id);
    ui::enable(m.use_btn, use_ok);
    let gpu_text = match models::nvidia_gpu_cached() {
        Some(g) => fmt(
            tr.models_gpu,
            &[("name", &g.name), ("vram", &util::format_bytes(g.vram_bytes)), ("pack", &if CudaPack::installed() { tr.pack_installed } else { tr.pack_missing })],
        ),
        None => tr.models_no_gpu.to_string(),
    };
    if gpu_text != m.gpu_text {
        ui::set_text(m.gpu, &gpu_text);
    }
    let permille = app.download_progress.filter(|_| busy).map(|(d, total)| d * 1000 / total.max(1));
    ui::show(m.progress, permille.is_some());
    unsafe {
        SendMessageW(m.progress, PBM_SETPOS, Some(WPARAM(permille.unwrap_or(0) as usize)), None);
    }
    let status = if busy {
        let (d, total) = app.download_progress.unwrap_or((0, 0));
        let name = app.download_item.map(item_name).unwrap_or_default();
        if d >= total && total > 0 {
            format!("{name}: {}", tr.st_installing)
        } else {
            format!(
                "{name}: {}",
                fmt(tr.setup_downloading, &[("done", &util::format_bytes(d)), ("total", &util::format_bytes(total)), ("pct", &(d * 100 / total.max(1)))])
            )
        }
    } else if let (Some(e), Some(item)) = (&app.download_error, app.download_item) {
        format!("{}: {}", item_name(item), fmt(tr.setup_error_retry, &[("e", e)]))
    } else if let Some(msg) = &m.message {
        msg.clone()
    } else {
        match sel {
            Some(DownloadItem::CudaPack) => fmt(
                tr.models_gpu_pack_size,
                &[("dl", &util::format_bytes(CudaPack::SIZE)), ("disk", &util::format_bytes(CudaPack::installed_bytes()))],
            ),
            Some(item) => format!("{} · {}", item_name(item), item_dir(item)),
            None => String::new(),
        }
    };
    ui::set_text(m.status, &status);
    if let Some(m) = app.models_ui.as_mut() {
        m.gpu_text = gpu_text;
    }
}

/// Called by the app when a download finished.
pub fn on_download_done(app: &mut App, item: DownloadItem, ok: bool) {
    if let Some(m) = app.models_ui.as_mut() {
        m.message = ok.then(|| fmt(t().models_downloaded, &[("name", &item_name(item))]));
    }
    refresh(app);
}

fn set_message(app: &mut App, msg: String) {
    if let Some(m) = app.models_ui.as_mut() {
        m.message = Some(msg);
    }
    // a new action replaces an old download error
    if app.download_cancel.is_none() {
        app.download_error = None;
    }
    refresh(app);
}

fn on_command(app: &mut App, id: i32, code: u32) {
    let Some(m) = app.models_ui.as_ref() else { return };
    if code != BN_CLICKED {
        return;
    }
    let owner = m.form.hwnd;
    let sel = m.selected();
    match (id, sel) {
        (ID_DOWNLOAD, Some(item)) => {
            if downloading(app, item).is_some() {
                app.cancel_download();
                return;
            }
            if app.download_cancel.is_some() {
                set_message(app, t().models_busy.to_string());
                return;
            }
            let text = fmt(
                t().models_confirm_download,
                &[("name", &item_name(item)), ("size", &util::format_bytes(item_size(item))), ("host", &item_host(item)), ("dir", &item_dir(item))],
            );
            // modal confirmation outside the App borrow
            defer(move || {
                if ui::message_box(Some(owner), &text, "WhistleType", MB_OKCANCEL | MB_ICONQUESTION) == IDOK {
                    with_app(|a| {
                        if let Some(m) = a.models_ui.as_mut() {
                            m.message = None;
                        }
                        a.start_item_download(item);
                        refresh(a);
                    });
                }
            });
        }
        (ID_DELETE, Some(DownloadItem::Whistle)) => set_message(app, t().models_cannot_delete_fast.to_string()),
        (ID_DELETE, Some(item)) => {
            let text = fmt(t().models_confirm_delete, &[("name", &item_name(item))]);
            defer(move || {
                if ui::message_box(Some(owner), &text, "WhistleType", MB_OKCANCEL | MB_ICONQUESTION) == IDOK {
                    with_app(|a| {
                        log_info!("models: delete {item:?} (user confirmed)");
                        let msg = match a.delete_item(item) {
                            Ok(()) => fmt(t().models_deleted, &[("name", &item_name(item))]),
                            Err(e) => fmt(t().models_error, &[("e", &e)]),
                        };
                        set_message(a, msg);
                        if a.settings_ui.is_some() {
                            crate::settings_ui::refresh_engine(a);
                        }
                    });
                }
            });
        }
        (ID_USE, Some(DownloadItem::Whisper(id))) => {
            app.settings.whisper_model = id.to_string();
            app.save_settings();
            app.ensure_whisper();
            if app.settings_ui.is_some() {
                crate::settings_ui::refresh_engine(app);
            }
            let msg = format!("{} → ACCURATE", item_name(DownloadItem::Whisper(id)));
            set_message(app, msg);
        }
        (ID_CLOSE, _) => unsafe {
            let _ = PostMessageW(Some(owner), WM_CLOSE, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if id == IDCANCEL.0 {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            } else {
                with_app(|a| on_command(a, id, code));
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
            if hdr.idFrom == ID_LIST as usize && hdr.code == LVN_ITEMCHANGED {
                let nm = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
                if (nm.uChanged.0 & LVIF_STATE.0) != 0 && (nm.uNewState ^ nm.uOldState) & LVIS_SELECTED.0 != 0 {
                    // selection changed: forget the last message, update the buttons (outside this notification)
                    defer(|| {
                        with_app(|a| {
                            if let Some(m) = a.models_ui.as_mut() {
                                m.message = None;
                            }
                            refresh(a);
                        });
                    });
                }
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => LRESULT(ui::ctl_color_static(wparam, lparam, &[ID_INFO, ID_LICENCES], &[])),
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let r = unsafe { &*(lparam.0 as *const RECT) };
            with_app(|a| {
                if let Some(m) = a.models_ui.as_mut() {
                    m.form.relayout(dpi);
                    for (i, w) in COL_WIDTHS.iter().enumerate() {
                        unsafe {
                            SendMessageW(m.list, LVM_SETCOLUMNWIDTH, Some(WPARAM(i)), Some(LPARAM(m.form.px(*w) as isize)));
                        }
                    }
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
            with_app(|a| a.models_ui = None);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
