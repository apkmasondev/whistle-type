//! User settings, persisted as JSON in %APPDATA%\WhistleType\settings.json.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SETTINGS_VERSION: u32 = 1;

/// Recognition language options (code, display name). `auto_pl_en` = let Whistle detect the language, but
/// accept only Polish or English; any other detection (e.g. fast Polish heard as French) is re-transcribed
/// as Polish. Names of the two automatic options are localised in the UI.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("auto_pl_en", "Auto (Polski + English)"),
    ("pl", "Polski"),
    ("en", "English"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("auto", "Detect automatically"),
];

pub const DEFAULT_VOCABULARY: &[&str] = &[
    "Codex", "Claude", "Claude Code", "Opus", "Astra", "ChatGPT", "ApkMason", "GitHub", "GitHub Pages",
    "Kotlin", "Compose", "Gradle", "Android Studio", "React", "TypeScript", "JavaScript", "Three.js",
    "Blender", "Python", "Vite", "WebGL", "Draco", "Electron",
];

/// Limits for what is passed to the engine as keyword-biasing input.
pub const MAX_VOCABULARY_ENTRIES: usize = 300;
pub const MAX_VOCABULARY_ENTRY_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub vk: u32,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
}

impl Default for Hotkey {
    fn default() -> Self {
        Hotkey { vk: 0x77, ctrl: false, alt: false, shift: false, win: false } // VK_F8
    }
}

impl Hotkey {
    pub fn pack(&self) -> u64 {
        (self.vk as u64)
            | ((self.ctrl as u64) << 32)
            | ((self.alt as u64) << 33)
            | ((self.shift as u64) << 34)
            | ((self.win as u64) << 35)
    }

    pub fn unpack(v: u64) -> Self {
        Hotkey {
            vk: (v & 0xFFFF_FFFF) as u32,
            ctrl: v & (1 << 32) != 0,
            alt: v & (1 << 33) != 0,
            shift: v & (1 << 34) != 0,
            win: v & (1 << 35) != 0,
        }
    }

    pub fn has_modifiers(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Hold,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertMethod {
    /// Clipboard + Ctrl+V (default, most compatible)
    CtrlV,
    /// Clipboard + Shift+Insert (classic terminals, mintty)
    ShiftInsert,
    /// Clipboard + Ctrl+Shift+V (some Linux-style terminals)
    CtrlShiftV,
    /// Type the characters with SendInput (no clipboard involved)
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    Bottom,
    Top,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// WASAPI endpoint id; empty = Windows default input device.
    pub microphone_id: String,
    /// Friendly name of the chosen device (shown when it is unplugged).
    pub microphone_name: String,
    /// Whistle language code, "auto" (any of Whistle's 7 languages) or "auto_pl_en" (default).
    pub language: String,
    pub hotkey: Hotkey,
    pub mode: Mode,
    pub auto_paste: bool,
    pub insert_method: InsertMethod,
    pub restore_clipboard: bool,
    pub raw_transcription: bool,
    pub append_space: bool,
    pub use_vocabulary: bool,
    pub vocabulary: Vec<String>,
    pub start_with_windows: bool,
    pub show_overlay: bool,
    pub overlay_position: OverlayPosition,
    /// Hard cap for one dictation; longer audio is split into 30 s segments for Whistle.
    pub max_recording_secs: u32,
    /// Developer option: write transcripts into the log. Off by default (privacy).
    pub log_transcripts: bool,
    /// Interface language: "pl" or "en" (default: the Windows display language).
    pub ui_language: String,
    /// Recognition mode: "fast" (Whistle, CPU), "accurate" (Whisper, GPU if available) or "auto".
    pub engine_mode: String,
    /// Whisper model id used by ACCURATE/AUTO (empty = the best installed one).
    pub whisper_model: String,
    /// Run Whisper on an NVIDIA GPU when the CUDA pack is installed (falls back to the CPU automatically).
    pub use_gpu: bool,
}

pub const ENGINE_MODES: &[&str] = &["auto", "fast", "accurate"];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            microphone_id: String::new(),
            microphone_name: String::new(),
            language: "auto_pl_en".into(),
            hotkey: Hotkey::default(),
            mode: Mode::Hold,
            auto_paste: true,
            insert_method: InsertMethod::CtrlV,
            restore_clipboard: true,
            raw_transcription: false,
            append_space: false,
            use_vocabulary: true,
            vocabulary: DEFAULT_VOCABULARY.iter().map(|s| s.to_string()).collect(),
            start_with_windows: false,
            show_overlay: true,
            overlay_position: OverlayPosition::Bottom,
            max_recording_secs: 300,
            log_transcripts: false,
            ui_language: crate::i18n::system_default().code().into(),
            engine_mode: "auto".into(),
            whisper_model: String::new(),
            use_gpu: true,
        }
    }
}

