//! The code help in an Avalonia or WPF XAML file (Passenger's `CodeAssist.Xaml`): completion, the hover card and Go to
//! Declaration from Passenger.Xaml in the XAML host, in place of what Roslyn gives a C# file.

use super::assist::Popup;
use super::*;
use crate::language::{self, offset};
use gpui::Task;
use rsharp_lsp::{CompletionItem, CompletionList, Span, TextEdit};
use serde_json::json;

/// An element's name in XAML, for the window to find its code-behind's uses through Roslyn: listed (Find Usages), or
/// renamed to `rename` with them.
pub struct CodeBehind {
    pub xaml: PathBuf,
    pub named: rsharp_lsp::xaml::XamlNamed,
    pub rename: Option<String>,
}

/// Go to Declaration from C# reached the field Avalonia's generator makes for a named element of `class`'s XAML.
pub struct XamlField {
    pub class: String,
    pub name: String,
}

impl gpui::EventEmitter<CodeBehind> for CodeEditor {}
impl gpui::EventEmitter<XamlField> for CodeEditor {}
impl gpui::EventEmitter<crate::xaml_dialog::OpenExtract> for CodeEditor {}

/// The class whose generated field a definition answer names: Avalonia's generator's document is `<Class>.g.cs`.
pub fn generated_field(answer: &serde_json::Value) -> Option<String> {
    let places = answer.as_array().cloned().unwrap_or_else(|| vec![answer.clone()]);
    places.iter().find_map(|place| {
        let uri = place["uri"].as_str().or(place["targetUri"].as_str())?;
        let query = uri.strip_prefix("roslyn-source-generated://")?.split_once('?')?.1;
        let field = |key: &str| query.split('&').find_map(|p| p.strip_prefix(key)?.strip_prefix('='));
        (field("assemblyName")? == "Avalonia.Generators").then_some(())?;
        Some(field("hintName")?.strip_suffix(".g.cs")?.to_string())
    })
}

/// WPF's MarkupCompile output for a XAML file, `obj/…/File.g.cs` (`.g.i.cs` at design time).
pub fn is_markup_compiled(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    (name.ends_with(".g.cs") || name.ends_with(".g.i.cs")) && path.components().any(|c| c.as_os_str() == "obj")
}

/// Where a named element's XAML names it, for MarkupCompile's field of it: the file and line of the `#line` before the
/// field, the caret on the name's value there; None for any other place.
pub(super) fn markup_compiled_name(field: &rsharp_lsp::Location, name: &str, cx: &App) -> Option<rsharp_lsp::Location> {
    if !is_markup_compiled(&field.path) {
        return None;
    }
    let generated = std::fs::read_to_string(&field.path).ok()?;
    let (line, file) =
        generated.lines().take(field.span.start.line + 1).collect::<Vec<_>>().into_iter().rev().find_map(|l| {
            let (line, file) = l.trim().strip_prefix("#line ")?.split_once(' ')?;
            Some((line.parse::<usize>().ok()?, file.trim().trim_matches('"').to_string()))
        })?;
    let xaml = rsharp_core::paths::canonical(&field.path.parent()?.join(file.replace('\\', "/")));
    let text = match crate::buffer::find(&xaml, cx) {
        Some(open) => open.read(cx).document.text().to_string(),
        None => std::fs::read_to_string(&xaml).ok()?,
    };
    let written = text.lines().nth(line.checked_sub(1)?)?;
    let quoted = format!("Name=\"{name}\"");
    let column =
        written.find(&quoted).map_or(0, |at| written[..at + quoted.len() - name.len() - 1].encode_utf16().count());
    let at = rsharp_lsp::Point::new(line - 1, column);
    Some(rsharp_lsp::Location { path: xaml, span: rsharp_lsp::Span { start: at, end: at } })
}

impl CodeEditor {
    pub(super) fn is_xaml(&self) -> bool {
        rsharp_lsp::xaml::is_xaml(&self.path)
    }

    /// After typing in a XAML file: the list opened at a tag's `<`, a start tag's space, a value's quote or brace, a
    /// path's dot or a word's first letter, and kept to the word as it grows.
    pub(super) fn xaml_typed(&mut self, c: char, cx: &mut Context<Self>) {
        let completing = matches!(self.assist.popup, Some(Popup::Completion { .. }));
        let before = self.primary().head.checked_sub(c.len_utf8() + 1).map(|at| self.text.byte(at) as char);
        match c {
            c if completing
                && (c.is_alphanumeric() || matches!(c, '_' | '-') || self.assist.xaml_word_characters.contains(c)) =>
            {
                self.filter_completion(cx)
            }
            '<' | '"' | '\'' | '{' | '.' | ':' | ',' => self.xaml_complete(false, cx),
            ' ' if !completing => self.xaml_complete(false, cx),
            c if c.is_alphanumeric()
                && !matches!(before, Some(b) if b.is_alphanumeric() || matches!(b, '_' | '.' | ':')) =>
            {
                self.xaml_complete(false, cx)
            }
            _ if completing => self.close_popup(cx),
            _ => {}
        }
    }

