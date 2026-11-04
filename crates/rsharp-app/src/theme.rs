//! Every colour the window paints: Rider 2026.2's Islands Dark, read from its own theme files
//! (ManyIslandsDark.theme.json over expUI_dark.theme.json), and Rider Dark's C# colours (RiderDark.xml).
//! A view names a token here, never a literal colour.
//!
//! Settings' Appearance | Design picks the world: Rider's Islands (the default), or Zed's — Zed's One Dark theme
//! (zed-industries/zed `assets/themes/one/one.json`) on flat panes split by 1px lines with square tabs, as zed.dev
//! shows it. Each token holds both values, Rider's first.

use gpui::{Hsla, Rgba, rgb, rgba};
use rsharp_languages::Kind;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

/// The look the whole IDE wears.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Design {
    /// Rider 2026.2's Islands Dark: rounded islands floating on a frame.
    #[default]
    Islands,
    /// Zed's One Dark: flat panes split by 1px lines, square tabs, a quieter blue.
    Zed,
}

impl Design {
    pub const ALL: [Design; 2] = [Design::Islands, Design::Zed];
    pub const NAMES: &'static [&'static str] = &["Islands (Rider)", "Zed"];
}

// A global rather than a gpui one: tokens are free functions called while painting, often without an App at hand.
static DESIGN: AtomicU8 = AtomicU8::new(0);

pub fn set_design(design: Design) {
    DESIGN.store(design as u8, Ordering::Relaxed);
}

pub fn design() -> Design {
    if DESIGN.load(Ordering::Relaxed) == Design::Zed as u8 { Design::Zed } else { Design::Islands }
}

pub fn is_zed() -> bool {
    design() == Design::Zed
}

/// Rider's value, else Zed's.
fn pick(islands: u32, zed: u32) -> Rgba {
    rgb(if is_zed() { zed } else { islands })
}
fn pick_a(islands: u32, zed: u32) -> Rgba {
    rgba(if is_zed() { zed } else { islands })
}

/// The shell's measures in the chosen design: Rider New UI's, or Zed's denser flat ones.
pub mod layout {
    use super::is_zed;
    /// Between islands; in Zed a 1px line of the frame between panes.
    pub fn gap() -> f32 {
        if is_zed() { 1. } else { 6. }
    }
    pub fn island_radius() -> f32 {
        if is_zed() { 0. } else { 10. }
    }
    pub fn toolbar_height() -> f32 {
        if is_zed() { 34. } else { 40. }
    }
    pub fn stripe_width() -> f32 {
        if is_zed() { 36. } else { 40. }
    }
    pub fn status_bar_height() -> f32 {
        if is_zed() { 28. } else { 30. }
    }
    pub fn tab_strip_height() -> f32 {
        if is_zed() { 32. } else { 40. }
    }
}

