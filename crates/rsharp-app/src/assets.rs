//! The fonts and icons carried inside the executable.

use gpui::{App, AssetSource, ImageSource, Img, Resource, SharedString, img, prelude::*};
use std::borrow::Cow;

macro_rules! carried {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $name)))),*]
    };
}

static FILES: &[(&str, &[u8])] = carried![
    "icons/add.svg",
    "icons/branch.svg",
    "icons/check.svg",
    "icons/chevron-down.svg",
    "icons/chevron-right.svg",
    "icons/clear.svg",
    "icons/close.svg",
    "icons/debug-mute.svg",
    "icons/debug-pause.svg",
    "icons/debug-resume.svg",
    "icons/debug-step-into.svg",
    "icons/debug-step-out.svg",
    "icons/debug-step-over.svg",
    "icons/debug.svg",
    "icons/dependencies.svg",
    "icons/down.svg",
    "icons/error.svg",
    "icons/file-any.svg",
    "icons/file-config.svg",
    "icons/file-csharp.svg",
    "icons/file-json.svg",
    "icons/file-text.svg",
    "icons/file-xml.svg",
    "icons/folder.svg",
    "icons/framework.svg",
    "icons/hide.svg",
    "icons/locked.svg",
    "icons/main-menu.svg",
    "icons/node-class.svg",
    "icons/node-constant.svg",
    "icons/node-constructor.svg",
    "icons/node-enum.svg",
    "icons/node-field.svg",
    "icons/node-interface.svg",
    "icons/node-method.svg",
    "icons/node-property.svg",
    "icons/match-case.svg",
    "icons/package.svg",
    "icons/pin.svg",
    "icons/project.svg",
    "icons/regex.svg",
    "icons/rerun.svg",
    "icons/run.svg",
    "icons/search.svg",
    "icons/settings.svg",
    "icons/show.svg",
    "icons/solution-folder.svg",
    "icons/solution.svg",
    "icons/stop.svg",
    "icons/tw-build.svg",
    "icons/tw-commit.svg",
    "icons/tw-debug.svg",
    "icons/tw-find.svg",
    "icons/tw-git.svg",
    "icons/tw-hierarchy.svg",
    "icons/tw-problems.svg",
    "icons/tw-profiler.svg",
    "icons/tw-project.svg",
    "icons/tw-run.svg",
    "icons/tw-terminal.svg",
    "icons/up.svg",
    "icons/warning.svg",
    "icons/words.svg",
    "icons/collapse-all.svg",
    "icons/expand-all.svg",
    "icons/test-failed.svg",
    "icons/test-ignored.svg",
    "icons/test-not-run.svg",
    "icons/test-passed.svg",
];

static FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.otf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.otf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    // Zed's own pair, worn by the Zed design.
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Lilex-Regular.ttf"),
    include_bytes!("../assets/fonts/Lilex-Bold.ttf"),
    include_bytes!("../assets/fonts/Lilex-Italic.ttf"),
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

#[cfg(test)]
mod tests {
    /// An icon left out of `FILES` draws as nothing, which is how the Debug window lost its step buttons.
    #[test]
    fn every_icon_on_disk_is_carried() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons");
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = format!("icons/{}", entry.unwrap().file_name().to_string_lossy());
            assert!(
                super::FILES.iter().any(|(carried, _)| *carried == name),
                "{name} is not carried, so it draws blank"
            );
        }
    }
}
