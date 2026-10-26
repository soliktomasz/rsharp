//! Git's working tree and index, through its executable. Call these blocking operations off the UI thread.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const ERROR_LIMIT: usize = 1024 * 1024;
static COORDINATORS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
static TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    coordinator: Arc<Mutex<()>>,
    cancelled: Arc<AtomicU64>,
    index_file: Option<PathBuf>,
    expected_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Unversioned,
    Conflicted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub from: Option<PathBuf>,
    pub kind: ChangeKind,
    pub index: char,
    pub worktree: char,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Operation {
    #[default]
    Idle,
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub changes: Vec<FileChange>,
    pub operation: Operation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug)]
pub struct GitRef {
    pub full_name: String,
    pub name: String,
    pub kind: RefKind,
    pub hash: String,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CommitInfo {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub refs: Vec<String>,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stages {
    pub base: Option<Vec<u8>>,
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Yours,
    Theirs,
}

impl Stages {
    pub fn side(&self, operation: Operation, side: Side) -> Option<&[u8]> {
        if git_theirs(operation, side) { self.theirs.as_deref() } else { self.ours.as_deref() }
    }
}

fn git_theirs(operation: Operation, side: Side) -> bool {
    (side == Side::Theirs) != (operation == Operation::Rebase)
}

struct Output {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Output {
    fn checked(self) -> Result<Vec<u8>, String> {
        if self.code == 0 { Ok(self.stdout) } else { Err(self.message()) }
    }
    fn message(&self) -> String {
        let bytes = if self.stderr.is_empty() { &self.stdout } else { &self.stderr };
        let message = String::from_utf8_lossy(bytes).trim().to_string();
        if message.is_empty() { format!("Git exited with status {}", self.code) } else { message }
    }
}

impl Repository {
    /// Finds the containing working tree; ordinary absence is different from Git or repository failure.
    pub fn open(folder: &Path) -> Result<Option<Self>, String> {
        let cancel = AtomicU64::new(0);
        let top = execute(folder, &words(&["rev-parse", "--show-toplevel"]), &cancel, None, None)?;
        if top.code != 0 {
            let has_marker = folder.ancestors().any(|p| p.join(".git").symlink_metadata().is_ok());
            if !has_marker && top.code == 128 && top.message().contains("not a git repository") {
                return Ok(None);
            }
            return Err(top.message());
        }
        let metadata = execute(folder, &words(&["rev-parse", "--absolute-git-dir"]), &cancel, None, None)?.checked()?;
        let root = crate::paths::canonical(&PathBuf::from(os(trim_newline(&top.stdout))));
        let git_dir = crate::paths::canonical(&PathBuf::from(os(trim_newline(&metadata))));
        let mut coordinators = COORDINATORS.get_or_init(Default::default).lock().unwrap();
        coordinators.retain(|_, value| value.strong_count() > 0);
        let coordinator = coordinators.get(&root).and_then(Weak::upgrade).unwrap_or_else(|| Arc::new(Mutex::new(())));
        coordinators.insert(root.clone(), Arc::downgrade(&coordinator));
        Ok(Some(Self {
            root,
            git_dir,
            coordinator,
            cancelled: Arc::new(AtomicU64::new(0)),
            index_file: None,
            expected_generation: None,
        }))
    }

    /// Ends active commands, including hooks and credential helpers, for this repository handle's clones.
    pub fn cancel(&self) {
        self.cancelled.fetch_add(1, Ordering::SeqCst);
    }

    /// Captures cancellation before queueing work, so later steps cannot revive an interrupted operation.
    pub fn operation_scope(&self) -> Self {
        let mut scoped = self.clone();
        scoped.expected_generation =
            Some(self.expected_generation.unwrap_or_else(|| self.cancelled.load(Ordering::SeqCst)));
        scoped
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self.expected_generation.is_some_and(|generation| self.cancelled.load(Ordering::SeqCst) != generation) {
            Err("Git operation cancelled".into())
        } else {
            Ok(())
        }
    }

    pub fn status(&self) -> Result<Status, String> {
        let _held = self.coordinator.lock().unwrap();
        self.status_inner()
    }

    fn status_inner(&self) -> Result<Status, String> {
        let bytes = self.run(&words(&[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--no-ahead-behind",
            "--untracked-files=all",
        ]))?;
        let mut status = Status { branch: None, head: None, changes: Vec::new(), operation: self.operation() };
        let mut entries = bytes.split(|b| *b == 0);
        while let Some(entry) = entries.next() {
            if let Some(head) = entry.strip_prefix(b"# branch.oid ") {
                if head != b"(initial)" {
                    status.head = Some(utf8(head)?.to_string());
                }
            } else if let Some(branch) = entry.strip_prefix(b"# branch.head ") {
                if branch != b"(detached)" {
                    status.branch = Some(utf8(branch)?.to_string());
                }
            } else if let Some(path) = entry.strip_prefix(b"? ") {
                status.changes.push(FileChange {
                    path: self.full(path),
                    from: None,
                    kind: ChangeKind::Unversioned,
                    index: '?',
                    worktree: '?',
                });
            } else if let Some(record) = entry.first().filter(|b| matches!(b, b'1' | b'2' | b'u')) {
                let count = match record {
                    b'1' => 9,
                    b'2' => 10,
                    _ => 11,
                };
                let fields: Vec<_> = entry.splitn(count, |b| *b == b' ').collect();
                if fields.len() != count || fields[1].len() != 2 {
                    return Err("Malformed Git status record".into());
                }
                let xy = fields[1];
                let kind = match record {
                    b'u' => ChangeKind::Conflicted,
                    b'2' => ChangeKind::Renamed,
                    _ if xy.contains(&b'D') => ChangeKind::Deleted,
                    _ if xy[0] == b'A' => ChangeKind::Added,
                    _ => ChangeKind::Modified,
                };
                let from = if *record == b'2' {
                    Some(self.full(entries.next().ok_or("Missing rename source")?))
                } else {
                    None
                };
                status.changes.push(FileChange {
                    path: self.full(fields[count - 1]),
                    from,
                    kind,
                    index: xy[0] as char,
                    worktree: xy[1] as char,
                });
            }
        }
        Ok(status)
    }

    pub fn operation(&self) -> Operation {
        if self.git_dir.join("rebase-merge").exists() || self.git_dir.join("rebase-apply").exists() {
            Operation::Rebase
        } else if self.git_dir.join("MERGE_HEAD").exists() {
            Operation::Merge
        } else if self.git_dir.join("CHERRY_PICK_HEAD").exists() {
            Operation::CherryPick
        } else if self.git_dir.join("REVERT_HEAD").exists() {
            Operation::Revert
        } else {
            Operation::Idle
        }
    }

    /// Additional metadata watched for linked worktrees, whose refs live outside the opened folder.
    pub fn metadata_dirs(&self) -> Result<Vec<PathBuf>, String> {
        let _held = self.coordinator.lock().unwrap();
        let common = self.run(&words(&["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
        let common = PathBuf::from(os(trim_newline(&common)));
        Ok(if common == self.git_dir { vec![self.git_dir.clone()] } else { vec![self.git_dir.clone(), common] })
    }

    pub fn bytes(&self, revision: &str, path: &Path) -> Result<Option<Vec<u8>>, String> {
        let _held = self.coordinator.lock().unwrap();
        if revision == "HEAD" && self.head()?.is_none() {
            return Ok(None);
        }
        let hash = self.resolve(revision)?;
        let relative = self.relative(path)?;
        let mut args = words(&["ls-tree", "-z", &hash, "--"]);
        args.push(literal(&relative));
        let tree = self.run(&args)?;
        if tree.is_empty() {
            return Ok(None);
        }
        let header = tree.split(|b| *b == b'\t').next().ok_or("Malformed Git tree")?;
        let fields: Vec<_> = header.split(|b| *b == b' ').collect();
        if fields.len() != 3 || fields[1] != b"blob" {
            return Err("The revision path is not a file".into());
        }
        let hash = utf8(fields[2])?;
        let mut args = words(&["cat-file", "--filters"]);
        let mut at_path = OsString::from("--path=");
        at_path.push(relative);
        args.push(at_path);
        args.push(hash.into());
        self.run(&args).map(Some)
    }

    pub fn text(&self, revision: &str, path: &Path) -> Result<Option<String>, String> {
        self.bytes(revision, path)?
            .map(|bytes| {
                if bytes.contains(&0) && !bytes.starts_with(&[0xff, 0xfe]) && !bytes.starts_with(&[0xfe, 0xff]) {
                    return Err("Binary files cannot be displayed as text".into());
                }
                Ok(crate::encodings::decode(&bytes, None).0.replace("\r\n", "\n"))
            })
            .transpose()
    }

    pub fn refs(&self) -> Result<Vec<GitRef>, String> {
        let _held = self.coordinator.lock().unwrap();
        let out = self.run(&words(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ]))?;
        utf8(&out)?
            .lines()
            .map(|line| {
                let fields: Vec<_> = line.split('\0').collect();
                if fields.len() != 4 {
                    return Err("Malformed Git reference".into());
                }
                let full_name = fields[0].to_string();
                let (kind, name) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
                    (RefKind::Local, name)
                } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
                    (RefKind::Remote, name)
                } else {
                    (RefKind::Tag, full_name.strip_prefix("refs/tags/").unwrap_or(&full_name))
                };
                Ok(GitRef {
                    name: name.into(),
                    full_name,
                    kind,
                    hash: fields[1].into(),
                    current: fields[2] == "*",
                    upstream: (!fields[3].is_empty()).then(|| fields[3].into()),
                })
            })
            .collect()
    }

    pub fn log(&self, revision: Option<&str>, limit: usize, skip: usize) -> Result<Vec<CommitInfo>, String> {
        let _held = self.coordinator.lock().unwrap();
        self.log_inner(revision, limit, skip, &[], false)
    }

    pub fn log_all(&self, limit: usize, skip: usize) -> Result<Vec<CommitInfo>, String> {
        let _held = self.coordinator.lock().unwrap();
        self.log_inner(None, limit, skip, &[], true)
    }

    pub fn history(&self, path: &Path, limit: usize) -> Result<Vec<CommitInfo>, String> {
        let _held = self.coordinator.lock().unwrap();
        let relative = self.relative(path)?;
        let mut extra = Vec::new();
        if !path.is_dir() {
            extra.push("--follow".into());
        }
        extra.push("--".into());
        extra.push(literal(&relative));
        self.log_inner(None, limit, 0, &extra, false)
    }

    /// History of the inclusive, one-based line range in the committed file.
    pub fn history_lines(
        &self,
        path: &Path,
        first: usize,
        last: usize,
        limit: usize,
    ) -> Result<Vec<CommitInfo>, String> {
        let _held = self.coordinator.lock().unwrap();
        if first == 0 || last < first {
            return Err("Choose a nonempty, one-based line range".into());
        }
        let relative = self.relative(path)?;
        let mut range = OsString::from(format!("-L{first},{last}:"));
        range.push(relative);
        self.log_inner(None, limit, 0, &["--no-patch".into(), range], false)
    }

    pub fn commit_files(&self, revision: &str) -> Result<Vec<FileChange>, String> {
        let _held = self.coordinator.lock().unwrap();
        let revision = self.resolve(revision)?;
        let parents = self.run(&words(&["show", "-s", "--format=%P", &revision]))?;
        let parent = utf8(&parents)?.split_whitespace().next();
        let mut args = words(&["diff-tree", "--no-commit-id", "--name-status", "-r", "-z", "-M"]);
        if let Some(parent) = parent {
            args.push(parent.into());
        } else {
            args.push("--root".into());
        }
        args.push(revision.into());
        let out = self.run(&args)?;
        let mut entries = out.split(|b| *b == 0).filter(|s| !s.is_empty());
        let mut changes = Vec::new();
        while let Some(kind) = entries.next() {
            let first = entries.next().ok_or("Missing changed-file path")?;
            let (kind, from, path) = match kind.first() {
                Some(b'R') | Some(b'C') => (
                    ChangeKind::Renamed,
                    Some(self.full(first)),
                    self.full(entries.next().ok_or("Missing rename destination")?),
                ),
                Some(b'A') => (ChangeKind::Added, None, self.full(first)),
                Some(b'D') => (ChangeKind::Deleted, None, self.full(first)),
                _ => (ChangeKind::Modified, None, self.full(first)),
            };
            changes.push(FileChange { path, from, kind, index: '.', worktree: '.' });
        }
        Ok(changes)
    }

    fn log_inner(
        &self,
        revision: Option<&str>,
        limit: usize,
        skip: usize,
        extra: &[OsString],
        all: bool,
    ) -> Result<Vec<CommitInfo>, String> {
        let mut args = words(&[
            "log",
            "-z",
            "--format=%H%x00%P%x00%an%x00%ae%x00%ct%x00%D%x00%s",
            &format!("--max-count={}", limit.min(2000)),
            &format!("--skip={skip}"),
        ]);
        if all {
            args.extend(words(&["--branches", "--remotes", "--tags", "--topo-order", "--ignore-missing", "HEAD"]));
        } else {
            let hash = match revision {
                Some(rev) => self.resolve(rev)?,
                None => match self.head()? {
                    Some(head) => head,
                    None => return Ok(Vec::new()),
                },
            };
            args.push(hash.into());
        }
        args.extend_from_slice(extra);
        let out = self.run(&args)?;
        let fields: Vec<_> = out.split(|b| *b == 0).collect();
        let mut commits = Vec::new();
        for record in fields.chunks(7) {
            if record.len() == 1 && record[0].is_empty() {
                continue;
            }
            if record.len() != 7 {
                return Err("Malformed Git log record".into());
            }
            commits.push(CommitInfo {
                hash: utf8(record[0])?.into(),
                parents: utf8(record[1])?.split_whitespace().map(str::to_string).collect(),
                author: utf8(record[2])?.into(),
                email: utf8(record[3])?.into(),
                time: utf8(record[4])?.parse().map_err(|_| "Invalid Git commit time")?,
                refs: utf8(record[5])?.split(", ").filter(|s| !s.is_empty()).map(str::to_string).collect(),
                subject: utf8(record[6])?.into(),
            });
        }
        Ok(commits)
    }

    pub fn commit(&self, paths: &[PathBuf], message: &str, amend: bool) -> Result<(), String> {
        self.commit_transaction(paths, None, message, amend)
    }

    /// Completes a whole-index operation with an explicit list of changes to leave uncommitted.
    pub fn commit_selection(
        &self,
        paths: &[PathBuf],
        excluded: &[PathBuf],
        message: &str,
        amend: bool,
    ) -> Result<(), String> {
        self.commit_transaction(paths, Some(excluded), message, amend)
    }

    fn commit_transaction(
        &self,
        paths: &[PathBuf],
        excluded: Option<&[PathBuf]>,
        message: &str,
        amend: bool,
    ) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let index = TemporaryIndex::new(self.git_dir.join("index"))?;
        let mut isolated = self.clone();
        isolated.index_file = Some(index.path.clone());
        isolated.commit_inner(paths, excluded, message, amend)?;
        index
            .install()
            .map_err(|error| format!("The commit succeeded, but its staging state could not be installed: {error}"))
    }

    fn commit_inner(
        &self,
        paths: &[PathBuf],
        excluded: Option<&[PathBuf]>,
        message: &str,
        amend: bool,
    ) -> Result<(), String> {
        if message.trim().is_empty() {
            return Err("Enter a commit message".into());
        }
        if paths.is_empty() && !amend {
            return Err("Choose files to commit".into());
        }
        let status = self.status_inner()?;
        if status.changes.iter().any(|c| c.kind == ChangeKind::Conflicted) {
            return Err("Resolve conflicts before committing".into());
        }
        if status.operation == Operation::Rebase {
            return Err("Use Continue Rebase to finish this operation".into());
        }
        let mut relative = Vec::new();
        for path in paths {
            relative.push(self.write_path(path)?);
            if let Some(from) = status.changes.iter().find(|c| c.path == *path).and_then(|c| c.from.as_ref()) {
                relative.push(self.write_path(from)?);
            }
        }
        relative.sort();
        relative.dedup();
        if !relative.is_empty() {
            let mut add = words(&["add", "-A", "--"]);
            add.extend(paths.iter().map(|p| self.relative(p).map(|p| literal(&p))).collect::<Result<Vec<_>, _>>()?);
            self.run(&add)?;
        }
        let completing = status.operation != Operation::Idle;
        if completing {
            let excluded = match excluded {
                Some(excluded) => excluded
                    .iter()
                    .filter(|p| !paths.contains(p))
                    .map(|p| self.write_path(p))
                    .collect::<Result<Vec<_>, _>>()?,
                None => status
                    .changes
                    .iter()
                    .filter(|c| !paths.contains(&c.path))
                    .map(|c| self.relative(&c.path))
                    .collect::<Result<Vec<_>, _>>()?,
            };
            if !excluded.is_empty() {
                let mut reset = words(&["reset", "-q", "HEAD", "--"]);
                reset.extend(excluded.iter().map(|p| literal(p)));
                self.run(&reset)?;
            }
        }
        let message_path = std::env::temp_dir().join(format!(
            "rsharp-git-message-{}-{}",
            std::process::id(),
            TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file =
            std::fs::OpenOptions::new().write(true).create_new(true).open(&message_path).map_err(|e| e.to_string())?;
        use std::io::Write;
        if let Err(error) = file.write_all(message.as_bytes()) {
            let _ = std::fs::remove_file(&message_path);
            return Err(error.to_string());
        }
        drop(file);
        let mut args = words(&["commit", "-F"]);
        args.push(message_path.as_os_str().into());
        if amend {
            args.push("--amend".into());
        }
        if !completing {
            args.push("--only".into());
            if !relative.is_empty() {
                args.push("--".into());
                args.extend(relative.iter().map(|p| literal(p)));
            }
        }
        let result = self.run(&args).map(|_| ());
        let _ = std::fs::remove_file(message_path);
        result
    }

    pub fn push(&self) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let status = self.status_inner()?;
        let branch = status.branch.ok_or("A detached HEAD has no branch to push")?;
        let upstream = self.raw(&words(&["config", "--get", &format!("branch.{branch}.remote")]))?;
        if upstream.code == 0 {
            return self.run(&words(&["push"])).map(|_| ());
        }
        if upstream.code != 1 {
            return Err(upstream.message());
        }
        let remotes = self.run(&words(&["remote"]))?;
        let remotes: Vec<_> = utf8(&remotes)?.lines().collect();
        if remotes.len() != 1 {
            return Err("Choose a remote: this branch has no upstream and the remote is not unambiguous".into());
        }
        self.run(&words(&["push", "-u", "--", remotes[0], &format!("refs/heads/{branch}:refs/heads/{branch}")]))
            .map(|_| ())
    }

    pub fn fetch(&self) -> Result<(), String> {
        self.command(&["fetch", "--all", "--prune"])
    }
    pub fn pull(&self) -> Result<(), String> {
        self.command(&["pull", "--no-edit"])
    }
    pub fn checkout(&self, branch: &str) -> Result<(), String> {
        self.command(&["switch", "--end-of-options", branch])
    }
    pub fn checkout_tracking(&self, branch: &str) -> Result<(), String> {
        self.command(&["switch", "--track", "--end-of-options", branch])
    }
    pub fn checkout_revision(&self, revision: &str) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let revision = self.resolve(revision)?;
        self.run(&words(&["switch", "--detach", "--end-of-options", &revision])).map(|_| ())
    }
    pub fn create_branch(&self, name: &str, revision: Option<&str>) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        self.run(&words(&["check-ref-format", "--branch", name]))?;
        let mut args = words(&["switch", "-c", name]);
        if let Some(revision) = revision {
            args.push("--end-of-options".into());
            args.push(self.resolve(revision)?.into());
        }
        self.run(&args).map(|_| ())
    }
    pub fn rename_branch(&self, from: &str, to: &str) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        self.run(&words(&["check-ref-format", "--branch", to]))?;
        self.run(&words(&["branch", "-m", "--", from, to])).map(|_| ())
    }
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<(), String> {
        self.command(&["branch", if force { "-D" } else { "-d" }, "--", name])
    }
    pub fn merge(&self, branch: &str) -> Result<(), String> {
        self.command(&["merge", "--no-edit", "--end-of-options", branch])
    }
    pub fn rebase(&self, branch: &str) -> Result<(), String> {
        self.command(&["rebase", "--end-of-options", branch])
    }

