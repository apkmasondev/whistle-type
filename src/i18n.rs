//! User-interface strings in English and Polish.
//!
//! Every string is declared once with both translations, so a missing translation is a compile error.
//! Placeholders like `{key}` are filled with [`fmt`]. Logs stay in English.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiLang {
    En,
    Pl,
}

impl UiLang {
    pub fn code(self) -> &'static str {
        match self {
            UiLang::En => "en",
            UiLang::Pl => "pl",
        }
    }
    pub fn from_code(code: &str) -> Option<UiLang> {
        match code {
            "en" => Some(UiLang::En),
            "pl" => Some(UiLang::Pl),
            _ => None,
        }
    }
}

/// (code, name shown in the selector) - each language is named in itself.
pub const UI_LANGUAGES: &[(&str, &str)] = &[("pl", "Polski"), ("en", "English")];

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: UiLang) {
    CURRENT.store(lang as u8, Ordering::SeqCst);
}

pub fn current() -> UiLang {
    if CURRENT.load(Ordering::SeqCst) == UiLang::Pl as u8 {
        UiLang::Pl
    } else {
        UiLang::En
    }
}

/// Polish when Windows' display language is Polish, English otherwise.
pub fn system_default() -> UiLang {
    let lang = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if lang & 0x3FF == 0x15 {
        UiLang::Pl
    } else {
        UiLang::En
    }
}

/// Strings of the current UI language.
pub fn t() -> &'static Strings {
    match current() {
        UiLang::En => &EN,
        UiLang::Pl => &PL,
    }
}

