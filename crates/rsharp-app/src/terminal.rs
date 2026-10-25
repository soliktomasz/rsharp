//! The Terminal tool window: one PTY per tab, with a native visible-cell view.

mod panel;
mod view;
pub use panel::{TerminalEvent, TerminalPanel};
pub use view::TerminalView;
