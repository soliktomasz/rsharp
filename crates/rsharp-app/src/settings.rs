//! The person's settings (`settings.json` in the state store), held once per process: a change is written,
//! put in force, and every window and editor follows it.

use crate::state::store;
use rsharp_core::state::StateDocument;
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    /// Appearance & Behavior | System Settings | Reopen projects on startup.
    pub reopen_projects_on_startup: bool,
    /// Editor | General | Appearance | Show line numbers.
    pub show_line_numbers: bool,
    /// Editor | General | Appearance | Show whitespaces.
    pub show_whitespace: bool,
    /// Editor | Font | Size.
    pub editor_font_size: f32,
    /// Editor | Font | Line height, as a multiple of the font's own.
    pub editor_line_spacing: f32,
    /// Build, Execution, Deployment | Build | Presentation.
    pub open_build_window: bool,
    pub build_results: BuildResults,
    /// Build, Execution, Deployment | Hot Reload | Hot Reload when running (dotnet watch): Run starts the program under
    /// `dotnet watch`, so a save applies its edits to it, or restarts it with them.
    pub hot_reload: bool,
    /// Always restart for a change neither dotnet watch nor the debugged program's runtime can apply, rather than saying a
    /// restart is needed and leaving the program on its old code: dotnet watch's `DOTNET_WATCH_RESTART_ON_RUDE_EDIT`, and
    /// Debug's restart after a save's rude edit. Nothing asks, the Run console taking no input until the terminal
    /// (Phase 6).
    pub hot_reload_restart_on_rude_edit: bool,
    /// Editor | Code Style | XAML: how Reformat Code lays out XAML, under each file's .editorconfig.
    pub xaml_format: XamlFormat,
}

/// Rider's XAML code style (Passenger's XamlFormatSettings, by its names, which the XAML host reads): its XML formatter's
/// options at Rider's defaults, and XAML Styler's attribute order and empty-element style, off as Rider has them.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct XamlFormat {
    pub attribute_style: AttributeStyle,
    pub attribute_indent: AttributeIndent,
    pub spaces_around_equals: bool,
    pub space_after_last_attribute: bool,
    pub space_before_self_closing: bool,
    pub keep_line_breaks: bool,
    pub max_blank_lines: u32,
    pub multiline_elements_on_new_line: bool,
    pub singleline_elements_on_new_line: bool,
    pub breaks_inside_elements_with_children: bool,
    pub breaks_inside_multiline_elements: bool,
    pub child_indent: ContentIndent,
    pub text_indent: ContentIndent,
    pub blank_line_after_processing_instructions: bool,
    pub wrap_tags: bool,
    pub wrap_around_elements: bool,
    pub right_margin: u32,
    pub reorder_attributes: bool,
    /// XAML Styler's rule groups, a group to each `;`.
    pub attribute_order: String,
    pub collapse_empty_elements: bool,
}

/// Attributes format, in Rider's order.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AttributeStyle {
    OnSingleLine,
    FirstAttributeOnSingleLine,
    OnDifferentLines,
    #[default]
    DoNotTouch,
}

/// Attributes indenting, in Rider's order.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AttributeIndent {
    SingleIndent,
    DoubleIndent,
    #[default]
    AlignByFirstAttribute,
}

/// Indent inside element, in Rider's order.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ContentIndent {
    DoNotTouch,
    #[default]
    OneIndent,
    ZeroIndent,
    RemoveIndent,
}

