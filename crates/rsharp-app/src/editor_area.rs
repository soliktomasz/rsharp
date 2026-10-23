//! The editors, as Rider's: one group of tabs, or several split right and down, each with its own tabs, the one
//! worked in last the current group. A file open in two groups is two editors of one buffer. Pinned tabs come
//! first; closed tabs are remembered for Reopen Closed Tab.

use crate::editor::CodeEditor;
use gpui::{App, Entity};
use std::path::PathBuf;

pub struct Tab {
    pub editor: Entity<CodeEditor>,
    pub pinned: bool,
}

#[derive(Default)]
pub struct Group {
    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
}

impl Group {
    pub fn active_editor(&self) -> Option<&Entity<CodeEditor>> {
        self.active.and_then(|i| self.tabs.get(i)).map(|t| &t.editor)
    }
}

/// How the groups are laid out: one group, or groups side by side (`Row`) or one above another (`Column`).
#[derive(Clone, PartialEq, Debug)]
pub enum Split {
    Group(usize),
    Row(Vec<Split>),
    Column(Vec<Split>),
}

/// A closed tab, as Reopen Closed Tab brings it back.
pub struct Closed {
    pub path: PathBuf,
    pub caret: (usize, usize),
    pub pinned: bool,
}

/// How many closed tabs Reopen Closed Tab remembers; the oldest goes first.
const CLOSED_KEPT: usize = 20;

pub struct EditorArea {
    pub groups: Vec<Group>,
    pub layout: Split,
    pub current: usize,
    pub closed: Vec<Closed>,
}

impl Default for EditorArea {
    fn default() -> Self {
        Self { groups: vec![Group::default()], layout: Split::Group(0), current: 0, closed: Vec::new() }
    }
}

impl EditorArea {
    pub fn group(&self) -> &Group {
        &self.groups[self.current]
    }

    pub fn active_editor(&self) -> Option<&Entity<CodeEditor>> {
        self.group().active_editor()
    }

    pub fn editors(&self) -> impl Iterator<Item = &Entity<CodeEditor>> {
        self.groups.iter().flat_map(|g| g.tabs.iter().map(|t| &t.editor))
    }

    pub fn is_split(&self) -> bool {
        self.groups.len() > 1
    }

    /// The tab of a file in a group.
    pub fn tab_of(&self, group: usize, path: &PathBuf, cx: &App) -> Option<usize> {
        self.groups[group].tabs.iter().position(|t| t.editor.read(cx).path == *path)
    }

    /// A new tab in the current group, after its pinned ones if it is pinned, else at the end; its index.
    pub fn add(&mut self, editor: Entity<CodeEditor>, pinned: bool) -> usize {
        let tabs = &mut self.groups[self.current].tabs;
        let at = if pinned { tabs.iter().take_while(|t| t.pinned).count() } else { tabs.len() };
        tabs.insert(at, Tab { editor, pinned });
        at
    }

    /// Closes a group's tab, remembering it for Reopen Closed Tab; a split left with no tab closes with it, as
    /// Rider's does. Gives back the editor closed.
    pub fn close(&mut self, group: usize, index: usize, cx: &App) -> Option<Entity<CodeEditor>> {
        let tabs = &mut self.groups.get_mut(group)?.tabs;
        if index >= tabs.len() {
            return None;
        }
        let tab = tabs.remove(index);
        let editor = tab.editor.read(cx);
        let caret = editor.caret();
        self.closed.push(Closed {
            path: editor.path.clone(),
            caret: (caret.line, caret.utf16_column),
            pinned: tab.pinned,
        });
        if self.closed.len() > CLOSED_KEPT {
            self.closed.remove(0);
        }
        let g = &mut self.groups[group];
        g.active = match g.active {
            _ if g.tabs.is_empty() => None,
            Some(active) if active > index || active == g.tabs.len() => Some(active.saturating_sub(1)),
            other => other,
        };
        if g.tabs.is_empty() && self.groups.len() > 1 {
            self.remove_group(group);
        }
        Some(tab.editor)
    }

    /// The unpinned tabs of a group that `which` (given a tab's index) picks, from the last, so closing each in
    /// turn leaves the others' indices standing.
    pub fn unpinned(&self, group: usize, which: impl Fn(usize) -> bool) -> Vec<usize> {
        let tabs = &self.groups[group].tabs;
        (0..tabs.len()).rev().filter(|i| !tabs[*i].pinned && which(*i)).collect()
    }

