//! Integration tests share one binary and one copy of their helpers.

mod about;
mod agent_chat;
mod canvas_settings;
mod chapters;
mod crop_dialog;
mod desktop;
mod effect_panels;
mod effects;
mod export;
mod freeze_replace;
mod gpu;
mod keybindings;
mod layout_prefs;
mod lifecycle;
mod media_import;
mod media_library;
mod models;
mod music;
mod narration;
mod onboarding;
mod panels;
mod preview;
mod projects;
mod save_status;
mod scenes;
mod shorts;
mod sounds;
mod startup;
mod timeline;
mod tracks;
mod transitions;
mod voiceover;

mod mcp_control;
mod media_probe;
mod missing_media;
mod subtitles;

/// Runs the app loop only when woken, so a result published without a wake fails.
#[path = "support/woken.rs"]
mod woken;