impl XamlFormat {
    /// XAML Styler's default rule groups (Passenger's XamlFormatSettings.DefaultAttributeOrder).
    pub const DEFAULT_ATTRIBUTE_ORDER: &'static str = "x:Class; xmlns, xmlns:x; xmlns:*; x:Key, Key, x:Name, Name, x:Uid, \
        Uid, Title; Grid.Row, Grid.RowSpan, Grid.Column, Grid.ColumnSpan, Canvas.Left, Canvas.Top, Canvas.Right, \
        Canvas.Bottom; Width, Height, MinWidth, MinHeight, MaxWidth, MaxHeight; Margin, Padding, HorizontalAlignment, \
        VerticalAlignment, HorizontalContentAlignment, VerticalContentAlignment, Panel.ZIndex; *:*, *; PageSource, \
        PageIndex, Offset, Color, TargetName, Property, Value, StartPoint, EndPoint; mc:Ignorable, d:IsDataSource, \
        d:LayoutOverrides, d:IsStaticText; Storyboard.*, From, To, Duration";
    pub const BLANK_LINES: RangeInclusive<f32> = 0.0..=100.0;
    pub const MARGINS: RangeInclusive<f32> = 0.0..=1000.0;
}

impl Default for XamlFormat {
    fn default() -> Self {
        Self {
            attribute_style: AttributeStyle::DoNotTouch,
            attribute_indent: AttributeIndent::AlignByFirstAttribute,
            spaces_around_equals: false,
            space_after_last_attribute: false,
            space_before_self_closing: true,
            keep_line_breaks: true,
            max_blank_lines: 2,
            multiline_elements_on_new_line: true,
            singleline_elements_on_new_line: false,
            breaks_inside_elements_with_children: true,
            breaks_inside_multiline_elements: true,
            child_indent: ContentIndent::OneIndent,
            text_indent: ContentIndent::OneIndent,
            blank_line_after_processing_instructions: true,
            wrap_tags: true,
            wrap_around_elements: true,
            right_margin: 120,
            reorder_attributes: false,
            attribute_order: Self::DEFAULT_ATTRIBUTE_ORDER.into(),
            collapse_empty_elements: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BuildResults {
    Always,
    WarningsOrErrors,
    #[default]
    Errors,
    Never,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            reopen_projects_on_startup: true,
            show_line_numbers: true,
            show_whitespace: false,
            editor_font_size: 13.,
            editor_line_spacing: 1.2,
            open_build_window: true,
            build_results: BuildResults::Errors,
            hot_reload: false,
            hot_reload_restart_on_rude_edit: true,
            xaml_format: XamlFormat::default(),
        }
    }
}

impl StateDocument for AppSettings {
    const FILE: &'static str = "settings.json";
}

impl AppSettings {
    pub const FONT_SIZES: RangeInclusive<f32> = 6.0..=72.0;
    pub const LINE_SPACINGS: RangeInclusive<f32> = 0.6..=3.0;

    /// Every value inside its range, as Rider's fields hold them.
    pub fn clamped(mut self) -> Self {
        self.editor_font_size = self.editor_font_size.clamp(*Self::FONT_SIZES.start(), *Self::FONT_SIZES.end());
        self.editor_line_spacing =
            self.editor_line_spacing.clamp(*Self::LINE_SPACINGS.start(), *Self::LINE_SPACINGS.end());
        let xaml = &mut self.xaml_format;
        xaml.max_blank_lines = xaml.max_blank_lines.min(*XamlFormat::BLANK_LINES.end() as u32);
        xaml.right_margin = xaml.right_margin.min(*XamlFormat::MARGINS.end() as u32);
        if xaml.attribute_order.trim().is_empty() {
            xaml.attribute_order = XamlFormat::DEFAULT_ATTRIBUTE_ORDER.into();
        }
        self
    }

    /// The editor's line height: the font's own (JetBrains Mono's ascent and descent, 1.32 em) times the spacing.
    pub fn line_height(&self) -> f32 {
        (self.editor_font_size * 1.32 * self.editor_line_spacing).round()
    }
}

/// The settings in force.
pub struct Settings(pub AppSettings);

impl gpui::Global for Settings {}

pub fn settings(cx: &gpui::App) -> &AppSettings {
    &cx.global::<Settings>().0
}

/// Puts `new` in force and keeps it; every view observing [`Settings`] draws again.
pub fn apply(new: AppSettings, cx: &mut gpui::App) {
    let new = new.clamped();
    store(cx).write(&new);
    cx.set_global(Settings(new));
}
