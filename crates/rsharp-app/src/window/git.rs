//! The workspace's Git operations, crossing to workers before running Git or comparing revisions.
use super::*;
use crate::git_view::{DiffView, GitRequest, GitView, MergeRequest, MergeView};
use rsharp_core::git::{Operation, Repository, Side, Stages, Status};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct GitChoices {
    draft: String,
    favorites: Vec<String>,
}
impl rsharp_core::state::StateDocument for GitChoices {
    const FILE: &'static str = "git.json";
}

pub(super) struct GitState {
    pub commit: Entity<GitView>,
    pub log: Entity<GitView>,
    repository: Option<Repository>,
    status: Option<Status>,
    busy: bool,
    pending: bool,
    cancel: Option<Arc<AtomicBool>>,
    epoch: u64,
    task: Option<Task<()>>,
    metadata: Vec<rsharp_core::watcher::Watcher>,
    watching: Option<Task<()>>,
    baseline: HashMap<PathBuf, (Option<String>, Option<Arc<str>>)>,
    baseline_reads: HashSet<(PathBuf, Option<String>)>,
    pub merge: Option<gpui::WindowHandle<MergeView>>,
}

impl GitState {
    pub fn new(cx: &mut Context<MainWindow>) -> Self {
        let commit = cx.new(GitView::new);
        let log = cx.new(GitView::new);
        log.update(cx, |v, cx| v.set_mode(false, cx));
        Self {
            commit,
            log,
            repository: None,
            status: None,
            busy: false,
            pending: false,
            cancel: None,
            epoch: 0,
            task: None,
            metadata: Vec::new(),
            watching: None,
            baseline: HashMap::new(),
            baseline_reads: HashSet::new(),
            merge: None,
        }
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > 64 * 1024 * 1024 => {
            Err("The file exceeds the 64 MiB Git preview limit".into())
        }
        Ok(_) => std::fs::read(path).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn text(bytes: &[u8], charset: Option<rsharp_core::encodings::Charset>) -> Result<String, String> {
    let (text, _) = rsharp_core::encodings::decode(bytes, charset);
    if text.contains('\0') {
        return Err("This is a binary file; a text comparison is unavailable".into());
    }
    Ok(rsharp_text::hold_newlines(text))
}

impl MainWindow {
    pub fn git_status(&self) -> Option<&Status> {
        self.git.status.as_ref()
    }
    pub fn git_busy(&self) -> bool {
        self.git.busy
    }
    pub fn git_commit_view(&self) -> &Entity<GitView> {
        &self.git.commit
    }
    pub fn git_log_view(&self) -> &Entity<GitView> {
        &self.git.log
    }
    pub fn git_merge_window(&self) -> Option<gpui::WindowHandle<MergeView>> {
        self.git.merge
    }

    pub(super) fn leave_git(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.git.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(repo) = self.git.repository.take() {
            repo.cancel();
        }
        self.git.epoch += 1;
        self.git.task = None;
        self.git.watching = None;
        self.git.metadata.clear();
        self.git.baseline.clear();
        self.git.baseline_reads.clear();
        self.git.status = None;
        self.git.pending = false;
        self.git.busy = false;
        for view in [&self.git.commit, &self.git.log] {
            view.update(cx, |v, cx| {
                v.set_status(None, cx);
                v.set_root(None, cx);
                v.set_busy(false, cx);
            });
        }
    }

    fn git_set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        for view in [&self.git.commit, &self.git.log] {
            view.update(cx, |v, cx| v.set_root(root.clone(), cx));
        }
    }

    pub(super) fn open_git(&mut self, cx: &mut Context<Self>) {
        let Some(folder) = self.workspace.as_ref().map(|w| w.folder.clone()) else { return };
        let choices: GitChoices = store(cx).read_for(&folder);
        self.git.commit.update(cx, |v, cx| v.set_message(&choices.draft, cx));
        self.git.log.update(cx, |v, cx| v.set_favorites(choices.favorites, cx));
        let epoch = self.git.epoch;
        self.git_busy_set(true, cx);
        let read = cx.background_executor().spawn(async move {
            let repo = Repository::open(&folder)?;
            let status = repo.as_ref().map(Repository::status).transpose()?;
            Ok::<_, String>((repo, status))
        });
        self.git.task = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                m.git_busy_set(false, cx);
                match result {
                    Ok((repo, status)) => {
                        let root = repo.as_ref().map(|r| r.root.clone());
                        m.git.repository = repo;
                        m.git_set_root(root, cx);
                        m.git_set_status(status, cx);
                        m.watch_git_metadata(cx);
                        m.attach_git_buffers(cx);
                        if m.dock.is_open(Tool::Git) {
                            m.refresh_git_log(cx);
                        }
                    }
                    Err(error) => m.git_problem(Some(error), cx),
                }
            });
        }));
    }

    fn git_busy_set(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.git.busy = busy;
        for view in [&self.git.commit, &self.git.log] {
            view.update(cx, |v, cx| v.set_busy(busy, cx));
        }
        cx.notify();
    }

    fn git_problem(&mut self, problem: Option<String>, cx: &mut Context<Self>) {
        if let Some(error) = &problem {
            self.said = Some(error.clone().into());
        }
        for view in [&self.git.commit, &self.git.log] {
            view.update(cx, |v, cx| v.set_problem(problem.clone(), cx));
        }
        cx.notify();
    }

    fn git_set_status(&mut self, status: Option<Status>, cx: &mut Context<Self>) {
        self.branch = status.as_ref().map(|s| {
            s.branch.clone().unwrap_or_else(|| {
                s.head.as_deref().map(|h| format!("Detached {}", &h[..h.len().min(8)])).unwrap_or("No commits".into())
            })
        });
        for view in [&self.git.commit, &self.git.log] {
            view.update(cx, |v, cx| v.set_status(status.clone(), cx));
        }
        self.git.status = status;
        cx.notify();
    }

    pub fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.git.busy {
            self.git.pending = true;
            return;
        }
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else {
            self.open_git(cx);
            return;
        };
        self.git_busy_set(true, cx);
        let epoch = self.git.epoch;
        let read = cx.background_executor().spawn(async move { repo.status() });
        self.git.task = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                let pending = std::mem::take(&mut m.git.pending);
                m.git_busy_set(false, cx);
                match result {
                    Ok(status) => {
                        m.git_set_status(Some(status), cx);
                        m.attach_git_buffers(cx);
                    }
                    Err(error) => m.git_problem(Some(error), cx),
                }
                if pending {
                    m.refresh_git(cx);
                }
            });
        }));
    }

    pub(super) fn refresh_git_log(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let epoch = self.git.epoch;
        let read =
            cx.background_executor().spawn(async move { Ok::<_, String>((repo.log_all(2000, 0)?, repo.refs()?)) });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                match result {
                    Ok((commits, refs)) => m.git.log.update(cx, |v, cx| v.set_log(commits, refs, cx)),
                    Err(error) => m.git_problem(Some(error), cx),
                }
            });
        })
        .detach();
    }

    fn watch_git_metadata(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let (send, mut receive) = mpsc::unbounded();
        let watching = cx.background_executor().spawn(async move {
            repo.metadata_dirs()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|dir| {
                    let send = send.clone();
                    rsharp_core::watcher::watch_git(&dir, move |paths| {
                        let _ = send.unbounded_send(paths);
                    })
                    .ok()
                })
                .collect::<Vec<_>>()
        });
        let epoch = self.git.epoch;
        self.git.watching = Some(cx.spawn(async move |this, cx| {
            let watchers = watching.await;
            let _ = this.update(cx, |m, _| {
                if m.git.epoch == epoch {
                    m.git.metadata = watchers;
                }
            });
            while receive.next().await.is_some() {
                if this
                    .update(cx, |m, cx| {
                        if m.git.epoch == epoch {
                            m.refresh_git(cx);
                            if m.dock.is_open(Tool::Git) {
                                m.refresh_git_log(cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn attach_git_buffers(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = &self.git.repository else { return };
        for buffer in crate::buffer::in_folders(&[repo.root.clone()], cx) {
            self.attach_git_buffer(&buffer, cx);
        }
    }

    pub(super) fn attach_git_buffer(&mut self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let path = buffer.read(cx).path.clone();
        if !path.starts_with(&repo.root) {
            return;
        }
        let head = self.git.status.as_ref().and_then(|s| s.head.clone());
        if let Some((cached_head, baseline)) = self.git.baseline.get(&path)
            && cached_head == &head
        {
            buffer.update(cx, |b, cx| b.set_git_baseline(baseline.clone(), cx));
            return;
        }
        if !self.git.baseline_reads.insert((path.clone(), head.clone())) {
            return;
        }
        let charset = buffer.read(cx).charset;
        let epoch = self.git.epoch;
        let revision = head.clone();
        let file = path.clone();
        let read = cx.background_executor().spawn(async move {
            match revision {
                Some(head) => repo
                    .bytes(&head, &file)
                    .and_then(|b| b.map(|b| text(&b, Some(charset)).map(Arc::<str>::from)).transpose()),
                None => Ok(None),
            }
        });
        let buffer = buffer.downgrade();
        cx.spawn(async move |this, cx| {
            let baseline = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                m.git.baseline_reads.remove(&(path.clone(), head.clone()));
                if m.git.status.as_ref().and_then(|s| s.head.as_ref()) != head.as_ref() {
                    return;
                }
                if let Ok(baseline) = baseline {
                    if m.git.baseline.len() >= 128
                        || m.git.baseline.values().filter_map(|(_, b)| b.as_ref()).map(|b| b.len()).sum::<usize>()
                            > 32 * 1024 * 1024
                    {
                        m.git.baseline.clear();
                    }
                    m.git.baseline.insert(path, (head, baseline.clone()));
                    if let Some(buffer) = buffer.upgrade() {
                        buffer.update(cx, |b, cx| b.set_git_baseline(baseline, cx));
                    }
                }
            });
        })
        .detach();
    }

    /// Commands from both Git views share saves, confirmation and asynchronous execution.
    pub fn git_request(&mut self, request: GitRequest, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            GitRequest::Refresh => {
                self.refresh_git(cx);
                self.refresh_git_log(cx);
                return;
            }
            GitRequest::Cancel => {
                if let Some(cancel) = &self.git.cancel {
                    cancel.store(true, Ordering::SeqCst);
                }
                if let Some(repo) = &self.git.repository {
                    repo.cancel();
                }
                return;
            }
            GitRequest::Diff(path) => {
                self.git_diff(path, "HEAD".into(), window, cx);
                return;
            }
            GitRequest::Compare { path, revision } => {
                self.git_diff(path, revision, window, cx);
                return;
            }
            GitRequest::History(path) => {
                self.load_git_history(path, None, cx);
                self.dock.show(Tool::Git);
                return;
            }
            GitRequest::CommitDiff { path, revision } => {
                self.git_commit_diff(path, revision, window, cx);
                return;
            }
            GitRequest::ShowCommit(hash) => {
                self.show_git_commit(hash, window, cx);
                return;
            }
            GitRequest::Resolve(path) => {
                self.open_git_merge(path, window, cx);
                return;
            }
            GitRequest::Rollback(paths) => {
                let names = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                self.confirm(
                    "Rollback",
                    format!("Discard changes to these files? Unversioned files will be deleted.\n{names}"),
                    "Rollback",
                    window,
                    cx,
                    move |m, _, cx| m.run_git(GitRequest::Rollback(paths), cx),
                );
                return;
            }
            GitRequest::Abort => {
                self.confirm(
                    "Abort Git Operation",
                    "Abort the current operation and restore its working files?".into(),
                    "Abort",
                    window,
                    cx,
                    |m, _, cx| m.run_git(GitRequest::Abort, cx),
                );
                return;
            }
            GitRequest::DeleteBranch(ref branch) => {
                let branch = branch.clone();
                self.confirm(
                    "Delete Branch",
                    format!("Delete branch {branch}?"),
                    "Delete",
                    window,
                    cx,
                    move |m, _, cx| m.run_git(GitRequest::DeleteBranch(branch), cx),
                );
                return;
            }
            _ => {}
        }
        self.run_git(request, cx);
    }

    fn run_git(&mut self, request: GitRequest, cx: &mut Context<Self>) {
        if self.git.busy {
            self.git_problem(Some("A Git operation is already running".into()), cx);
            return;
        }
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else {
            self.git_problem(Some("The workspace is not in a Git repository".into()), cx);
            return;
        };
        if let Err(error) = self.save_git_buffers(&repo.root, cx) {
            self.git_problem(Some(error), cx);
            return;
        }
        self.git_problem(None, cx);
        self.git_busy_set(true, cx);
        let epoch = self.git.epoch;
        let cancel = Arc::new(AtomicBool::new(false));
        self.git.cancel = Some(cancel.clone());
        let preserve = matches!(
            request,
            GitRequest::Update
                | GitRequest::Checkout(_)
                | GitRequest::MergeBranch(_)
                | GitRequest::Rebase(_)
                | GitRequest::Accept { .. }
                | GitRequest::Abort
                | GitRequest::Rollback(_)
        );
        let before = self.git.status.as_ref().map(|s| s.changes.clone()).unwrap_or_default();
        let history = crate::history_window::local_history(cx);
        let changed_paths: Vec<_> = before.iter().map(|c| c.path.clone()).collect();
        let read = cx.background_executor().spawn(async move {
            for change in before.iter().filter(|_| preserve) {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(bytes) = std::fs::read(&change.path) {
                    let _ = history.record(&change.path, &bytes, Some("Before Git operation"));
                }
            }
            let result = if cancel.load(Ordering::SeqCst) {
                Err("Git operation cancelled".into())
            } else {
                match request {
                    GitRequest::Commit { paths, message, amend, push } => {
                        repo.commit(&paths, &message, amend).and_then(|_| {
                            if push {
                                repo.push().map_err(|e| format!("Committed, but push failed: {e}"))
                            } else {
                                Ok(())
                            }
                        })
                    }
                    GitRequest::Push => repo.push(),
                    GitRequest::Update => repo.pull(),
                    GitRequest::Fetch => repo.fetch(),
                    GitRequest::Checkout(branch) => {
                        if branch.starts_with("refs/remotes/") {
                            repo.checkout_tracking(&branch)
                        } else if branch.starts_with("refs/tags/") {
                            repo.checkout_revision(&branch)
                        } else {
                            repo.checkout(branch.strip_prefix("refs/heads/").unwrap_or(&branch))
                        }
                    }
                    GitRequest::NewBranch { name, from } => repo.create_branch(&name, from.as_deref()),
                    GitRequest::RenameBranch { from, to } => {
                        repo.rename_branch(from.strip_prefix("refs/heads/").unwrap_or(&from), &to)
                    }
                    GitRequest::DeleteBranch(branch) => {
                        repo.delete_branch(branch.strip_prefix("refs/heads/").unwrap_or(&branch), false)
                    }
                    GitRequest::MergeBranch(branch) => repo.merge(&branch),
                    GitRequest::Rebase(branch) => repo.rebase(&branch),
                    GitRequest::Accept { path, yours } => {
                        repo.accept_side(&path, if yours { Side::Yours } else { Side::Theirs })
                    }
                    GitRequest::Continue => repo.continue_operation(),
                    GitRequest::Abort => repo.abort_operation(),
                    GitRequest::Rollback(paths) => {
                        repo.rollback(&before.iter().filter(|c| paths.contains(&c.path)).cloned().collect::<Vec<_>>())
                    }
                    _ => Err("This Git operation is unavailable".into()),
                }
            };
            let status = repo.status();
            (result, status)
        });
        self.git.task = Some(cx.spawn(async move |this, cx| {
            let (result, status) = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                m.git_busy_set(false, cx);
                let paths = changed_paths;
                if let Ok(status) = status {
                    m.git_set_status(Some(status), cx);
                }
                if let Err(error) = result {
                    m.git_problem(Some(error), cx);
                } else {
                    m.said = Some("Git operation completed".into());
                }
                for path in paths {
                    if let Some(buffer) = crate::buffer::find(&path, cx) {
                        buffer.update(cx, |b, cx| b.refresh_from_disk(cx));
                    }
                    m.follow_written(&path, cx);
                }
                if let Some(repo) = &m.git.repository {
                    for buffer in crate::buffer::in_folders(&[repo.root.clone()], cx) {
                        buffer.update(cx, |b, cx| b.refresh_from_disk(cx));
                    }
                }
                m.rebuild_rows();
                m.attach_git_buffers(cx);
                m.refresh_git_log(cx);
                if m.git.pending {
                    m.git.pending = false;
                    m.refresh_git(cx);
                }
                cx.notify();
            });
        }));
    }

    fn git_diff(&mut self, path: PathBuf, revision: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        // The rope is a snapshot another thread reads: the working text is turned into a string there, not here.
        let current = crate::buffer::find(&path, cx).map(|b| (b.read(cx).document.text().clone(), b.read(cx).charset));
        let epoch = self.git.epoch;
        let read = cx.background_executor().spawn(async move {
            let (after, charset) = match current {
                Some((text, c)) => (text.to_string(), Some(c)),
                None => {
                    let bytes = match std::fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                        Err(e) => return Err(e.to_string()),
                    };
                    (text(&bytes, None)?, None)
                }
            };
            let before = text(&repo.bytes(&revision, &path)?.unwrap_or_default(), charset)?;
            Ok::<_, String>((format!("{} — {revision}", path.display()), before, after))
        });
        cx.spawn_in(window, async move |this, cx| match read.await {
            Ok((title, before, after)) => {
                let _ = this.update_in(cx, |m, _, cx| {
                    if m.git.epoch == epoch {
                        let _ = cx.open_window(git_window("Diff"), |_, cx| {
                            cx.new(|cx| DiffView::new(title, before, "Working tree".into(), after, cx))
                        });
                    }
                });
            }
            Err(error) => {
                let _ = this.update(cx, |m, cx| {
                    if m.git.epoch == epoch {
                        m.git_problem(Some(error), cx);
                    }
                });
            }
        })
        .detach();
    }

    fn git_commit_diff(&mut self, path: PathBuf, revision: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let epoch = self.git.epoch;
        let read = cx.background_executor().spawn(async move {
            let files = repo.commit_files(&revision)?;
            let from = files.iter().find(|c| c.path == path).and_then(|c| c.from.as_ref()).unwrap_or(&path);
            let commit = repo.log(Some(&revision), 1, 0)?.into_iter().next().ok_or("The commit no longer exists")?;
            let before = if let Some(parent) = commit.parents.first() {
                repo.bytes(parent, from)?.unwrap_or_default()
            } else {
                Vec::new()
            };
            let after = repo.bytes(&revision, &path)?.unwrap_or_default();
            Ok::<_, String>((text(&before, None)?, text(&after, None)?, path))
        });
        cx.spawn_in(window, async move |this, cx| match read.await {
            Ok((before, after, path)) => {
                let _ = this.update_in(cx, |m, _, cx| {
                    if m.git.epoch == epoch {
                        let name = path.display().to_string();
                        let _ = cx.open_window(git_window("Commit Diff"), |_, cx| {
                            cx.new(|cx| {
                                DiffView::new(format!("{name} — Parent"), before, format!("{name} — Commit"), after, cx)
                            })
                        });
                    }
                });
            }
            Err(e) => {
                let _ = this.update(cx, |m, cx| {
                    if m.git.epoch == epoch {
                        m.git_problem(Some(e), cx);
                    }
                });
            }
        })
        .detach();
    }

    fn load_git_history(&mut self, path: PathBuf, lines: Option<(usize, usize)>, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let epoch = self.git.epoch;
        let read = cx.background_executor().spawn(async move {
            let title = if let Some((first, last)) = lines {
                format!("{}:{first}–{last}", path.display())
            } else {
                path.display().to_string()
            };
            let commits = if let Some((first, last)) = lines {
                repo.history_lines(&path, first, last, 2000)?
            } else {
                repo.history(&path, 2000)?
            };
            Ok::<_, String>((title, commits, repo.refs()?))
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                match result {
                    Ok((title, commits, refs)) => m.git.log.update(cx, |v, cx| v.set_history(title, commits, refs, cx)),
                    Err(e) => m.git_problem(Some(e), cx),
                }
            });
        })
        .detach();
    }

    pub(super) fn git_history(&mut self, selection: bool, _: &mut Window, cx: &mut Context<Self>) {
        let path = self
            .active_editor()
            .map(|e| e.read(cx).path.clone())
            .or_else(|| self.rows.get(self.selected?).and_then(|r| r.path.clone()));
        let Some(path) = path else { return };
        let lines = selection
            .then(|| {
                self.active_editor().map(|e| {
                    let e = e.read(cx);
                    let b = e.buffer.read(cx);
                    let s = e.selections().last().unwrap();
                    (b.document.text().byte_to_line(s.start()) + 1, b.document.text().byte_to_line(s.end()) + 1)
                })
            })
            .flatten();
        self.dock.show(Tool::Git);
        self.load_git_history(path, lines, cx);
    }

    pub(super) fn git_compare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            self.git_diff(editor.read(cx).path.clone(), "HEAD".into(), window, cx);
        }
    }

    pub(super) fn save_git_choices(&self, cx: &App) {
        if let Some(folder) = self.workspace.as_ref().map(|w| &w.folder) {
            store(cx).write_for(
                folder,
                &GitChoices {
                    draft: self.git.commit.read(cx).message(cx),
                    favorites: self.git.log.read(cx).favorites(),
                },
            );
        }
    }

    fn show_git_commit(&mut self, hash: String, _: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let epoch = self.git.epoch;
        let requested_hash = hash.clone();
        let read = cx.background_executor().spawn(async move { repo.commit_files(&hash) });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch || m.git.log.read(cx).selected_commit() != Some(requested_hash.as_str()) {
                    return;
                }
                match result {
                    Ok(files) => m.git.log.update(cx, |v, cx| v.set_commit_files(files, cx)),
                    Err(error) => m.git_problem(Some(error), cx),
                }
            });
        })
        .detach();
    }

    fn save_git_buffers(&mut self, root: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        for buffer in crate::buffer::in_folders(&[root.to_path_buf()], cx) {
            if buffer.read(cx).is_modified() {
                buffer
                    .update(cx, |b, cx| b.save(cx))
                    .map_err(|e| format!("Could not save {}: {e}", buffer.read(cx).path.display()))?;
            }
        }
        Ok(())
    }

    pub(super) fn move_git_index(&mut self, moves: Vec<(PathBuf, PathBuf)>, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let epoch = self.git.epoch;
        let work = cx.background_executor().spawn(async move {
            for (from, to) in moves {
                repo.move_index(&from, &to)?;
            }
            Ok::<_, String>(())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                if let Err(e) = result {
                    m.git_problem(Some(e), cx);
                }
                m.refresh_git(cx);
            });
        })
        .detach();
    }

    pub(super) fn git_resolve_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self
            .git
            .status
            .as_ref()
            .and_then(|s| s.changes.iter().find(|c| c.kind == rsharp_core::git::ChangeKind::Conflicted))
            .map(|c| c.path.clone());
        if let Some(path) = path {
            self.open_git_merge(path, window, cx);
        }
    }

    fn open_git_merge(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repository.as_ref().map(Repository::operation_scope) else { return };
        let configured_charset = crate::buffer::find(&path, cx).map(|b| b.read(cx).charset);
        let epoch = self.git.epoch;
        let read = cx.background_executor().spawn(async move {
            let stages = repo.stages(&path)?;
            let before = read_optional(&path)?;
            let configured_charset = configured_charset.or_else(|| {
                rsharp_core::editorconfig::EditorConfig::for_file(&path)
                    .get("charset")
                    .and_then(rsharp_core::encodings::Charset::from_editorconfig)
            });
            let (_, charset) =
                rsharp_core::encodings::decode(before.as_deref().unwrap_or_default(), configured_charset);
            let base = text(stages.base.as_deref().unwrap_or_default(), Some(charset))?;
            let (yours, theirs) = if repo.operation() == Operation::Rebase {
                (&stages.theirs, &stages.ours)
            } else {
                (&stages.ours, &stages.theirs)
            };
            let yours = text(yours.as_deref().unwrap_or_default(), Some(charset))?;
            let theirs = text(theirs.as_deref().unwrap_or_default(), Some(charset))?;
            Ok::<_, String>((repo, path, stages, before, charset, base, yours, theirs))
        });
        cx.spawn_in(window, async move |this, cx| match read.await {
            Ok((repo, path, stages, before, charset, base, yours, theirs)) => {
                let _ = this.update_in(cx, |m, _, cx| {
                    if m.git.epoch != epoch {
                        return;
                    }
                    let owner = cx.entity().downgrade();
                    let opened = cx.open_window(git_window("Merge Conflicts"), move |window, cx| {
                        cx.new(|cx| {
                            cx.subscribe_in(
                                &cx.entity(),
                                window,
                                move |view: &mut MergeView, _, event: &MergeRequest, window, cx| {
                                    if matches!(event, MergeRequest::Cancel) {
                                        window.remove_window();
                                        return;
                                    }
                                    if let MergeRequest::Apply(result) = event {
                                        let result = result.clone();
                                        let path = path.clone();
                                        let repo = repo.clone();
                                        let stages = stages.clone();
                                        let before = before.clone();
                                        let handle =
                                            window.window_handle().downcast::<MergeView>().expect("merge window");
                                        let started = owner.update(cx, |m, cx| {
                                            m.apply_git_merge(
                                                repo, path, stages, before, charset, result, epoch, handle, cx,
                                            )
                                        });
                                        match started {
                                            Ok(Ok(())) => {
                                                view.set_busy(true, cx);
                                                view.set_problem(None, cx);
                                            }
                                            Ok(Err(error)) => view.set_problem(Some(error), cx),
                                            Err(_) => view.set_problem(Some("The workspace closed".into()), cx),
                                        }
                                    }
                                },
                            )
                            .detach();
                            MergeView::new(base, yours, theirs, cx)
                        })
                    });
                    if let Ok(handle) = opened {
                        m.git.merge = Some(handle);
                    }
                });
            }
            Err(error) => {
                let _ = this.update(cx, |m, cx| {
                    if m.git.epoch == epoch {
                        m.git_problem(Some(error), cx);
                    }
                });
            }
        })
        .detach();
    }

    fn apply_git_merge(
        &mut self,
        repo: Repository,
        path: PathBuf,
        stages: Stages,
        before: Option<Vec<u8>>,
        charset: rsharp_core::encodings::Charset,
        result: String,
        epoch: u64,
        handle: gpui::WindowHandle<MergeView>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.git.busy || self.git.epoch != epoch {
            return Err("The workspace changed or another Git operation is running".into());
        }
        if crate::buffer::find(&path, cx).is_some_and(|b| b.read(cx).is_modified()) {
            return Err("Save or discard edits made since opening the merge before applying".into());
        }
        self.git_busy_set(true, cx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.git.cancel = Some(cancel.clone());
        let history = crate::history_window::local_history(cx);
        let file = path.clone();
        let read = cx.background_executor().spawn(async move {
            if cancel.load(Ordering::SeqCst) {
                return Err("Merge application cancelled".into());
            }
            if repo.stages(&file)? != stages || read_optional(&file)? != before {
                return Err("The conflict changed while the merge was open; reopen it before applying".into());
            }
            let (old, charset) = rsharp_core::encodings::decode(before.as_deref().unwrap_or_default(), Some(charset));
            let ending = rsharp_text::Document::new(&old).line_ending;
            let mut document = rsharp_text::Document::new(&result);
            document.line_ending = ending;
            let bytes = rsharp_core::encodings::encode(&document.contents(), charset)
                .map_err(|c| format!("'{c}' cannot be written in {}", charset.name()))?;
            if let Some(before) = &before {
                history.record(&file, before, Some("Before resolving conflict")).map_err(|e| e.to_string())?;
            }
            if cancel.load(Ordering::SeqCst) {
                return Err("Merge application cancelled".into());
            }
            if repo.stages(&file)? != stages || read_optional(&file)? != before {
                return Err("The conflict changed while the merge was open; reopen it before applying".into());
            }
            rsharp_core::files::write(&file, &bytes).map_err(|e| e.to_string())?;
            history.record(&file, &bytes, Some("Resolved conflict")).map_err(|e| e.to_string())?;
            repo.mark_resolved(&file)
        });
        self.git.task = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |m, cx| {
                if m.git.epoch != epoch {
                    return;
                }
                m.git_busy_set(false, cx);
                match result {
                    Err(e) => {
                        m.git_problem(Some(e.clone()), cx);
                        let _ = handle.update(cx, |v, _, cx| {
                            v.set_busy(false, cx);
                            v.set_problem(Some(e), cx);
                        });
                    }
                    Ok(()) => {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                        m.git.merge = None;
                    }
                }
                if let Some(b) = crate::buffer::find(&path, cx) {
                    b.update(cx, |b, cx| b.refresh_from_disk(cx));
                }
                m.follow_written(&path, cx);
                m.refresh_git(cx);
            });
        }));
        Ok(())
    }

    pub(super) fn git_change(&mut self, previous: bool, rollback: bool, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().cloned() else { return };
        let (line, hunks, base, now) = {
            let e = editor.read(cx);
            let b = e.buffer.read(cx);
            (e.caret().line, b.git_hunks.clone(), b.git_base.clone(), b.document.text().to_string())
        };
        if rollback {
            if let (Some(base), Some(h)) =
                (base, hunks.iter().find(|h| h.after.contains(&line) || h.after.start == line))
            {
                let edit = rsharp_text::rollback_hunk(&base, &now, h);
                editor.update(cx, |e, cx| {
                    e.apply_plan(
                        rsharp_text::EditPlan {
                            selections: vec![rsharp_text::Selection::caret(edit.range.start)],
                            edits: vec![edit],
                            ..Default::default()
                        },
                        cx,
                    )
                });
            }
        } else {
            let h = if previous {
                hunks.iter().rev().find(|h| h.after.start < line).or(hunks.last())
            } else {
                hunks.iter().find(|h| h.after.start > line).or(hunks.first())
            };
            if let Some(h) = h {
                editor.update(cx, |e, cx| e.place_caret(h.after.start, 0, cx));
            }
        }
    }
}

fn git_window(title: &str) -> gpui::WindowOptions {
    gpui::WindowOptions {
        titlebar: Some(gpui::TitlebarOptions { title: Some(title.to_string().into()), ..Default::default() }),
        window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
            point(px(100.), px(100.)),
            gpui::size(px(1100.), px(700.)),
        ))),
        ..Default::default()
    }
}
