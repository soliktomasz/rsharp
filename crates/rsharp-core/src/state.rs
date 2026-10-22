//! The IDE's own state, in one store: the app's documents at its root (`settings.json`, `recentSolutions.json`,
//! ...) and each folder's under `workspaces/<name>-<hash>/`. A document is a serde type; a write is debounced
//! on the store's own thread and merged over the file, so keys this version does not know stay.

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a write waits for the next one to the same file, so a burst is one write.
const WRITE_DELAY: Duration = Duration::from_millis(500);

/// A record kept in the store, by its file's name.
pub trait StateDocument: Serialize + DeserializeOwned + Default {
    const FILE: &'static str;
}

enum Job {
    Write(PathBuf, Value),
    Flush(Sender<()>),
}

#[derive(Clone)]
pub struct StateStore {
    root: PathBuf,
    writer: Sender<Job>,
    /// Files that did not read, set aside as `.bad`, until a window says so.
    unsaid: Arc<Mutex<Vec<String>>>,
    /// What each file was last told, so a read never sees the disk from before a write still waiting.
    written: Arc<Mutex<HashMap<PathBuf, Value>>>,
}

impl StateStore {
    /// The store under `RSHARP_STATE`, else the person's local application data.
    pub fn default_root() -> PathBuf {
        if let Some(root) = std::env::var_os("RSHARP_STATE") {
            return root.into();
        }
        let home = crate::programs::home();
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/RSharp")
        } else if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or(home).join("RSharp")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("rsharp")
        }
    }

    pub fn open(root: PathBuf) -> Self {
        let (writer, jobs) = mpsc::channel();
        std::thread::Builder::new()
            .name("state-writer".into())
            .spawn(move || write_loop(jobs))
            .expect("the state writer starts");
        Self { root, writer, unsaid: Arc::default(), written: Arc::default() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A folder's own place in the store, named for the person and unique to its path.
    pub fn folder(&self, folder: &Path) -> PathBuf {
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.root.join("workspaces").join(format!("{name}-{:016x}", fnv(folder.to_string_lossy().as_bytes())))
    }

    /// An app document.
    pub fn read<T: StateDocument>(&self) -> T {
        self.read_at(&self.root.join(T::FILE))
    }

    pub fn write<T: StateDocument>(&self, value: &T) {
        self.write_at(self.root.join(T::FILE), value);
    }

    /// A folder's document.
    pub fn read_for<T: StateDocument>(&self, folder: &Path) -> T {
        self.read_at(&self.folder(folder).join(T::FILE))
    }

    pub fn write_for<T: StateDocument>(&self, folder: &Path, value: &T) {
        self.write_at(self.folder(folder).join(T::FILE), value);
    }

    /// Waits until every write asked for so far is on disk.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.writer.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }

    /// The files set aside since last asked, each said once.
    pub fn take_unsaid(&self) -> Vec<String> {
        std::mem::take(&mut *self.unsaid.lock().unwrap())
    }

    fn read_at<T: StateDocument>(&self, path: &Path) -> T {
        if let Some(value) = self.written.lock().unwrap().get(path)
            && let Ok(value) = serde_json::from_value(value.clone())
        {
            return value;
        }
        let Ok(bytes) = std::fs::read(path) else { return T::default() };
        match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                // Kept for the person to look at, never written over: the next write starts a fresh file.
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                self.unsaid
                    .lock()
                    .unwrap()
                    .push(format!("{} could not be read ({error}) and was set aside as .bad", path.display()));
                T::default()
            }
        }
    }

    fn write_at<T: StateDocument>(&self, path: PathBuf, value: &T) {
        if let Ok(value) = serde_json::to_value(value) {
            self.written.lock().unwrap().insert(path.clone(), value.clone());
            let _ = self.writer.send(Job::Write(path, value));
        }
    }
}

fn write_loop(jobs: Receiver<Job>) {
    let mut waiting: HashMap<PathBuf, (Value, Instant)> = HashMap::new();
    loop {
        let next_due = waiting.values().map(|(_, due)| *due).min();
        let job = match next_due {
            Some(due) => jobs.recv_timeout(due.saturating_duration_since(Instant::now())),
            None => jobs.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match job {
            Ok(Job::Write(path, value)) => {
                waiting.insert(path, (value, Instant::now() + WRITE_DELAY));
            }
            Ok(Job::Flush(done)) => {
                for (path, (value, _)) in waiting.drain() {
                    write_merged(&path, value);
                }
                let _ = done.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                let due: Vec<PathBuf> =
                    waiting.iter().filter(|(_, (_, at))| *at <= now).map(|(p, _)| p.clone()).collect();
                for path in due {
                    let (value, _) = waiting.remove(&path).expect("it was waiting");
                    write_merged(&path, value);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                for (path, (value, _)) in waiting.drain() {
                    write_merged(&path, value);
                }
                return;
            }
        }
    }
}

/// Writes `value` over what the file holds, key by key, so what a newer version or another window wrote stays.
fn write_merged(path: &Path, value: Value) {
    let merged = match (std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()), value) {
        (Some(Value::Object(mut old)), Value::Object(new)) => {
            old.extend(new);
            Value::Object(old)
        }
        (_, value) => value,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&merged) {
        let _ = crate::files::write(path, &bytes);
    }
}

/// FNV-1a: a stable hash, so a folder keeps its place across versions.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, b| (hash ^ *b as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
    #[serde(default, rename_all = "camelCase")]
    struct Recent {
        names: Vec<String>,
    }

    impl StateDocument for Recent {
        const FILE: &'static str = "recent.json";
    }

    fn store(name: &str) -> StateStore {
        let root = std::env::temp_dir().join(format!("rsharp-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        StateStore::open(root)
    }

    #[test]
    fn state_written_reads_back_and_keeps_what_a_newer_version_wrote() {
        let store = store("merge");
        std::fs::create_dir_all(store.root()).unwrap();
        std::fs::write(store.root().join("recent.json"), r#"{"names":["old"],"fromNewer":42}"#).unwrap();
        store.write(&Recent { names: vec!["a".into()] });
        store.write(&Recent { names: vec!["a".into(), "b".into()] });
        store.flush();
        assert_eq!(store.read::<Recent>(), Recent { names: vec!["a".into(), "b".into()] });
        let on_disk: Value = serde_json::from_slice(&std::fs::read(store.root().join("recent.json")).unwrap()).unwrap();
        assert_eq!(on_disk["fromNewer"], 42, "a key this version does not know was lost");
        std::fs::remove_dir_all(store.root()).unwrap();
    }

    #[test]
    fn a_file_that_does_not_read_is_set_aside_and_said_once() {
        let store = store("bad");
        let folder = Path::new("/some/where/App");
        std::fs::create_dir_all(store.folder(folder)).unwrap();
        std::fs::write(store.folder(folder).join("recent.json"), "{ not json").unwrap();
        assert_eq!(store.read_for::<Recent>(folder), Recent::default());
        assert!(store.folder(folder).join("recent.json.bad").exists(), "the unreadable file was not kept as .bad");
        assert_eq!(store.take_unsaid().len(), 1);
        assert!(store.take_unsaid().is_empty(), "a set-aside file was said twice");
        std::fs::remove_dir_all(store.root()).unwrap();
    }
}
