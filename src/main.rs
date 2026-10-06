#![windows_subsystem = "windows"]
//! WhistleType - local push-to-talk dictation for Windows.

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::*;

use whistletype::settings::{LoadOutcome, Settings};
use whistletype::{app, log_error, log_info, log_warn, msg, overlay, paths, ui, util};

unsafe extern "system" fn crash_filter(info: *const EXCEPTION_POINTERS) -> i32 {
    unsafe {
        if let Some(rec) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) {
            log_error!("fatal: native exception 0x{:08X} at {:?}", rec.ExceptionCode.0 as u32, rec.ExceptionAddress);
        }
    }
    whistletype::log::flush();
    0 // EXCEPTION_CONTINUE_SEARCH: let Windows Error Reporting handle it
}

fn main() {
    let background = std::env::args().any(|a| a == "--background");

    // Single instance per user session: a second launch just opens the settings of the running one.
    // A separate data folder (WHISTLETYPE_DATA_DIR, used by tests) is a separate instance scope.
    let mutex_name = match std::env::var_os("WHISTLETYPE_DATA_DIR") {
        Some(dir) => format!("Local\\WhistleType.SingleInstance.{}", &util::sha256_bytes(dir.to_string_lossy().as_bytes())[..16]),
        None => "Local\\WhistleType.SingleInstance".to_string(),
    };
    let mutex_name = util::WStr::new(&mutex_name);
    let _mutex = unsafe { CreateMutexW(None, false, mutex_name.pcwstr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            if let Ok(h) = FindWindowW(util::WStr::new(msg::MAIN_WINDOW_CLASS).pcwstr(), None) {
                let _ = AllowSetForegroundWindow(u32::MAX); // ASFW_ANY
                let _ = PostMessageW(Some(h), msg::WM_WT_ACTIVATE, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }

    let log_path = whistletype::log::init(&paths::logs_dir());
    std::panic::set_hook(Box::new(|info| {
        log_error!("panic: {info}");
        whistletype::log::flush();
    }));
    unsafe {
        SetUnhandledExceptionFilter(Some(crash_filter));
    }
    let t0 = std::time::Instant::now();
    log_info!(
        "WhistleType {} starting (pid {}, portable: {}, background: {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        paths::is_portable(),
        background
    );
    if let Err(e) = &log_path {
        // still run; there is just no log file
        eprintln!("log: {e}");
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    ui::init_common_controls();

    let settings_path = paths::settings_file();
    let (settings, outcome) = Settings::load(&settings_path);
    whistletype::models::CudaPack::apply_scheduled_removal();
    whistletype::i18n::set(whistletype::i18n::UiLang::from_code(&settings.ui_language).unwrap_or(whistletype::i18n::UiLang::En));
    let first_run = outcome == LoadOutcome::FirstRun;
    match &outcome {
        LoadOutcome::Corrupt(e) => log_warn!("settings: file was corrupt ({e}); kept as settings.json.corrupt, using defaults"),
        LoadOutcome::Error(e) => log_warn!("settings: cannot read ({e}); using defaults"),
        _ => {}
    }

    // Hidden top-level window: receives hotkey/tray/worker messages and Explorer's TaskbarCreated broadcast.
    app::register_taskbar_created();
    let class = util::WStr::new(msg::MAIN_WINDOW_CLASS);
    let hwnd = unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(app::main_wndproc), hInstance: hinst.into(), lpszClassName: class.pcwstr(), ..Default::default() };
        RegisterClassW(&wc);
        CreateWindowExW(WINDOW_EX_STYLE(0), class.pcwstr(), w!("WhistleType"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(hinst.into()), None)
    };
    let hwnd = match hwnd {
        Ok(h) => h,
        Err(e) => {
            log_error!("cannot create the main window: {}", e.message());
            let text = whistletype::i18n::fmt(whistletype::i18n::t().start_failed, &[("e", &e.message())]);
            ui::message_box(None, &text, "WhistleType", MB_OK | MB_ICONERROR);
            return;
        }
    };
    msg::set_main_hwnd(hwnd);

    let ov = match overlay::Overlay::create() {
        Ok(o) => Some(o),
        Err(e) => {
            log_warn!("overlay unavailable: {}", e.message());
            None
        }
    };
    let mut a = app::App::new(hwnd, settings, settings_path.clone());
    a.background = background;
    if first_run {
        a.save_settings();
    }
    app::install(a, ov);
    app::with_app(|a| a.start());
    log_info!("started in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);

    app::with_app(|a| {
        if a.needs_model_setup() {
            if background {
                let t = whistletype::i18n::t();
                let size = util::format_bytes(whistletype::model::MODEL_SIZE);
                a.tray.notify(t.needs_model_title, &whistletype::i18n::fmt(t.needs_model_text, &[("size", &size)]), true);
            } else {
                a.open_setup();
            }
        } else if !background {
            a.open_settings();
        }
    });

    unsafe {
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            // Tab/Enter/Esc navigation in our forms
            let forms: [Option<HWND>; 4] = app::with_app(|a| {
                [
                    a.settings_ui.as_ref().map(|s| s.hwnd()),
                    a.vocab_ui.as_ref().map(|v| v.hwnd()),
                    a.setup_ui.as_ref().map(|s| s.hwnd()),
                    a.models_ui.as_ref().map(|s| s.hwnd()),
                ]
            })
            .unwrap_or([None; 4]);
            if forms.iter().flatten().any(|&f| IsDialogMessageW(f, &m).as_bool()) {
                if m.message == WM_KEYDOWN {
                    app::with_app(|a| a.focus_moved());
                }
                continue;
            }
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    app::uninstall();
    log_info!("exit");
    whistletype::log::flush();
    unsafe { CoUninitialize() };
}
