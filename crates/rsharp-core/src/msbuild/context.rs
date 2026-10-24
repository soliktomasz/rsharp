//! Where the caret is in an MSBuild file's text as typed, half-written markup included: an element's name, an
//! attribute's name or value, an element's text, or a `$(`, `@(` or `%(` reference, with the elements it is inside.

/// The elements open around a place, outermost first.
pub type Path = Vec<String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// Where its `<` is.
    pub start: usize,
    /// The attributes written before the caret, with their values.
    pub attributes: Vec<(String, String)>,
}

impl Tag {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A start or end tag's name being written, from `start`.
    ElementName {
        path: Path,
        start: usize,
        closing: bool,
    },
    AttributeName {
        tag: Tag,
        path: Path,
        start: usize,
    },
    /// Inside an attribute's quotes: the value from `start` to `end`, its closing quote (or the caret, without one).
    AttributeValue {
        tag: Tag,
        path: Path,
        attribute: String,
        start: usize,
        end: usize,
    },
    /// An element's text, from `start` (after its `>`).
    Text {
        path: Path,
        start: usize,
    },
    /// A `$(`, `@(` or `%(` reference's name being written, from `start` (after its parenthesis).
    Reference {
        kind: char,
        start: usize,
    },
    /// A comment, a CDATA section, a processing instruction, or nothing to help with.
    None,
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':')
}

/// The open reference before `at` in `segment` (which starts at `from`), if its name is still being written.
fn reference(text: &str, from: usize, at: usize) -> Option<Place> {
    let bytes = text.as_bytes();
    let mut name = at;
    while name > from && (bytes[name - 1].is_ascii_alphanumeric() || bytes[name - 1] == b'_') {
        name -= 1;
    }
    if name >= from + 2 && bytes[name - 1] == b'(' && matches!(bytes[name - 2], b'$' | b'@' | b'%') {
        return Some(Place::Reference { kind: bytes[name - 2] as char, start: name });
    }
    None
}

/// Where `at` is in `text`, read from the start: comments, CDATA and declarations passed over, elements opened and
/// closed (a closing tag closing every element up to its name, as tolerant readers do).
// shortcut: the text is read from its start on each ask, fine for project files of a few thousand lines; read from
// a nearby known place when a project file that large needs it.
pub fn at(text: &str, at: usize) -> Place {
    let bytes = text.as_bytes();
    let at = at.min(bytes.len());
    let mut path: Path = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < at {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let skip = |open: &str, close: &str| -> Option<Option<usize>> {
            text[i..]
                .starts_with(open)
                .then(|| text[i + open.len()..].find(close).map(|e| i + open.len() + e + close.len()))
        };
        if let Some(end) = skip("<!--", "-->")
            .or_else(|| skip("<![CDATA[", "]]>"))
            .or_else(|| skip("<?", "?>"))
            .or_else(|| skip("<!", ">"))
        {
            match end {
                Some(end) if end <= at => {
                    i = end;
                    text_start = end;
                    continue;
                }
                _ => return Place::None,
            }
        }
        let start = i;
        let closing = bytes.get(i + 1) == Some(&b'/');
        let name_start = i + 1 + usize::from(closing);
        let mut j = name_start;
        while j < bytes.len() && is_name(bytes[j]) {
            j += 1;
        }
        if at <= j {
            return Place::ElementName { path, start: name_start, closing };
        }
        let name = text[name_start..j].to_string();
        if closing {
            let end = text[j..].find('>').map(|e| j + e + 1);
            match end {
                Some(end) if end <= at => {
                    if let Some(open) = path.iter().rposition(|p| p == &name) {
                        path.truncate(open);
                    }
                    i = end;
                    text_start = end;
                    continue;
                }
                _ => return Place::None,
            }
        }
        // The start tag's attributes, up to its end or the caret.
        let mut tag = Tag { name: name.clone(), start, attributes: Vec::new() };
        loop {
            while j < at && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= at {
                return Place::AttributeName { tag, path, start: at };
            }
            match bytes[j] {
                b'>' => {
                    path.push(name);
                    i = j + 1;
                    text_start = i;
                    break;
                }
                b'/' if bytes.get(j + 1) == Some(&b'>') => {
                    i = j + 2;
                    text_start = i;
                    break;
                }
                c if is_name(c) => {
                    let attribute_start = j;
                    while j < bytes.len() && is_name(bytes[j]) {
                        j += 1;
                    }
                    if at <= j {
                        return Place::AttributeName { tag, path, start: attribute_start };
                    }
                    let attribute = text[attribute_start..j].to_string();
                    while j < at && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= at || bytes[j] != b'=' {
                        tag.attributes.push((attribute, String::new()));
                        continue;
                    }
                    j += 1;
                    while j < at && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= at || !matches!(bytes[j], b'"' | b'\'') {
                        if j >= at {
                            return Place::None;
                        }
                        continue;
                    }
                    let quote = bytes[j];
                    let value_start = j + 1;
                    let close = text[value_start..].find(quote as char).map(|e| value_start + e);
                    if close.is_none_or(|c| at <= c) {
                        if let Some(found) = reference(text, value_start, at) {
                            return found;
                        }
                        let end = close.unwrap_or(at);
                        return Place::AttributeValue { tag, path, attribute, start: value_start, end };
                    }
                    let close = close.unwrap();
                    tag.attributes.push((attribute, text[value_start..close].to_string()));
                    j = close + 1;
                }
                _ => j += 1,
            }
        }
    }
    if let Some(found) = reference(text, text_start, at) {
        return found;
    }
    Place::Text { path, start: text_start }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(marked: &str) -> Place {
        let caret = marked.find('|').unwrap();
        at(&marked.replace('|', ""), caret)
    }

    #[test]
    fn half_written_markup_says_where_the_caret_is() {
        let path = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            place("<Project>\n  <PropertyGroup>\n    <Targ|"),
            Place::ElementName { path: path(&["Project", "PropertyGroup"]), start: 33, closing: false }
        );
        assert_eq!(place("<Project><!-- <Prop| -->"), Place::None);
        assert_eq!(
            place("<Project><PropertyGroup><A>x</A></PropertyGroup><ItemGroup><|"),
            Place::ElementName { path: path(&["Project", "ItemGroup"]), start: 60, closing: false }
        );
        let Place::AttributeValue { tag, attribute, start, end, .. } =
            place(r#"<Project><ItemGroup><PackageReference Include="Newton|soft" Version="1.0" />"#)
        else {
            panic!("not an attribute's value")
        };
        assert_eq!((tag.name.as_str(), attribute.as_str(), start, end), ("PackageReference", "Include", 47, 57));
        let Place::AttributeValue { tag, attribute, .. } =
            place(r#"<Project><ItemGroup><PackageReference Include="Demo.Core" Version="|"#)
        else {
            panic!("not an attribute's value")
        };
        assert_eq!((attribute.as_str(), tag.attribute("include")), ("Version", Some("Demo.Core")));
        assert!(matches!(place("<Project><Target Name=\"A\" |"), Place::AttributeName { .. }));
        assert_eq!(place("<Project><PropertyGroup><Out>$(Base|"), Place::Reference { kind: '$', start: 31 });
        assert_eq!(place("<Project><Target Name='A' Condition=\"'@(Comp|"), Place::Reference { kind: '@', start: 40 });
        assert_eq!(
            place("<Project><PropertyGroup><TargetFramework>net|"),
            Place::Text { path: path(&["Project", "PropertyGroup", "TargetFramework"]), start: 41 }
        );
    }
}
