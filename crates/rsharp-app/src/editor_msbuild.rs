//! The code help in an MSBuild file (Passenger's `CodeAssist.MSBuild`): completion, the hover card and Go to
//! Declaration from MSBuild's schemas and the last evaluation of its project, with package ids and versions asked of
//! the folder's NuGet feeds, in place of what Roslyn gives a C# file.

use super::assist::Popup;
use super::*;
use crate::language;
use gpui::Task;
use rsharp_core::msbuild::help::{self, Ask, Kind, Suggestion};
use rsharp_core::nuget::feed::{FeedSet, PackageFeed};
use rsharp_core::nuget::sources::{Places, SourceSet};
use rsharp_lsp::{CompletionItem, CompletionList, Location, Span, TextEdit};
use serde_json::json;

impl CodeEditor {
    pub(super) fn is_msbuild(&self) -> bool {
        rsharp_core::msbuild::is_msbuild(&self.path)
    }

    /// After typing in an MSBuild file: the list opened at an element's `<`, an attribute's quote, a reference's
    /// parenthesis, a list's semicolon or a word's first letter, and kept to the word as it grows.
    pub(super) fn msbuild_typed(&mut self, c: char, cx: &mut Context<Self>) {
        let completing = matches!(self.assist.popup, Some(Popup::Completion { .. }));
        let before = self.primary().head.checked_sub(c.len_utf8() + 1).map(|at| self.text.byte(at) as char);
        match c {
            c if completing && (c.is_alphanumeric() || matches!(c, '_' | '.' | '-')) => self.filter_completion(cx),
            '<' | '"' | '\'' | ';' => self.msbuild_complete(false, cx),
            '(' if matches!(before, Some('$' | '@' | '%')) => self.msbuild_complete(false, cx),
            ' ' if !completing => self.msbuild_complete(false, cx),
            c if c.is_alphanumeric()
                && (self.primary().head == c.len_utf8()
                    || !matches!(before, Some(b) if b.is_alphanumeric() || matches!(b, '_' | '.' | '-'))) =>
            {
                self.msbuild_complete(false, cx)
            }
            _ if completing => self.close_popup(cx),
            _ => {}
        }
    }

    /// What may go where the caret is, as a completion list over the word it replaces.
    pub(super) fn msbuild_complete(&mut self, explicit: bool, cx: &mut Context<Self>) {
        let head = self.primary().head;
        let text = self.text.to_string();
        let insight = self.buffer.read(cx).insight.clone();
        let completion = help::complete(&text, head, insight.as_deref());
        let rope = self.text.clone();
        let (start, list) = match completion.ask {
            None => {
                let Some(first) = completion.suggestions.first() else {
                    if explicit {
                        cx.emit(Said("No suggestions here".into()));
                    }
                    return;
                };
                let start = first.replace.start;
                (
                    start,
                    Task::ready(Some(
                        completion.suggestions.iter().map(|s| msbuild_item(&rope, s)).collect::<Vec<_>>(),
                    )),
                )
            }
            Some(ask) => {
                // Packages come from the feeds of the project's folder, as its restore would take them.
                let folder = self.path.parent().map(PathBuf::from).unwrap_or_default();
                let start = match &ask {
                    Ask::Packages { replace, .. } | Ask::Versions { replace, .. } => replace.start,
                };
                let found = cx.background_executor().spawn(async move {
                    let sources = SourceSet::load(&folder, &Places::current());
                    let feeds = FeedSet { feeds: sources.enabled().map(PackageFeed::for_source).collect() };
                    let allows = |f: &str, id: &str| sources.allows(f, id);
                    let suggestions: Vec<Suggestion> = match ask {
                        Ask::Packages { prefix, replace } if !prefix.trim().is_empty() => feeds
                            .search(&prefix, false, &allows)
                            .packages
                            .into_iter()
                            .map(|p| Suggestion {
                                caret: p.id.len(),
                                insert: p.id.clone(),
                                detail: p.description.lines().next().unwrap_or("").to_string(),
                                label: p.id,
                                replace: replace.clone(),
                                kind: Kind::Item,
                            })
                            .collect(),
                        // shortcut: versions carry no advisories; mark the vulnerable ones from `Vulnerabilities`, as
                        // the NuGet window does, once this list is read for audits.
                        Ask::Versions { id, replace } => feeds
                            .versions(&id, false, &allows)
                            .unwrap_or_default()
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| Suggestion {
                                caret: v.len(),
                                insert: v.clone(),
                                // Newest first, as the feeds give them, whatever the letters say.
                                detail: if i == 0 { "newest".into() } else { String::new() },
                                label: v,
                                replace: replace.clone(),
                                kind: Kind::Value,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    Some(suggestions)
                });
                let mapped = cx.background_executor().spawn(async move {
                    let suggestions = found.await?;
                    Some(
                        suggestions
                            .iter()
                            .enumerate()
                            .map(|(i, s)| CompletionItem { sort_text: format!("{i:05}"), ..msbuild_item(&rope, s) })
                            .collect::<Vec<_>>(),
                    )
                });
                (start, mapped)
            }
        };
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let Some(items) = list.await else { return };
            let _ = this.update(cx, |e, cx| {
                let head = e.primary().head;
                if head < start || e.text.byte_to_line(head) != e.text.byte_to_line(start) || items.is_empty() {
                    return;
                }
                let list = CompletionList { incomplete: false, items };
                e.show(Popup::Completion { list, shown: Vec::new(), selected: 0, start, explicit }, start, cx);
                e.filter_completion(cx);
            });
        }));
    }

    /// The hover's words for the name under `at`, from the last evaluation.
    pub(super) fn msbuild_hover(&self, at: usize, cx: &App) -> Option<String> {
        let insight = self.buffer.read(cx).insight.clone()?;
        help::hover(&self.text.to_string(), at, &rsharp_core::paths::canonical(&self.path), &insight)
    }

    /// Go to Declaration in an MSBuild file: where a property is set, a target defined, an import's file.
    pub(super) fn msbuild_declaration(&mut self, cx: &mut Context<Self>) {
        let insight = self.buffer.read(cx).insight.clone();
        let head = self.primary().head;
        let found = insight.and_then(|i| {
            help::declaration(&self.text.to_string(), head, &rsharp_core::paths::canonical(&self.path), &i)
        });
        match found {
            Some((path, line, column)) => {
                let at = rsharp_lsp::Point::new(line, column);
                cx.emit(Navigate {
                    title: "Declaration".into(),
                    to: vec![Location { path, span: Span { start: at, end: at } }],
                    list: false,
                });
            }
            None => cx.emit(Said("Cannot find declaration to go to".into())),
        }
    }
}

fn msbuild_item(rope: &Rope, s: &Suggestion) -> CompletionItem {
    CompletionItem {
        label: s.label.clone(),
        detail: s.detail.clone(),
        kind: match s.kind {
            Kind::Element => 7,
            Kind::Attribute | Kind::Metadata => 5,
            Kind::Property => 10,
            Kind::Item => 9,
            Kind::Task => 3,
            Kind::Target => 23,
            Kind::Value => 12,
        },
        filter_text: s.label.clone(),
        sort_text: s.label.to_lowercase(),
        edit: Some(TextEdit {
            span: Span { start: language::point(rope, s.replace.start), end: language::point(rope, s.replace.end) },
            text: s.insert.clone(),
        }),
        additional: Vec::new(),
        raw: json!({ "caret": s.caret }),
    }
}
