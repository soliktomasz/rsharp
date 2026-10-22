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
    /// Editor | Font | Size.
    pub editor_font_size: f32,
    /// Editor | Font | Line height, as a multiple of the font's own.
    pub editor_line_spacing: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            reopen_projects_on_startup: true,
            show_line_numbers: true,
            editor_font_size: 13.,
            editor_line_spacing: 1.2,
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
