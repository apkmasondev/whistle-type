//! WhistleType - local push-to-talk dictation for Windows, powered by Cactus Compute Whistle.

#![cfg(windows)]

pub mod log;
pub mod i18n;
pub mod util;
pub mod paths;
pub mod settings;
pub mod text;
pub mod vad;
pub mod segment;
pub mod resample;
pub mod wav;
pub mod engine;
pub mod model;
pub mod pipeline;
pub mod msg;
pub mod hotkey;
pub mod audio;
pub mod clipboard;
pub mod insert;
pub mod overlay;
pub mod tray;
pub mod autostart;
pub mod download;
pub mod app;
pub mod ui;
pub mod settings_ui;
pub mod vocab_ui;
pub mod setup_ui;
pub mod stt;
pub mod whisper;
pub mod models;
pub mod models_ui;
