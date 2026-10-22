//! The fonts and icons carried inside the executable.

use gpui::{App, AssetSource, ImageSource, Img, Resource, SharedString, img, prelude::*};
use std::borrow::Cow;

macro_rules! carried {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $name)))),*]
    };
}

static FILES: &[(&str, &[u8])] = carried![
    "icons/branch.svg",
    "icons/check.svg",
    "icons/chevron-down.svg",
    "icons/chevron-right.svg",
    "icons/close.svg",
    "icons/down.svg",
    "icons/error.svg",
    "icons/file-any.svg",
    "icons/file-config.svg",
    "icons/file-csharp.svg",
    "icons/file-json.svg",
    "icons/file-text.svg",
    "icons/file-xml.svg",
    "icons/folder.svg",
    "icons/hide.svg",
    "icons/main-menu.svg",
    "icons/match-case.svg",
    "icons/pin.svg",
    "icons/project.svg",
    "icons/regex.svg",
    "icons/search.svg",
    "icons/tw-problems.svg",
    "icons/tw-project.svg",
    "icons/up.svg",
    "icons/warning.svg",
    "icons/words.svg",
];

static FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.otf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.otf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(name, _)| name.starts_with(path)).map(|(name, _)| (*name).into()).collect())
    }
}

pub fn load_fonts(cx: &App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("the carried fonts load");
}

/// One of IntelliJ's New UI icons, drawn in its own colours.
pub fn icon(name: &'static str, size: f32) -> Img {
    img(ImageSource::Resource(Resource::Embedded(format!("icons/{name}.svg").into()))).size(gpui::px(size)).flex_none()
}

/// The icon the Solution view and a tab give a file, by its name.
pub fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase().as_str() {
        "cs" | "csx" => "file-csharp",
        "csproj" | "fsproj" | "vbproj" => "project",
        "xml" | "xaml" | "axaml" | "props" | "targets" | "sln" | "slnx" | "config" | "resx" => "file-xml",
        "json" => "file-json",
        "md" | "txt" => "file-text",
        "editorconfig" | "gitignore" | "gitattributes" | "toml" | "yml" | "yaml" => "file-config",
        _ => "file-any",
    }
}
