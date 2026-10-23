//! Rider's named refactorings over the editor: those Roslyn offers as code actions (Optimize Usings, Introduce
//! Variable, Field and Parameter, Encapsulate Field, Extract Method, Inline), each run by the title Roslyn gives it;
//! and those the editor makes itself: Safe Delete, Surround With and Complete Current Statement.

use super::assist::{Navigate, Popup, Said};
use super::*;
use crate::language::{self, ask};
use rsharp_lsp::{self as lsp, CodeAction};
use serde_json::json;

/// Surround With's constructs, as Rider's list names them, each written round the lines with `$` where they go and
/// `@` the placeholder selected after.
pub const SURROUNDS: &[(&str, &str)] = &[
    ("if", "if (@condition)\n{\n$\n}"),
    ("if / else", "if (@condition)\n{\n$\n}\nelse\n{\n}"),
    ("while", "while (@condition)\n{\n$\n}"),
    ("for", "for (var i = 0; i < @length; i++)\n{\n$\n}"),
    ("foreach", "foreach (var item in @items)\n{\n$\n}"),
    ("try / catch", "try\n{\n$\n}\ncatch (@Exception e)\n{\n}"),
    ("try / finally", "try\n{\n$\n}\nfinally\n{\n@\n}"),
    ("using", "using (@resource)\n{\n$\n}"),
    ("lock", "lock (@this)\n{\n$\n}"),
    ("#region", "#region @Region\n$\n#endregion"),
    ("{ }", "{\n$@\n}"),
];

/// The code actions that are a named refactoring: their titles start so, Roslyn's Fix All left out.
fn named<'a>(actions: &'a [CodeAction], prefixes: &[&str]) -> Vec<&'a CodeAction> {
    actions.iter().filter(|a| prefixes.iter().any(|p| a.title.starts_with(p))).collect()
}

