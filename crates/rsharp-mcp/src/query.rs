//! How an agent names an app's elements (Passenger's AppQuery, ported from its src/Passenger.App/LiveTree/AppQuery.cs):
//! steps joined by `/`, each finding elements inside the last one's: `@12` (an id the tree shows), or a type, `#name`,
//! `.class`es and `"text"` (what it shows) in any mix, with `[n]` for the nth of its matches from 1; a bare word no type
//! has is taken as a name. A step finds the outermost elements matching it, so `"Save"` is the button, not the text
//! inside it, and a template's parts only by #name or id. The tree is the inspector's, as JSON.

use serde_json::Value;

/// An element of the tree, its parent's place among the nodes, and whether it shows (it and every ancestor visible).
pub struct Node<'a> {
    pub element: &'a Value,
    parent: Option<usize>,
    shows: bool,
}

/// Every element under the windows given, in tree order, each before what it holds.
pub fn flatten(roots: &[Value]) -> Vec<Node<'_>> {
    fn add<'a>(element: &'a Value, parent: Option<usize>, shows: bool, all: &mut Vec<Node<'a>>) {
        let shows = shows && element["visible"] != false;
        all.push(Node { element, parent, shows });
        let at = all.len() - 1;
        for child in element["children"].as_array().into_iter().flatten() {
            add(child, Some(at), shows, all);
        }
    }
    let mut all = Vec::new();
    for root in roots {
        add(root, None, true, &mut all);
    }
    all
}

fn inside(nodes: &[Node], node: usize, of: usize) -> bool {
    let mut at = nodes[node].parent;
    while let Some(parent) = at {
        if parent == of {
            return true;
        }
        at = nodes[parent].parent;
    }
    false
}

#[derive(Clone, Debug, Default)]
struct Step {
    id: Option<i64>,
    kind: Option<String>,
    name: Option<String>,
    classes: Vec<String>,
    shown: Option<String>,
    index: Option<usize>,
    written: String,
}

impl Step {
    fn matches(&self, element: &Value) -> bool {
        if let Some(id) = self.id {
            return element["id"].as_i64() == Some(id);
        }
        let text = |key: &str| element[key].as_str();
        (element["fromTemplate"] != true || self.name.is_some())
            && self.kind.as_ref().is_none_or(|k| text("type").is_some_and(|t| t.eq_ignore_ascii_case(k)))
            && self.name.as_ref().is_none_or(|n| text("name") == Some(n.as_str()))
            && self
                .classes
                .iter()
                .all(|c| element["classes"].as_array().is_some_and(|cs| cs.iter().any(|x| x == c.as_str())))
            && self.shown.as_ref().is_none_or(|s| text("text").is_some_and(|t| t.trim().eq_ignore_ascii_case(s.trim())))
    }
}

/// The elements a query finds, by their places among `nodes`, in tree order.
pub fn find(nodes: &[Node], query: &str) -> Result<Vec<usize>, String> {
    let mut found: Vec<usize> = Vec::new();
    for (i, step) in parse(query)?.into_iter().enumerate() {
        let scope: Vec<usize> =
            (0..nodes.len()).filter(|&n| i == 0 || found.iter().any(|&under| inside(nodes, n, under))).collect();
        let mut matches: Vec<usize> = scope.iter().copied().filter(|&n| step.matches(nodes[n].element)).collect();
        // A bare word that is no type of the tree's is a name.
        if matches.is_empty()
            && step.name.is_none()
            && step.classes.is_empty()
            && step.shown.is_none()
            && step.id.is_none()
        {
            if let Some(word) = &step.kind {
                let named = Step { kind: None, name: Some(word.clone()), ..step.clone() };
                matches = scope.iter().copied().filter(|&n| named.matches(nodes[n].element)).collect();
            }
        }
        let outermost: Vec<usize> =
            matches.iter().copied().filter(|&m| !matches.iter().any(|&o| inside(nodes, m, o))).collect();
        found = match step.index {
            Some(n) if n >= 1 && n <= outermost.len() => vec![outermost[n - 1]],
            Some(n) => {
                let s = if outermost.len() == 1 { "" } else { "es" };
                return Err(format!("{} has {} match{s}, so no [{n}]", step.written, outermost.len()));
            }
            None => outermost,
        };
    }
    Ok(found)
}

/// The one element a query names: of several, the one a person can see when only one is, else why not, listing them.
pub fn one(nodes: &[Node], query: &str) -> Result<usize, String> {
    let mut found = find(nodes, query)?;
    let seen: Vec<usize> = found
        .iter()
        .copied()
        .filter(|&n| {
            nodes[n].shows && nodes[n].element["outOfView"] != true && nodes[n].element["fromTemplate"] != true
        })
        .collect();
    if found.len() > 1 && seen.len() == 1 {
        found = seen;
    }
    match found[..] {
        [only] => Ok(only),
        [] => Err(format!("Nothing in the app matches {query}; app_tree shows what there is")),
        _ => {
            let listed: Vec<String> = found.iter().take(10).map(|&n| short(nodes[n].element)).collect();
            Err(format!(
                "{} elements match {query}: {}. Name one by its id (@N) or by [n] after the query",
                found.len(),
                listed.join("; ")
            ))
        }
    }
}

