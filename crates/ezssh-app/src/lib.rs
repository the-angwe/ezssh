//! ezSsh native app: the eframe shell around `ezssh-core` (backend) and
//! `ezssh-term` (terminal widget). `main.rs` only parses arguments and
//! opens the window; everything else is here so the headless UI tests in
//! `tests/` can drive the real [`App`].

pub mod app;
mod bell;
mod conn;
mod core;
mod icons;
pub mod layout;
pub mod settings;
pub mod style;
pub mod tabs;
mod toasts;
#[cfg(test)]
mod ui_tests;
mod ui_font;
mod views;
mod widgets;

pub use app::{App, Options};
