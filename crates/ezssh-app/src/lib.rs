//! ezSsh native app: the eframe shell around `ezssh-core` (backend) and
//! `ezssh-term` (terminal widget). `main.rs` only parses arguments and
//! opens the window; everything else is here so the headless UI tests in
//! `tests/` can drive the real [`App`].

pub mod app;
mod bell;
mod conn;
mod core;
mod creds;
mod files;
mod font_presets;
mod form;
mod icons;
pub mod layout;
pub mod settings;
pub mod style;
pub mod tabs;
mod toasts;
mod ui_font;
#[cfg(test)]
mod ui_tests;
mod views;
mod widgets;

pub use app::{App, Options};
