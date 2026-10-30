//! ezSsh terminal widget.
//!
//! `alacritty_terminal` holds the grid and parses VT output; this crate
//! paints it with egui and encodes keyboard / mouse input. Byte sources
//! (SSH channel, local PTY) live outside and talk to it through
//! [`terminal::Pty`] and [`terminal::Feeder`].

pub mod boxdraw;
pub mod fonts;
mod hook;
pub mod keys;
pub mod links;
pub mod mouse;
pub mod osc52;
pub mod palette;
pub mod search;
pub mod terminal;
pub mod view;

pub use alacritty_terminal::event::WindowSize;
pub use palette::Palette;
pub use terminal::{Feeder, Pty, TermOptions, Terminal};
pub use view::{BellMode, Overlay, RightClick, TermAction, TermOutput, TermStyle, TerminalView};
pub use fonts::{Faces, FontRegistry};
pub use terminal::CursorKind;
