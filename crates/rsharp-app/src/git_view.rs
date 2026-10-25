//! The Commit and Git islands share repository snapshots; rendering never starts a Git process.

mod diff;
mod merge;

pub use diff::DiffView;
pub use merge::{MergeRequest, MergeView};

use crate::buffer::Buffer;
use crate::editor::CodeEditor;
use crate::field::TextField;
use crate::{assets::icon, theme};
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, div,
    prelude::*, px, uniform_list,
};
use rsharp_core::git::{ChangeKind, CommitInfo, FileChange, GitRef, Operation, RefKind, Status};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum GitRequest {
    Refresh,
    Commit { paths: Vec<PathBuf>, message: String, amend: bool, push: bool },
    Push,
    Update,
    Fetch,
    Checkout(String),
    NewBranch { name: String, from: Option<String> },
    RenameBranch { from: String, to: String },
    DeleteBranch(String),
    MergeBranch(String),
    Rebase(String),
    Diff(PathBuf),
    CommitDiff { path: PathBuf, revision: String },
    History(PathBuf),
    Compare { path: PathBuf, revision: String },
    ShowCommit(String),
    Resolve(PathBuf),
    Accept { path: PathBuf, yours: bool },
    Continue,
    Abort,
    Rollback(Vec<PathBuf>),
    Cancel,
}

pub struct GitView {
    focus: FocusHandle,
    status: Option<Status>,
    checked: BTreeSet<PathBuf>,
    selected: Option<PathBuf>,
    message: Entity<CodeEditor>,
    amend: bool,
    commit_mode: bool,
    busy: bool,
    problem: Option<String>,
    commits: Vec<CommitInfo>,
    refs: Vec<GitRef>,
    selected_commit: Option<String>,
    selected_ref: Option<String>,
    branch_name: Entity<TextField>,
    compare_revision: Entity<TextField>,
    commit_files: Vec<FileChange>,
    lanes: Vec<GraphRow>,
    default_log: (Vec<CommitInfo>, Vec<GitRef>),
    histories: Vec<(String, Vec<CommitInfo>, Vec<GitRef>)>,
    active_history: Option<usize>,
    favorites: BTreeSet<String>,
    /// The repository's root: a change is named by its path under it, as Rider names it.
    root: Option<PathBuf>,
}