impl Settings {
    /// Repairs out-of-range values (hand-edited or older files).
    pub fn sanitize(&mut self) {
        if !LANGUAGES.iter().any(|(c, _)| *c == self.language) {
            self.language = "auto_pl_en".into();
        }
        if self.hotkey.vk == 0 || self.hotkey.vk > 0xFE {
            self.hotkey = Hotkey::default();
        }
        self.max_recording_secs = self.max_recording_secs.clamp(10, 600);
        if !ENGINE_MODES.contains(&self.engine_mode.as_str()) {
            self.engine_mode = "auto".into();
        }
        if !self.whisper_model.is_empty() && crate::models::whisper_model(&self.whisper_model).is_none() {
            self.whisper_model.clear();
        }
        if crate::i18n::UiLang::from_code(&self.ui_language).is_none() {
            self.ui_language = crate::i18n::system_default().code().into();
        }
        self.vocabulary = clean_vocabulary(&self.vocabulary);
        self.version = SETTINGS_VERSION;
    }

    /// How the engine should choose the recognition language.
    pub fn language_plan(&self) -> crate::engine::LanguagePlan {
        use crate::engine::LanguagePlan;
        match self.language.as_str() {
            "auto" => LanguagePlan::detect(),
            "auto_pl_en" => LanguagePlan { language: None, accept: vec!["pl".into(), "en".into()], fallback: Some("pl".into()) },
            code => LanguagePlan::forced(code),
        }
    }

    /// Custom words for the engines (Whistle: keyword biasing, Whisper: prompt). Empty when disabled.
    pub fn engine_vocabulary(&self) -> Vec<String> {
        if !self.use_vocabulary {
            return Vec::new();
        }
        clean_vocabulary(&self.vocabulary)
    }

    pub fn load(path: &Path) -> (Settings, LoadOutcome) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), LoadOutcome::FirstRun),
            Err(e) => return (Settings::default(), LoadOutcome::Error(e.to_string())),
        };
        // editors (and Windows PowerShell) often save UTF-8 with a BOM
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        match serde_json::from_str::<Settings>(text) {
            Ok(mut s) => {
                s.sanitize();
                (s, LoadOutcome::Loaded)
            }
            Err(e) => {
                // keep the broken file for the user, start with defaults
                let backup = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, &backup);
                (Settings::default(), LoadOutcome::Corrupt(e.to_string()))
            }
        }
    }

    /// Atomic save: write a temp file, then replace.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoadOutcome {
    FirstRun,
    Loaded,
    Corrupt(String),
    Error(String),
}

