//! Test identities and the native protocols of the SDK's VSTest and Microsoft.Testing.Platform runners.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

mod assemblies;
mod platform;
mod protocol;
mod runners;
mod vstest;

pub use assemblies::resolve_assemblies;
pub use runners::TestRunners;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestItem {
    pub project: PathBuf,
    pub fully_qualified_name: String,
    pub display_name: String,
    pub framework: Option<String>,
    pub row: Option<String>,
    pub path: Option<PathBuf>,
    pub line: usize,
    pub traits: Vec<(String, String)>,
}

impl TestItem {
    pub fn method(&self) -> &str {
        Self::method_of(&self.fully_qualified_name)
    }

    pub fn method_of(name: &str) -> &str {
        name.split_once('(').map_or(name, |(method, _)| method)
    }

    pub fn method_id(&self) -> String {
        format!("{}::{}", self.project.display(), self.method())
    }

    pub fn id(&self) -> String {
        match &self.row {
            Some(row) => format!("{}::{row}", self.method_id()),
            None => self.method_id(),
        }
    }

    pub fn key(&self) -> String {
        format!("{}|{}", self.framework.as_deref().unwrap_or_default(), self.id())
    }

    pub fn row_of(name: &str, display: &str) -> Option<String> {
        let method = Self::method_of(name);
        let (class, short) = method.rsplit_once('.').map_or(("", method), |(class, short)| (class, short));
        let shown = display.strip_prefix(&format!("{class}.")).unwrap_or(display);
        (shown != short && shown != method && (shown.contains('(') || name != method)).then(|| shown.to_string())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestState {
    #[default]
    NotRun,
    Running,
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestAttachment {
    pub name: String,
    pub path: PathBuf,
}

impl TestAttachment {
    pub fn of(name: &str, uri_or_path: &str) -> Self {
        let path = crate::paths::from_uri(uri_or_path).unwrap_or_else(|| PathBuf::from(uri_or_path));
        let name = if name.is_empty() {
            path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            name.to_string()
        };
        Self { name, path }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestOutcome {
    pub test: TestItem,
    pub state: TestState,
    pub duration: Duration,
    pub message: Option<String>,
    pub stack_trace: Option<String>,
    pub output: Option<String>,
    pub attachments: Vec<TestAttachment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestAssembly {
    pub project: PathBuf,
    pub framework: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum TestEvent {
    Log(String),
    Discovered(Vec<TestItem>),
    Outcomes(Vec<TestOutcome>),
}

#[derive(Clone, Debug, Default)]
pub struct RunSummary {
    pub elapsed: Duration,
    pub canceled: bool,
    pub aborted: bool,
    pub error: Option<String>,
    pub outcomes: Vec<TestOutcome>,
}

type Events<'a> = &'a mut dyn FnMut(TestEvent);

fn canceled(cancel: &AtomicBool) -> bool {
    cancel.load(std::sync::atomic::Ordering::Relaxed)
}

fn source_path(folder: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    crate::paths::canonical(&if path.is_absolute() { path.to_path_buf() } else { folder.join(path) })
}
