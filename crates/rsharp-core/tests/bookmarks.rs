//! Bookmarks: toggled on lines, walked in order across files, following the text as it changes.

use rsharp_core::bookmarks::{Bookmark, Bookmarks};
use std::path::{Path, PathBuf};

fn at(path: &str, line: usize) -> Bookmark {
    Bookmark { path: PathBuf::from(path), line }
}

#[test]
fn toggling_adds_and_takes_away() {
    let mut marks = Bookmarks::default();
    assert!(marks.toggle(Path::new("/b.cs"), 4), "a new bookmark was not added");
    assert!(marks.toggle(Path::new("/b.cs"), 1));
    assert_eq!(marks.lines_in(Path::new("/b.cs")), [1, 4], "a file's lines are not in order");
    assert!(!marks.toggle(Path::new("/b.cs"), 4), "toggling a bookmark again did not take it away");
    assert_eq!(marks.lines_in(Path::new("/b.cs")), [1]);
}

#[test]
fn next_and_previous_wrap_across_files() {
    let mut marks = Bookmarks::default();
    for (path, line) in [("/b.cs", 2), ("/a.cs", 9), ("/b.cs", 7)] {
        marks.toggle(Path::new(path), line);
    }
    assert_eq!(marks.all(), [at("/a.cs", 9), at("/b.cs", 2), at("/b.cs", 7)], "bookmarks are not by file then line");
    assert_eq!(marks.next(Path::new("/b.cs"), 2), Some(at("/b.cs", 7)));
    assert_eq!(marks.next(Path::new("/b.cs"), 7), Some(at("/a.cs", 9)), "Next did not wrap to the first");
    assert_eq!(marks.next(Path::new("/a.cs"), 0), Some(at("/a.cs", 9)), "Next skipped one after the caret");
    assert_eq!(marks.previous(Path::new("/a.cs"), 9), Some(at("/b.cs", 7)), "Previous did not wrap to the last");
    assert_eq!(marks.previous(Path::new("/b.cs"), 5), Some(at("/b.cs", 2)));
    assert_eq!(Bookmarks::default().next(Path::new("/a.cs"), 0), None);
}

#[test]
fn following_an_edit_moves_drops_and_merges() {
    let mut marks = Bookmarks::default();
    for line in [1, 3, 5] {
        marks.toggle(Path::new("/a.cs"), line);
    }
    marks.toggle(Path::new("/other.cs"), 3);
    // Line 3 deleted whole; 5 joined onto 1's line; another file untouched.
    marks.follow(Path::new("/a.cs"), |line| match line {
        3 => None,
        5 => Some(1),
        line => Some(line),
    });
    assert_eq!(marks.lines_in(Path::new("/a.cs")), [1], "a deleted line's bookmark stayed, or a joined one doubled");
    assert_eq!(marks.lines_in(Path::new("/other.cs")), [3], "another file's bookmark moved");
}

#[test]
fn bookmarks_are_kept_as_json() {
    let mut marks = Bookmarks::default();
    marks.toggle(Path::new("/a.cs"), 3);
    let kept = serde_json::to_string(&marks).unwrap();
    let back: Bookmarks = serde_json::from_str(&kept).unwrap();
    assert_eq!(back.all(), marks.all(), "bookmarks did not survive being kept: {kept}");
}