/// Trims, drops empty/duplicate entries (case-insensitive), strips separators and enforces limits.
pub fn clean_vocabulary(items: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let cleaned: String = item
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let cleaned: String = cleaned.chars().take(MAX_VOCABULARY_ENTRY_CHARS).collect();
        if cleaned.is_empty() || !seen.insert(cleaned.to_lowercase()) {
            continue;
        }
        out.push(cleaned);
        if out.len() >= MAX_VOCABULARY_ENTRIES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_brief() {
        let s = Settings::default();
        assert_eq!(s.language, "auto_pl_en");
        assert_eq!(s.hotkey.vk, 0x77);
        assert_eq!(s.mode, Mode::Hold);
        assert!(s.auto_paste && s.restore_clipboard && !s.raw_transcription);
        assert!(s.vocabulary.contains(&"Claude Code".to_string()));
        assert_eq!(s.vocabulary.len(), DEFAULT_VOCABULARY.len());
    }

    #[test]
    fn hotkey_pack_roundtrip() {
        let h = Hotkey { vk: 0x20, ctrl: true, alt: false, shift: true, win: true };
        assert_eq!(Hotkey::unpack(h.pack()), h);
    }

    #[test]
    fn json_roundtrip_and_partial_files() {
        let s = Settings::default();
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&j).unwrap(), s);
        // a file from an older/newer version with missing fields still loads
        let partial: Settings = serde_json::from_str(r#"{"language":"en","mode":"toggle"}"#).unwrap();
        assert_eq!(partial.language, "en");
        assert_eq!(partial.mode, Mode::Toggle);
        assert!(partial.auto_paste);
    }

    #[test]
    fn engine_settings_default_and_repair() {
        // a settings file from 1.0 has none of the engine fields: AUTO, no model chosen, GPU allowed
        let old: Settings = serde_json::from_str(r#"{"language":"pl"}"#).unwrap();
        assert_eq!((old.engine_mode.as_str(), old.whisper_model.as_str(), old.use_gpu), ("auto", "", true));
        let mut bad: Settings = serde_json::from_str(r#"{"engine_mode":"turbo","whisper_model":"whisper-huge"}"#).unwrap();
        bad.sanitize();
        assert_eq!((bad.engine_mode.as_str(), bad.whisper_model.as_str()), ("auto", ""));
        let mut ok: Settings = serde_json::from_str(r#"{"engine_mode":"accurate","whisper_model":"whisper-small"}"#).unwrap();
        ok.sanitize();
        assert_eq!((ok.engine_mode.as_str(), ok.whisper_model.as_str()), ("accurate", "whisper-small"));
    }

    #[test]
    fn sanitize_repairs_values() {
        let mut s = Settings { language: "xx".into(), max_recording_secs: 99999, ..Default::default() };
        s.hotkey.vk = 0;
        s.sanitize();
        assert_eq!(s.language, "auto_pl_en");
        assert_eq!(s.max_recording_secs, 600);
        assert_eq!(s.hotkey, Hotkey::default());
    }

    #[test]
    fn vocabulary_cleaning() {
        let v = clean_vocabulary(&[
            "  React ".into(),
            "react".into(),
            "".into(),
            "Claude\nCode".into(),
            "x".repeat(200),
        ]);
        assert_eq!(v[0], "React");
        assert_eq!(v[1], "Claude Code");
        assert_eq!(v[2].chars().count(), MAX_VOCABULARY_ENTRY_CHARS);
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn keywords_and_language() {
        let mut s = Settings::default();
        let p = s.language_plan();
        assert_eq!(p.language, None);
        assert_eq!(p.fallback.as_deref(), Some("pl"));
        assert!(p.needs_fallback("fr") && !p.needs_fallback("en") && !p.needs_fallback("pl") && !p.needs_fallback(""));
        assert!(s.engine_vocabulary().contains(&"Claude Code".to_string()));
        s.use_vocabulary = false;
        assert!(s.engine_vocabulary().is_empty());
        s.language = "auto".into();
        assert_eq!(s.language_plan(), crate::engine::LanguagePlan::detect());
        assert!(!s.language_plan().needs_fallback("fr"));
        s.language = "pl".into();
        assert_eq!(s.language_plan().language.as_deref(), Some("pl"));
    }

    #[test]
    fn load_corrupt_and_missing() {
        let dir = std::env::temp_dir().join(format!("wt-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let (_, o) = Settings::load(&p);
        assert_eq!(o, LoadOutcome::FirstRun);
        std::fs::write(&p, "{ not json").unwrap();
        let (s, o) = Settings::load(&p);
        assert!(matches!(o, LoadOutcome::Corrupt(_)));
        assert_eq!(s, Settings::default());
        assert!(dir.join("settings.json.corrupt").exists());
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).1, LoadOutcome::Loaded);
        std::fs::write(&p, "\u{FEFF}{\"language\":\"en\"}").unwrap();
        let (s, o) = Settings::load(&p);
        assert_eq!((o, s.language.as_str()), (LoadOutcome::Loaded, "en"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