    pub fn stages(&self, path: &Path) -> Result<Stages, String> {
        let _held = self.coordinator.lock().unwrap();
        self.stages_inner(path)
    }

    fn stages_inner(&self, path: &Path) -> Result<Stages, String> {
        let relative = self.relative(path)?;
        let mut args = words(&["ls-files", "-u", "-z", "--"]);
        args.push(literal(&relative));
        let entries = self.run(&args)?;
        let mut stages = Stages { base: None, ours: None, theirs: None };
        for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let header = entry.split(|b| *b == b'\t').next().unwrap();
            let fields: Vec<_> = header.split(|b| *b == b' ').collect();
            if fields.len() != 3 {
                return Err("Malformed Git index entry".into());
            }
            if fields[0] == b"120000" {
                return Err("Symlink conflicts cannot be merged as text; accept a whole side".into());
            }
            let mut filter = words(&["cat-file", "--filters"]);
            let mut at_path = OsString::from("--path=");
            at_path.push(&relative);
            filter.push(at_path);
            filter.push(os(fields[1]));
            let bytes = self.run(&filter)?;
            match fields[2] {
                b"1" => stages.base = Some(bytes),
                b"2" => stages.ours = Some(bytes),
                b"3" => stages.theirs = Some(bytes),
                _ => return Err("Invalid Git index stage".into()),
            }
        }
        Ok(stages)
    }

    pub fn mark_resolved(&self, path: &Path) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        self.add_path(path)
    }
    fn add_path(&self, path: &Path) -> Result<(), String> {
        let relative = self.write_path(path)?;
        let mut args = words(&["add", "-A", "--"]);
        args.push(literal(&relative));
        self.run(&args).map(|_| ())
    }

    /// Yours/Theirs uses the person's branch during rebase, where Git's stage labels are reversed.
    pub fn accept_side(&self, path: &Path, side: Side) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let relative = self.write_path(path)?;
        let theirs = git_theirs(self.operation(), side);
        let stage = if theirs { b'3' } else { b'2' };
        let mut args = words(&["ls-files", "-u", "-z", "--"]);
        args.push(literal(&relative));
        let entries = self.run(&args)?;
        if entries.is_empty() {
            return Err("The file is no longer conflicted".into());
        }
        let exists = entries
            .split(|b| *b == 0)
            .any(|entry| entry.split(|b| *b == b'\t').next().is_some_and(|head| head.last() == Some(&stage)));
        if exists {
            let mut args = words(&["checkout", if theirs { "--theirs" } else { "--ours" }, "--"]);
            args.push(literal(&relative));
            self.run(&args)?;
        } else {
            self.check_cancelled()?;
            match std::fs::remove_file(self.root.join(&relative)) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.to_string()),
            }
        }
        self.add_path(path)
    }

    pub fn continue_operation(&self) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let args = match self.operation() {
            Operation::Rebase => words(&["rebase", "--continue"]),
            Operation::Merge => words(&["commit", "--no-edit"]),
            Operation::CherryPick => words(&["cherry-pick", "--continue"]),
            Operation::Revert => words(&["revert", "--continue"]),
            Operation::Idle => return Err("No Git operation is in progress".into()),
        };
        self.run(&args).map(|_| ())
    }
    pub fn abort_operation(&self) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        let command = match self.operation() {
            Operation::Rebase => "rebase",
            Operation::Merge => "merge",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
            Operation::Idle => return Err("No Git operation is in progress".into()),
        };
        self.run(&words(&[command, "--abort"])).map(|_| ())
    }

    /// Moves the staged blob only; the caller has already moved the working file.
    pub fn move_index(&self, from: &Path, to: &Path) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        if from == to {
            return Ok(());
        }
        let index = TemporaryIndex::new(self.git_dir.join("index"))?;
        let mut isolated = self.clone();
        isolated.index_file = Some(index.path.clone());
        if isolated.move_index_inner(from, to)? {
            self.check_cancelled()?;
            index.install()?;
        }
        Ok(())
    }

    fn move_index_inner(&self, from: &Path, to: &Path) -> Result<bool, String> {
        let from = self.write_path(from)?;
        let to = self.write_path(to)?;
        let mut args = words(&["ls-files", "--stage", "-z", "--"]);
        args.push(literal(&from));
        let entries = self.run(&args)?;
        if entries.is_empty() {
            return Ok(false);
        }
        let mut destination = words(&["ls-files", "--stage", "-z", "--"]);
        destination.push(literal(&to));
        if !self.run(&destination)?.is_empty() {
            return Err("The destination already has an index entry".into());
        }
        let mut moves = Vec::new();
        for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let tab = entry.iter().position(|b| *b == b'\t').ok_or("Malformed Git index entry")?;
            let fields: Vec<_> = entry[..tab].split(|b| *b == b' ').collect();
            if fields.len() != 3 {
                return Err("Malformed Git index entry".into());
            }
            if fields[2] != b"0" {
                return Err("Resolve the file's conflict before moving its index entry".into());
            }
            let path = PathBuf::from(os(&entry[tab + 1..]));
            let moved = if path == from {
                to.clone()
            } else {
                let relative =
                    path.strip_prefix(&from).map_err(|_| "Git returned an index entry outside the moved path")?;
                to.join(relative)
            };
            moves.push((fields[0], fields[1], path, moved));
        }
        for (mode, hash, from, to) in moves {
            let mut insert = words(&["update-index", "--add", "--cacheinfo", utf8(mode)?, utf8(hash)?]);
            // An index path is Git's own, '/' between its names: a path joined on Windows has '\' (a Unix name may hold
            // one, so only Windows' separators are turned).
            insert.push(if cfg!(windows) {
                OsString::from(to.to_string_lossy().replace('\\', "/"))
            } else {
                to.into_os_string()
            });
            self.run(&insert)?;
            let mut remove = words(&["update-index", "--force-remove", "--"]);
            remove.push(from.into_os_string());
            self.run(&remove)?;
        }
        Ok(true)
    }

    pub fn rollback(&self, changes: &[FileChange]) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        for change in changes {
            self.write_path(&change.path)?;
            if let Some(from) = &change.from {
                self.write_path(from)?;
            }
        }
        for change in changes {
            self.check_cancelled()?;
            let relative = self.relative(&change.path)?;
            if matches!(change.kind, ChangeKind::Unversioned | ChangeKind::Added) {
                if change.kind == ChangeKind::Added {
                    let mut args = words(&["rm", "--cached", "-f", "--"]);
                    args.push(literal(&relative));
                    self.run(&args)?;
                }
                self.check_cancelled()?;
                match std::fs::remove_file(&change.path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.to_string()),
                }
            } else {
                let mut args = words(&["restore", "--source=HEAD", "--staged", "--worktree", "--"]);
                if let Some(from) = &change.from {
                    args.push(literal(&self.relative(from)?));
                } else {
                    args.push(literal(&relative));
                }
                self.run(&args)?;
                if change.from.is_some() {
                    let mut args = words(&["rm", "--cached", "-f", "--"]);
                    args.push(literal(&relative));
                    self.run(&args)?;
                    self.check_cancelled()?;
                    match std::fs::remove_file(&change.path) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.to_string()),
                    }
                }
            }
        }
        Ok(())
    }

    fn command(&self, args: &[&str]) -> Result<(), String> {
        let _held = self.coordinator.lock().unwrap();
        self.run(&words(args)).map(|_| ())
    }
    fn resolve(&self, revision: &str) -> Result<String, String> {
        let out =
            self.run(&words(&["rev-parse", "--verify", "--end-of-options", &format!("{revision}^{{commit}}")]))?;
        Ok(utf8(&out)?.trim().into())
    }
    fn head(&self) -> Result<Option<String>, String> {
        let out = self.raw(&words(&["rev-parse", "--verify", "HEAD"]))?;
        if out.code == 0 {
            return Ok(Some(utf8(&out.stdout)?.trim().into()));
        }
        if self.status_inner()?.head.is_none() { Ok(None) } else { Err(out.message()) }
    }
    fn raw(&self, args: &[OsString]) -> Result<Output, String> {
        execute(&self.root, args, &self.cancelled, self.index_file.as_deref(), self.expected_generation)
    }
    fn run(&self, args: &[OsString]) -> Result<Vec<u8>, String> {
        self.raw(args)?.checked()
    }
    fn full(&self, relative: &[u8]) -> PathBuf {
        self.root.join(os(relative))
    }
    fn relative(&self, path: &Path) -> Result<PathBuf, String> {
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.root).map_err(|_| "The path is outside the Git working tree")?
        } else {
            path
        };
        if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err("Choose a file inside the Git working tree".into());
        }
        if relative.components().any(|c| c.as_os_str() == ".git") {
            return Err("Git metadata cannot be changed as a working file".into());
        }
        Ok(relative.into())
    }
    fn write_path(&self, path: &Path) -> Result<PathBuf, String> {
        let relative = self.relative(path)?;
        let full = self.root.join(&relative);
        if let Some(existing) = full.parent().and_then(|p| p.ancestors().find(|a| a.exists())) {
            let parent = crate::paths::canonical(existing);
            if !parent.starts_with(&self.root) {
                return Err("The path follows a directory symlink outside the Git working tree".into());
            }
        }
        Ok(relative)
    }
}

