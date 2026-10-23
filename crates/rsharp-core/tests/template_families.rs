use rsharp_core::templates::{ProjectTemplate, TemplateFamily, TemplateVariant, families, options::is_enabled, packs};
use std::collections::BTreeMap;

fn template(name: &str, short: &str, tags: &[&str]) -> ProjectTemplate {
    ProjectTemplate {
        name: name.into(),
        short_names: vec![short.into()],
        classifications: tags.iter().map(|s| (*s).into()).collect(),
        default_name: name.replace(' ', ""),
        variants: vec![TemplateVariant { identity: short.into(), language: Some("C#".into()), ..Default::default() }],
        ..Default::default()
    }
}

#[test]
fn families_use_riders_order_and_type_names() {
    let mut cross = template("Avalonia Cross Platform Application", "avalonia.xplat", &["Desktop", "Avalonia"]);
    cross.is_solution = true;
    let families = families::of(vec![
        template("MSTest Test Project", "mstest", &["Test"]),
        template("NUnit Test Project", "nunit", &["Test"]),
        template("xUnit Test Project", "xunit", &["Test"]),
        template("Console App", "console", &["Console"]),
        template("Class Library", "classlib", &["Library"]),
        template("ASP.NET Core Web App", "webapp", &["Web"]),
        template("Avalonia .NET App", "avalonia.app", &["Desktop", "Avalonia"]),
        template("Avalonia .NET MVVM App", "avalonia.mvvm", &["Desktop", "Avalonia"]),
        cross,
        template("Aspire AppHost", "aspire-apphost", &["Aspire", "Web", "Test"]),
    ]);
    assert_eq!(
        families.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["Console", "Class Library", "Web", "Unit Test", "Aspire", "Avalonia"]
    );
    assert_eq!(
        families[3].types.iter().map(|t| t.type_name.as_str()).collect::<Vec<_>>(),
        ["xUnit", "NUnit", "MSTest"]
    );
    assert_eq!(
        families[5].types.iter().map(|t| t.type_name.as_str()).collect::<Vec<_>>(),
        ["App", "MVVM App", "Cross Platform Application"]
    );
}

#[test]
fn product_classifications_win_over_test_and_web() {
    let families = families::of(vec![
        template("A test", "a", &["AI", "Aspire", "Test", "Web"]),
        template("B test", "b", &["Aspire", "Test", "Web"]),
        template("C test", "c", &["Test", "Desktop", "Web"]),
        template("Worker Service", "worker", &["Web", "Worker"]),
    ]);
    assert_eq!(
        families.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["AI Extensions", "Services", "Unit Test", "Aspire"]
    );
}

#[test]
fn search_names_a_type_and_requires_every_word() {
    let f = families::of(vec![
        template("xUnit Test Project", "xunit", &["Test"]),
        template("NUnit Test Project", "nunit", &["Test"]),
    ])
    .remove(0);
    assert_eq!(f.matches(&["unit", "nunit"]), Some(vec![1]));
    assert_eq!(f.matches(&["UNIT", "test"]), Some(vec![]));
    assert_eq!(f.matches(&["nunit", "web"]), None);
}

#[test]
fn solution_types_are_absent_from_add_project() {
    let mut solution = template("Avalonia Cross Platform Application", "avalonia.xplat", &["Avalonia"]);
    solution.is_solution = true;
    let family = TemplateFamily {
        name: "Avalonia".into(),
        types: vec![template("Avalonia .NET App", "avalonia.app", &["Avalonia"]), solution.clone()],
    };
    assert_eq!(family.projects().unwrap().types.len(), 1);
    assert!(TemplateFamily { name: "Only".into(), types: vec![solution] }.projects().is_none());
}

#[test]
fn known_pack_offers_disappear_when_installed() {
    let offers = packs::offered(&[]);
    assert!(offers.iter().any(|t| t.short_name() == "avalonia.xplat" && t.is_solution));
    assert!(offers.iter().any(|t| t.short_name() == "func"));
    assert!(offers.iter().any(|t| t.short_name() == "aichatweb"));
    assert!(!offers.iter().any(|t| t.family == "MAUI"));
    let installed = vec![template("Avalonia .NET App", "AVALONIA.APP", &["Avalonia"])];
    assert!(!packs::offered(&installed).iter().any(|t| t.short_name() == "avalonia.app"));
    let families = families::of(offers);
    assert!(families.iter().find(|f| f.name == "Avalonia").unwrap().not_installed());
}

#[test]
fn enabled_conditions_match_passenger() {
    let values = BTreeMap::from([("UseMSTestSdk".into(), "false".into()), ("TestRunner".into(), "MSTest".into())]);
    for (condition, expected) in [
        ("UseMSTestSdk && (TestRunner == MSTest)", false),
        ("!UseMSTestSdk && TestRunner == 'MSTest'", true),
        ("TestRunner != NUnit && false || true", true),
        ("true || false && false", true),
        ("TestRunner == mstest", true),
        ("Unknown", false),
        ("(!broken", true),
        ("true false", true),
        ("()", true),
        ("true &&", true),
        ("!(true || false)", false),
    ] {
        assert_eq!(is_enabled(Some(condition), &values), expected, "condition {condition}");
    }
    assert!(is_enabled(None, &values));
}

#[test]
fn unrelated_names_with_the_same_short_prefix_do_not_become_a_family() {
    let f = families::of(vec![
        template("First Project", "mine.first", &[]),
        template("Different Kind", "mine.second", &[]),
    ]);
    assert_eq!(f.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Different Kind", "First Project"]);
}
