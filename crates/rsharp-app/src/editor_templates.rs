//! Rider's live templates: a template's shortcut typed and Tab pressed writes it at the line's indent, then Tab goes
//! from each of its variables to the next (every place one is used selected at once, so typing writes them all) and
//! last to where it ends. The built-in templates are Rider's, as Passenger carries them.

use super::*;

/// A template: its shortcut, and its text with `$NAME$` for each variable, `$END$` where it ends, a tab for each
/// indent; and each variable's default, in the order Tab visits them.
pub struct Template {
    pub shortcut: &'static str,
    /// What it is, as completion lists it beside its shortcut.
    pub name: &'static str,
    pub text: &'static str,
    pub defaults: &'static [(&'static str, &'static str)],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        shortcut: "for",
        name: "Simple \"for\" loop",
        text: "for (int $INDEX$ = 0; $INDEX$ < $UPPER$; $INDEX$++)\n{\n\t$END$\n}",
        defaults: &[("INDEX", "i"), ("UPPER", "length")],
    },
    Template {
        shortcut: "forr",
        name: "Reverse 'for' loop",
        text: "for (int $INDEX$ = $MAX$ - 1; $INDEX$ >= 0; $INDEX$--)\n{\n\t$END$\n}",
        defaults: &[("INDEX", "i"), ("MAX", "length")],
    },
    Template {
        shortcut: "foreach",
        name: "foreach block",
        text: "foreach (var $VARIABLE$ in $COLLECTION$)\n{\n\t$END$\n}",
        defaults: &[("VARIABLE", "item"), ("COLLECTION", "collection")],
    },
    Template {
        shortcut: "if",
        name: "if statement",
        text: "if ($CONDITION$)\n{\n\t$END$\n}",
        defaults: &[("CONDITION", "true")],
    },
    Template { shortcut: "else", name: "else statement", text: "else\n{\n\t$END$\n}", defaults: &[] },
    Template {
        shortcut: "while",
        name: "while loop",
        text: "while ($CONDITION$)\n{\n\t$END$\n}",
        defaults: &[("CONDITION", "true")],
    },
    Template {
        shortcut: "do",
        name: "do...while loop",
        text: "do\n{\n\t$END$\n} while ($CONDITION$);",
        defaults: &[("CONDITION", "true")],
    },
    Template {
        shortcut: "switch",
        name: "switch statement",
        text: "switch ($EXPRESSION$)\n{\n\t$END$\n}",
        defaults: &[("EXPRESSION", "expression")],
    },
    Template {
        shortcut: "try",
        name: "try catch",
        text: "try\n{\n\t$END$\n}\ncatch ($EXCEPTION$ $EX_NAME$)\n{\n\tConsole.WriteLine($EX_NAME$);\n\tthrow;\n}",
        defaults: &[("EXCEPTION", "Exception"), ("EX_NAME", "e")],
    },
    Template { shortcut: "tryf", name: "try finally", text: "try\n{\n\t$END$\n}\nfinally\n{\n}", defaults: &[] },
    Template {
        shortcut: "using",
        name: "using statement",
        text: "using ($RESOURCE$)\n{\n\t$END$\n}",
        defaults: &[("RESOURCE", "resource")],
    },
    Template {
        shortcut: "lock",
        name: "lock statement",
        text: "lock ($EXPRESSION$)\n{\n\t$END$\n}",
        defaults: &[("EXPRESSION", "this")],
    },
    Template { shortcut: "cw", name: "Console.WriteLine", text: "Console.WriteLine($END$);", defaults: &[] },
    Template {
        shortcut: "thr",
        name: "throw new",
        text: "throw new $EXCEPTION$($END$);",
        defaults: &[("EXCEPTION", "InvalidOperationException")],
    },
    Template {
        shortcut: "prop",
        name: "Property",
        text: "public $TYPE$ $NAME$ { get; set; }$END$",
        defaults: &[("TYPE", "int"), ("NAME", "Name")],
    },
    Template {
        shortcut: "propg",
        name: "Property with a 'get' accessor",
        text: "public $TYPE$ $NAME$ { get; }$END$",
        defaults: &[("TYPE", "int"), ("NAME", "Name")],
    },
    Template { shortcut: "ctor", name: "Constructor", text: "public $TYPE_NAME$($END$)\n{\n}", defaults: &[] },
    Template {
        shortcut: "svm",
        name: "'void Main' method",
        text: "static void Main(string[] args)\n{\n\t$END$\n}",
        defaults: &[],
    },
    Template {
        shortcut: "sim",
        name: "'int Main' method",
        text: "static int Main(string[] args)\n{\n\t$END$\n\treturn 0;\n}",
        defaults: &[],
    },
    Template {
        shortcut: "class",
        name: "Class",
        text: "public class $NAME$\n{\n\t$END$\n}",
        defaults: &[("NAME", "Name")],
    },
    Template {
        shortcut: "record",
        name: "Record",
        text: "public record $NAME$($END$);",
        defaults: &[("NAME", "Name")],
    },
    Template {
        shortcut: "interface",
        name: "Interface",
        text: "public interface $NAME$\n{\n\t$END$\n}",
        defaults: &[("NAME", "IName")],
    },
    Template {
        shortcut: "struct",
        name: "Struct",
        text: "public struct $NAME$\n{\n\t$END$\n}",
        defaults: &[("NAME", "Name")],
    },
    Template {
        shortcut: "enum",
        name: "Enum",
        text: "public enum $NAME$\n{\n\t$END$\n}",
        defaults: &[("NAME", "Name")],
    },
];

