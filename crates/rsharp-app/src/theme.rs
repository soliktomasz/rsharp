//! Every colour the window paints: Rider 2026.2's Islands Dark, read from its own theme files
//! (ManyIslandsDark.theme.json over expUI_dark.theme.json), and Rider Dark's C# colours (RiderDark.xml).
//! A view names a token here, never a literal colour.

use gpui::{Hsla, Rgba, rgb, rgba};
use rsharp_languages::Kind;

/// The IntelliJ platform's default terminal palette (the Terminal plugin's own 16), on Islands Dark's
/// CONSOLE_BACKGROUND_KEY (191A1C) and its normal output's grey; a colour above 15 is xterm's protocol, so the
/// 6x6x6 cube and the 24 greys are computed rather than named.
pub fn terminal_color(color: rsharp_terminal::Color, bold: bool) -> Rgba {
    use rsharp_terminal::{Color, NamedColor};
    const ANSI: [u32; 16] = [
        0x000000, 0xFF5647, 0x85C46C, 0xD9B72B, 0x6C95EB, 0xD688D4, 0x39CC9B, 0xBDBDBD, 0x666666, 0xFF8870, 0xADEB96,
        0xF5D86A, 0xADD3FF, 0xFFBFFE, 0x7DF0C0, 0xF0F0F0,
    ];
    let index = match color {
        Color::Spec(c) => return rgb((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)),
        Color::Named(NamedColor::Background) => return island(),
        Color::Named(NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground) => {
            return rgb(0xBDBDBD);
        }
        Color::Named(NamedColor::Cursor) => return editor::caret(),
        Color::Named(c) => {
            let n = c as usize;
            if n < 8 && bold {
                n + 8
            } else if n >= 259 {
                (n - 259).min(7)
            } else {
                n
            }
        }
        Color::Indexed(n) => {
            let n = usize::from(n);
            if n < 8 && bold { n + 8 } else { n }
        }
    };
    if index < 16 {
        return rgb(ANSI[index]);
    }
    if index >= 232 {
        let v = (8 + 10 * (index.min(255) - 232)) as u32;
        return rgb(v * 0x010101);
    }
    let n = index - 16;
    let cube = |v: usize| if v == 0 { 0 } else { (55 + 40 * v) as u32 };
    rgb((cube(n / 36) << 16) | (cube(n / 6 % 6) << 8) | cube(n % 6))
}

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
/// Editor.SearchField.background (editor-bg-inline, gray-20): the find bar's ground; its border is the frame's.
pub fn search_field() -> Rgba {
    rgb(0x212326)
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
/// tab-selected-bg-inactive (gray-30): the tab a split shows while another split has the keys.
pub fn tab_selected_inactive() -> Rgba {
    rgb(0x26282C)
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
    /// MATCHED_BRACE_ATTRIBUTES' background (IslandSchemeDark.xml).
    pub fn matched_brace() -> Rgba {
        rgb(0x43454A)
    }
    /// FOLDED_TEXT_ATTRIBUTES: a folded region's placeholder.
    pub fn folded_text() -> Rgba {
        rgb(0x868991)
    }
    pub fn folded_background() -> Rgba {
        rgb(0x393B40)
    }
    /// WHITESPACES: the dots and arrows Show Whitespaces draws.
    pub fn whitespace() -> Rgba {
        rgb(0x6F737A)
    }
    /// DIFF_INSERTED's and DIFF_DELETED's backgrounds (Darcula's, which Islands Dark inherits).
    pub fn diff_inserted() -> Rgba {
        rgb(0x294436)
    }
    pub fn diff_deleted() -> Rgba {
        rgb(0x484A4A)
    }
    /// TEXT_SEARCH_RESULT_ATTRIBUTES' background (IslandSchemeDark.xml): a find bar's match.
    /// Rider's line breakpoint: the red dot its gutter shows (#DB5C5C, its stop icon's stroke).
    pub fn breakpoint() -> Rgba {
        rgb(0xDB5C5C)
    }

    /// Rider's execution point: the band under the statement the debugger is stopped at, a step above the current
    /// line's grey so the stopped statement stands out without colouring the code.
    pub fn execution_point() -> Rgba {
        rgb(0x2F3E54)
    }

    pub fn search_result() -> Rgba {
        rgb(0x114957)
    }
    /// IDENTIFIER_UNDER_CARET_ATTRIBUTES and WRITE_IDENTIFIER_UNDER_CARET_ATTRIBUTES' backgrounds: the usages of
    /// what the caret is on, read and written (Rider Dark, as Passenger's RiderScheme.axaml carries it).
    pub fn usage_read() -> Rgba {
        rgb(0x373B39)
    }
    pub fn usage_write() -> Rgba {
        rgb(0x402F33)
    }
    /// INLINE_PARAMETER_HINT's foreground and background (Rider Dark, as Passenger carries it).
    pub fn inlay_hint() -> Rgba {
        rgb(0x8F9298)
    }
    pub fn inlay_hint_background() -> Rgba {
        rgb(0x2B2D30)
    }
}

/// Rider Dark's colour for a kind of code: ReSharper's for C#, Rider's own attributes for markup and data (XML_TAG,
/// XML_ATTRIBUTE_NAME and _VALUE, XML_ENTITY_REFERENCE, JSON.PROPERTY_KEY, YAML_SCALAR_KEY, MARKDOWN_HEADER_LEVEL_1,
/// MARKDOWN_CODE_SPAN, MARKDOWN_LINK_DESTINATION), as Passenger's RiderScheme.axaml carries them.
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
        Kind::Tag | Kind::YamlKey => 0xC191FF,
        Kind::AttributeName | Kind::Code => 0x85C46C,
        Kind::AttributeValue => 0xC9A26D,
        Kind::Entity => 0xFFD49E,
        Kind::JsonKey | Kind::Heading => 0x66C3CC,
        Kind::LinkDestination => 0x39CC9B,
    })
    .into()
}

/// Rider Dark's colour for what Roslyn names a word (its semantic token types, by its legend's names), where it says more than the syntax:
/// ReSharper's identifier colours, as Passenger's RiderScheme.axaml carries them. A local or a parameter is plain
/// text, which takes back a colour the syntax guessed. None leaves the syntax's colour.
pub fn semantic(kind: &str) -> Option<Hsla> {
    let colour = match kind {
        "class" | "recordClass" | "interface" | "typeParameter" => 0xC191FF,
        "struct" | "recordStruct" | "enum" | "delegate" => 0xE1BFFF,
        "method" | "extensionMethod" | "function" => 0x39CC9B,
        "property" | "field" | "constant" | "enumMember" => 0x66C3CC,
        "event" => 0xED94C0,
        "parameter" | "variable" | "label" | "namespace" => 0xBCBEC4,
        "excludedCode" => 0x6F737A,
        _ => return None,
    };
    Some(rgb(colour).into())
}

pub const UI_FONT: &str = "Inter";
pub const CODE_FONT: &str = "JetBrains Mono";
