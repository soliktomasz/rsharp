use rsharp_app::test_view::{GroupLevel, TestModel, UnitTestsState, categories, duration_grade};
use rsharp_core::testing::{TestItem, TestOutcome, TestState};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

fn item(method: &str, row: Option<&str>, framework: Option<&str>) -> TestItem {
    TestItem {
        project: "/shop/Tests/Tests.csproj".into(),
        fully_qualified_name: format!("Shop.Checks.{method}"),
        display_name: row.unwrap_or(method).into(),
        framework: framework.map(str::to_owned),
        row: row.map(str::to_owned),
        path: Some("/shop/Tests/Checks.cs".into()),
        line: 8,
        traits: vec![("Category".into(), "Quick, Database".into())],
    }
}

#[test]
fn grouping_matches_riders_levels_categories_and_duration_boundaries() {
    assert_eq!(UnitTestsState::default().grouping, [GroupLevel::Project, GroupLevel::Namespace]);
    assert_eq!(
        categories(&[("Category".into(), "Quick, ,Database".into()), ("Owner".into(), "Ada".into())]),
        ["Quick", "Database", "Owner [Ada]"]
    );
    assert_eq!(duration_grade(None), ("Not Run", 5));
    assert_eq!(duration_grade(Some(Duration::from_millis(99))).0, "Fast (<100 ms)");
    assert_eq!(duration_grade(Some(Duration::from_millis(100))).0, "Medium (>100 ms)");
    assert_eq!(duration_grade(Some(Duration::from_secs(30))).0, "Slowest (>30 sec)");
}

#[test]
fn source_rows_merge_runtime_cases_without_losing_current_source_location() {
    let mut model = TestModel::default();
    let code = item("Adds", None, None);
    let project = code.project.clone();
    model.set_code(BTreeMap::from([(
        project,
        rsharp_app::test_finder::CodeTests {
            tests: vec![code],
            methods_by_file: BTreeMap::from([("/shop/Tests/Checks.cs".into(), BTreeSet::from(["Adds".into()]))]),
        },
    )]));
    let mut first = item("Adds", Some("Adds(1)"), Some("net10.0"));
    first.line = 2;
    model.discovered(vec![
        first,
        item("Adds", Some("Adds(2)"), Some("net10.0")),
        item("Adds", Some("Adds(1)"), Some("net9.0")),
    ]);
    assert_eq!(model.tests().len(), 3, "source placeholder duplicated the runner's data rows");
    assert!(model.tests().iter().all(|t| t.line == 8), "a stale built location replaced the source location");
    let rows = model.rows(&UnitTestsState::default(), "", &BTreeSet::new());
    assert_eq!(rows.iter().filter(|r| r.label == "net10.0" || r.label == "net9.0").count(), 2);
    assert_eq!(rows.iter().filter(|r| r.label == "Adds(1)").count(), 2, "same data row in two frameworks was merged");
}

#[test]
fn outcome_filter_keeps_ancestors_and_stale_runtime_methods_disappear_after_edit() {
    let mut model = TestModel::default();
    let first = item("Adds", None, Some("net10.0"));
    let second = item("Removes", None, Some("net10.0"));
    model.discovered(vec![first.clone(), second.clone()]);
    model.outcomes(vec![TestOutcome {
        test: first.clone(),
        state: TestState::Failed,
        duration: Duration::from_millis(4),
        message: Some("expected two".into()),
        stack_trace: None,
        output: None,
        attachments: vec![],
    }]);
    let state = UnitTestsState { outcomes: vec![TestState::Failed], ..Default::default() };
    let rows = model.rows(&state, "adds", &BTreeSet::new());
    assert!(rows.iter().any(|r| r.label == "Tests"), "the failed test lost its project ancestor");
    assert!(rows.iter().any(|r| r.label == "Adds"));
    assert!(!rows.iter().any(|r| r.label == "Removes"));
    model.set_code(BTreeMap::from([(
        first.project.clone(),
        rsharp_app::test_finder::CodeTests {
            tests: vec![],
            methods_by_file: BTreeMap::from([(first.path.clone().unwrap(), BTreeSet::new())]),
        },
    )]));
    assert!(model.tests().is_empty(), "a removed method survived from an old assembly");
}

#[test]
fn category_duplicates_and_collapsed_groups_run_each_case_once() {
    let mut model = TestModel::default();
    let first = item("Adds", Some("Adds(1)"), Some("net10.0"));
    model.discovered(vec![first.clone()]);
    let state = UnitTestsState { grouping: vec![GroupLevel::Category], ..Default::default() };
    let all = model.rows(&state, "", &BTreeSet::new());
    assert!(all.iter().any(|r| r.label == "Quick"));
    assert!(all.iter().any(|r| r.label == "Database"));
    assert_eq!(model.tests().len(), 1, "a multi-category case was counted twice");
    let key = all.iter().find(|r| r.label == "Quick").unwrap().key.clone();
    let collapsed = model.rows(&state, "", &BTreeSet::from([key.clone()]));
    let group = collapsed.iter().find(|r| r.key == key).unwrap();
    assert_eq!(group.tests.len(), 1, "a closed group could not run its case");
    assert!(!group.expanded);
    assert!(!collapsed.iter().any(|r| r.key.starts_with(&format!("{key}/"))));
}

