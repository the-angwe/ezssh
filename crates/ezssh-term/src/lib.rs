//! ezSsh terminal widget.
//!
//! `alacritty_terminal` holds the grid and parses VT output; this crate
//! paints it with egui and encodes keyboard / mouse input. Byte sources
//! (SSH channel, local PTY) live outside and talk to it through
//! [`terminal::Pty`] and [`terminal::Feeder`].

pub mod boxdraw;
pub mod fonts;
pub mod keys;
pub mod mouse;
pub mod palette;
pub mod terminal;
pub mod view;

pub use alacritty_terminal::event::WindowSize;
pub use palette::Palette;
pub use terminal::{Feeder, Pty, TermOptions, Terminal};
pub use view::{TermStyle, TerminalView};
