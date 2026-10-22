//! Every colour the window paints: Rider 2026.2's Islands Dark, read from its own theme files
//! (ManyIslandsDark.theme.json over expUI_dark.theme.json), and Rider Dark's C# colours (RiderDark.xml).
//! A view names a token here, never a literal colour.

use gpui::{Hsla, Rgba, rgb, rgba};
use rsharp_languages::Kind;

/// MainWindow.background: the frame round the islands, the toolbar, the stripes and the status bar.
pub fn frame() -> Rgba {
    rgb(0x26282C)
}
/// ToolWindow.background, Editor.background: an island's ground.
pub fn island() -> Rgba {
    rgb(0x191A1C)
}
pub fn text() -> Rgba {
    rgb(0xD1D3D9)
}
/// StatusBar.Widget.foreground, ToolWindow.Button.foreground.
pub fn text_muted() -> Rgba {
    rgb(0x9FA2A8)
}
pub fn text_secondary() -> Rgba {
    rgb(0x73767C)
}
pub fn accent() -> Rgba {
    rgb(0x3871E1)
}
pub fn on_accent() -> Rgba {
    rgb(0xFFFFFF)
}
/// Tree.selectionBackground with the tree focused, and without.
pub fn selection() -> Rgba {
    rgb(0x2A4371)
}
pub fn selection_inactive() -> Rgba {
    rgb(0x33353B)
}
pub fn row_hover() -> Rgba {
    rgba(0xFFFFFF10)
}
pub fn button_hover() -> Rgba {
    rgba(0xFFFFFF17)
}
pub fn button_pressed() -> Rgba {
    rgba(0xFFFFFF29)
}
/// Component.borderColor: a field's, a checkbox's, a plain button's edge.
pub fn control_border() -> Rgba {
    rgb(0x40434A)
}
/// Popup.background and Popup.borderColor.
pub fn popup() -> Rgba {
    rgb(0x26282C)
}
pub fn popup_border() -> Rgba {
    rgb(0x33353B)
}
/// MainToolbar.separatorColor and ToolWindow.Stripe.separatorColor.
pub fn separator() -> Rgba {
    rgba(0xFFFFFF21)
}
/// ToolWindow.Button.DragAndDrop.buttonDropBackground: where a dragged window would land.
pub fn drop_zone() -> Rgba {
    rgba(0x3671E580)
}
/// Windows' own red under the pointer on a caption's Close.
pub fn close_hover() -> Rgba {
    rgb(0xE81123)
}
pub fn tab_selected() -> Rgba {
    rgb(0x233558)
}
pub fn tab_selected_border() -> Rgba {
    rgb(0x2E4D89)
}
pub fn modified() -> Rgba {
    rgb(0x548AF7)
}
pub fn error() -> Rgba {
    rgb(0xF75464)
}
pub fn warning() -> Rgba {
    rgb(0xE5A336)
}

/// The editor's own colours (Islands Dark's editor scheme over Rider Dark's).
pub mod editor {
    use super::*;
    pub fn text() -> Rgba {
        rgb(0xBCBEC4)
    }
    pub fn line_number() -> Rgba {
        rgb(0x4B5059)
    }
    pub fn line_number_current() -> Rgba {
        rgb(0xA1A3AB)
    }
    pub fn current_line() -> Rgba {
        rgb(0x1F2024)
    }
    pub fn selection() -> Rgba {
        rgb(0x214283)
    }
    pub fn caret() -> Rgba {
        rgb(0xCED0D6)
    }
}

/// Rider Dark's colour for a kind of C#.
pub fn syntax(kind: Kind) -> Hsla {
    rgb(match kind {
        Kind::Keyword => 0x6C95EB,
        Kind::String => 0xC9A26D,
        Kind::StringEscape => 0xD688D4,
        Kind::Number => 0xED94C0,
        Kind::Comment | Kind::DocComment => 0x85C46C,
        Kind::Type | Kind::Namespace => 0xC191FF,
        Kind::Method => 0x39CC9B,
        Kind::Property => 0x66C3CC,
    })
    .into()
}

pub const UI_FONT: &str = "Inter";
pub const CODE_FONT: &str = "JetBrains Mono";
