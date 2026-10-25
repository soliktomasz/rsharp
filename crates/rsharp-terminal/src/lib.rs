//! Terminal emulation and sessions without UI.

pub mod input;
mod screen;
mod session;

pub use alacritty_terminal::selection::SelectionType;
pub use alacritty_terminal::term::{TermMode, cell::Flags};
pub use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
pub use portable_pty::CommandBuilder;
pub use screen::{Cell, HISTORY_LINES, Screen, Snapshot};
pub use session::{Session, shell};