impl EventEmitter<GitRequest> for GitView {}
impl Focusable for GitView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl GitView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let message = scratch_editor("COMMIT_EDITMSG", "", cx);
        let buffer = message.read(cx).buffer.clone();
        cx.observe(&buffer, |_, _, cx| cx.notify()).detach();
        Self {
            focus: cx.focus_handle(),
            status: None,
            checked: BTreeSet::new(),
            selected: None,
            message,
            amend: false,
            commit_mode: true,
            busy: false,
            problem: None,
            commits: Vec::new(),
            refs: Vec::new(),
            selected_commit: None,
            selected_ref: None,
            branch_name: cx.new(|cx| TextField::new("Branch name", cx)),
            compare_revision: cx.new(|cx| TextField::new("Branch, tag or revision", cx)),
            commit_files: Vec::new(),
            lanes: Vec::new(),
            default_log: (Vec::new(), Vec::new()),
            histories: Vec::new(),
            active_history: None,
            favorites: BTreeSet::new(),
            root: None,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root == root {
            return;
        }
        self.root = root;
        cx.notify();
    }

    pub fn set_mode(&mut self, commit: bool, cx: &mut Context<Self>) {
        self.commit_mode = commit;
        cx.notify();
    }
    pub fn set_status(&mut self, status: Option<Status>, cx: &mut Context<Self>) {
        let previous: HashSet<_> =
            self.status.as_ref().map(|s| s.changes.iter().map(|c| c.path.clone()).collect()).unwrap_or_default();
        if let Some(status) = &status {
            let paths: HashSet<_> = status.changes.iter().map(|c| c.path.clone()).collect();
            self.checked.retain(|p| paths.contains(p));
            for change in &status.changes {
                if !previous.contains(&change.path)
                    && !matches!(change.kind, ChangeKind::Unversioned | ChangeKind::Conflicted)
                {
                    self.checked.insert(change.path.clone());
                }
            }
            if self.commit_mode && self.selected.as_ref().is_none_or(|p| !paths.contains(p)) {
                self.selected = status.changes.first().map(|c| c.path.clone());
            }
        } else {
            self.checked.clear();
            self.selected = None;
            self.histories.clear();
            self.active_history = None;
            self.default_log = (Vec::new(), Vec::new());
            self.commits.clear();
            self.refs.clear();
            self.lanes.clear();
            self.selected_commit = None;
            self.selected_ref = None;
            self.commit_files.clear();
        }
        self.status = status;
        cx.notify();
    }
    pub fn set_log(&mut self, commits: Vec<CommitInfo>, refs: Vec<GitRef>, cx: &mut Context<Self>) {
        self.default_log = (commits.clone(), refs.clone());
        if self.active_history.is_some() {
            cx.notify();
            return;
        }
        self.apply_log(commits, refs, cx);
    }
    fn apply_log(&mut self, commits: Vec<CommitInfo>, mut refs: Vec<GitRef>, cx: &mut Context<Self>) {
        refs.sort_by_key(|r| (!self.favorites.contains(&r.full_name), r.name.clone()));
        self.lanes = graph_rows(&commits);
        self.commits = commits;
        self.refs = refs;
        if self.selected_commit.as_ref().is_some_and(|h| !self.commits.iter().any(|c| &c.hash == h)) {
            self.selected_commit = None;
            self.commit_files.clear();
        }
        cx.notify();
    }
    pub fn set_history(&mut self, title: String, commits: Vec<CommitInfo>, refs: Vec<GitRef>, cx: &mut Context<Self>) {
        let existing = self.histories.iter().position(|(t, _, _)| t == &title);
        let index = if let Some(index) = existing {
            self.histories[index] = (title, commits.clone(), refs.clone());
            index
        } else {
            if self.histories.len() == 12 {
                self.histories.remove(0);
            }
            self.histories.push((title, commits.clone(), refs.clone()));
            self.histories.len() - 1
        };
        self.active_history = Some(index);
        self.selected_ref = None;
        self.selected_commit = None;
        self.commit_files.clear();
        self.apply_log(commits, refs, cx);
    }
    pub fn selected_commit(&self) -> Option<&str> {
        self.selected_commit.as_deref()
    }
    pub fn refs(&self) -> &[GitRef] {
        &self.refs
    }
    pub fn favorites(&self) -> Vec<String> {
        self.favorites.iter().cloned().collect()
    }
    pub fn set_favorites(&mut self, favorites: Vec<String>, cx: &mut Context<Self>) {
        self.favorites = favorites.into_iter().collect();
        cx.notify();
    }
    pub fn show_log(&mut self, cx: &mut Context<Self>) {
        self.active_history = None;
        self.selected_ref = None;
        self.apply_log(self.default_log.0.clone(), self.default_log.1.clone(), cx);
    }
    fn history_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(28.))
            .flex_none()
            .flex()
            .gap(px(4.))
            .px(px(8.))
            .child(
                action("git log tab", "Log", true)
                    .when(self.active_history.is_none(), |d| d.bg(theme::tab_selected()))
                    .on_click(cx.listener(|v, _, _, cx| v.show_log(cx))),
            )
            .children(self.histories.iter().enumerate().map(|(index, (title, _, _))| {
                div()
                    .id(("git history tab", index))
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .truncate()
                    .when(self.active_history == Some(index), |d| d.bg(theme::tab_selected()))
                    .child(title.clone())
                    .on_click(cx.listener(move |v, _, _, cx| {
                        let (_, commits, refs) = v.histories[index].clone();
                        v.active_history = Some(index);
                        v.selected_ref = None;
                        v.apply_log(commits, refs, cx);
                    }))
            }))
    }
    pub fn set_commit_files(&mut self, files: Vec<FileChange>, cx: &mut Context<Self>) {
        if self.selected.as_ref().is_none_or(|p| !files.iter().any(|c| &c.path == p)) {
            self.selected = files.first().map(|c| c.path.clone());
        }
        self.commit_files = files;
        cx.notify();
    }
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        cx.notify();
    }
    pub fn set_problem(&mut self, problem: Option<String>, cx: &mut Context<Self>) {
        self.problem = problem;
        cx.notify();
    }
    pub fn message_editor(&self) -> &Entity<CodeEditor> {
        &self.message
    }
    pub fn message(&self, cx: &App) -> String {
        self.message.read(cx).text().to_string()
    }
    pub fn set_message(&mut self, text: &str, cx: &mut Context<Self>) {
        let buffer = self.message.read(cx).buffer.clone();
        buffer.update(cx, |b, cx| b.replace_all(text.to_string(), cx));
        cx.notify();
    }
    pub fn checked_paths(&self) -> Vec<PathBuf> {
        self.checked.iter().cloned().collect()
    }
    pub fn set_checked(&mut self, path: PathBuf, checked: bool, cx: &mut Context<Self>) {
        if checked {
            self.checked.insert(path);
        } else {
            self.checked.remove(&path);
        }
        cx.notify();
    }
    pub fn listed(&self) -> &[FileChange] {
        self.status.as_ref().map_or(&[], |s| s.changes.as_slice())
    }
    pub fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }
    pub fn comparison_field(&self) -> &Entity<TextField> {
        &self.compare_revision
    }

    fn compare_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.selected.is_some() && !self.busy;
        div()
            .h(px(32.))
            .flex_none()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(26.))
                    .px(px(6.))
                    .border_1()
                    .border_color(theme::control_border())
                    .child(self.compare_revision.clone()),
            )
            .child(action("git compare", "Compare With", enabled).on_click(cx.listener(|v, _, _, cx| {
                let revision = v.compare_revision.read(cx).text();
                if let Some(path) = v.selected.clone() {
                    if !revision.trim().is_empty() {
                        v.request(GitRequest::Compare { path, revision }, cx);
                    }
                }
            })))
    }

    fn request(&mut self, request: GitRequest, cx: &mut Context<Self>) {
        if !self.busy {
            cx.emit(request);
        }
    }
    fn commit(&mut self, push: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let message = self.message(cx);
        if message.trim().is_empty() {
            self.set_problem(Some("Enter a commit message.".into()), cx);
            return;
        }
        if self.listed().iter().any(|c| c.kind == ChangeKind::Conflicted) {
            self.set_problem(Some("Resolve merge conflicts before committing.".into()), cx);
            return;
        }
        if self.checked.is_empty() && !self.amend {
            self.set_problem(Some("Choose files to commit.".into()), cx);
            return;
        }
        cx.emit(GitRequest::Commit { paths: self.checked_paths(), message, amend: self.amend, push });
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = self.status.as_ref().and_then(|s| s.branch.clone()).unwrap_or_else(|| "Detached HEAD".into());
        let available = self.status.is_some() && !self.busy;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(5.))
            .child(icon("branch", 16.))
            .child(div().flex_1().truncate().child(if self.status.is_none() {
                "No Git repository".into()
            } else {
                branch
            }))
            .child(
                action("git refresh", "Refresh", !self.busy)
                    .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Refresh, cx))),
            )
            .when(!self.commit_mode, |d| {
                d.child(
                    action("git fetch", "Fetch", available)
                        .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Fetch, cx))),
                )
                .child(
                    action("git update", "Update", available)
                        .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Update, cx))),
                )
                .child(
                    action("git push", "Push", available)
                        .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Push, cx))),
                )
            })
            .when(self.busy, |d| {
                d.child(
                    action("git cancel", "Cancel", true)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(GitRequest::Cancel))),
                )
            })
    }

    fn change_row(&self, change: &FileChange, index: usize, check: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let path = change.path.clone();
        let chosen = self.checked.contains(&path);
        let conflict = change.kind == ChangeKind::Conflicted;
        let selected = self.selected.as_ref() == Some(&path);
        let checkbox_path = path.clone();
        let selector = path.display().to_string();
        let shown = |path: &std::path::Path| {
            path.strip_prefix(self.root.as_deref().unwrap_or(std::path::Path::new("")))
                .unwrap_or(path)
                .display()
                .to_string()
        };
        let title = match &change.from {
            Some(from) => format!("{} → {}", shown(from), shown(&path)),
            None => shown(&path),
        };
        div()
            .id(("git file", index))
            .debug_selector(move || format!("git file: {selector}"))
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when(selected, |d| d.bg(theme::selection()))
            .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
            .when(check && !conflict, |d| {
                d.child(
                    div()
                        .id(("git checked", index))
                        .debug_selector({
                            let p = path.display().to_string();
                            move || format!("git checked: {p}")
                        })
                        .size(px(16.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(theme::control_border())
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(chosen, |d| d.bg(theme::accent()).child(icon("check", 14.)))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            cx.stop_propagation();
                            if !v.busy {
                                v.set_checked(checkbox_path.clone(), !chosen, cx);
                            }
                        })),
                )
            })
            .child(div().flex_1().truncate().child(title))
            .child(
                div()
                    .text_color(if conflict { theme::error() } else { theme::text_muted() })
                    .child(change_label(change)),
            )
            .on_click(cx.listener(move |v, e: &ClickEvent, window, cx| {
                window.focus(&v.focus);
                v.selected = Some(path.clone());
                if e.click_count() == 2 {
                    let request = if conflict {
                        GitRequest::Resolve(path.clone())
                    } else if !check {
                        GitRequest::CommitDiff {
                            path: path.clone(),
                            revision: v.selected_commit.clone().unwrap_or_default(),
                        }
                    } else {
                        GitRequest::Diff(path.clone())
                    };
                    v.request(request, cx);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn commit_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.listed().len();
        let can = !self.busy && self.status.is_some();
        let selected = self.selected.clone();
        let conflict = selected
            .as_ref()
            .is_some_and(|p| self.listed().iter().any(|c| &c.path == p && c.kind == ChangeKind::Conflicted));
        let ongoing = self.status.as_ref().is_some_and(|s| s.operation != Operation::Idle);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                uniform_list(
                    "git changes",
                    count,
                    cx.processor(|v, range: std::ops::Range<usize>, _, cx| {
                        range.map(|i| v.change_row(&v.listed()[i], i, true, cx)).collect()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .when(count == 0, |d| {
                d.child(div().p(px(12.)).text_color(theme::text_muted()).child(if self.status.is_some() {
                    "No changes"
                } else {
                    "Open a folder containing a Git repository."
                }))
            })
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .child(
                        action("git diff", if conflict { "Merge…" } else { "Show Diff" }, can && selected.is_some())
                            .on_click(cx.listener(move |v, _, _, cx| {
                                if let Some(p) = v.selected.clone() {
                                    v.request(if conflict { GitRequest::Resolve(p) } else { GitRequest::Diff(p) }, cx);
                                }
                            })),
                    )
                    .child(
                        action("git rollback", "Rollback…", can && !self.checked.is_empty()).on_click(
                            cx.listener(|v, _, _, cx| v.request(GitRequest::Rollback(v.checked_paths()), cx)),
                        ),
                    )
                    .child(action("git history", "History", can && self.selected.is_some()).on_click(cx.listener(
                        |v, _, _, cx| {
                            if let Some(path) = v.selected.clone() {
                                v.request(GitRequest::History(path), cx);
                            }
                        },
                    )))
                    .when(conflict, |d| {
                        d.child(action("git yours", "Accept Yours", can).on_click(cx.listener(|v, _, _, cx| {
                            if let Some(path) = v.selected.clone() {
                                v.request(GitRequest::Accept { path, yours: true }, cx);
                            }
                        })))
                        .child(action("git theirs", "Accept Theirs", can).on_click(
                            cx.listener(|v, _, _, cx| {
                                if let Some(path) = v.selected.clone() {
                                    v.request(GitRequest::Accept { path, yours: false }, cx);
                                }
                            }),
                        ))
                    })
                    .when(ongoing, |d| {
                        d.child(
                            action(
                                "git continue",
                                "Continue",
                                can && !self.listed().iter().any(|c| c.kind == ChangeKind::Conflicted),
                            )
                            .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Continue, cx))),
                        )
                        .child(
                            action("git abort", "Abort…", can)
                                .on_click(cx.listener(|v, _, _, cx| v.request(GitRequest::Abort, cx))),
                        )
                    }),
            )
            .child(self.compare_controls(cx))
            .child(
                div()
                    .h(px(100.))
                    .min_h(px(70.))
                    .mx(px(8.))
                    .border_1()
                    .border_color(theme::control_border())
                    .debug_selector(|| "git message".into())
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .p(px(8.))
                    .child(action("git amend", if self.amend { "Amend: on" } else { "Amend" }, can).on_click(
                        cx.listener(|v, _, _, cx| {
                            if !v.busy {
                                v.amend = !v.amend;
                                cx.notify();
                            }
                        }),
                    ))
                    .child(div().flex_1())
                    .child(action("git commit", "Commit", can).on_click(cx.listener(|v, _, _, cx| v.commit(false, cx))))
                    .child(
                        action("git commit push", "Commit and Push", can)
                            .on_click(cx.listener(|v, _, _, cx| v.commit(true, cx))),
                    ),
            )
    }

    fn log_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_ref = self.selected_ref.clone();
        let selected_hashes: Option<HashSet<String>> =
            selected_ref.as_ref().and_then(|name| self.refs.iter().find(|r| &r.full_name == name)).map(|reference| {
                let mut pending = vec![reference.hash.clone()];
                let mut found = HashSet::new();
                while let Some(hash) = pending.pop() {
                    if found.insert(hash.clone()) {
                        if let Some(commit) = self.commits.iter().find(|c| c.hash == hash) {
                            pending.extend(commit.parents.iter().cloned());
                        }
                    }
                }
                found
            });
        let shown: Vec<usize> = self
            .commits
            .iter()
            .enumerate()
            .filter(|(_, c)| selected_hashes.as_ref().is_none_or(|h| h.contains(&c.hash)))
            .map(|(i, _)| i)
            .collect();
        let can = !self.busy && self.status.is_some();
        let local = selected_ref.as_ref().is_some_and(|name| {
            self.refs.iter().any(|r| &r.full_name == name && r.kind == RefKind::Local && !r.current)
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .w(px(210.))
                    .min_w(px(150.))
                    .flex_none()
                    .border_r_1()
                    .border_color(theme::separator())
                    .flex()
                    .flex_col()
                    .child(action("git all branches", "All branches", true).on_click(cx.listener(|v, _, _, cx| {
                        v.selected_ref = None;
                        cx.notify();
                    })))
                    .child(
                        uniform_list(
                            "git refs",
                            self.refs.len(),
                            cx.processor(|v, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|i| {
                                        let reference = &v.refs[i];
                                        let name = reference.full_name.clone();
                                        div()
                                            .id(("git ref", i))
                                            .debug_selector({
                                                let n = name.clone();
                                                move || format!("git ref: {n}")
                                            })
                                            .h(px(25.))
                                            .px(px(8.))
                                            .flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .when(v.selected_ref.as_ref() == Some(&name), |d| d.bg(theme::selection()))
                                            .when(reference.current, |d| d.child(icon("check", 14.)))
                                            .when(v.favorites.contains(&name), |d| d.child(icon("pin", 14.)))
                                            .child(div().truncate().child(reference.name.clone()))
                                            .on_click(cx.listener(move |v, _, _, cx| {
                                                v.selected_ref = Some(name.clone());
                                                cx.notify();
                                            }))
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .flex_1()
                        .min_h_0(),
                    )
                    // The branch controls scroll rather than drawing past a short island, as Rider's panel does.
                    .child(
                        div()
                            .id("git branch controls")
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .h(px(28.))
                                    .mx(px(6.))
                                    .border_1()
                                    .border_color(theme::control_border())
                                    .child(self.branch_name.clone()),
                            )
                            .child(action("git new branch", "New Branch", can).on_click(cx.listener(|v, _, _, cx| {
                                let name = v.branch_name.read(cx).text();
                                if !name.trim().is_empty() {
                                    v.request(
                                        GitRequest::NewBranch {
                                            name,
                                            from: v.selected_commit.clone().or_else(|| v.selected_ref.clone()),
                                        },
                                        cx,
                                    );
                                }
                            })))
                            .child(action("git checkout", "Checkout", can && selected_ref.is_some()).on_click(
                                cx.listener(|v, _, _, cx| {
                                    if let Some(name) = v.selected_ref.clone() {
                                        v.request(GitRequest::Checkout(name), cx);
                                    }
                                }),
                            ))
                            .child(
                                action("git merge branch", "Merge into Current", can && selected_ref.is_some())
                                    .on_click(cx.listener(|v, _, _, cx| {
                                        if let Some(name) = v.selected_ref.clone() {
                                            v.request(GitRequest::MergeBranch(name), cx);
                                        }
                                    })),
                            )
                            .child(action("git rebase", "Rebase onto Branch", can && selected_ref.is_some()).on_click(
                                cx.listener(|v, _, _, cx| {
                                    if let Some(name) = v.selected_ref.clone() {
                                        v.request(GitRequest::Rebase(name), cx);
                                    }
                                }),
                            ))
                            .child(action("git rename branch", "Rename Branch", can && local).on_click(cx.listener(
                                |v, _, _, cx| {
                                    let to = v.branch_name.read(cx).text();
                                    if !to.trim().is_empty() {
                                        if let Some(from) = v.selected_ref.clone() {
                                            v.request(GitRequest::RenameBranch { from, to }, cx);
                                        }
                                    }
                                },
                            )))
                            .child(action("git delete branch", "Delete Branch…", can && local).on_click(cx.listener(
                                |v, _, _, cx| {
                                    if let Some(name) = v.selected_ref.clone() {
                                        v.request(GitRequest::DeleteBranch(name), cx);
                                    }
                                },
                            )))
                            .child(
                                action(
                                    "git favorite branch",
                                    if selected_ref.as_ref().is_some_and(|r| self.favorites.contains(r)) {
                                        "Remove Favorite"
                                    } else {
                                        "Favorite Branch"
                                    },
                                    selected_ref.is_some(),
                                )
                                .on_click(cx.listener(|v, _, _, cx| {
                                    if let Some(name) = v.selected_ref.clone() {
                                        if !v.favorites.remove(&name) {
                                            v.favorites.insert(name);
                                        }
                                        cx.notify();
                                    }
                                })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        uniform_list(
                            "git log",
                            shown.len(),
                            cx.processor(move |v, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|row| {
                                        let i = shown[row];
                                        let commit = &v.commits[i];
                                        let hash = commit.hash.clone();
                                        div()
                                            .id(("git commit row", i))
                                            .debug_selector({
                                                let h = hash.clone();
                                                move || format!("git commit: {h}")
                                            })
                                            .h(px(28.))
                                            .px(px(8.))
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .when(v.selected_commit.as_ref() == Some(&hash), |d| {
                                                d.bg(theme::selection())
                                            })
                                            .hover(|d| d.bg(theme::row_hover()))
                                            .child(graph(&v.lanes[i]))
                                            .child(
                                                div()
                                                    .w(px(65.))
                                                    .font_family(theme::CODE_FONT)
                                                    .text_color(theme::text_muted())
                                                    .child(commit.hash.chars().take(8).collect::<String>()),
                                            )
                                            .child(div().flex_1().truncate().child(format!(
                                                "{}{}",
                                                if commit.refs.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!("[{}] ", commit.refs.join(", "))
                                                },
                                                commit.subject
                                            )))
                                            .child(
                                                div()
                                                    .w(px(110.))
                                                    .truncate()
                                                    .text_color(theme::text_muted())
                                                    .child(commit.author.clone()),
                                            )
                                            .child(
                                                div()
                                                    .w(px(80.))
                                                    .text_color(theme::text_muted())
                                                    .child(date(commit.time)),
                                            )
                                            .on_click(cx.listener(move |v, _, _, cx| {
                                                v.selected_commit = Some(hash.clone());
                                                v.commit_files.clear();
                                                v.request(GitRequest::ShowCommit(hash.clone()), cx);
                                                cx.notify();
                                            }))
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .flex_1()
                        .min_h_0(),
                    )
                    .when(self.commits.is_empty(), |d| {
                        d.child(div().p(px(12.)).text_color(theme::text_muted()).child("No commits to show"))
                    })
                    .when_some(
                        self.selected_commit.as_ref().and_then(|h| self.commits.iter().find(|c| &c.hash == h)),
                        |d, c| {
                            d.child(div().p(px(8.)).border_t_1().border_color(theme::separator()).child(format!(
                                "{}\n{} <{}>  {}\n{}",
                                c.hash,
                                c.author,
                                c.email,
                                date(c.time),
                                c.subject
                            )))
                        },
                    )
                    .child(self.compare_controls(cx))
                    .child(
                        uniform_list(
                            "git commit files",
                            self.commit_files.len(),
                            cx.processor(|v, range: std::ops::Range<usize>, _, cx| {
                                range.map(|i| v.change_row(&v.commit_files[i], i, false, cx)).collect()
                            }),
                        )
                        .h(px(if self.commit_files.is_empty() { 0. } else { 110. })),
                    ),
            )
    }
}

impl Render for GitView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Git")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(theme::text())
            .text_size(px(13.))
            .font_family(theme::UI_FONT)
            .on_key_down(cx.listener(|v, event: &gpui::KeyDownEvent, window, cx| {
                if !v.focus.is_focused(window) {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "up" | "down" if v.commit_mode => {
                        let at =
                            v.selected.as_ref().and_then(|p| v.listed().iter().position(|c| &c.path == p)).unwrap_or(0);
                        let next = if event.keystroke.key == "up" {
                            at.saturating_sub(1)
                        } else {
                            (at + 1).min(v.listed().len().saturating_sub(1))
                        };
                        v.selected = v.listed().get(next).map(|c| c.path.clone());
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "space" if v.commit_mode && !v.busy => {
                        if let Some(path) = v.selected.clone() {
                            if v.listed().iter().any(|c| c.path == path && c.kind != ChangeKind::Conflicted) {
                                let checked = !v.checked.contains(&path);
                                v.set_checked(path, checked, cx);
                            }
                        }
                        cx.stop_propagation();
                    }
                    "enter" if v.commit_mode => {
                        if let Some(path) = v.selected.clone() {
                            let conflict =
                                v.listed().iter().any(|c| c.path == path && c.kind == ChangeKind::Conflicted);
                            v.request(if conflict { GitRequest::Resolve(path) } else { GitRequest::Diff(path) }, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(self.toolbar(cx))
            .when(!self.commit_mode, |d| d.child(self.history_tabs(cx)))
            .child(if self.commit_mode {
                self.commit_body(cx).into_any_element()
            } else {
                self.log_body(cx).into_any_element()
            })
            .when_some(self.problem.clone(), |d, p| {
                d.child(
                    div()
                        .px(px(8.))
                        .py(px(5.))
                        .text_color(theme::error())
                        .debug_selector(|| "git problem".into())
                        .child(p),
                )
            })
    }
}

#[derive(gpui::IntoElement)]
struct ActionButton {
    element: gpui::Stateful<gpui::Div>,
    enabled: bool,
}
impl ActionButton {
    fn on_click(mut self, listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        if self.enabled {
            self.element = self.element.on_click(listener);
        }
        self
    }
    fn bg(mut self, color: gpui::Rgba) -> Self {
        self.element = self.element.bg(color);
        self
    }
}
impl gpui::RenderOnce for ActionButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.element
    }
}
fn action(id: &'static str, title: impl Into<SharedString>, enabled: bool) -> ActionButton {
    let element = div()
        .id(id)
        .debug_selector(move || id.into())
        .h(px(26.))
        .px(px(8.))
        .flex_none()
        .flex()
        .items_center()
        .rounded(px(4.))
        .when(enabled, |d| d.hover(|d| d.bg(theme::button_hover())).active(|d| d.bg(theme::button_pressed())))
        .when(!enabled, |d| d.text_color(theme::text_secondary()))
        .child(title.into());
    ActionButton { element, enabled }
}

fn scratch_editor<T: 'static>(name: &str, text: &str, cx: &mut Context<T>) -> Entity<CodeEditor> {
    // No scratch buffer enters the process's open-file registry, so it cannot shadow a real file.
    let path = std::env::temp_dir().join(format!("rsharp-scratch-{}-{}-{name}", std::process::id(), cx.entity_id()));
    let buffer = cx.new(|cx| Buffer::new(path, text.as_bytes(), cx));
    cx.new(|cx| CodeEditor::new(buffer, cx))
}

fn change_label(change: &FileChange) -> String {
    let kind = match change.kind {
        ChangeKind::Modified => "Modified",
        ChangeKind::Added => "Added",
        ChangeKind::Deleted => "Deleted",
        ChangeKind::Renamed => "Renamed",
        ChangeKind::Unversioned => "Unversioned",
        ChangeKind::Conflicted => "Conflict",
    };
    // An unversioned file has no index entry to stage, and a conflict has three.
    let location = match change.kind {
        ChangeKind::Unversioned | ChangeKind::Conflicted => "",
        _ => match (change.index != '.' && change.index != ' ', change.worktree != '.' && change.worktree != ' ') {
            (true, true) => " · staged + unstaged",
            (true, false) => " · staged",
            _ => "",
        },
    };
    format!("{kind}{location}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GraphRow {
    lane: usize,
    lanes: usize,
    parents: Vec<usize>,
}
fn graph_rows(commits: &[CommitInfo]) -> Vec<GraphRow> {
    let mut lanes = Vec::<String>::new();
    commits
        .iter()
        .map(|commit| {
            let lane = lanes.iter().position(|h| h == &commit.hash).unwrap_or_else(|| {
                lanes.push(commit.hash.clone());
                lanes.len() - 1
            });
            let width = lanes.len();
            lanes.remove(lane);
            let mut parents = Vec::new();
            for (i, parent) in commit.parents.iter().enumerate() {
                let position = lanes.iter().position(|h| h == parent).unwrap_or_else(|| {
                    let at = (lane + i).min(lanes.len());
                    lanes.insert(at, parent.clone());
                    at
                });
                parents.push(position);
            }
            GraphRow { lane, lanes: width.max(lanes.len()), parents }
        })
        .collect()
}
fn graph(row: &GraphRow) -> impl IntoElement {
    let row = row.clone();
    let width = (row.lanes.max(1).min(10) * 12) as f32;
    gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let x = |lane: usize| bounds.origin.x + px((lane.min(9) * 12 + 6) as f32);
            let center = gpui::point(x(row.lane), bounds.origin.y + px(14.));
            let mut path = gpui::PathBuilder::stroke(px(1.5));
            for lane in 0..row.lanes.min(10) {
                path.move_to(gpui::point(x(lane), bounds.origin.y));
                path.line_to(if lane == row.lane { center } else { gpui::point(x(lane), bounds.bottom()) });
            }
            for parent in &row.parents {
                path.move_to(center);
                path.line_to(gpui::point(x(*parent), bounds.bottom()));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, theme::modified());
            }
            window.paint_quad(gpui::fill(
                gpui::Bounds::new(center - gpui::point(px(3.5), px(3.5)), gpui::size(px(7.), px(7.))),
                theme::accent(),
            ));
        },
    )
    .w(px(width))
    .h(px(28.))
}
fn date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commit(hash: &str, parents: &[&str]) -> CommitInfo {
        CommitInfo {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: Vec::new(),
            subject: String::new(),
        }
    }
    #[test]
    fn graph_reuses_a_fork_lane_when_its_two_parents_join() {
        let rows = graph_rows(&[
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &[]),
        ]);
        assert_eq!(rows[0].parents, [0, 1]);
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[2].parents, [0]);
        assert_eq!(rows[3].lanes, 1);
    }
    #[test]
    fn git_dates_are_calendar_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_709_164_800), "2024-02-29");
    }
}