    /// Pins or unpins a tab: a pinned one goes to the end of the pinned tabs, an unpinned one to the start of the rest.
    pub fn pin(&mut self, group: usize, index: usize, pinned: bool) {
        let g = &mut self.groups[group];
        let Some(mut tab) = (index < g.tabs.len()).then(|| g.tabs.remove(index)) else { return };
        let was_active = g.active == Some(index);
        tab.pinned = pinned;
        let at = g.tabs.iter().take_while(|t| t.pinned).count();
        g.tabs.insert(at, tab);
        if was_active {
            g.active = Some(at);
        } else if let Some(active) = g.active {
            // The tabs between the old place and the new one each moved one along.
            g.active = Some(match (index < active, at <= active) {
                (true, false) => active - 1,
                (false, true) => active + 1,
                _ => active,
            });
        }
    }

    /// Splits the current group, right (side by side) or down, the new group showing `editor` and becoming current.
    pub fn split(&mut self, right: bool, editor: Entity<CodeEditor>) {
        let new = self.groups.len();
        self.groups.push(Group { tabs: vec![Tab { editor, pinned: false }], active: Some(0) });
        let current = self.current;
        insert_beside(&mut self.layout, current, new, right);
        self.current = new;
    }

    /// Unsplit: the current group's tabs joined into the one beside it (all groups into one, for Unsplit All), the
    /// files the target shows already left where they are. Gives back the editors that went.
    pub fn unsplit(&mut self, all: bool, cx: &App) -> Vec<Entity<CodeEditor>> {
        let mut gone = Vec::new();
        while self.groups.len() > 1 {
            let from = self.current;
            let into = if from == 0 { 1 } else { from - 1 };
            let shown = self.groups[from].active_editor().map(|e| e.read(cx).path.clone());
            let tabs = std::mem::take(&mut self.groups[from].tabs);
            for tab in tabs {
                let path = tab.editor.read(cx).path.clone();
                if self.tab_of(into, &path, cx).is_some() {
                    gone.push(tab.editor);
                } else {
                    self.groups[into].tabs.push(tab);
                }
            }
            self.groups[into].active = shown.and_then(|p| self.tab_of(into, &p, cx)).or(self.groups[into].active);
            self.remove_group(from);
            self.current = if into > from { into - 1 } else { into };
            if !all {
                break;
            }
        }
        gone
    }

    fn remove_group(&mut self, group: usize) {
        self.groups.remove(group);
        self.layout = without(&self.layout, group).unwrap_or(Split::Group(0));
        if self.current > group || self.current >= self.groups.len() {
            self.current = self.current.saturating_sub(1);
        }
    }
}

/// Puts `new` beside `at` in the layout: in the same row (or column) if `at` is in one that goes the way asked,
/// else in a new one in its place.
fn insert_beside(layout: &mut Split, at: usize, new: usize, right: bool) {
    let wanted = |s: &Split| matches!((s, right), (Split::Row(_), true) | (Split::Column(_), false));
    match layout {
        Split::Group(g) if *g == at => {
            let pair = vec![Split::Group(at), Split::Group(new)];
            *layout = if right { Split::Row(pair) } else { Split::Column(pair) };
        }
        Split::Group(_) => {}
        Split::Row(children) | Split::Column(children) => {
            let direct = children.iter().position(|c| *c == Split::Group(at));
            if let (Some(i), true) = (direct, wanted(layout)) {
                let (Split::Row(children) | Split::Column(children)) = layout else { unreachable!() };
                children.insert(i + 1, Split::Group(new));
            } else {
                let (Split::Row(children) | Split::Column(children)) = layout else { unreachable!() };
                for child in children {
                    insert_beside(child, at, new, right);
                }
            }
        }
    }
}

/// The layout with a group gone and the later groups' indices one less; a row or column left with one child is
/// that child.
fn without(layout: &Split, group: usize) -> Option<Split> {
    match layout {
        Split::Group(g) if *g == group => None,
        Split::Group(g) => Some(Split::Group(if *g > group { g - 1 } else { *g })),
        Split::Row(children) | Split::Column(children) => {
            let mut kept: Vec<Split> = children.iter().filter_map(|c| without(c, group)).collect();
            match kept.len() {
                0 => None,
                1 => kept.pop(),
                _ => Some(if matches!(layout, Split::Row(_)) { Split::Row(kept) } else { Split::Column(kept) }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_goes_beside_its_group_and_a_group_gone_takes_its_place_with_it() {
        let mut layout = Split::Group(0);
        insert_beside(&mut layout, 0, 1, true);
        insert_beside(&mut layout, 1, 2, true);
        insert_beside(&mut layout, 2, 3, false);
        assert_eq!(
            layout,
            Split::Row(vec![Split::Group(0), Split::Group(1), Split::Column(vec![Split::Group(2), Split::Group(3)])]),
            "a split right of a group in a row did not join that row, or one down did not make a column"
        );
        assert_eq!(
            without(&layout, 2),
            Some(Split::Row(vec![Split::Group(0), Split::Group(1), Split::Group(2)])),
            "a column left with one group did not become that group, or the later groups kept their indices"
        );
    }
}
