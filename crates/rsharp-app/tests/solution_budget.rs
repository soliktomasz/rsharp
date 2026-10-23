//! A click costs what it changed: opening a project in the Solution view takes the same on a solution ten times
//! larger, and an open folder of ten times the files costs about ten times, never a hundred.

use rsharp_app::solution_view::SolutionTree;
use rsharp_core::project::{EvaluatedItem, EvaluatedProject};
use rsharp_core::solution::{Project, Solution, SolutionEntry};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

/// A solution of `projects` projects, each holding `files` files in a folder Big, none of them on disk.
fn tree(projects: usize, files: usize) -> SolutionTree {
    let root = std::env::temp_dir().join(format!("rsharp-budget-{}", std::process::id()));
    let mut entries = Vec::new();
    let mut evaluated = HashMap::new();
    for p in 0..projects {
        let path = root.join(format!("P{p:04}/P{p:04}.csproj"));
        let folder = path.parent().unwrap().to_path_buf();
        let items = (0..files)
            .map(|f| EvaluatedItem {
                r#type: "Compile".into(),
                path: folder.join(format!("Big/F{f:05}.cs")),
                link: None,
                dependent_upon: (f % 2 == 1).then(|| format!("F{:05}.cs", f - 1)),
            })
            .collect();
        evaluated.insert(path.clone(), EvaluatedProject { path: path.clone(), items, ..Default::default() });
        entries.push(SolutionEntry::Project(Project { path, name: format!("P{p:04}"), exists: true }));
    }
    let solution = Solution { path: root.join("Big.slnx"), name: "Big".into(), entries };
    let open: HashSet<String> = ["p:P0000/P0000.csproj".to_string(), "pf:P0000/P0000.csproj|Big".to_string()].into();
    SolutionTree::new(solution, evaluated, open)
}

/// The median time, in microseconds, to open and close the second project, the first one's Big folder open.
fn per_click(projects: usize, files: usize) -> f64 {
    let mut tree = tree(projects, files);
    let second = tree.rows().iter().position(|r| r.name == "P0001").expect("the second project's row");
    let mut times = Vec::new();
    for _ in 0..21 {
        let started = Instant::now();
        tree.toggle(second, Some(true));
        tree.toggle(second, Some(false));
        times.push(started.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

#[test]
fn a_click_costs_the_same_in_a_solution_ten_times_larger() {
    let small = per_click(15, 300);
    let large = per_click(150, 300);
    eprintln!("a click: {small:.0} µs with 15 projects, {large:.0} µs with 150");
    assert!(large < small * 3., "a click with 150 projects cost {large:.0} µs against {small:.0} µs with 15");
}

#[test]
fn an_open_folder_costs_in_proportion_to_its_files() {
    let small = per_click(3, 400);
    let large = per_click(3, 4000);
    eprintln!("a click: {small:.0} µs beside 400 files, {large:.0} µs beside 4,000");
    // Ten times the rows shown is about ten times the work; work by pairs of files would be a hundred.
    assert!(large < small * 30., "a click beside 4,000 files cost {large:.0} µs against {small:.0} µs beside 400");
}

#[test]
fn nested_files_still_show_under_their_parents() {
    let tree = tree(1, 4);
    let names: Vec<&str> = tree.rows().iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Big", "P0000", "Dependencies", "Big", "F00000.cs", "F00002.cs"], "the nesting would change");
    let _ = PathBuf::new();
}