impl CodeEditor {
    /// A refactoring Roslyn offers as a code action, run where the selection (or the caret) is: one action is made,
    /// several are listed, none is said. Each step of `steps` is asked again of the text the step before left.
    pub(super) fn named_refactoring(
        &mut self,
        name: &'static str,
        steps: &'static [&'static [&'static str]],
        cx: &mut Context<Self>,
    ) {
        let Some((server, uri)) = self.server(cx) else { return };
        let selection = self.primary();
        let head = selection.head;
        let mut span = lsp::Span {
            start: language::point(&self.text, selection.start()),
            end: language::point(&self.text, selection.end()),
        };
        // Optimize Usings works on the file whatever the caret is on: its usings are at its top.
        if name == "Optimize Usings" {
            span = lsp::Span::default();
        }
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let mut made_any = false;
            for prefixes in steps {
                let params = json!({ "textDocument": { "uri": uri }, "range": span.to_json(), "context": { "diagnostics": [] } });
                let Ok(answer) = this.update(cx, |_, cx| ask(&server, "textDocument/codeAction", params, cx)) else { return };
                let actions = answer.await.map(|a| lsp::code_actions(&a)).unwrap_or_default();
                let found: Vec<CodeAction> = named(&actions, prefixes).into_iter().cloned().collect();
                match found.as_slice() {
                    [] => continue,
                    [one] => {
                        made_any = true;
                        let Ok(resolved) = this.update(cx, |_, cx| ask(&server, "codeAction/resolve", one.raw.clone(), cx)) else {
                            return;
                        };
                        let Some(resolved) = resolved.await else { continue };
                        let ended = this.update(cx, |e, cx| {
                            let nested = lsp::code_actions(&resolved["command"]["arguments"][0]["NestedCodeActions"]);
                            if !nested.is_empty() {
                                e.show(Popup::Actions { actions: nested, selected: 0 }, head, cx);
                                return true;
                            }
                            if let Err(why) = language::apply_workspace_edit(&lsp::workspace_edit(&resolved["edit"]), Some(&server), cx) {
                                cx.emit(Said(why));
                                return true;
                            }
                            false
                        });
                        if ended.unwrap_or(true) {
                            return;
                        }
                    }
                    several => {
                        let actions = several.to_vec();
                        let _ = this.update(cx, |e, cx| e.show(Popup::Actions { actions, selected: 0 }, head, cx));
                        return;
                    }
                }
            }
            if !made_any {
                let _ = this.update(cx, |_, cx| cx.emit(Said(format!("{name} is not available here"))));
            }
        }));
    }

    /// Safe Delete at a declaration: deleted, its whole lines, when nothing uses it; else its usages listed and it kept.
    // shortcut: from its declaration only, and without Passenger's search of what deleting it would break beyond its
    // usages (an override, an implementation); widen it with Roslyn.Extension (decision 1).
    pub(super) fn safe_delete(&mut self, cx: &mut Context<Self>) {
        let Some((server, uri)) = self.server(cx) else { return };
        let head = self.primary().head;
        let here = language::point(&self.text, head);
        let name = self.text.byte_slice(word_at(&self.text, head).range()).to_string();
        let mut params = lsp::at(&uri, here);
        params["context"] = json!({ "includeDeclaration": false });
        let usages = ask(&server, "textDocument/references", params, cx);
        let symbols = ask(&server, "textDocument/documentSymbol", json!({ "textDocument": { "uri": uri } }), cx);
        let version = self.version;
        self.assist.popup_task = Some(cx.spawn(async move |this, cx| {
            let used = usages.await.map(|u| lsp::locations(&u)).unwrap_or_default();
            let symbols = symbols.await.unwrap_or_default();
            let _ = this.update(cx, |e, cx| {
                if e.version != version {
                    return;
                }
                if !used.is_empty() {
                    cx.emit(Said(format!("{name} is used in {} places, so it was kept", used.len())));
                    cx.emit(Navigate { title: format!("Usages of {name}"), to: used, list: true });
                    return;
                }
                let Some(whole) = declaration_at(&symbols, here) else {
                    cx.emit(Said("Safe Delete works at a declaration".into()));
                    return;
                };
                // The declaration's whole lines, so no blank line or indent is left where it was.
                let text = &e.text;
                let (first, last) = (whole.start.line, whole.end.line);
                let start = text.line_to_byte(first);
                let end = if last + 1 < text.len_lines() { text.line_to_byte(last + 1) } else { text.len_bytes() };
                let plan = EditPlan {
                    edits: vec![Edit { range: start..end, text: String::new() }],
                    selections: vec![Selection::caret(start)],
                    ..Default::default()
                };
                e.apply(plan, cx);
            });
        }));
    }

    /// Surround With: its constructs listed over the selection.
    pub(super) fn open_surround(&mut self, cx: &mut Context<Self>) {
        let at = self.primary().start();
        self.show(Popup::Surround { selected: 0 }, at, cx);
    }

    /// The selected lines (or the caret's) written inside a construct at their indent, its placeholder selected.
    pub(super) fn surround(&mut self, chosen: usize, cx: &mut Context<Self>) {
        let Some((_, template)) = SURROUNDS.get(chosen) else { return };
        let text = self.text.clone();
        let selection = self.primary();
        let first = text.byte_to_line(selection.start());
        // A selection ending at a line's start does not take that line in.
        let end_line = text.byte_to_line(selection.end());
        let last =
            if end_line > first && selection.end() == text.line_to_byte(end_line) { end_line - 1 } else { end_line };
        let start = text.line_to_byte(first);
        let end = line_end(&text, last);
        let lines: Vec<String> = text.byte_slice(start..end).to_string().split('\n').map(str::to_string).collect();
        let outer: String = lines[0].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let unit = self.buffer.read(cx).indent.unit();
        let inner: Vec<String> =
            lines.iter().map(|l| if l.trim().is_empty() { String::new() } else { format!("{unit}{l}") }).collect();
        let mut written = String::new();
        let mut placeholder = None;
        for (i, line) in template.split('\n').enumerate() {
            if i > 0 {
                written.push('\n');
            }
            if line.starts_with('$') {
                written.push_str(&inner.join("\n"));
                if line.contains('@') {
                    placeholder = Some((written.len(), 0));
                }
                continue;
            }
            let line_start = written.len() + outer.len();
            written.push_str(&outer);
            match line.find('@') {
                Some(at) => {
                    let word: String = line[at + 1..].chars().take_while(|c| c.is_alphanumeric()).collect();
                    placeholder = Some((line_start + at, word.len()));
                    written.push_str(&line.replacen('@', "", 1));
                }
                None => written.push_str(line),
            }
        }
        let (at, len) = placeholder.unwrap_or((written.len(), 0));
        let plan = EditPlan {
            edits: vec![Edit { range: start..end, text: written }],
            selections: vec![Selection { anchor: start + at, head: start + at + len }],
            ..Default::default()
        };
        self.apply(plan, cx);
    }

    /// Complete Current Statement: the caret's line closed as Rider closes it, then the caret on the line after.
    pub(super) fn complete_statement(&mut self, cx: &mut Context<Self>) {
        let text = self.text.clone();
        let line = text.byte_to_line(self.primary().head);
        let end = line_end(&text, line);
        let content = text.byte_slice(text.line_to_byte(line)..end).to_string();
        let unit = self.buffer.read(cx).indent.unit();
        let (added, caret) = complete(&content, &unit);
        let plan = EditPlan {
            edits: vec![Edit { range: end..end, text: added }],
            selections: vec![Selection::caret(end + caret)],
            ..Default::default()
        };
        self.apply(plan, cx);
    }
}