/// A template being filled in: each variable still to visit, every place it is used, and where it ends.
#[derive(Clone, Debug, Default)]
pub struct Filling {
    pub stops: Vec<Vec<Range<usize>>>,
    pub end: usize,
}

impl Filling {
    pub fn map_through(&mut self, change: &Change) {
        for ranges in &mut self.stops {
            for range in ranges.iter_mut() {
                // A change inside a variable (what is typed over it) widens it rather than moving its start.
                let start =
                    if change.start.offset >= range.start { range.start } else { map_through(range.start, change) };
                *range = start..map_through(range.end, change).max(start);
            }
        }
        self.end = map_through(self.end, change);
    }
}

/// A template written out at `indent` (`unit` for each tab), the type's name in for `$TYPE_NAME$`: its text, each
/// variable's ranges in it in the order Tab visits them, and where it ends.
pub fn expand(
    template: &Template,
    indent: &str,
    unit: &str,
    type_name: &str,
) -> (String, Vec<Vec<Range<usize>>>, usize) {
    let mut text = String::new();
    let mut found: Vec<(&str, Range<usize>)> = Vec::new();
    let mut end = None;
    let source = template.text.replace('\t', unit).replace('\n', &format!("\n{indent}"));
    let mut rest = source.as_str();
    while let Some(open) = rest.find('$') {
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('$') else { break };
        let name = &after[..close];
        match name {
            "END" => end = Some(text.len()),
            "TYPE_NAME" => text.push_str(type_name),
            _ => {
                let value = template.defaults.iter().find(|(n, _)| *n == name).map_or(name, |(_, v)| v);
                found.push((name, text.len()..text.len() + value.len()));
                text.push_str(value);
            }
        }
        rest = &after[close + 1..];
    }
    text.push_str(rest);
    let stops = template
        .defaults
        .iter()
        .map(|(name, _)| found.iter().filter(|(n, _)| n == name).map(|(_, r)| r.clone()).collect::<Vec<_>>())
        .filter(|r| !r.is_empty())
        .collect();
    let end = end.unwrap_or(text.len());
    (text, stops, end)
}

