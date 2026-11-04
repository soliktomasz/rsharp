//! The version control and NuGet tools an agent calls in a window (Passenger's `vcs_*` and `nuget_*`): the repository's
//! status, a file's diff, the log, commits, branches and conflicts, answered from Git itself, the changes made through
//! the window's own Git controller so its views follow; and a package search over the folder's feeds and a package added
//! to or removed from a project as `dotnet` does it.

use super::agent::said;
use super::*;
use crate::git_view::GitRequest;
use crate::settings::settings;
use rsharp_core::git::{ChangeKind, Repository};
use rsharp_text::LineChange;
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Task<Result<Value, String>>;

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments[key].as_str().map(str::to_string)
}

fn kind(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Modified => "modified",
        ChangeKind::Added => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Unversioned => "unversioned",
        ChangeKind::Conflicted => "conflicted",
    }
}

impl MainWindow {
    pub(super) fn agent_vcs_tool(&mut self, tool: &str, arguments: &Value, cx: &mut Context<Self>) -> Option<Reply> {
        let done = |result: Result<Value, String>| Some(Task::ready(result));
        let Some(folder) = self.folder().map(Path::to_path_buf) else {
            return matches!(
                tool,
                "vcs_status"
                    | "vcs_diff"
                    | "vcs_log"
                    | "vcs_commit"
                    | "vcs_checkout"
                    | "vcs_resolve"
                    | "nuget_search"
                    | "nuget_install"
                    | "nuget_remove"
            )
            .then(|| Task::ready(Err("No folder is open".into())));
        };
        match tool {
            "vcs_status" => Some(cx.spawn(async move |this, cx| {
                let read = cx
                    .background_executor()
                    .spawn(async move {
                        let repo = Repository::open(&folder)?.ok_or("The folder is not in a Git repository")?;
                        let status = repo.status()?;
                        Ok::<_, String>((repo.root.clone(), status))
                    })
                    .await?;
                this.update(cx, |_, _| {
                    let (root, status) = read;
                    let changes: Vec<Value> = status
                        .changes
                        .iter()
                        .map(|c| {
                            let shown = c.path.strip_prefix(&root).unwrap_or(&c.path).display().to_string();
                            json!({ "file": shown, "kind": kind(c.kind), "index": c.index.to_string(), "worktree": c.worktree.to_string() })
                        })
                        .collect();
                    said(json!({ "branch": status.branch, "head": status.head, "operation": format!("{:?}", status.operation), "changes": changes }).to_string())
                })
                .map_err(|e| e.to_string())
            })),
            "vcs_log" => {
                let limit = arguments["limit"].as_u64().unwrap_or(20).clamp(1, 200) as usize;
                let file = text(arguments, "file").map(|f| self.agent_path(&f));
                Some(cx.background_executor().spawn(async move {
                    let repo = Repository::open(&folder)?.ok_or("The folder is not in a Git repository")?;
                    let commits = match &file {
                        Some(file) => repo.history(file, limit)?,
                        None => repo.log(None, limit, 0)?,
                    };
                    let rows: Vec<Value> = commits
                        .iter()
                        .map(|c| json!({ "hash": c.hash, "author": c.author, "time": c.time, "refs": c.refs, "subject": c.subject }))
                        .collect();
                    Ok(said(json!(rows).to_string()))
                }))
            }
            "vcs_diff" => {
                let Some(file) = text(arguments, "file") else { return done(Err("vcs_diff needs a file".into())) };
                let path = self.agent_path(&file);
                let (open, shown) = (
                    crate::buffer::find(&path, cx).map(|b| b.read(cx).document.text().to_string()),
                    self.relative(&path),
                );
                Some(cx.background_executor().spawn(async move {
                    let repo = Repository::open(&folder)?.ok_or("The folder is not in a Git repository")?;
                    let before = repo.text("HEAD", &path)?.unwrap_or_default();
                    let after = match open {
                        Some(text) => text,
                        None => std::fs::read_to_string(&path).map_err(|e| format!("{shown} could not be read: {e}"))?,
                    };
                    Ok(said(diff_text(&shown, &before, &after)))
                }))
            }
            "vcs_commit" => {
                let Some(message) = text(arguments, "message").filter(|m| !m.trim().is_empty()) else {
                    return done(Err("A commit needs a message".into()));
                };
                let named: Option<Vec<PathBuf>> = arguments["files"]
                    .as_array()
                    .map(|files| files.iter().filter_map(Value::as_str).map(|f| self.agent_path(f)).collect());
                let (amend, push) = (arguments["amend"] == true, arguments["push"] == true);
                Some(self.agent_git(cx, move |w| {
                    let paths = named.unwrap_or_else(|| {
                        w.git_status()
                            .map(|s| {
                                s.changes
                                    .iter()
                                    .filter(|c| !matches!(c.kind, ChangeKind::Unversioned | ChangeKind::Conflicted))
                                    .map(|c| c.path.clone())
                                    .collect()
                            })
                            .unwrap_or_default()
                    });
                    GitRequest::Commit { paths, message, amend, push }
                }))
            }
            "vcs_checkout" => {
                let Some(branch) = text(arguments, "branch") else { return done(Err("vcs_checkout needs a branch".into())) };
                let create = arguments["create"] == true;
                let from = text(arguments, "from");
                Some(self.agent_git(cx, move |_| {
                    if create { GitRequest::NewBranch { name: branch, from } } else { GitRequest::Checkout(branch) }
                }))
            }
            "vcs_resolve" => {
                let request = match (text(arguments, "action").as_deref(), text(arguments, "side").as_deref()) {
                    (Some("continue"), _) => GitRequest::Continue,
                    (Some("abort"), _) => GitRequest::Abort,
                    (_, Some(side @ ("yours" | "theirs"))) => {
                        let Some(file) = text(arguments, "file") else { return done(Err("Resolving a file needs file".into())) };
                        GitRequest::Accept { path: self.agent_path(&file), yours: side == "yours" }
                    }
                    _ => return done(Err("vcs_resolve takes a file with side yours or theirs, or action continue or abort".into())),
                };
                Some(self.agent_git(cx, move |_| request))
            }
            "nuget_search" => {
                let Some(query) = text(arguments, "query").filter(|q| !q.trim().is_empty()) else {
                    return done(Err("nuget_search needs a query".into()));
                };
                let prerelease = arguments["prerelease"] == true;
                Some(cx.background_executor().spawn(async move {
                    let sources = rsharp_core::nuget::sources::SourceSet::load(&folder, &rsharp_core::nuget::sources::Places::current());
                    let feeds: Vec<_> = sources.enabled().map(rsharp_core::nuget::feed::PackageFeed::for_source).collect();
                    if feeds.is_empty() {
                        return Err("No enabled package source".to_string());
                    }
                    let found = rsharp_core::nuget::feed::FeedSet { feeds }.search(&query, prerelease, &|f, id| sources.allows(f, id));
                    let packages: Vec<Value> = found
                        .packages
                        .iter()
                        .take(30)
                        .map(|p| json!({ "id": p.id, "version": p.version, "description": p.description, "authors": p.authors, "feeds": p.feeds }))
                        .collect();
                    let failed: Vec<String> = found.failed.iter().map(|(f, why)| format!("{f}: {why}")).collect();
                    Ok(said(json!({ "packages": packages, "failed": failed }).to_string()))
                }))
            }
            "nuget_install" | "nuget_remove" => {
                let (Some(project), Some(id)) = (text(arguments, "project"), text(arguments, "package")) else {
                    return done(Err(format!("{tool} needs project and package")));
                };
                let version = text(arguments, "version");
                if tool == "nuget_install" && version.is_none() {
                    return done(Err("nuget_install needs a version: nuget_search names the newest".into()));
                }
                let path = self.agent_path(&project);
                if !path.is_file() {
                    return done(Err(format!("{} is not a project file", path.display())));
                }
                // The files a change may write are saved first, so dotnet works from what the editors hold.
                let files = rsharp_core::nuget::change::touched(&path);
                for file in &files {
                    if let Some(buffer) = crate::buffer::find(file, cx).filter(|b| b.read(cx).is_modified())
                        && let Err(e) = buffer.update(cx, |b, cx| b.save(cx))
                    {
                        return done(Err(format!("{} could not be saved: {e}", file.display())));
                    }
                }
                let shared = crate::templates::shared(cx);
                let before: Vec<_> = files.iter().map(|f| (f.clone(), std::fs::read(f).ok())).collect();
                let install = tool == "nuget_install";
                let (project_path, package) = (path.clone(), id.clone());
                let work = cx.background_executor().spawn(async move {
                    let service = crate::templates::service(&shared)?;
                    let cancel = std::sync::atomic::AtomicBool::new(false);
                    let mut output = String::new();
                    let result = if install {
                        rsharp_core::nuget::change::install(service.environment(), &project_path, &package, &version.unwrap_or_default(), &cancel, &mut |s| output.push_str(s))
                    } else {
                        rsharp_core::nuget::change::remove(service.environment(), &project_path, &package, &cancel, &mut |s| output.push_str(s))
                    };
                    result.map(|()| output)
                });
                Some(cx.spawn(async move |this, cx| {
                    let output = work.await?;
                    this.update(cx, |w, cx| {
                        let mut wrote = Vec::new();
                        for (file, old) in before {
                            if std::fs::read(&file).ok() == old {
                                continue;
                            }
                            if let Some(buffer) = crate::buffer::find(&file, cx).filter(|b| !b.read(cx).is_modified()) {
                                buffer.update(cx, |b, cx| b.reload(cx));
                            }
                            wrote.push(w.relative(&file));
                        }
                        let tail: Vec<&str> = output.lines().rev().take(8).collect();
                        said(format!(
                            "{} {id}: changed {}\n{}",
                            if install { "Installed" } else { "Removed" },
                            if wrote.is_empty() { "nothing".into() } else { wrote.join(", ") },
                            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                        ))
                    })
                    .map_err(|e| e.to_string())
                }))
            }
            _ => None,
        }
    }