/// The range of the declaration whose name is at `here` in a documentSymbol answer.
fn declaration_at(symbols: &serde_json::Value, here: lsp::Point) -> Option<lsp::Span> {
    for symbol in symbols.as_array().into_iter().flatten() {
        if let Some(found) = declaration_at(&symbol["children"], here) {
            return Some(found);
        }
        let point = |v: &serde_json::Value| {
            Some(lsp::Point::new(v["line"].as_u64()? as usize, v["character"].as_u64()? as usize))
        };
        let (Some(start), Some(end)) =
            (point(&symbol["selectionRange"]["start"]), point(&symbol["selectionRange"]["end"]))
        else {
            continue;
        };
        if start <= here && here <= end {
            return Some(lsp::Span { start: point(&symbol["range"]["start"])?, end: point(&symbol["range"]["end"])? });
        }
    }
    None
}

/// What Complete Current Statement adds at a line's end, and where the caret goes in it: a header (`if (…)`,
/// `foreach (…)`, `try`) gets its block, its missing `)` first; a statement its `;`; a finished line just the next.
fn complete(content: &str, unit: &str) -> (String, usize) {
    let trimmed = content.trim();
    let indent: String = content.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let open = content.matches('(').count().saturating_sub(content.matches(')').count());
    let closing = ")".repeat(open);
    let headers =
        ["if ", "if(", "for ", "for(", "foreach", "while", "switch", "using (", "using(", "lock", "catch", "else if"];
    let bare = ["try", "finally", "else", "do"];
    if headers.iter().any(|h| trimmed.starts_with(h)) || bare.contains(&trimmed) {
        let head = format!("{closing}\n{indent}{{\n{indent}{unit}");
        let caret = head.len();
        return (format!("{head}\n{indent}}}"), caret);
    }
    let finished = trimmed.is_empty() || trimmed.ends_with([';', '{', '}']) || trimmed.starts_with("//");
    let added = if finished { format!("{closing}\n{indent}") } else { format!("{closing};\n{indent}") };
    let caret = added.len();
    (added, caret)
}

#[cfg(test)]
mod tests {
    use super::complete;

    #[test]
    fn a_statement_gets_its_semicolon_and_a_header_its_block_its_parentheses_closed_first() {
        assert_eq!(complete("    var x = f(1", "    "), (");\n    ".into(), 7));
        let (added, caret) = complete("    if (ready", "    ");
        assert_eq!(added, ")\n    {\n        \n    }");
        assert_eq!(&added[..caret], ")\n    {\n        ", "the caret is not inside the new block");
        assert_eq!(complete("    return;", "    ").0, "\n    ", "a finished line got a second semicolon");
    }
}