/// The name of the type a place is inside, as `ctor` takes it: the nearest `class`, `struct` or `record` before it.
// shortcut: by the text before it, so a nested type's end is not seen; ask Roslyn once a constructor lands in the
// wrong type.
fn type_name_before(text: &str) -> String {
    ["class ", "struct ", "record "]
        .iter()
        .filter_map(|k| text.rfind(k).map(|at| (at, &text[at + k.len()..])))
        .max_by_key(|(at, _)| *at)
        .map(|(_, after)| after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect())
        .unwrap_or_default()
}

impl CodeEditor {
    /// Tab after a template's shortcut: the template written over it, its first variable selected. False when the
    /// word before the caret is none's.
    pub(super) fn expand_template(&mut self, cx: &mut Context<Self>) -> bool {
        let selection = self.primary();
        if !selection.is_empty() || self.carets.as_slice().len() > 1 {
            return false;
        }
        let head = selection.head;
        let text = self.text.clone();
        let mut start = text.byte_to_char(head);
        while start > 0 && text.char(start - 1).is_alphanumeric() {
            start -= 1;
        }
        let start = text.char_to_byte(start);
        let word = text.byte_slice(start..head).to_string();
        let Some(template) = TEMPLATES.iter().find(|t| t.shortcut == word) else { return false };
        let line = text.byte_to_line(head);
        let line_start = text.line_to_byte(line);
        // Only a shortcut standing alone where a statement or a member begins, never the end of a name.
        if !text.byte_slice(line_start..start).to_string().trim().is_empty() {
            return false;
        }
        let indent = text.byte_slice(line_start..start).to_string();
        let unit = self.buffer.read(cx).indent.unit();
        let type_name = type_name_before(&text.byte_slice(..start).to_string());
        let (written, stops, end) = expand(template, &indent, &unit, &type_name);
        let place = |r: &Range<usize>| start + r.start..start + r.end;
        let stops: Vec<Vec<Range<usize>>> = stops.iter().map(|ranges| ranges.iter().map(place).collect()).collect();
        let first: Vec<Selection> = match stops.first() {
            Some(ranges) => ranges.iter().map(|r| Selection { anchor: r.start, head: r.end }).collect(),
            None => vec![Selection::caret(start + end)],
        };
        let plan = EditPlan {
            edits: vec![Edit { range: start..head, text: written }],
            selections: first,
            ..Default::default()
        };
        self.apply(plan, cx);
        let remaining = Filling { stops: stops.into_iter().skip(1).collect(), end: start + end };
        self.assist.template =
            (!remaining.stops.is_empty() || self.primary().head != remaining.end).then_some(remaining);
        true
    }

    /// Tab while a template is being filled in: its next variable selected, or the caret where it ends. False with
    /// none being filled in.
    pub(super) fn next_template_stop(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(filling) = self.assist.template.as_mut() else { return false };
        let carets: Vec<Selection> = if filling.stops.is_empty() {
            let end = filling.end;
            self.assist.template = None;
            vec![Selection::caret(end)]
        } else {
            filling.stops.remove(0).iter().map(|r| Selection { anchor: r.start, head: r.end }).collect()
        };
        self.set_carets(carets, cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_written_at_its_indent_with_every_use_of_a_variable_one_stop() {
        let for_loop = TEMPLATES.iter().find(|t| t.shortcut == "for").unwrap();
        let (text, stops, end) = expand(for_loop, "    ", "    ", "");
        assert_eq!(text, "for (int i = 0; i < length; i++)\n    {\n        \n    }");
        assert_eq!(stops[0].iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["i", "i", "i"]);
        assert_eq!(&text[stops[1][0].clone()], "length");
        assert_eq!(&text[..end], "for (int i = 0; i < length; i++)\n    {\n        ");
        let ctor = TEMPLATES.iter().find(|t| t.shortcut == "ctor").unwrap();
        assert_eq!(expand(ctor, "", "    ", "Shop").0, "public Shop()\n{\n}");
    }
}