    /// What may go where the caret is, asked of the XAML host on the text as typed.
    pub(super) fn xaml_complete(&mut self, explicit: bool, cx: &mut Context<Self>) {
        self.close_popup(cx);
        let Some((host, project)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text, rope) = (self.path.clone(), self.text.to_string(), self.text.clone());
        let at = language::point(&rope, self.primary().head);
        let found = cx.background_executor().spawn(async move { host.complete(&project, &path, &text, at) });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let list = match found.await {
                Ok(list) => list,
                Err(failed) => {
                    log::info!("XAML completion: {failed}");
                    None
                }
            };
            let _ = this.update(cx, |e, cx| {
                let Some(list) = list.filter(|l| !l.items.is_empty()) else {
                    if explicit {
                        cx.emit(Said("No suggestions here".into()));
                    }
                    return;
                };
                let start = offset(&e.text, list.start);
                e.assist.xaml_word_characters = list.word_characters;
                let head = e.primary().head;
                if head < start || e.text.byte_to_line(head) != e.text.byte_to_line(start) {
                    return;
                }
                let replaced = Span { start: list.start, end: at };
                let items = list
                    .items
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| CompletionItem {
                        filter_text: item.label.clone(),
                        // The host's own order, Passenger's, among names that match alike.
                        sort_text: format!("{i:05}"),
                        kind: kind(&item.kind),
                        detail: item.detail,
                        edit: Some(TextEdit { span: replaced, text: item.insert }),
                        additional: Vec::new(),
                        raw: json!({ "caret": item.caret, "reopen": item.reopen }),
                        label: item.label,
                    })
                    .collect();
                let list = CompletionList { incomplete: false, items };
                e.show(Popup::Completion { list, shown: Vec::new(), selected: 0, start, explicit }, start, cx);
                e.filter_completion(cx);
            });
        }));
    }

    /// The hover's words for the name under `at`: its type, a member's declaration, a resource's definitions.
    pub(super) fn xaml_card(&self, at: usize, cx: &App) -> Option<Task<Option<String>>> {
        let (host, project) = self.buffer.read(cx).xaml.clone()?;
        let (path, text, at) = (self.path.clone(), self.text.to_string(), language::point(&self.text, at));
        Some(cx.background_executor().spawn(async move {
            host.symbol(&project, &path, &text, at).ok().flatten().map(|s| s.description).filter(|d| !d.is_empty())
        }))
    }

    /// Find Usages (`rename` None) or Rename in XAML: an element's name, where the file names it and where its
    /// code-behind uses its field, which the window finds through Roslyn.
    pub(super) fn xaml_named(&mut self, rename: Option<String>, at: usize, cx: &mut Context<Self>) {
        let Some((host, project)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text) = (self.path.clone(), self.text.to_string());
        let at = language::point(&self.text, at);
        let found = cx.background_executor().spawn(async move { host.symbol(&project, &path, &text, at) });
        let xaml = self.path.clone();
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let named = found.await.ok().flatten().and_then(|s| s.named);
            let _ = this.update(cx, |_, cx| match named {
                Some(named) => cx.emit(CodeBehind { xaml, named, rename }),
                None if rename.is_some() => cx.emit(Said("Rename in XAML renames an element's name".into())),
                None => cx.emit(Said("Find Usages in XAML finds an element's name".into())),
            });
        }));
    }

    /// Alt+Enter in XAML: the fixes of the problems on the caret's line, the file read as it stands.
    pub(super) fn xaml_fixes(&mut self, cx: &mut Context<Self>) {
        let Some((host, project)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text, head) = (self.path.clone(), self.text.to_string(), self.primary().head);
        let line = self.text.byte_to_line(head);
        // A fix's edits to another open file (its view model) are made from that file as typed, not as saved.
        let folder = project.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let texts: std::collections::HashMap<PathBuf, String> = crate::buffer::in_folders(&[folder], cx)
            .into_iter()
            .filter(|b| b.read(cx).is_modified())
            .map(|b| (b.read(cx).path.clone(), b.read(cx).document.text().to_string()))
            .collect();
        let found = cx.background_executor().spawn(async move { host.fixes(&project, &path, &text, line, &texts) });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let actions = found.await.map(|a| rsharp_lsp::code_actions(&a)).unwrap_or_default();
            let _ = this.update(cx, |e, cx| {
                if actions.is_empty() {
                    cx.emit(Said("No fixes here".into()));
                } else if e.primary().head == head {
                    e.show(Popup::Actions { actions, selected: 0 }, head, cx);
                }
            });
        }));
    }

    /// Reformat Code in XAML: the selection's lines, or the whole file, by Rider's XAML code style with the file's
    /// .editorconfig over it (read as it is now), as one undo step.
    // shortcut: Rider's defaults stand under the .editorconfig, with no Settings page of their own (Passenger's Code
    // Style: XAML); add it once someone wants a style without an .editorconfig.
    pub(super) fn xaml_reformat(&mut self, cx: &mut Context<Self>) {
        let Some((host, _)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text, version) = (self.path.clone(), self.text.to_string(), self.version);
        let indent = self.buffer.read(cx).indent;
        let selection = self.primary();
        let range = (!selection.is_empty()).then(|| rsharp_lsp::Span {
            start: language::point(&self.text, selection.start()),
            end: language::point(&self.text, selection.end()),
        });
        let found = cx.background_executor().spawn(async move {
            // shortcut: values come lower case from RSharp's .editorconfig reading, so a custom
            // xaml_attribute_ordering_rule_groups loses its letter case; keep values' case once someone sets one.
            let config: serde_json::Map<String, serde_json::Value> =
                rsharp_core::editorconfig::EditorConfig::for_file(&path)
                    .properties()
                    .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string())))
                    .collect();
            host.format(&text, &config, (indent.size, indent.tabs), range)
        });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let formatted = match found.await {
                Ok(formatted) => formatted,
                Err(failed) => Err(failed.0),
            };
            let _ = this.update(cx, |e, cx| match formatted {
                Err(problem) => cx.emit(Said(format!("Reformat Code was not applied: {problem}"))),
                Ok(edits) if edits.is_empty() => cx.emit(Said("Already formatted".into())),
                Ok(_) if e.version != version => {
                    cx.emit(Said("Reformat Code was not applied: the file was edited meanwhile".into()))
                }
                Ok(edits) => {
                    let edits = language::edits(&e.text, &edits);
                    e.apply_server_edits(edits, cx);
                    cx.emit(Said("Reformatted".into()));
                }
            });
        }));
    }

    /// Extract Resource or Extract Style: what is at the caret asked of the host, then its dialog over the window, or
    /// why there is nothing to extract.
    // shortcut: no Refactor This list (Rider's Ctrl+T) offers them; add one once a second XAML refactoring needs it.
    pub(super) fn xaml_extract(&mut self, extract: crate::xaml_dialog::Extract, cx: &mut Context<Self>) {
        if !self.is_xaml() {
            cx.emit(Said(format!("{} is for XAML files", extract.title())));
            return;
        }
        let Some((host, project)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text) = (self.path.clone(), self.text.to_string());
        let at = language::point(&self.text, self.primary().head);
        let asked = (host.clone(), project.clone(), path.clone(), text.clone());
        let found = cx
            .background_executor()
            .spawn(async move { host.extract(&project, &path, &text, at, extract.kind(), serde_json::json!({})) });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |_, cx| match found {
                Ok(found) if found["why"].is_string() => {
                    cx.emit(Said(found["why"].as_str().unwrap_or_default().into()))
                }
                Ok(found) => {
                    let (host, project, path, text) = asked;
                    cx.emit(crate::xaml_dialog::OpenExtract { extract, host, project, path, text, at, found })
                }
                Err(failed) => cx.emit(Said(format!("{} is not available: {}", extract.title(), failed.0))),
            });
        }));
    }

    /// Go to Declaration in a XAML file: a type's or member's C#, a resource's definition, a named element, a file.
    pub(super) fn xaml_declaration(&mut self, cx: &mut Context<Self>) {
        let Some((host, project)) = self.buffer.read(cx).xaml.clone() else { return };
        let (path, text) = (self.path.clone(), self.text.to_string());
        let at = language::point(&self.text, self.primary().head);
        let found = cx.background_executor().spawn(async move { host.symbol(&project, &path, &text, at) });
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let to = found.await.ok().flatten().map(|s| s.targets).unwrap_or_default();
            let _ = this.update(cx, |_, cx| match to.is_empty() {
                true => cx.emit(Said("Cannot find declaration to go to".into())),
                false => cx.emit(Navigate { title: "Declaration".into(), to, list: false }),
            });
        }));
    }
}

/// Passenger.Xaml's item kinds as LSP's, which choose the list's badge.
fn kind(kind: &str) -> u64 {
    match kind {
        "Element" | "Type" => 7,
        "Property" | "AttachedProperty" => 10,
        "Event" => 23,
        "EnumValue" => 20,
        "Colour" => 16,
        "Namespace" => 9,
        "Resource" | "Extension" | "Directive" => 14,
        "Asset" => 17,
        _ => 12,
    }
}