/// The IntelliJ platform's default terminal palette (the Terminal plugin's own 16), on Islands Dark's
/// CONSOLE_BACKGROUND_KEY (191A1C) and its normal output's grey; a colour above 15 is xterm's protocol, so the
/// 6x6x6 cube and the 24 greys are computed rather than named.
pub fn terminal_color(color: rsharp_terminal::Color, bold: bool) -> Rgba {
    use rsharp_terminal::{Color, NamedColor};
    const ANSI: [u32; 16] = [
        0x000000, 0xFF5647, 0x85C46C, 0xD9B72B, 0x6C95EB, 0xD688D4, 0x39CC9B, 0xBDBDBD, 0x666666, 0xFF8870, 0xADEB96,
        0xF5D86A, 0xADD3FF, 0xFFBFFE, 0x7DF0C0, 0xF0F0F0,
    ];
    // One Dark's terminal.ansi.*.
    const ZED_ANSI: [u32; 16] = [
        0x282C33, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xDCDFE4, 0x525561, 0xEA858B, 0xAAD581,
        0xFFD885, 0x85C1FF, 0xD398EB, 0x6ED5DE, 0xFAFAFA,
    ];
    let ansi = if is_zed() { &ZED_ANSI } else { &ANSI };
    let index = match color {
        Color::Spec(c) => return rgb((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)),
        Color::Named(NamedColor::Background) => return editor::background(),
        Color::Named(NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground) => {
            return pick(0xBDBDBD, 0xDCE0E5);
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
        return rgb(ansi[index]);
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
    pick(0x26282C, 0x3B414D)
}
/// ToolWindow.background, Editor.background: an island's ground.
pub fn island() -> Rgba {
    pick(0x191A1C, 0x2F343E)
}
pub fn text() -> Rgba {
    pick(0xD1D3D9, 0xDCE0E5)
}
/// StatusBar.Widget.foreground, ToolWindow.Button.foreground.
pub fn text_muted() -> Rgba {
    pick(0x9FA2A8, 0xA9AFBC)
}
pub fn text_secondary() -> Rgba {
    pick(0x73767C, 0x878A98)
}
pub fn accent() -> Rgba {
    pick(0x3871E1, 0x74ADE8)
}
/// ProgressBar.trackColor: Islands Dark's control-bg-small (gray-50).
pub fn progress_track() -> Rgba {
    pick(0x40434A, 0x363C46)
}
/// ProgressBar.indeterminateStartColor: Islands Dark's blue-110; it ends in progressColor, the accent.
pub fn progress_start() -> Rgba {
    pick(0x92B7FF, 0xA3C8F0)
}
pub fn on_accent() -> Rgba {
    pick(0xFFFFFF, 0x282C33)
}
/// Tree.selectionBackground with the tree focused, and without.
pub fn selection() -> Rgba {
    pick(0x2A4371, 0x454A56)
}
pub fn selection_inactive() -> Rgba {
    pick(0x33353B, 0x363C46)
}
pub fn row_hover() -> Rgba {
    pick_a(0xFFFFFF10, 0xFFFFFF0C)
}
pub fn button_hover() -> Rgba {
    pick_a(0xFFFFFF17, 0xFFFFFF12)
}
pub fn button_pressed() -> Rgba {
    pick_a(0xFFFFFF29, 0xFFFFFF20)
}
/// Component.borderColor: a field's, a checkbox's, a plain button's edge.
pub fn control_border() -> Rgba {
    pick(0x40434A, 0x464B57)
}
/// Popup.background and Popup.borderColor.
pub fn popup() -> Rgba {
    pick(0x26282C, 0x2F343E)
}
pub fn popup_border() -> Rgba {
    pick(0x33353B, 0x464B57)
}
/// Editor.SearchField.background (editor-bg-inline, gray-20): the find bar's ground; its border is the frame's.
pub fn search_field() -> Rgba {
    pick(0x212326, 0x282C33)
}
/// MainToolbar.separatorColor and ToolWindow.Stripe.separatorColor.
pub fn separator() -> Rgba {
    pick_a(0xFFFFFF21, 0x464B57FF)
}
/// ToolWindow.Button.DragAndDrop.buttonDropBackground: where a dragged window would land.
pub fn drop_zone() -> Rgba {
    pick_a(0x3671E580, 0x74ADE84D)
}
/// Windows' own red under the pointer on a caption's Close.
pub fn close_hover() -> Rgba {
    rgb(0xE81123)
}
pub fn tab_selected() -> Rgba {
    pick(0x233558, 0x282C33)
}
/// tab-selected-bg-inactive (gray-30): the tab a split shows while another split has the keys.
pub fn tab_selected_inactive() -> Rgba {
    pick(0x26282C, 0x2F343E)
}
pub fn tab_selected_border() -> Rgba {
    pick(0x2E4D89, 0x363C46)
}
pub fn modified() -> Rgba {
    pick(0x548AF7, 0xDEC184)
}
pub fn error() -> Rgba {
    pick(0xF75464, 0xD07277)
}
pub fn warning() -> Rgba {
    pick(0xE5A336, 0xDEC184)
}
/// A passed test's green: the stroke of expui's `run/testPassed_dark.svg` (and of `run/run_dark.svg`).
pub fn passed() -> Rgba {
    pick(0x57965C, 0xA1C181)
}
/// A bookmark's yellow: the fill of expui's `gutter/bookmark_dark.svg`.
pub fn bookmark() -> Rgba {
    rgb(0xF2C55C)
}
/// A failed test's red: the fill of expui's `run/testError_dark.svg` and `general/error_dark.svg`.
pub fn failed() -> Rgba {
    pick(0xDB5C5C, 0xD07277)
}

/// The colour picker's marker ring and its shadow, light over dark so it shows on any colour. Rider's picker paints
/// its own in code, not from its theme, so these are RSharp's.
pub fn colour_marker() -> Rgba {
    rgb(0xFFFFFF)
}
pub fn colour_marker_shadow() -> Rgba {
    rgba(0x0000_0099)
}
/// The checkerboard under a clear colour, a light square and a darker one, as every picker draws transparency.
pub fn colour_checker_light() -> Rgba {
    rgb(0xFFFFFF)
}
pub fn colour_checker_dark() -> Rgba {
    rgb(0xCCCCCC)
}

/// The editor's own colours (Islands Dark's editor scheme over Rider Dark's).
pub mod editor {
    use super::*;
    /// Editor.background: Rider's island; Zed's editor is a step darker than its panels.
    pub fn background() -> Rgba {
        pick(0x191A1C, 0x282C33)
    }
    pub fn text() -> Rgba {
        pick(0xBCBEC4, 0xACB2BE)
    }
    pub fn line_number() -> Rgba {
        pick(0x4B5059, 0x4E5A5F)
    }
    pub fn line_number_current() -> Rgba {
        pick(0xA1A3AB, 0xD0D4DA)
    }
    pub fn current_line() -> Rgba {
        pick(0x1F2024, 0x2E333C)
    }
    pub fn selection() -> Rgba {
        pick(0x214283, 0x3A4B5E)
    }
    pub fn caret() -> Rgba {
        pick(0xCED0D6, 0x74ADE8)
    }
    /// MATCHED_BRACE_ATTRIBUTES' background (IslandSchemeDark.xml).
    pub fn matched_brace() -> Rgba {
        pick(0x43454A, 0x454A56)
    }
    /// FOLDED_TEXT_ATTRIBUTES: a folded region's placeholder.
    pub fn folded_text() -> Rgba {
        pick(0x868991, 0x878A98)
    }
    pub fn folded_background() -> Rgba {
        pick(0x393B40, 0x3B414D)
    }
    /// WHITESPACES: the dots and arrows Show Whitespaces draws.
    pub fn whitespace() -> Rgba {
        pick(0x6F737A, 0x575D65)
    }
    /// DIFF_INSERTED's and DIFF_DELETED's backgrounds (Darcula's, which Islands Dark inherits).
    pub fn diff_inserted() -> Rgba {
        pick(0x294436, 0x354036)
    }
    pub fn diff_deleted() -> Rgba {
        pick(0x484A4A, 0x42343A)
    }
    /// TEXT_SEARCH_RESULT_ATTRIBUTES' background (IslandSchemeDark.xml): a find bar's match.
    /// Rider's line breakpoint: the red dot its gutter shows (#DB5C5C, its stop icon's stroke).
    pub fn breakpoint() -> Rgba {
        pick(0xDB5C5C, 0xE06C75)
    }

    /// Rider's execution point: the band under the statement the debugger is stopped at, a step above the current
    /// line's grey so the stopped statement stands out without colouring the code.
    pub fn execution_point() -> Rgba {
        pick(0x2F3E54, 0x43423F)
    }

    pub fn search_result() -> Rgba {
        pick(0x114957, 0x455C78)
    }
    /// IDENTIFIER_UNDER_CARET_ATTRIBUTES and WRITE_IDENTIFIER_UNDER_CARET_ATTRIBUTES' backgrounds: the usages of
    /// what the caret is on, read and written (Rider Dark, as Passenger's RiderScheme.axaml carries it).
    pub fn usage_read() -> Rgba {
        pick(0x373B39, 0x343C48)
    }
    pub fn usage_write() -> Rgba {
        pick(0x402F33, 0x45383C)
    }
    /// INLINE_PARAMETER_HINT's foreground and background (Rider Dark, as Passenger carries it).
    pub fn inlay_hint() -> Rgba {
        pick(0x8F9298, 0x878A98)
    }
    pub fn inlay_hint_background() -> Rgba {
        pick(0x2B2D30, 0x2F343E)
    }
}

/// How a kind of code is drawn beside its colour: a Markdown heading and strong emphasis bold, emphasis italic, as
/// Rider draws MARKDOWN_HEADER_LEVEL_1, _BOLD and _ITALIC; none for the rest, drawn as the font is.
pub fn face(kind: Kind) -> Option<(gpui::FontWeight, gpui::FontStyle)> {
    match kind {
        Kind::Heading | Kind::Bold => Some((gpui::FontWeight::BOLD, gpui::FontStyle::Normal)),
        Kind::Italic => Some((gpui::FontWeight::NORMAL, gpui::FontStyle::Italic)),
        _ => None,
    }
}

/// Rider Dark's colour for a kind of code: ReSharper's for C#, Rider's own attributes for markup and data (XML_TAG,
/// XML_ATTRIBUTE_NAME and _VALUE, XML_ENTITY_REFERENCE, JSON.PROPERTY_KEY, YAML_SCALAR_KEY, MARKDOWN_HEADER_LEVEL_1,
/// MARKDOWN_CODE_SPAN, MARKDOWN_LINK_DESTINATION), as Passenger's RiderScheme.axaml carries them.
pub fn syntax(kind: Kind) -> Hsla {
    if matches!(kind, Kind::Bold | Kind::Italic) {
        return editor::text().into();
    }
    if is_zed() {
        return rgb(match kind {
            Kind::Keyword => 0xB477CF,
            Kind::String | Kind::AttributeValue | Kind::Code => 0xA1C181,
            Kind::StringEscape => 0x878E98,
            Kind::Number => 0xBF956A,
            Kind::Comment | Kind::DocComment => 0x5D636F,
            Kind::Type | Kind::Namespace | Kind::LinkDestination => 0x6EB4BF,
            Kind::Method => 0x73ADE9,
            Kind::Property | Kind::JsonKey | Kind::YamlKey | Kind::Heading => 0xD07277,
            Kind::Tag | Kind::AttributeName => 0x74ADE8,
            Kind::Entity => 0xDFC184,
            Kind::Bold | Kind::Italic => unreachable!("in the text's colour, above"),
        })
        .into();
    }
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
        Kind::Bold | Kind::Italic => unreachable!("in the text's colour, above"),
    })
    .into()
}

