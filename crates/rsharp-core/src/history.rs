//! Rider's Local History: each version of a file the IDE saw, kept under the state root's `localHistory/`, each
//! content once by its hash in `blobs/` and a line per version in `changes.log`, the oldest pruned after Rider's
//! 5 days. Plain file I/O: the app calls it off the UI thread.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a version is kept, as Rider's default.
pub const KEPT: Duration = Duration::from_secs(5 * 24 * 60 * 60);

/// One version of a file: when it was seen, its content's name, and what the IDE called the moment if anything.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Revision {
    pub path: PathBuf,
    /// Seconds since 1970.
    pub time: u64,
    pub blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone)]
pub struct LocalHistory {
    root: PathBuf,
}

impl LocalHistory {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn log(&self) -> PathBuf {
        self.root.join("changes.log")
    }

    fn blob_path(&self, blob: &str) -> PathBuf {
        self.root.join("blobs").join(blob)
    }

    /// Keeps `bytes` as the file's newest version unless they are what its newest is already. A first version is
    /// unlabelled; a later one takes `label` (an External change seen as the file opens, say).
    pub fn record(&self, path: &Path, bytes: &[u8], label: Option<&str>) -> std::io::Result<bool> {
        let blob = format!("{:016x}-{:x}", crate::state::fnv(bytes), bytes.len());
        let previous = self.revisions(path);
        if previous.last().is_some_and(|r| r.blob == blob) {
            return Ok(false);
        }
        std::fs::create_dir_all(self.root.join("blobs"))?;
        let stored = self.blob_path(&blob);
        if !stored.exists() {
            crate::files::write(&stored, bytes)?;
        }
        let revision = Revision {
            path: path.to_path_buf(),
            time: now(),
            blob,
            label: label.filter(|_| !previous.is_empty()).map(str::to_string),
        };
        let mut line = serde_json::to_string(&revision).map_err(std::io::Error::other)?;
        line.push('\n');
        // One write per line, so lines from tasks running at once never interleave.
        std::fs::OpenOptions::new().create(true).append(true).open(self.log())?.write_all(line.as_bytes())?;
        Ok(true)
    }

    /// A file's versions, oldest first.
    pub fn revisions(&self, path: &Path) -> Vec<Revision> {
        let Ok(log) = std::fs::read_to_string(self.log()) else { return Vec::new() };
        log.lines().filter_map(|l| serde_json::from_str::<Revision>(l).ok()).filter(|r| r.path == path).collect()
    }

    pub fn content(&self, revision: &Revision) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.blob_path(&revision.blob))
    }

    /// Forgets the versions older than [`KEPT`] and the content nothing names any more.
    // shortcut: kept by the clock, not by Rider's days of activity; count those once someone away a week misses
    // their history.
    pub fn prune(&self) -> std::io::Result<()> {
        let Ok(log) = std::fs::read_to_string(self.log()) else { return Ok(()) };
        let oldest = now().saturating_sub(KEPT.as_secs());
        let kept: Vec<Revision> =
            log.lines().filter_map(|l| serde_json::from_str::<Revision>(l).ok()).filter(|r| r.time >= oldest).collect();
        let mut text = String::new();
        for revision in &kept {
            text.push_str(&serde_json::to_string(revision).map_err(std::io::Error::other)?);
            text.push('\n');
        }
        crate::files::write(&self.log(), text.as_bytes())?;
        if let Ok(blobs) = std::fs::read_dir(self.root.join("blobs")) {
            for blob in blobs.flatten() {
                let name = blob.file_name().to_string_lossy().into_owned();
                if kept.iter().all(|r| r.blob != name) {
                    let _ = std::fs::remove_file(blob.path());
                }
            }
        }
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_keeps_each_new_version_once_and_its_content_comes_back() {
        let root = std::env::temp_dir().join(format!("rsharp-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let history = LocalHistory::new(root.clone());
        let file = Path::new("/work/Program.cs");
        assert!(history.record(file, b"one", Some("External change")).unwrap());
        assert!(!history.record(file, b"one", None).unwrap(), "the same content was kept twice");
        assert!(history.record(file, b"two", Some("External change")).unwrap());
        let revisions = history.revisions(file);
        let labels: Vec<_> = revisions.iter().map(|r| r.label.as_deref()).collect();
        assert_eq!(labels, [None, Some("External change")], "a first version was labelled, or a later one not");
        assert_eq!(history.content(&revisions[0]).unwrap(), b"one");
        assert!(history.revisions(Path::new("/work/Other.cs")).is_empty(), "another file's versions were listed");
        history.prune().unwrap();
        assert_eq!(history.revisions(file).len(), 2, "pruning took versions younger than five days");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