/// An element as an answer names it in passing: its type, #name and @id.
pub fn short(element: &Value) -> String {
    let name = element["name"].as_str().map(|n| format!("#{n}")).unwrap_or_default();
    format!("{}{name} @{}", element["type"].as_str().unwrap_or_default(), element["id"])
}

fn parse(query: &str) -> Result<Vec<Step>, String> {
    let chars: Vec<char> = query.trim().chars().collect();
    let mut steps = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        steps.push(step(&chars, &mut i, query)?);
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() {
            if chars[i] != '/' {
                return Err(format!(
                    "In {query}, '{}' at {} is not a step's part: steps are joined by /",
                    chars[i],
                    i + 1
                ));
            }
            i += 1;
        }
    }
    if steps.is_empty() { Err("An empty query names nothing".into()) } else { Ok(steps) }
}

fn word(chars: &[char], i: &mut usize, extra: &str) -> String {
    let start = *i;
    while *i < chars.len()
        && (chars[*i].is_alphanumeric() || matches!(chars[*i], '_' | '`') || extra.contains(chars[*i]))
    {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

fn step(chars: &[char], i: &mut usize, query: &str) -> Result<Step, String> {
    let start = *i;
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
    let written = |i: usize| chars[start..i].iter().collect::<String>().trim().to_string();
    if chars.get(*i) == Some(&'@') {
        *i += 1;
        let digits = word(chars, i, "");
        let id = digits.parse().map_err(|_| format!("In {query}, @{digits} is no element's id"))?;
        return Ok(Step { id: Some(id), written: written(*i), ..Step::default() });
    }
    let mut step = Step::default();
    if chars.get(*i).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
        step.kind = Some(word(chars, i, ""));
    }
    while *i < chars.len() {
        match chars[*i] {
            '#' => {
                *i += 1;
                step.name = Some(word(chars, i, ""));
            }
            '.' => {
                *i += 1;
                step.classes.push(word(chars, i, "-:"));
            }
            '"' => {
                let mut value = String::new();
                *i += 1;
                loop {
                    match chars.get(*i) {
                        None => return Err(format!("In {query}, a \"text\" is not closed")),
                        Some('\\') if *i + 1 < chars.len() => {
                            value.push(chars[*i + 1]);
                            *i += 2;
                        }
                        Some('"') => {
                            *i += 1;
                            break;
                        }
                        Some(c) => {
                            value.push(*c);
                            *i += 1;
                        }
                    }
                }
                step.shown = Some(value);
            }
            '[' => {
                let close = chars[*i..].iter().position(|c| *c == ']').map(|p| *i + p);
                let number: Option<usize> =
                    close.and_then(|close| chars[*i + 1..close].iter().collect::<String>().parse().ok());
                match (close, number) {
                    (Some(close), Some(n)) => {
                        step.index = Some(n);
                        *i = close + 1;
                    }
                    _ => return Err(format!("In {query}, [n] takes a number from 1")),
                }
            }
            c if c.is_whitespace() && chars[*i + 1..].iter().find(|c| !c.is_whitespace()) == Some(&'"') => *i += 1,
            _ => break,
        }
    }
    if step.kind.is_none() && step.name.is_none() && step.classes.is_empty() && step.shown.is_none() {
        return Err(format!(
            "In {query}, a step at {} names nothing: give a type, #name, .class, \"text\" or @id",
            start + 1
        ));
    }
    step.written = written(*i);
    Ok(step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree() -> Vec<Value> {
        let text = |id: i64, shown: &str| json!({ "id": id, "type": "TextBlock", "text": shown, "classes": [], "children": [] });
        let button = |id: i64, name: &str, shown: &str| json!({ "id": id, "type": "Button", "name": name, "text": shown, "classes": ["danger"], "children": [text(id + 1, shown)] });
        vec![json!({ "id": 1, "type": "Window", "name": null, "classes": [], "children": [
            { "id": 2, "type": "StackPanel", "name": "Panel", "classes": [], "children": [
                text(3, "Hello"), button(4, "Save", "Save"), button(6, "Delete", "Delete") ] } ] })]
    }

    #[test]
    fn a_query_names_elements_as_passengers_does() {
        let roots = tree();
        let nodes = flatten(&roots);
        let id = |query: &str| one(&nodes, query).map(|n| nodes[n].element["id"].as_i64().unwrap());
        assert_eq!(id("\"Save\""), Ok(4), "the text found the text inside the button, not the outermost button");
        assert_eq!(id("Button#Delete"), Ok(6));
        assert_eq!(id("Save"), Ok(4), "a bare word that is no type is a name");
        assert_eq!(id("#Panel/Button[2]"), Ok(6), "a step inside the last, the second of its matches");
        assert_eq!(id("@3"), Ok(3));
        assert!(id("Button.danger").unwrap_err().contains("2 elements match"), "two matches named no one element");
        assert!(id("Button[3]").unwrap_err().contains("has 2 matches"));
        assert!(id("\"Save").unwrap_err().contains("not closed"));
    }
}