/// Rider Dark's colour for what Roslyn names a word (its semantic token types, by its legend's names), where it says more than the syntax:
/// ReSharper's identifier colours, as Passenger's RiderScheme.axaml carries them. A local or a parameter is plain
/// text, which takes back a colour the syntax guessed. None leaves the syntax's colour.
pub fn semantic(kind: &str) -> Option<Hsla> {
    if is_zed() {
        if let Some(colour) = zed_semantic(kind) {
            return Some(rgb(colour).into());
        }
    }
    let colour = match kind {
        "class" | "recordClass" | "interface" | "typeParameter" => 0xC191FF,
        "struct" | "recordStruct" | "enum" | "delegate" => 0xE1BFFF,
        "method" | "extensionMethod" | "function" => 0x39CC9B,
        "property" | "field" | "constant" | "enumMember" => 0x66C3CC,
        "event" => 0xED94C0,
        "parameter" | "variable" | "label" | "namespace" => 0xBCBEC4,
        "excludedCode" => 0x6F737A,
        // Razor's own, for a Razor file's markup: a component as a type and its parameters as properties, as Rider has
        // them, HTML's as the HTML grammar colours it, Razor's directives and transitions as keywords (Passenger's map).
        "razorTransition" | "razorDirective" | "razorDirectiveColon" | "razorDirectiveAttribute" => {
            return Some(syntax(Kind::Keyword));
        }
        "razorComponentElement" | "razorTagHelperElement" => 0xC191FF,
        "razorComponentAttribute" | "razorTagHelperAttribute" => 0x66C3CC,
        "razorComment"
        | "razorCommentStar"
        | "razorCommentTransition"
        | "markupComment"
        | "markupCommentPunctuation" => {
            return Some(syntax(Kind::Comment));
        }
        "markupElement" | "markupTagDelimiter" | "markupOperator" => return Some(syntax(Kind::Tag)),
        "markupAttribute" => return Some(syntax(Kind::AttributeName)),
        "markupAttributeValue" | "markupAttributeQuote" => return Some(syntax(Kind::AttributeValue)),
        _ => return None,
    };
    Some(rgb(colour).into())
}

