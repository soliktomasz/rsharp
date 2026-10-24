//! What the Host's `inspect` knows of one project's evaluation for the code help in its MSBuild files (Passenger.Host's
//! `ProjectInsight`): places name their file by its index in `files`, lines and columns from 1 as MSBuild counts them.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInsight {
    pub project: PathBuf,
    /// False for a project that did not evaluate (an unknown SDK, XML that does not read): its diagnostics alone.
    pub evaluated: bool,
    pub framework: Option<String>,
    pub files: Vec<PathBuf>,
    pub diagnostics: Vec<InsightDiagnostic>,
    pub properties: Vec<InsightProperty>,
    pub item_types: Vec<InsightItemType>,
    pub items: Vec<InsightItem>,
    pub targets: Vec<InsightTarget>,
    pub imports: Vec<InsightImport>,
    /// The target frameworks the SDK knows, newest first.
    pub frameworks: Vec<InsightFramework>,
    pub installed: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InsightPlace {
    pub file: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightDiagnostic {
    pub error: bool,
    pub code: Option<String>,
    pub message: String,
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightProperty {
    pub name: String,
    pub value: String,
    /// 0 set in a file, 1 a global property, 2 from the environment, 3 MSBuild's own.
    pub origin: u8,
    /// The places that set it, the one that last did first.
    pub set: Vec<InsightPlace>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightItemType {
    pub name: String,
    pub count: usize,
    pub includes: Vec<String>,
    pub metadata: Vec<String>,
    pub defined: Vec<InsightPlace>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightItem {
    #[serde(rename = "type")]
    pub item_type: String,
    pub element: InsightPlace,
    pub count: usize,
    pub includes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightTarget {
    pub name: String,
    /// Every place it is defined, the one in force first.
    pub defined: Vec<InsightPlace>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightImport {
    pub element: InsightPlace,
    pub imported: PathBuf,
    pub sdk: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsightFramework {
    pub alias: String,
    pub name: String,
}

impl ProjectInsight {
    pub fn property(&self, name: &str) -> Option<&InsightProperty> {
        self.properties.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn target(&self, name: &str) -> Option<&InsightTarget> {
        self.targets.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn item_type(&self, name: &str) -> Option<&InsightItemType> {
        self.item_types.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// A place's file, and its line and column from 0.
    pub fn at(&self, place: InsightPlace) -> Option<(PathBuf, usize, usize)> {
        Some((self.files.get(place.file)?.clone(), place.line.saturating_sub(1), place.column.saturating_sub(1)))
    }

    /// Whether a place is in this file at this line (from 1).
    pub fn is_here(&self, place: InsightPlace, file: &Path, line: usize) -> bool {
        place.line == line && self.files.get(place.file).is_some_and(|f| crate::paths::canonical(f) == file)
    }
}
