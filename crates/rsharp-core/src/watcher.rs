//! Watches a folder for what other programs change in it (a terminal's `git pull`, an agent's edits, `dotnet new`), as
//! Rider's fsnotifier does, through the `notify` crate. Changes are gathered for a moment and handed over together.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a burst of changes is gathered before it is handed over (Passenger's `Gather`).
const GATHER: Duration = Duration::from_millis(150);

/// The longest a burst is gathered, so writes that never pause (a build's log, a long checkout) still reach the window.
const LONGEST: Duration = Duration::from_secs(1);

/// Folders whose changes say nothing a person sees: build output, dependencies, and Git's own store.
const PASSED: &[&str] = &["bin", "obj", "node_modules", ".git", ".vs", ".idea"];

/// Watches until dropped.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
}

/// Whether a change under `folder` is one to hand over: not build output or Git's objects, but Git's HEAD, index and
/// refs, which say a commit or checkout was made elsewhere.
fn counts(folder: &Path, path: &Path) -> bool {
    let Ok(inside) = path.strip_prefix(folder) else { return false };
    let parts: Vec<String> = inside
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match parts.iter().position(|p| PASSED.contains(&p.as_str())) {
        None => true,
        Some(at) if parts[at] == ".git" => {
            matches!(parts.get(at + 1).map(String::as_str), Some("HEAD" | "index" | "refs"))
        }
        Some(_) => false,
    }
}

/// Watches `folder` recursively, and calls `changed` on a thread of its own with the paths each burst changed.
pub fn watch(folder: &Path, changed: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Watcher> {
    use notify::Watcher as _;
    let folder = crate::paths::canonical(folder);
    let (sender, events) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(sender)?;
    watcher.watch(&folder, notify::RecursiveMode::Recursive)?;
    std::thread::Builder::new()
        .name("folder watcher".into())
        .spawn(move || {
            // Ends as the watcher is dropped, which drops the sender.
            while let Ok(first) = events.recv() {
                let mut paths: Vec<PathBuf> = Vec::new();
                let mut seen: HashSet<PathBuf> = HashSet::new();
                let mut take = |event: notify::Result<notify::Event>| {
                    for path in event.map(|e| e.paths).unwrap_or_default() {
                        if counts(&folder, &path) && seen.insert(path.clone()) {
                            paths.push(path);
                        }
                    }
                };
                take(first);
                let started = Instant::now();
                while started.elapsed() < LONGEST
                    && let Ok(more) = events.recv_timeout(GATHER.min(LONGEST.saturating_sub(started.elapsed())))
                {
                    take(more);
                }
                if !paths.is_empty() {
                    changed(paths);
                }
            }
        })
        .map_err(|e| notify::Error::generic(&e.to_string()))?;
    Ok(Watcher { _watcher: watcher })
}

/// Watches repository metadata without subscribing to its object database. Linked worktrees also call this for
/// their common directory, which may be outside the workspace the person opened.
pub fn watch_git(folder: &Path, changed: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Watcher> {
    use notify::Watcher as _;
    let (sender, events) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(sender)?;
    watcher.watch(folder, notify::RecursiveMode::NonRecursive)?;
    for name in ["refs", "rebase-merge", "rebase-apply", "sequencer"] {
        let path = folder.join(name);
        if path.is_dir() {
            watcher.watch(&path, notify::RecursiveMode::Recursive)?;
        }
    }
    std::thread::Builder::new()
        .name("git watcher".into())
        .spawn(move || {
            while let Ok(first) = events.recv() {
                let mut paths = first.map(|e| e.paths).unwrap_or_default();
                let started = Instant::now();
                while started.elapsed() < LONGEST {
                    match events.recv_timeout(GATHER) {
                        Ok(event) => paths.extend(event.map(|e| e.paths).unwrap_or_default()),
                        Err(_) => break,
                    }
                }
                paths.retain(|p| {
                    !p.components().any(|c| c.as_os_str() == "objects") && p.extension().is_none_or(|e| e != "lock")
                });
                paths.sort();
                paths.dedup();
                if !paths.is_empty() {
                    changed(paths);
                }
            }
        })
        .map_err(|e| notify::Error::generic(&e.to_string()))?;
    Ok(Watcher { _watcher: watcher })
}
