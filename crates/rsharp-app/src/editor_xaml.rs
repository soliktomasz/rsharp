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
        let found = cx.background_executor().spawn(async move { host.fixes(&project, &path, &text, line) });
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