/// Replaces `{name}` placeholders.
pub fn fmt(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut s = template.to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

macro_rules! strings {
    ($($name:ident: $en:expr, $pl:expr;)*) => {
        pub struct Strings { $(pub $name: &'static str,)* }
        pub static EN: Strings = Strings { $($name: $en,)* };
        pub static PL: Strings = Strings { $($name: $pl,)* };
        /// (name, English, Polish) of every string, for tests.
        #[cfg(test)]
        pub static ALL: &[(&str, &str, &str)] = &[$((stringify!($name), $en, $pl),)*];
    };
}

strings! {
    // ---- overlay ---------------------------------------------------------------------------------
    listening: "Listening…", "Słucham…";
    transcribing: "Transcribing…", "Rozpoznaję…";
    model_missing_opening_setup: "Speech model not installed — opening setup", "Brak modelu mowy — otwieram konfigurację";
    engine_unavailable_see_settings: "Speech engine is not available — see Settings", "Silnik mowy jest niedostępny — zobacz Ustawienia";
    cancelled: "Cancelled", "Anulowano";
    too_short_hold: "Too short — hold {key} while you speak", "Za krótko — przytrzymaj {key}, gdy mówisz";
    too_short_press: "Too short — press {key}, speak, press {key} again", "Za krótko — naciśnij {key}, mów i naciśnij {key} ponownie";
    no_sound: "No sound from the microphone", "Brak dźwięku z mikrofonu";
    no_speech: "No speech detected", "Nie wykryto mowy";
    no_speech_recognized: "No speech recognized", "Nie rozpoznano mowy";
    engine_not_running: "Speech engine is not running", "Silnik mowy nie działa";
    not_pasted: "Not pasted here — the text is on the clipboard (Ctrl+V)", "Nie wklejono — tekst jest w schowku (Ctrl+V)";
    copied: "Copied to the clipboard", "Skopiowano do schowka";
    admin_window: "Admin window: press {key} to paste", "Okno administratora: naciśnij {key}, aby wkleić";
    insert_failed: "Could not insert the text: {e}", "Nie udało się wstawić tekstu: {e}";
    mic_disconnected: "Microphone disconnected", "Mikrofon został odłączony";
    restart_to_remove: "The GPU pack is in use; it will be removed when WhistleType restarts.", "Pakiet GPU jest w użyciu; zostanie usunięty po ponownym uruchomieniu WhistleType.";
    used_fast_instead: "Accurate model not ready — used FAST for this dictation", "Model dokładny nie jest gotowy — użyto trybu FAST";
    restart_for_gpu: "Restart WhistleType to use the GPU pack you installed.", "Uruchom WhistleType ponownie, aby użyć zainstalowanego pakietu GPU.";
    gpu_fallback_cpu: "Whisper could not start on the GPU and runs on the CPU (slower). See the log for details.", "Whisper nie uruchomił się na GPU i działa na CPU (wolniej). Szczegóły w logu.";
    accurate_unavailable: "The accurate model could not be loaded: {e}", "Nie udało się wczytać modelu dokładnego: {e}";

    // ---- notifications ---------------------------------------------------------------------------
    mic_blocked_title: "Microphone access is blocked", "Dostęp do mikrofonu jest zablokowany";
    mic_blocked_text: "Allow desktop apps to use the microphone: Settings → Privacy & security → Microphone.",
        "Zezwól aplikacjom klasycznym na dostęp do mikrofonu: Ustawienia → Prywatność i zabezpieczenia → Mikrofon.";
    engine_start_failed_title: "WhistleType cannot start the speech engine", "WhistleType nie może uruchomić silnika mowy";
    start_failed: "WhistleType could not start: {e}", "Nie udało się uruchomić WhistleType: {e}";
    needs_model_title: "WhistleType needs its speech model", "WhistleType potrzebuje modelu mowy";
    needs_model_text: "Click the WhistleType icon to download it (one time, {size}).", "Kliknij ikonę WhistleType, aby go pobrać (jednorazowo, {size}).";

    // ---- tray --------------------------------------------------------------------------------------
    tip_listening: "WhistleType — listening…", "WhistleType — słucham…";
    tip_transcribing: "WhistleType — transcribing…", "WhistleType — rozpoznaję…";
    tip_ready_hold: "WhistleType — ready (hold {key})", "WhistleType — gotowy (przytrzymaj {key})";
    tip_ready_press: "WhistleType — ready (press {key})", "WhistleType — gotowy (naciśnij {key})";
    tip_loading: "WhistleType — loading the model…", "WhistleType — wczytuję model…";
    tip_no_model: "WhistleType — speech model not installed", "WhistleType — brak modelu mowy";
    tip_model_problem: "WhistleType — model problem", "WhistleType — problem z modelem";
    tip_engine_problem: "WhistleType — speech engine problem", "WhistleType — problem z silnikiem mowy";
    menu_start: "Start dictation", "Rozpocznij dyktowanie";
    menu_stop: "Stop dictation", "Zakończ dyktowanie";
    menu_mic: "Microphone", "Mikrofon";
    menu_system_default: "System default ({name})", "Domyślny systemowy ({name})";
    menu_none: "none", "brak";
    menu_model_ready: "Model: {name} — ready", "Model: {name} — gotowy";
    menu_model_loading: "Model: loading…", "Model: wczytywanie…";
    menu_model_missing: "Model: not installed — set up…", "Model: niezainstalowany — konfiguruj…";
    menu_model_problem: "Model: problem — repair…", "Model: problem — napraw…";
    menu_engine_na: "Speech engine: not available", "Silnik mowy: niedostępny";
    menu_copy_last: "Copy last transcription", "Kopiuj ostatnią transkrypcję";
    menu_engine: "Recognition", "Rozpoznawanie";
    menu_models: "Speech models…", "Modele mowy…";
    menu_settings: "Settings…", "Ustawienia…";
    menu_exit: "Exit", "Zakończ";

    // ---- settings window ----------------------------------------------------------------------------
    subtitle: "Local speech-to-text · Whistle (FAST) and Whisper (ACCURATE) · v{ver}", "Lokalne rozpoznawanie mowy · Whistle (FAST) i Whisper (ACCURATE) · v{ver}";
    sec_recognition: "Recognition", "Rozpoznawanie";
    lbl_engine: "Engine", "Silnik";
    engine_auto: "AUTO — ACCURATE on the GPU, otherwise FAST", "AUTO — ACCURATE na GPU, w innym razie FAST";
    engine_fast: "FAST — Whistle / CPU", "FAST — Whistle / CPU";
    engine_accurate: "ACCURATE — Whisper / GPU if available", "ACCURATE — Whisper / GPU, jeśli dostępne";
    lbl_accurate_model: "Accurate model", "Model dokładny";
    model_not_downloaded: "{name} (not downloaded)", "{name} (niepobrany)";
    btn_models: "Models…", "Modele…";
    lbl_gpu: "Graphics card", "Karta graficzna";
    chk_gpu: "Use the NVIDIA GPU (CUDA) for Whisper", "Używaj karty NVIDIA (CUDA) dla Whispera";
    acc_fast_only: "FAST mode: Whisper is not loaded (no extra memory used).", "Tryb FAST: Whisper nie jest wczytany (nie zajmuje pamięci).";
    acc_auto_no_gpu: "AUTO uses FAST: Whisper is used automatically only on an NVIDIA GPU with the GPU pack (Models…).",
        "AUTO używa FAST: Whisper włącza się automatycznie tylko na karcie NVIDIA z pakietem GPU (Modele…).";
    acc_no_model: "No Whisper model downloaded yet — click Models…", "Nie pobrano jeszcze modelu Whisper — kliknij Modele…";
    acc_loading: "Loading {name}…", "Wczytywanie {name}…";
    acc_ready: "✓ {name} ready on {device} · loaded in {ms} ms", "✓ {name} gotowy na {device} · wczytany w {ms} ms";
    acc_failed: "⚠ {name}: {e}", "⚠ {name}: {e}";
    acc_restart_gpu: " · restart to use the GPU", " · uruchom ponownie, aby użyć GPU";
    acc_using_other: "{wanted} is not downloaded — using {name}", "{wanted} nie jest pobrany — używam {name}";
    lbl_fast_model: "FAST model", "Model FAST";
    lbl_accurate_status: "ACCURATE model", "Model ACCURATE";
    sec_input: "Input", "Wejście";
    lbl_microphone: "Microphone", "Mikrofon";
    btn_test: "Test", "Test";
    btn_stop: "Stop", "Stop";
    lbl_language: "Speech language", "Język mowy";
    lang_auto_pl_en: "Auto (Polish + English) — recommended", "Auto (polski + angielski) — zalecane";
    lang_auto: "Detect automatically (any language)", "Wykrywaj automatycznie (dowolny język)";
    lbl_shortcut: "Push-to-talk shortcut", "Skrót dyktowania";
    hint_shortcut: "Click, then press the new keys", "Kliknij, potem naciśnij nowe klawisze";
    lbl_mode: "Mode", "Tryb";
    mode_hold: "Hold to talk", "Przytrzymaj, aby mówić";
    mode_toggle: "Press to start, press again to stop", "Naciśnij, aby zacząć; ponownie, aby skończyć";
    sec_output: "Output", "Wynik";
    lbl_autopaste: "Automatic paste", "Automatyczne wklejanie";
    chk_autopaste: "Insert the text into the active app", "Wstawiaj tekst do aktywnej aplikacji";
    lbl_insert_using: "Insert using", "Sposób wstawiania";
    method_ctrlv: "Clipboard + Ctrl+V (recommended)", "Schowek + Ctrl+V (zalecane)";
    method_shiftins: "Clipboard + Shift+Insert (terminals)", "Schowek + Shift+Insert (terminale)";
    method_ctrlshiftv: "Clipboard + Ctrl+Shift+V", "Schowek + Ctrl+Shift+V";
    method_type: "Type characters (no clipboard)", "Wpisywanie znaków (bez schowka)";
    chk_restore: "Restore my previous clipboard afterwards", "Przywracaj potem poprzednią zawartość schowka";
    lbl_raw: "Raw transcription", "Surowa transkrypcja";
    chk_raw: "Insert exactly what the speech model returns", "Wstawiaj dokładnie to, co zwraca model mowy";
    chk_space: "Add a space after each dictation", "Dodawaj spację po każdym dyktowaniu";
    lbl_vocab: "Custom vocabulary", "Własny słownik";
    btn_manage: "Manage…", "Zarządzaj…";
    vocab_info_on: "{n} words · keyword biasing on", "{n} słów · podpowiadanie włączone";
    vocab_info_off: "{n} words · off", "{n} słów · wyłączony";
    sec_general: "General", "Ogólne";
    lbl_ui_language: "Interface language", "Język interfejsu";
    lbl_autostart: "Start with Windows", "Start z Windows";
    chk_autostart: "Run WhistleType when I sign in", "Uruchamiaj WhistleType po zalogowaniu";
    lbl_overlay: "Status overlay", "Nakładka stanu";
    overlay_bottom: "Bottom of the screen", "Na dole ekranu";
    overlay_top: "Top of the screen", "Na górze ekranu";
    overlay_off: "Off", "Wyłączona";
    sec_status: "Status", "Stan";
    lbl_model: "Speech model", "Model mowy";
    lbl_offline: "Run offline", "Praca offline";
    btn_logs: "Open log folder", "Otwórz folder logów";
    btn_close: "Close", "Zamknij";
    btn_setup: "Set up…", "Konfiguruj…";
    btn_repair: "Repair…", "Napraw…";
    hint_hook_unavailable: "Keyboard hook unavailable — see log", "Przechwytywanie klawiatury niedostępne — zobacz log";
    hint_also_used: "Also used by another app; works except in admin windows", "Używany też przez inną aplikację; działa poza oknami administratora";
    hint_saved: "Saved", "Zapisano";
    hint_unchanged: "Unchanged", "Bez zmian";
    hint_press_keys: "Press keys…", "Naciśnij klawisze…";
    hint_esc_cancels: "Esc cancels", "Esc anuluje";
    mic_system_default: "System default — {name}", "Domyślny systemowy — {name}";
    mic_none_found: "no microphone found", "nie znaleziono mikrofonu";
    mic_selected: "Selected microphone", "Wybrany mikrofon";
    mic_not_connected: "{name} (not connected — using default)", "{name} (niepodłączony — używam domyślnego)";
    mic_no_devices: "No microphone found. Connect one — the list updates automatically.", "Nie znaleziono mikrofonu. Podłącz go — lista odświeży się sama.";
    mic_test_hint: "Speak — the bar should move. Nothing is recorded.", "Mów — pasek powinien się ruszać. Nic nie jest nagrywane.";
    status_ready: "✓ {model} ready ({size}) · loaded in {ms} ms", "✓ {model} gotowy ({size}) · wczytany w {ms} ms";
    status_loading: "Loading the model…", "Wczytywanie modelu…";
    status_downloading: "Downloading… {pct}%", "Pobieranie… {pct}%";
    status_not_installed: "Not installed yet — one-time download ({size})", "Jeszcze niezainstalowany — jednorazowe pobranie ({size})";
    offline_ready: "✓ Speech is recognised on this PC. Audio and text never leave it; no internet connection is used.",
        "✓ Mowa jest rozpoznawana na tym komputerze. Dźwięk i tekst nigdy go nie opuszczają; internet nie jest używany.";
    offline_loading: "Speech is recognised on this PC; no internet connection is used.", "Mowa jest rozpoznawana na tym komputerze; internet nie jest używany.";
    offline_need_download: "Internet is needed once, to download the model. After that WhistleType works fully offline.",
        "Internet jest potrzebny raz, do pobrania modelu. Potem WhistleType działa całkowicie offline.";
    offline_repair: "The model must be repaired before dictation works.", "Model trzeba naprawić, zanim dyktowanie zadziała.";
    offline_unavailable: "Speech recognition is unavailable.", "Rozpoznawanie mowy jest niedostępne.";

    // ---- vocabulary window ----------------------------------------------------------------------------
    vocab_title: "Custom vocabulary — WhistleType", "Własny słownik — WhistleType";
    vocab_info: "The speech model favours these words and phrases (Whistle: keyword biasing; Whisper: a prompt with the first entries). Add names, products and technical terms you dictate often. It is a hint, not a guarantee.",
        "Model mowy faworyzuje te słowa i frazy (Whistle: keyword biasing; Whisper: podpowiedź z pierwszymi wpisami). Dodaj nazwy, produkty i terminy techniczne, które często dyktujesz. To podpowiedź, nie gwarancja.";
    vocab_use: "Use custom vocabulary", "Używaj własnego słownika";
    btn_remove: "Remove", "Usuń";
    btn_defaults: "Restore defaults", "Przywróć domyślne";
    btn_add: "Add", "Dodaj";
    btn_replace: "Replace", "Zamień";
    vocab_count: "{n} of {max} entries", "{n} z {max} wpisów";
    vocab_exists: "Already in the list.", "Już jest na liście.";
    vocab_full: "The list is full.", "Lista jest pełna.";
    vocab_confirm_defaults: "Replace your list with the default vocabulary?", "Zastąpić Twoją listę domyślnym słownikiem?";

    // ---- model manager --------------------------------------------------------------------------------
    models_title: "Speech models — WhistleType", "Modele mowy — WhistleType";
    models_info: "All models run on this PC. Nothing is downloaded until you click Download; every file is checked with SHA-256.",
        "Wszystkie modele działają na tym komputerze. Nic nie jest pobierane, dopóki nie klikniesz „Pobierz”; każdy plik jest sprawdzany SHA-256.";
    models_gpu: "GPU: {name} ({vram}) · GPU pack: {pack}", "GPU: {name} ({vram}) · pakiet GPU: {pack}";
    models_no_gpu: "No NVIDIA GPU found — Whisper runs on the CPU (use base or small).", "Nie znaleziono karty NVIDIA — Whisper działa na CPU (wybierz base lub small).";
    pack_installed: "installed", "zainstalowany";
    pack_missing: "not installed", "niezainstalowany";
    col_model: "Model", "Model";
    col_mode: "Mode", "Tryb";
    col_size: "Size", "Rozmiar";
    col_polish: "Polish", "Polski";
    col_runs_on: "Runs on", "Działa na";
    col_status: "Status", "Stan";
    q_1: "basic", "podstawowa";
    q_2: "good", "dobra";
    q_3: "very good", "bardzo dobra";
    q_4: "best", "najlepsza";
    runs_cpu: "CPU", "CPU";
    runs_cpu_gpu: "CPU or GPU", "CPU lub GPU";
    runs_gpu: "GPU (slow on CPU)", "GPU (na CPU wolno)";
    runs_nvidia: "NVIDIA GPU", "karta NVIDIA";
    gpu_pack_name: "GPU pack (CUDA 12.4, whisper.cpp)", "Pakiet GPU (CUDA 12.4, whisper.cpp)";
    st_installed: "Installed", "Zainstalowany";
    st_in_use: "In use", "W użyciu";
    st_not_downloaded: "Not downloaded", "Niepobrany";
    st_downloading: "Downloading {pct}%", "Pobieranie {pct}%";
    st_installing: "Installing…", "Instalowanie…";
    st_failed: "Failed", "Błąd";
    st_selected: "Installed · selected", "Zainstalowany · wybrany";
    btn_delete: "Delete", "Usuń";
    btn_use_accurate: "Use for ACCURATE", "Użyj w ACCURATE";
    models_confirm_download: "Download {name} ({size}) from {host}?\n\nThe file is saved to:\n{dir}",
        "Pobrać {name} ({size}) z {host}?\n\nPlik zostanie zapisany w:\n{dir}";
    models_confirm_delete: "Delete {name}? You can download it again later.", "Usunąć {name}? Można go później pobrać ponownie.";
    models_cannot_delete_fast: "The FAST model is required and cannot be deleted here.", "Model FAST jest wymagany i nie można go tu usunąć.";
    models_busy: "Another download is in progress.", "Trwa inne pobieranie.";
    models_downloaded: "✓ {name} downloaded and verified.", "✓ {name} pobrany i zweryfikowany.";
    models_deleted: "{name} deleted.", "Usunięto {name}.";
    models_error: "⚠ {e}", "⚠ {e}";
    models_licences: "Whistle: Cactus Compute, Apache-2.0 · Whisper: OpenAI, MIT · whisper.cpp: MIT · CUDA: NVIDIA redistributable",
        "Whistle: Cactus Compute, Apache-2.0 · Whisper: OpenAI, MIT · whisper.cpp: MIT · CUDA: redystrybucja NVIDIA";
    models_gpu_pack_size: "{dl} download, {disk} on disk", "{dl} pobrania, {disk} na dysku";

    // ---- setup window ---------------------------------------------------------------------------------
    setup_window: "Set up WhistleType", "Konfiguracja WhistleType";
    setup_title: "Download the speech model", "Pobierz model mowy";
    setup_text: "WhistleType recognises speech on this computer with {model}, an open speech-to-text model by Cactus Compute. It has to be downloaded once ({size}). After that, dictation works fully offline — your voice never leaves this PC.",
        "WhistleType rozpoznaje mowę na tym komputerze za pomocą {model} — otwartego modelu zamiany mowy na tekst od Cactus Compute. Trzeba go pobrać jeden raz ({size}). Potem dyktowanie działa całkowicie offline — Twój głos nigdy nie opuszcza tego komputera.";
    setup_source: "Source: {url}\nRevision {rev} · License {lic} · verified with SHA-256 {sha}…\nSaved to: {dir}",
        "Źródło: {url}\nRewizja {rev} · Licencja {lic} · weryfikacja SHA-256 {sha}…\nZapis do: {dir}";
    btn_model_page: "Open the model page", "Otwórz stronę modelu";
    btn_download: "Download", "Pobierz";
    btn_try_again: "Try again", "Spróbuj ponownie";
    btn_import: "Import file…", "Importuj plik…";
    btn_not_now: "Not now", "Nie teraz";
    btn_cancel: "Cancel", "Anuluj";
    btn_done: "Done", "Gotowe";
    setup_downloading: "Downloading… {done} of {total} ({pct}%)", "Pobieranie… {done} z {total} ({pct}%)";
    setup_ready: "✓ Model installed and verified. You're ready: hold the shortcut and speak.", "✓ Model zainstalowany i zweryfikowany. Gotowe: przytrzymaj skrót i mów.";
    setup_loading: "Verifying and loading the model…", "Weryfikuję i wczytuję model…";
    setup_error_retry: "⚠ {e} You can try again — the download resumes where it stopped.", "⚠ {e} Możesz spróbować ponownie — pobieranie wznowi się od miejsca przerwania.";
    setup_idle: "Nothing is downloaded until you click Download.", "Nic nie zostanie pobrane, dopóki nie klikniesz „Pobierz”.";
    file_filter_model: "Whistle model (*.cact)", "Model Whistle (*.cact)";
    file_filter_all: "All files", "Wszystkie pliki";

    // ---- errors shown to the user ---------------------------------------------------------------------
    err_no_mic: "No microphone found", "Nie znaleziono mikrofonu";
    err_mic_not_connected: "Microphone \"{name}\" is not connected", "Mikrofon „{name}” nie jest podłączony";
    err_mic_blocked: "Microphone access is blocked in Windows privacy settings", "Dostęp do mikrofonu jest zablokowany w ustawieniach prywatności Windows";
    err_mic_in_use: "The microphone is in use by another app (exclusive mode)", "Mikrofon jest zajęty przez inną aplikację (tryb wyłączny)";
    err_mic_disconnected: "The microphone was disconnected", "Mikrofon został odłączony";
    err_audio_service: "The Windows Audio service is not running", "Usługa Windows Audio nie działa";
    err_mic_other: "Microphone error: {m}", "Błąd mikrofonu: {m}";
    err_engine_missing: "Speech engine not found ({path}). Please reinstall WhistleType.", "Nie znaleziono silnika mowy ({path}). Zainstaluj WhistleType ponownie.";
    err_engine_corrupt: "Speech engine file is damaged ({m}). Please reinstall WhistleType.", "Plik silnika mowy jest uszkodzony ({m}). Zainstaluj WhistleType ponownie.";
    err_engine_load: "Speech engine could not be loaded: {m}", "Nie udało się wczytać silnika mowy: {m}";
    err_model_missing: "The Whistle speech model is not installed yet.", "Model mowy Whistle nie jest jeszcze zainstalowany.";
    err_model_corrupt: "The Whistle model file is damaged ({m}). Download it again.", "Plik modelu Whistle jest uszkodzony ({m}). Pobierz go ponownie.";
    err_model_incomplete: "The model file is incomplete or damaged.", "Plik modelu jest niekompletny lub uszkodzony.";
    err_model_load: "The Whistle model could not be loaded: {m}", "Nie udało się wczytać modelu Whistle: {m}";
    err_not_loaded: "The speech model is not loaded.", "Model mowy nie jest wczytany.";
    err_too_long: "Audio segment too long for Whistle ({s} s > 30 s).", "Fragment nagrania za długi dla Whistle ({s} s > 30 s).";
    err_transcribe: "Transcription failed: {m}", "Rozpoznawanie nie powiodło się: {m}";
    err_dl_cancelled: "Download cancelled.", "Pobieranie anulowane.";
    err_dl_network: "Network error: {m}", "Błąd sieci: {m}";
    err_dl_http: "The server answered HTTP {c}.", "Serwer odpowiedział kodem HTTP {c}.";
    err_dl_integrity: "The downloaded file failed verification: {m}", "Pobrany plik nie przeszedł weryfikacji: {m}";
    err_dl_disk: "Could not save the file: {m}", "Nie udało się zapisać pliku: {m}";
    err_import_size: "This is not Whistle {ver}: expected {exp} bytes, the file has {got}.", "To nie jest Whistle {ver}: oczekiwano {exp} bajtów, plik ma {got}.";
    err_import_sha: "The file's SHA-256 does not match the official Whistle {ver} model.", "SHA-256 pliku nie zgadza się z oficjalnym modelem Whistle {ver}.";
    hk_modifier: "Choose a key other than Ctrl/Alt/Shift/Win.", "Wybierz klawisz inny niż Ctrl/Alt/Shift/Win.";
    hk_esc: "Esc is reserved for cancelling a dictation.", "Esc służy do anulowania dyktowania.";
    hk_typing: "{key} alone would stop you from typing it. Use F-keys or add Ctrl/Alt/Win.", "Sam {key} uniemożliwiłby jego pisanie. Użyj klawiszy F albo dodaj Ctrl/Alt/Win.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_and_languages() {
        assert_eq!(fmt(EN.too_short_hold, &[("key", &"F8")]), "Too short — hold F8 while you speak");
        assert_eq!(fmt(PL.vocab_count, &[("n", &3), ("max", &300)]), "3 z 300 wpisów");
        assert_eq!(UiLang::from_code("pl"), Some(UiLang::Pl));
        assert_eq!(UiLang::from_code("de"), None);
        // every Polish template keeps exactly the placeholders of its English counterpart
        fn placeholders(s: &str) -> Vec<&str> {
            let mut v: Vec<&str> = s.match_indices('{').filter_map(|(i, _)| s[i..].find('}').map(|j| &s[i..i + j + 1])).collect();
            v.sort_unstable();
            v.dedup();
            v
        }
        assert!(ALL.len() > 150);
        for (name, en, pl) in ALL {
            assert_eq!(placeholders(en), placeholders(pl), "placeholders of `{name}`");
            assert!(!en.is_empty() && !pl.is_empty(), "`{name}` is empty");
        }
    }
}
