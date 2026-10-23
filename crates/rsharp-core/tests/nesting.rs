use rsharp_core::nesting::parents;
use std::path::{Path, PathBuf};

fn files(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| PathBuf::from("/Shop").join(n)).collect()
}

fn parent_of(found: &std::collections::HashMap<PathBuf, PathBuf>, child: &str) -> Option<String> {
    found.get(&PathBuf::from("/Shop").join(child)).map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
}

fn none(_: &Path) -> Option<PathBuf> {
    None
}

#[test]
fn each_file_nests_under_the_one_it_belongs_to_by_riders_rules() {
    let cases = [
        (
            ["Counter.razor", "Counter.razor.cs", "Counter.razor.css"].as_slice(),
            [("Counter.razor.cs", "Counter.razor"), ("Counter.razor.css", "Counter.razor")].as_slice(),
        ),
        (&["Index.cshtml", "Index.cshtml.cs"], &[("Index.cshtml.cs", "Index.cshtml")]),
        (
            &["Form1.cs", "Form1.Designer.cs", "Form1.resx"],
            &[("Form1.Designer.cs", "Form1.cs"), ("Form1.resx", "Form1.cs")],
        ),
        (&["Resources.resx", "Resources.Designer.cs"], &[("Resources.Designer.cs", "Resources.resx")]),
        (
            &["appsettings.json", "appsettings.Development.json"],
            &[("appsettings.Development.json", "appsettings.json")],
        ),
        (&["site.ts", "site.js", "site.js.map"], &[("site.js", "site.ts"), ("site.js.map", "site.ts")]),
    ];
    for (names, expected) in cases {
        let found = parents(&files(names), none, true);
        for (child, parent) in expected {
            assert_eq!(parent_of(&found, child).as_deref(), Some(*parent), "{child} would not show under {parent}");
        }
        assert_eq!(found.len(), expected.len(), "a file of {names:?} would nest that Rider shows flat: {found:?}");
    }
}

#[test]
fn dependent_upon_nests_and_wins_over_a_rule() {
    let names = ["Shapes.cs", "Shapes.Circle.cs", "Shapes.Notes.txt", "Form1.cs", "Form1.resx"];
    let declared = |file: &Path| {
        let name = file.file_name()?.to_str()?;
        match name {
            "Shapes.Circle.cs" => Some(PathBuf::from("/Shop/Shapes.cs")),
            // The project says the resources depend on the shapes, not on Form1 as the rule would.
            "Form1.resx" => Some(PathBuf::from("/Shop/Shapes.cs")),
            "Shapes.Notes.txt" => Some(PathBuf::from("/Shop/Shapes.Circle.cs")),
            _ => None,
        }
    };
    let found = parents(&files(&names), declared, true);
    assert_eq!(parent_of(&found, "Shapes.Circle.cs").as_deref(), Some("Shapes.cs"), "DependentUpon would not nest");
    assert_eq!(parent_of(&found, "Form1.resx").as_deref(), Some("Shapes.cs"), "DependentUpon would lose to a rule");
    // Nesting is one level: under the circle's own parent.
    assert_eq!(parent_of(&found, "Shapes.Notes.txt").as_deref(), Some("Shapes.cs"), "nesting would go two deep");
}

#[test]
fn without_the_rules_only_dependent_upon_nests() {
    let declared = |file: &Path| (file.file_name()? == "Shapes.Circle.cs").then(|| PathBuf::from("/Shop/Shapes.cs"));
    let found = parents(&files(&["Shapes.cs", "Shapes.Circle.cs", "Form1.cs", "Form1.resx"]), declared, false);
    assert_eq!(found.len(), 1, "File Nesting off would still nest by Rider's rules: {found:?}");
}

#[test]
fn files_that_depend_on_each_other_nest_neither() {
    let declared = |file: &Path| match file.file_name()?.to_str()? {
        "A.cs" => Some(PathBuf::from("/Shop/B.cs")),
        "B.cs" => Some(PathBuf::from("/Shop/A.cs")),
        _ => None,
    };
    assert!(parents(&files(&["A.cs", "B.cs"]), declared, true).is_empty(), "a circle would hide both files");
}
