//! File Structure: the active file's types and members as Passenger's `Navigator.ShowFileStructure` lists them, indented,
//! filtered as typed with the containers of what matches kept.

use rsharp_lsp::Span;

/// One symbol of the outline: how deep it sits, its name, its LSP kind, all it spans and where its name is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlineRow {
    pub depth: usize,
    pub name: String,
    pub kind: u64,
    pub range: Span,
    pub selection: Span,
}

impl OutlineRow {
    /// The name as Rider shows it, and the type Roslyn gives after it (`Pay() : void` is `Pay()` and `void`).
    pub fn label(&self) -> (&str, &str) {
        self.name.split_once(" : ").unwrap_or((&self.name, ""))
    }
}

/// A symbol kind's icon, by the protocol's numbers: IntelliJ's node icons, as Rider's File Structure draws them.
pub fn kind_icon(kind: u64) -> &'static str {
    match kind {
        5 | 23 => "node-class",
        11 => "node-interface",
        10 => "node-enum",
        6 | 12 => "node-method",
        9 => "node-constructor",
        7 | 24 => "node-property",
        14 | 22 => "node-constant",
        8 | 13 => "node-field",
        _ => "folder",
    }
}

/// The rows a query keeps, in order: those whose name holds it or matches it fuzzily, and the containers above each,
/// so a match reads in its place (Passenger's `FilterOutline`).
pub fn filter(rows: &[OutlineRow], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..rows.len()).collect();
    }
    let lowered = query.to_lowercase();
    let mut shown = Vec::new();
    let mut path: Vec<usize> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        while path.last().is_some_and(|&p| rows[p].depth >= row.depth) {
            path.pop();
        }
        let name = row.label().0;
        if name.to_lowercase().contains(&lowered) || crate::fuzzy::score(query, name).is_some() {
            for &container in &path {
                if !shown.contains(&container) {
                    shown.push(container);
                }
            }
            shown.push(i);
        }
        path.push(i);
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(depth: usize, name: &str) -> OutlineRow {
        OutlineRow { depth, name: name.into(), ..Default::default() }
    }

    #[test]
    fn filter_keeps_the_containers_of_a_match() {
        let rows = [row(0, "Shop"), row(1, "Cart"), row(2, "Add"), row(2, "Remove"), row(1, "Order"), row(2, "Pay")];
        assert_eq!(filter(&rows, "rem"), [0, 1, 3], "a match lost the types it reads in");
        assert_eq!(filter(&rows, "pay"), [0, 4, 5]);
        assert_eq!(filter(&rows, ""), [0, 1, 2, 3, 4, 5], "an empty query hid members");
        assert!(filter(&rows, "zzz").is_empty(), "rows listed that match nothing");
        let typed = [row(0, "Order"), row(1, "Pay() : void"), row(1, "Total : int")];
        assert_eq!(filter(&typed, "void"), Vec::<usize>::new(), "a member's type matched as its name");
        assert_eq!(filter(&typed, "pay"), [0, 1]);
    }
}