fn words(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}
fn literal(path: &Path) -> OsString {
    let mut out = OsString::from(":(literal)");
    out.push(path);
    out
}
fn utf8(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|_| "Git returned non-UTF-8 metadata".into())
}
fn trim_newline(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}
#[cfg(unix)]
fn os(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}
#[cfg(not(unix))]
fn os(bytes: &[u8]) -> OsString {
    String::from_utf8_lossy(bytes).into_owned().into()
}

fn execute(
    root: &Path,
    args: &[OsString],
    cancelled: &AtomicU64,
    index_file: Option<&Path>,
    expected_generation: Option<u64>,
) -> Result<Output, String> {
    let generation = expected_generation.unwrap_or_else(|| cancelled.load(Ordering::SeqCst));
    if cancelled.load(Ordering::SeqCst) != generation {
        return Err("Git operation cancelled".into());
    }
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(["-c", "i18n.logOutputEncoding=UTF-8"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
        "GIT_PREFIX",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_LITERAL_PATHSPECS",
        "GIT_GLOB_PATHSPECS",
        "GIT_NOGLOB_PATHSPECS",
        "GIT_ICASE_PATHSPECS",
    ] {
        command.env_remove(key);
    }
    if let Some(index_file) = index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("LC_ALL", "C");
    let mut child = crate::process::spawn(command).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Git is not installed; install Git and reopen the workspace".into()
        } else {
            format!("Could not start Git: {e}")
        }
    })?;
    let stdout = child.process.stdout.take().unwrap();
    let stderr = child.process.stderr.take().unwrap();
    let output = std::thread::spawn(move || drain(stdout, OUTPUT_LIMIT));
    let errors = std::thread::spawn(move || drain(stderr, ERROR_LIMIT));
    let status = loop {
        if cancelled.load(Ordering::SeqCst) != generation {
            let pid = child.id();
            child.kill();
            if let Some(folder) = index_file.and_then(Path::parent) {
                let _ = std::fs::remove_file(folder.join(format!("next-index-{pid}")));
                let _ = std::fs::remove_file(folder.join(format!("next-index-{pid}.lock")));
            }
            break Err("Git operation cancelled".to_string());
        }
        match child.process.try_wait() {
            Ok(Some(status)) => {
                child.wait().map_err(|e| e.to_string())?;
                break Ok(status);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                child.kill();
                break Err(error.to_string());
            }
        }
    };
    let stdout = output.join().map_err(|_| "Git output reader failed")?;
    let stderr = errors.join().map_err(|_| "Git error reader failed")?;
    let status = status?;
    Ok(Output { code: status.code().unwrap_or(-1), stdout: stdout?, stderr: stderr? })
}

