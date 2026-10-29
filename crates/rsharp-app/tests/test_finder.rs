mod support;

use rsharp_app::test_finder::{TestFinder, holds_tests};
use rsharp_core::project::{EvaluatedItem, EvaluatedProject, FrameworkDependencies, PackageDependency};
use rsharp_languages::{Grammar, parse, testing};
use rsharp_text::Rope;
use std::collections::HashMap;
use std::path::Path;
use support::Run;

fn project(path: &Path, tests: bool) -> EvaluatedProject {
    EvaluatedProject { path: path.into(), is_test_project: tests, ..Default::default() }
}

fn source(code: &str) -> testing::FileTests {
    let text = Rope::from_str(code);
    testing::read(&parse(Grammar::CSharp, &text, None), &text)
}

#[test]
fn framework_packages_and_the_test_flag_require_an_existing_project() {
    let run = Run::new("finder-packages", &[("Tests.csproj", "<Project />")]);
    let mut p = project(&run.folder.join("Tests.csproj"), true);
    assert!(holds_tests(&p));
    p.is_test_project = false;
    assert!(!holds_tests(&p));
    for name in ["XUNIT", "xunit.core", "xunit.v3", "xunit.v3.core", "nunit", "MSTest", "mstest.testframework"] {
        p.dependencies = vec![FrameworkDependencies {
            packages: vec![PackageDependency { name: name.into(), ..Default::default() }],
            ..Default::default()
        }];
        assert!(holds_tests(&p), "the {name} framework package did not identify its test project");
    }
    p.path = run.folder.join("Gone.csproj");
    assert!(!holds_tests(&p), "a removed project still held tests");
}

#[test]
fn closed_files_skip_build_output_hidden_dependencies_and_nested_projects() {
    let code = "namespace Tests; class Cases { [Fact] void Works() {} void Helper() {} }";
    let run = Run::new(
        "finder-folders",
        &[
            ("Tests.csproj", "<Project />"),
            ("Cases.cs", code),
            ("sub/More.cs", "class Other { [Test] void More() {} }"),
            ("bin/Bin.cs", code),
            ("obj/Obj.cs", code),
            ("node_modules/Node.cs", code),
            (".hidden/Hidden.cs", code),
            ("nested/Library.csproj", "<Project />"),
            ("nested/NotOurs.cs", code),
            ("other/Other.csproj", "<Project />"),
            ("other/Ours.cs", code),
        ],
    );
    let p = project(&run.folder.join("Tests.csproj"), true);
    let child = project(&run.folder.join("nested/Library.csproj"), false);
    let other = project(&run.folder.join("other/Other.csproj"), true);
    let found = TestFinder::default().scan(&[p.clone(), child, other.clone()], &HashMap::new());
    assert_eq!(
        found[&p.path].tests.len(),
        2,
        "a parent picked up output, dependencies, hidden files or another project's tests"
    );
    assert_eq!(found[&other.path].tests.len(), 1);
    assert!(found[&p.path].methods_by_file[&run.folder.join("Cases.cs")].contains("Helper"));
    assert_eq!(found[&p.path].tests[0].line, 1);
    assert_eq!(found[&p.path].tests[0].project, p.path);
}

#[test]
fn open_edits_new_files_and_linked_compile_items_override_disk() {
    let run = Run::new(
        "finder-open",
        &[
            ("tests/Tests.csproj", "<Project />"),
            ("tests/Cases.cs", "class Cases { [Fact] void Before() {} }"),
            ("Shared.cs", "class Shared { [TestMethod] void Linked() {} }"),
            ("tests/nested/Library.csproj", "<Project />"),
        ],
    );
    let mut p = project(&run.folder.join("tests/Tests.csproj"), true);
    p.items.push(EvaluatedItem {
        r#type: "Compile".into(),
        path: run.folder.join("Shared.cs"),
        link: Some("Shared.cs".into()),
        ..Default::default()
    });
    let child = project(&run.folder.join("tests/nested/Library.csproj"), false);
    let open = HashMap::from([
        (run.folder.join("tests/Cases.cs"), source("class Cases { [Fact] void Typed() {} }")),
        (run.folder.join("tests/New.cs"), source("class New { [Theory] void Fresh() {} }")),
        (run.folder.join("Shared.cs"), source("class Shared { [TestMethod] void LinkedTyped() {} }")),
        (run.folder.join("tests/nested/New.cs"), source("class Wrong { [Fact] void NotOurs() {} }")),
    ]);
    let mut finder = TestFinder::default();
    let found = finder.scan(&[p.clone(), child], &open);
    let names: Vec<_> = found[&p.path].tests.iter().map(|t| t.fully_qualified_name.as_str()).collect();
    assert_eq!(names, ["Shared.LinkedTyped", "Cases.Typed", "New.Fresh"]);
    let disk = finder.scan(&[p.clone()], &HashMap::new());
    assert_eq!(
        disk[&p.path].tests.iter().map(|t| t.fully_qualified_name.as_str()).collect::<Vec<_>>(),
        ["Shared.Linked", "Cases.Before"]
    );
}