    /// A Git request made by the window's controller, so its views follow, answered once it has ended: with what the
    /// status bar says of it, which is the error when it failed. A refresh the controller is in the middle of is waited
    /// out first, as a person's click would meet it already done.
    fn agent_git(&mut self, cx: &mut Context<Self>, request: impl FnOnce(&Self) -> GitRequest + 'static) -> Reply {
        if !settings(cx).agents_enabled {
            return Task::ready(Err("Agents are not enabled".into()));
        }
        cx.spawn(async move |this, cx| {
            let mut request = Some(request);
            let mut started = false;
            // The controller is busy from the request until its status is read again.
            for _ in 0..3000 {
                let busy = this.update(cx, |w, cx| {
                    if !started && !w.git_busy() {
                        let request = (request.take().expect("started once"))(w);
                        w.said = None;
                        w.run_git(request, cx);
                        started = true;
                    }
                    w.git_busy()
                });
                if !busy.map_err(|e| e.to_string())? && started {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
            }
            this.update(cx, |w, _| {
                let said_now = w.said.as_ref().map(|s| s.to_string()).unwrap_or_default();
                if said_now == "Git operation completed" { Ok(said(said_now)) } else { Err(said_now) }
            })
            .map_err(|e| e.to_string())?
        })
    }
}

/// A file's change as a diff: each hunk's changed lines (`-` before, `+` after) with two lines around them, numbered by
/// the text after.
fn diff_text(name: &str, before: &str, after: &str) -> String {
    let lines = rsharp_text::line_diff(before, after);
    if lines.iter().all(|(c, _)| *c == LineChange::Same) {
        return format!("{name} has no changes from HEAD.");
    }
    let mut out = vec![format!("--- {name}")];
    let mut line = 0;
    let numbered: Vec<(LineChange, usize, &str)> = lines
        .into_iter()
        .map(|(change, text)| {
            if change != LineChange::Removed {
                line += 1;
            }
            (change, line, text)
        })
        .collect();
    let near = |i: usize| {
        numbered[i.saturating_sub(2)..(i + 3).min(numbered.len())].iter().any(|(c, ..)| *c != LineChange::Same)
    };
    let mut last: Option<usize> = None;
    for (i, (change, n, text)) in numbered.iter().enumerate() {
        if !near(i) {
            continue;
        }
        if last.is_some_and(|l| i > l + 1) {
            out.push("...".into());
        }
        last = Some(i);
        let mark = match change {
            LineChange::Same => ' ',
            LineChange::Removed => '-',
            LineChange::Added => '+',
        };
        out.push(format!("{mark}{n}: {text}"));
    }
    out.join("\n")
}