#[gpui::test]
fn controls_regroup_filter_search_and_show_the_selected_failure(cx: &mut gpui::TestAppContext) {
    use gpui::{Focusable, Modifiers};
    use rsharp_app::test_view::TestView;
    cx.update(rsharp_app::keymap::bind);
    let (view, cx) = cx.add_window_view(|_, cx| TestView::new(cx));
    view.update(cx, |v, cx| {
        let test = item("Adds", Some("Adds(1)"), Some("net10.0"));
        v.discovered(vec![test.clone(), item("Removes", None, Some("net10.0"))], cx);
        v.outcomes(
            vec![TestOutcome {
                test,
                state: TestState::Failed,
                duration: Duration::from_millis(12),
                message: Some("Expected 2, received 3".into()),
                stack_trace: Some("at Shop.Checks.Adds() in /shop/Tests/Checks.cs:line 8".into()),
                output: Some("actual: 3".into()),
                attachments: vec![],
            }],
            cx,
        );
    });
    cx.run_until_parked();
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} was not visible"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
    };
    click(cx, "tests group by");
    click(cx, "tests preset: Categories");
    assert_eq!(view.read_with(cx, |v, _| v.state().grouping), [GroupLevel::Category]);
    click(cx, "tests outcomes");
    click(cx, "tests outcome: Failed");
    assert_eq!(view.read_with(cx, |v, _| v.state().outcomes), [TestState::Failed]);
    click(cx, "tests outcomes");
    let search = view.read_with(cx, |v, _| v.search_field().clone());
    cx.update(|window, cx| window.focus(&search.focus_handle(cx)));
    cx.simulate_input("adds");
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.rows().iter().any(|r| r.label == "Removes")));
    let selected = view.read_with(cx, |v, _| v.rows().iter().find(|r| r.label == "Adds(1)").unwrap().key.clone());
    view.update(cx, |v, cx| v.select(Some(selected), cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.selected_tests().len()), 1);
    assert!(cx.debug_bounds("tests source").is_some(), "the failure did not offer its source location");
    assert!(cx.debug_bounds("tests run failed").is_some(), "the failure had no rerun control");
    view.update(cx, |v, cx| v.set_running(true, cx));
    view.update(cx, |v, cx| v.set_running(false, cx));
    view.update(cx, |v, cx| v.clear(cx));
    assert!(view.read_with(cx, |v, _| v.tests().is_empty()), "the next workspace inherited previous tests");
}

#[test]
fn persistence_ignores_unknown_future_levels_and_deduplicates_existing_choices() {
    let state: UnitTestsState = serde_json::from_str(
        r#"{"grouping":["Project","Future","Namespace","Project"],"outcomes":["Failed","Future","Failed"]}"#,
    )
    .unwrap();
    let state = state.clamped();
    assert_eq!(state.grouping, [GroupLevel::Project, GroupLevel::Namespace]);
    assert_eq!(state.outcomes, [TestState::Failed]);
}

#[test]
fn source_rename_and_deleted_file_remove_results_from_an_earlier_build() {
    let mut model = TestModel::default();
    let old = item("Adds", None, None);
    let code = |test: TestItem| {
        BTreeMap::from([(
            test.project.clone(),
            rsharp_app::test_finder::CodeTests {
                methods_by_file: BTreeMap::from([(test.path.clone().unwrap(), BTreeSet::from(["Adds".into()]))]),
                tests: vec![test],
            },
        )])
    };
    assert!(model.set_code(code(old.clone())));
    assert!(!model.set_code(code(old.clone())), "unchanged source declarations needlessly rebuilt the tree");
    model.discovered(vec![item("Adds", Some("Adds(1)"), Some("net10.0"))]);
    let mut renamed = old.clone();
    renamed.fully_qualified_name = "Shop.Renamed.Adds".into();
    model.set_code(code(renamed));
    assert_eq!(model.tests().len(), 1, "the old class survived from the previous assembly");
    assert_eq!(model.tests()[0].fully_qualified_name, "Shop.Renamed.Adds");
    model.discovered(vec![model.tests()[0].clone()]);
    model.set_code(BTreeMap::from([(old.project.clone(), Default::default())]));
    assert!(model.tests().is_empty(), "a deleted source file survived from the previous assembly");
}