fn drain(mut pipe: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut exceeded = false;
    loop {
        let count = pipe.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() + count <= limit {
            bytes.extend_from_slice(&chunk[..count]);
        } else {
            exceeded = true;
        }
    }
    if exceeded { Err(format!("Git output exceeded the {} MiB limit", limit / 1024 / 1024)) } else { Ok(bytes) }
}

struct TemporaryIndex {
    path: PathBuf,
    destination: PathBuf,
    before: Option<Vec<u8>>,
}

impl TemporaryIndex {
    fn new(destination: PathBuf) -> Result<Self, String> {
        let before = Self::read(&destination)?;
        let path = destination.with_file_name(format!(
            "rsharp-index-{}-{}",
            std::process::id(),
            TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file =
            std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        if let Some(bytes) = &before {
            use std::io::Write;
            if let Err(error) = file.write_all(bytes) {
                let _ = std::fs::remove_file(&path);
                return Err(error.to_string());
            }
        } else {
            drop(file);
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        Ok(Self { path, destination, before })
    }

    fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.len() > OUTPUT_LIMIT as u64 => {
                Err("The Git index exceeds the 64 MiB limit".into())
            }
            Ok(_) => std::fs::read(path).map(Some).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn install(&self) -> Result<(), String> {
        let lock_path = self.destination.with_file_name("index.lock");
        let mut lock =
            std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path).map_err(|e| e.to_string())?;
        let install = (|| {
            if Self::read(&self.destination)? != self.before {
                return Err("The index changed outside this commit; it was left intact".into());
            }
            if let Ok(metadata) = std::fs::metadata(&self.destination) {
                lock.set_permissions(metadata.permissions()).map_err(|e| e.to_string())?;
            }
            let bytes = Self::read(&self.path)?.ok_or("Git did not leave its committed index")?;
            use std::io::Write;
            lock.write_all(&bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&lock_path, &self.destination).map_err(|e| e.to_string())
        })();
        drop(lock);
        let _ = std::fs::remove_file(lock_path);
        install
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let mut lock = self.path.as_os_str().to_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock));
    }
}
