//! Custom vocabulary editor (keyword biasing for Whistle).

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Controls::EM_SETLIMITTEXT;

use crate::app::{defer, with_app, App};
use crate::settings::{clean_vocabulary, DEFAULT_VOCABULARY, MAX_VOCABULARY_ENTRIES, MAX_VOCABULARY_ENTRY_CHARS};
use crate::i18n::{fmt, t};
use crate::ui::{self, Font, Form};

const ID_LIST: i32 = 201;
const ID_EDIT: i32 = 202;
const ID_ADD: i32 = 203;
const ID_REPLACE: i32 = 204;
const ID_REMOVE: i32 = 205;
const ID_USE: i32 = 206;
const ID_DEFAULTS: i32 = 207;
const ID_CLOSE: i32 = 208;
const ID_INFO: i32 = 209;
const ID_STATUS: i32 = 210;

pub struct VocabUi {
    form: Form,
    list: HWND,
    edit: HWND,
    use_box: HWND,
    status: HWND,
}

impl VocabUi {
    pub fn hwnd(&self) -> HWND {
        self.form.hwnd
    }

    pub fn create(app: &mut App) -> Option<VocabUi> {
        ui::register_class(w!("WhistleType.Vocabulary"), Some(wndproc));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let hwnd = ui::create_centered(w!("WhistleType.Vocabulary"), t().vocab_title, 452.0, 500.0, style, WINDOW_EX_STYLE(0))
            .map_err(|e| crate::log_error!("ui: cannot create the vocabulary window: {}", e.message()))
            .ok()?;
        let mut f = Form::new(hwnd);
        f.label(
            t().vocab_info,
            (20.0, 16.0, 412.0, 54.0),
            Font::Small,
            ID_INFO,
        );
        let use_box = f.check(t().vocab_use, ID_USE, (20.0, 74.0, 400.0, 22.0));
        let list = f.add(
            w!("LISTBOX"),
            "",
            WS_TABSTOP | WS_VSCROLL | WS_BORDER | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
            WINDOW_EX_STYLE(0),
            ID_LIST,
            (20.0, 104.0, 270.0, 296.0),
            Font::Normal,
        );
        f.button(t().btn_remove, ID_REMOVE, (300.0, 104.0, 132.0, 28.0), false);
        f.button(t().btn_defaults, ID_DEFAULTS, (300.0, 140.0, 132.0, 28.0), false);
        let edit = f.add(w!("EDIT"), "", WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32), WINDOW_EX_STYLE(0), ID_EDIT, (20.0, 410.0, 270.0, 26.0), Font::Normal);
        unsafe {
            SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(MAX_VOCABULARY_ENTRY_CHARS)), None);
        }
        f.button(t().btn_add, ID_ADD, (300.0, 409.0, 64.0, 28.0), true);
        f.button(t().btn_replace, ID_REPLACE, (368.0, 409.0, 64.0, 28.0), false);
        let status = f.label("", (20.0, 446.0, 270.0, 40.0), Font::Small, ID_STATUS);
        f.button(t().btn_close, ID_CLOSE, (300.0, 456.0, 132.0, 28.0), false);
        let v = VocabUi { form: f, list, edit, use_box, status };
        ui::check_set(v.use_box, app.settings.use_vocabulary);
        v.fill(app, -1);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
        }
        Some(v)
    }

    fn fill(&self, app: &App, select: i32) {
        ui::list_clear(self.list);
        for v in &app.settings.vocabulary {
            ui::list_add(self.list, v);
        }
        if select >= 0 {
            ui::list_set(self.list, select);
        }
        ui::set_text(self.status, &fmt(t().vocab_count, &[("n", &app.settings.vocabulary.len()), ("max", &MAX_VOCABULARY_ENTRIES)]));
    }
}

fn commit(app: &mut App, select: i32) {
    app.settings.vocabulary = clean_vocabulary(&app.settings.vocabulary);
    app.save_settings();
    if let Some(v) = app.vocab_ui.as_ref() {
        v.fill(app, select);
    }
    if let Some(s) = app.settings_ui.as_ref() {
        s.update_vocab_info(app);
    }
}

fn on_command(app: &mut App, id: i32, code: u32) {
    let Some(v) = app.vocab_ui.as_ref() else { return };
    let (list, edit) = (v.list, v.edit);
    let clicked = code == BN_CLICKED;
    match id {
        ID_LIST if code == LBN_SELCHANGE => {
            let i = ui::list_get(list);
            if let Some(s) = app.settings.vocabulary.get(i.max(0) as usize) {
                ui::set_text(edit, s);
            }
        }
        ID_ADD if clicked => {
            let text = ui::get_text(edit).trim().to_string();
            if text.is_empty() {
                return;
            }
            if app.settings.vocabulary.iter().any(|x| x.eq_ignore_ascii_case(&text)) {
                if let Some(v) = app.vocab_ui.as_ref() {
                    ui::set_text(v.status, t().vocab_exists);
                }
                return;
            }
            if app.settings.vocabulary.len() >= MAX_VOCABULARY_ENTRIES {
                if let Some(v) = app.vocab_ui.as_ref() {
                    ui::set_text(v.status, t().vocab_full);
                }
                return;
            }
            app.settings.vocabulary.push(text);
            let idx = app.settings.vocabulary.len() as i32 - 1;
            ui::set_text(edit, "");
            commit(app, idx);
        }
        ID_REPLACE if clicked => {
            let i = ui::list_get(list);
            let text = ui::get_text(edit).trim().to_string();
            if i < 0 || text.is_empty() {
                return;
            }
            if let Some(slot) = app.settings.vocabulary.get_mut(i as usize) {
                *slot = text;
            }
            commit(app, i);
        }
        ID_REMOVE if clicked => {
            let i = ui::list_get(list);
            if i < 0 || i as usize >= app.settings.vocabulary.len() {
                return;
            }
            app.settings.vocabulary.remove(i as usize);
            let next = (i as usize).min(app.settings.vocabulary.len().saturating_sub(1)) as i32;
            commit(app, if app.settings.vocabulary.is_empty() { -1 } else { next });
        }
        ID_USE if clicked => {
            app.settings.use_vocabulary = ui::check_get(app.vocab_ui.as_ref().unwrap().use_box);
            commit(app, ui::list_get(list));
        }
        ID_DEFAULTS if clicked => {
            let owner = app.vocab_ui.as_ref().map(|v| v.form.hwnd);
            // modal confirmation outside the App borrow
            defer(move || {
                let r = ui::message_box(owner, t().vocab_confirm_defaults, "WhistleType", MB_OKCANCEL | MB_ICONQUESTION);
                if r == IDOK {
                    with_app(|a| {
                        a.settings.vocabulary = DEFAULT_VOCABULARY.iter().map(|s| s.to_string()).collect();
                        commit(a, -1);
                    });
                }
            });
        }
        ID_CLOSE if clicked => unsafe {
            let _ = PostMessageW(Some(app.vocab_ui.as_ref().unwrap().form.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, m: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match m {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if id == IDOK.0 {
                // Enter in the edit box
                with_app(|a| on_command(a, ID_ADD, BN_CLICKED));
            } else if id == IDCANCEL.0 {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            } else {
                with_app(|a| on_command(a, id, code));
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => LRESULT(ui::ctl_color_static(wparam, lparam, &[ID_INFO, ID_STATUS], &[])),
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let r = unsafe { &*(lparam.0 as *const RECT) };
            with_app(|a| {
                if let Some(v) = a.vocab_ui.as_mut() {
                    v.form.relayout(dpi);
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
            with_app(|a| a.vocab_ui = None);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, m, wparam, lparam) },
    }
}