#[test]
fn a_rerun_clears_only_selected_results_and_duration_groups_keep_theory_cases_together() {
    let mut model = TestModel::default();
    let first = item("Adds", Some("Adds(1)"), Some("net10.0"));
    let second = item("Adds", Some("Adds(2)"), Some("net10.0"));
    model.discovered(vec![first.clone(), second.clone()]);
    model.outcomes(
        [first.clone(), second.clone()]
            .into_iter()
            .map(|test| TestOutcome {
                test,
                state: TestState::Failed,
                duration: Duration::from_millis(80),
                ..Default::default()
            })
            .collect(),
    );
    let state = UnitTestsState { grouping: vec![GroupLevel::Duration], ..Default::default() };
    let rows = model.rows(&state, "", &BTreeSet::new());
    assert_eq!(rows[0].label, "Medium (>100 ms)", "theory cases were graded individually instead of as a method");
    assert_eq!(rows[0].tests.len(), 2);
    model.begin(std::slice::from_ref(&first));
    assert_eq!(model.failed_tests(), [second], "Run Selected erased unrelated failures");
    let rows = model.rows(&UnitTestsState::default(), "", &BTreeSet::new());
    assert_eq!(rows.iter().find(|r| r.label == "Adds(1)").unwrap().state, TestState::Running);
    model.end();
    let rows = model.rows(&UnitTestsState::default(), "", &BTreeSet::new());
    assert_eq!(
        rows.iter().find(|r| r.label == "Adds(1)").unwrap().state,
        TestState::NotRun,
        "Stop left a case running forever"
    );
}

#[test]
fn nested_type_notation_from_an_adapter_merges_with_source_and_keeps_its_namespace() {
    let mut model = TestModel::default();
    let mut code = item("Adds", None, None);
    code.fully_qualified_name = "Shop.Outer+Nested.Adds".into();
    model.set_code(BTreeMap::from([(
        code.project.clone(),
        rsharp_app::test_finder::CodeTests {
            tests: vec![code.clone()],
            methods_by_file: BTreeMap::from([(code.path.clone().unwrap(), BTreeSet::from(["Adds".into()]))]),
        },
    )]));
    let mut runtime = code;
    runtime.framework = Some("net10.0".into());
    runtime.fully_qualified_name = "Shop.Outer.Nested.Adds".into();
    runtime.line = 2;
    model.discovered(vec![runtime]);
    assert_eq!(model.tests().len(), 1, "nested test appeared once from source and again from the adapter");
    assert_eq!(model.tests()[0].line, 8, "nested adapter result did not follow unsaved source location");
    let rows = model.rows(&UnitTestsState::default(), "", &BTreeSet::new());
    assert!(rows.iter().any(|row| row.label == "Shop"));
    assert!(rows.iter().any(|row| row.label == "Outer+Nested"));
    assert!(!rows.iter().any(|row| row.label == "Shop.Outer"));
}

#[test]
fn rediscovering_one_project_preserves_other_projects_cases_frameworks_and_failures() {
    let mut model = TestModel::default();
    let first = item("Adds", Some("Adds(1)"), Some("net10.0"));
    let mut other = item("Removes", Some("Removes(2)"), Some("net9.0"));
    other.project = "/shop/Other/Other.csproj".into();
    other.path = Some("/shop/Other/Checks.cs".into());
    model.discovered(vec![first.clone(), other.clone()]);
    model.outcomes(vec![TestOutcome { test: other.clone(), state: TestState::Failed, ..Default::default() }]);
    model.clear_discovered_in(std::slice::from_ref(&first.project));
    assert_eq!(model.tests(), [other.clone()], "selected-project discovery removed another project's cases");
    assert_eq!(model.failed_tests(), [other], "selected-project discovery lost another project's failures");
    let rows = model.rows(
        &UnitTestsState { grouping: vec![GroupLevel::TargetFramework, GroupLevel::Project], ..Default::default() },
        "",
        &BTreeSet::new(),
    );
    assert!(rows.iter().any(|row| row.label == "net9.0"), "another project's framework disappeared");
}

#[test]
fn three_thousand_runtime_results_merge_with_the_indexed_current_source() {
    let mut model = TestModel::default();
    let tests: Vec<_> = (0..3000).map(|index| item(&format!("Case{index}"), None, None)).collect();
    let project = tests[0].project.clone();
    let path = tests[0].path.clone().unwrap();
    model.set_code(BTreeMap::from([(
        project,
        rsharp_app::test_finder::CodeTests {
            methods_by_file: BTreeMap::from([(path, (0..3000).map(|index| format!("Case{index}")).collect())]),
            tests: tests.clone(),
        },
    )]));
    let runtime: Vec<_> = tests
        .into_iter()
        .map(|mut test| {
            test.framework = Some("net10.0".into());
            test.line = 2;
            test
        })
        .collect();
    let started = std::time::Instant::now();
    model.discovered(runtime.clone());
    model.outcomes(
        runtime
            .into_iter()
            .map(|test| TestOutcome {
                test,
                state: TestState::Passed,
                duration: Duration::from_millis(1),
                ..Default::default()
            })
            .collect(),
    );
    let elapsed = started.elapsed();
    let merged = model.tests();
    assert_eq!(merged.len(), 3000, "a large run dropped or duplicated tests while merging source and results");
    assert!(merged.iter().all(|test| test.line == 8), "a large run replaced source positions with old build positions");
    let rows =
        model.rows(&UnitTestsState { outcomes: vec![TestState::Passed], ..Default::default() }, "", &BTreeSet::new());
    assert_eq!(
        rows.iter().filter(|row| !row.expandable).count(),
        3000,
        "not every runtime result reached its current source row"
    );
    eprintln!("3000 indexed discovery/results merged in {elapsed:?}");
}