#[test]
fn rescans_follow_changed_removed_and_new_files_and_exclude_abstract_tests() {
    let run = Run::new(
        "finder-rescan",
        &[
            ("Tests.csproj", "<Project />"),
            ("Cases.cs", "class Cases { [Fact] void Before() {} }"),
            ("Abstract.cs", "abstract class Base { [Fact] void Inherited() {} }"),
        ],
    );
    let p = project(&run.folder.join("Tests.csproj"), true);
    let mut finder = TestFinder::default();
    let projects = [p.clone()];
    assert_eq!(finder.scan(&projects, &HashMap::new())[&p.path].tests.len(), 1);
    std::fs::write(run.folder.join("Cases.cs"), "class Cases { [Fact] void AfterTheEdit() {} }").unwrap();
    let edited = finder.scan(&projects, &HashMap::new());
    assert_eq!(edited[&p.path].tests[0].fully_qualified_name, "Cases.AfterTheEdit");
    assert_eq!(finder.scan(&projects, &HashMap::new()), edited);
    std::fs::remove_file(run.folder.join("Cases.cs")).unwrap();
    assert!(finder.scan(&projects, &HashMap::new())[&p.path].tests.is_empty());
    std::fs::write(run.folder.join("New.cs"), "class Cases { [Fact] void NewFile() {} }").unwrap();
    assert_eq!(finder.scan(&projects, &HashMap::new())[&p.path].tests.len(), 1);
    assert!(finder.scan(&[], &HashMap::new()).is_empty());
}

#[test]
fn open_updates_use_cached_listings_without_reading_external_disk_changes() {
    let run = Run::new(
        "finder-memory-update",
        &[
            ("Tests.csproj", "<Project />"),
            ("Closed.cs", "class Closed { [Fact] void Before() {} }"),
            ("Open.cs", "class Open { [Fact] void Before() {} }"),
        ],
    );
    let p = project(&run.folder.join("Tests.csproj"), true);
    let projects = [p.clone()];
    let mut finder = TestFinder::default();
    finder.scan(&projects, &HashMap::new());
    std::fs::write(run.folder.join("Closed.cs"), "class Closed { [Fact] void AfterAnExternalEdit() {} }").unwrap();
    std::fs::write(run.folder.join("DiskNew.cs"), "class DiskNew { [Fact] void NotListedYet() {} }").unwrap();
    let open = HashMap::from([
        (run.folder.join("Open.cs"), source("class Open { [Fact] void Typed() {} }")),
        (run.folder.join("Unsaved.cs"), source("class Unsaved { [Fact] void Fresh() {} }")),
    ]);
    let updated = finder.update_open(&projects, &open);
    let names: Vec<_> = updated[&p.path].tests.iter().map(|t| t.fully_qualified_name.as_str()).collect();
    assert_eq!(
        names,
        ["Closed.Before", "Open.Typed", "Unsaved.Fresh"],
        "an editor-only update read or listed disk files"
    );
    let refreshed = finder.scan(&projects, &open);
    assert_eq!(refreshed[&p.path].tests.len(), 4);
    assert!(refreshed[&p.path].tests.iter().any(|t| t.fully_qualified_name == "Closed.AfterAnExternalEdit"));
    assert!(finder.update_open(&[], &open).is_empty(), "a removed project's cached listing remained visible");
}

#[test]
fn source_files_use_their_encoding_and_symlink_folders_are_not_walked() {
    let run = Run::new(
        "finder-encoding",
        &[("Tests.csproj", "<Project />"), ("outside/More.cs", "class Wrong { [Fact] void Duplicate() {} }")],
    );
    let p = project(&run.folder.join("Tests.csproj"), true);
    let source = "class Café { [Fact] void Works() {} }";
    let charset = rsharp_core::encodings::Charset::from_editorconfig("utf-16le").unwrap();
    std::fs::write(run.folder.join("Cases.cs"), rsharp_core::encodings::encode(source, charset).unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(run.folder.join("outside"), run.folder.join("linked")).unwrap();
    let found = TestFinder::default().scan(&[p.clone()], &HashMap::new());
    assert_eq!(found[&p.path].tests.len(), 2, "a symlink duplicated tests or UTF-16 was not read");
    assert!(found[&p.path].tests.iter().any(|t| t.fully_qualified_name == "Café.Works"));
}