/// One Dark's colours for Roslyn's identifier kinds; the markup kinds fall through to the syntax's, which already follow.
fn zed_semantic(kind: &str) -> Option<u32> {
    Some(match kind {
        "class"
        | "recordClass"
        | "interface"
        | "typeParameter"
        | "struct"
        | "recordStruct"
        | "enum"
        | "delegate"
        | "razorComponentElement"
        | "razorTagHelperElement" => 0x6EB4BF,
        "method" | "extensionMethod" | "function" => 0x73ADE9,
        "property" | "field" | "event" | "razorComponentAttribute" | "razorTagHelperAttribute" => 0xD07277,
        "constant" | "enumMember" => 0xDFC184,
        "parameter" | "variable" | "label" | "namespace" => 0xACB2BE,
        "excludedCode" => 0x5D636F,
        _ => return None,
    })
}

/// The shown tool window's stripe button: Rider fills it with the accent, Zed marks a toggled button with
/// element.selected.
pub fn toggled() -> Rgba {
    pick(0x3871E1, 0x454A56)
}

/// Rider's pair, Inter and JetBrains Mono; Zed's, IBM Plex Sans and Lilex (its `.ZedSans` and `.ZedMono`).
pub fn ui_font() -> &'static str {
    if is_zed() { "IBM Plex Sans" } else { "Inter" }
}
pub fn code_font() -> &'static str {
    if is_zed() { "Lilex" } else { "JetBrains Mono" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zed_design_flattens_the_islands_and_wears_one_dark() {
        set_design(Design::Zed);
        assert_eq!(island(), rgb(0x2F343E), "Zed's panels should take One Dark's panel ground");
        assert_eq!(layout::island_radius(), 0., "Zed's panes should be square, not rounded islands");
        assert_eq!(layout::gap(), 1., "Zed's panes should be split by a 1px line, not a 6px gap");
        assert_eq!((ui_font(), code_font()), ("IBM Plex Sans", "Lilex"), "Zed's design should wear Zed's fonts");
        set_design(Design::Islands);
        assert_eq!(island(), rgb(0x191A1C), "switching back should restore Rider's island ground");
    }
}
