//! Bookmarks, as Passenger's BookmarkStore keeps them: a folder's lines, by file then line, following each file's text.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bookmark {
    pub path: PathBuf,
    /// From 0.
    pub line: usize,
}

/// A folder's bookmarks, kept sorted by file then line, one per line.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
pub struct Bookmarks(Vec<Bookmark>);

impl Bookmarks {
    /// Adds a bookmark on the line, or takes the one there away; true when one was added.
    pub fn toggle(&mut self, path: &Path, line: usize) -> bool {
        let mark = Bookmark { path: path.to_path_buf(), line };
        match self.0.binary_search(&mark) {
            Ok(at) => {
                self.0.remove(at);
                false
            }
            Err(at) => {
                self.0.insert(at, mark);
                true
            }
        }
    }

    pub fn lines_in(&self, path: &Path) -> Vec<usize> {
        self.0.iter().filter(|m| m.path == path).map(|m| m.line).collect()
    }

    pub fn all(&self) -> &[Bookmark] {
        &self.0
    }

    /// The first bookmark after a place, wrapping to the first of all.
    pub fn next(&self, path: &Path, line: usize) -> Option<Bookmark> {
        let here = (path, line);
        self.0.iter().find(|m| (m.path.as_path(), m.line) > here).or(self.0.first()).cloned()
    }

    /// The last bookmark before a place, wrapping to the last of all.
    pub fn previous(&self, path: &Path, line: usize) -> Option<Bookmark> {
        let here = (path, line);
        self.0.iter().rev().find(|m| (m.path.as_path(), m.line) < here).or(self.0.last()).cloned()
    }

    /// A file's bookmarks carried through an edit by `map`, which gives none for a line the edit removed.
    pub fn follow(&mut self, path: &Path, map: impl Fn(usize) -> Option<usize>) {
        self.0.retain_mut(|m| {
            if m.path != path {
                return true;
            }
            match map(m.line) {
                Some(line) => {
                    m.line = line;
                    true
                }
                None => false,
            }
        });
        self.0.sort();
        self.0.dedup();
    }
}
