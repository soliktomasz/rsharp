use super::*;
use rsharp_core::templates::{
    ProjectTemplate, TemplateCatalog, TemplateChoice, TemplateOption, TemplateOptionKind, TemplateVariant, families,
    packs,
};
use std::sync::Arc;

fn fixture() -> Arc<TemplateCatalog> {
    let option = |name: &str, kind, default: &str, choices: &[&str]| TemplateOption {
        name: name.into(),
        option: name.into(),
        label: name.into(),
        description: None,
        kind,
        default: default.into(),
        choices: choices
            .iter()
            .map(|s| TemplateChoice { value: (*s).into(), label: (*s).into(), description: None })
            .collect(),
        inverted: false,
        required: false,
        enabled_when: None,
    };
    let template = |name: &str, short: &str, tags: &[&str], base: &str, choices: &[&str]| ProjectTemplate {
        name: name.into(),
        short_names: vec![short.into()],
        classifications: tags.iter().map(|s| (*s).into()).collect(),
        default_name: base.into(),
        variants: vec![TemplateVariant {
            identity: short.into(),
            language: Some("C#".into()),
            frameworks: vec!["net10.0".into(), "net9.0".into()],
            default_framework: Some("net10.0".into()),
            options: vec![
                option("UseProgramMain", TemplateOptionKind::Bool, "false", &[]),
                option("Mode", TemplateOptionKind::Choice, choices[0], choices),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut templates = vec![
        template("Console App", "console", &["Console"], "ConsoleApp", &["zeta", "alpha"]),
        template("xUnit Test Project", "xunit", &["Test"], "TestProject", &["zeta", "alpha"]),
        template("NUnit Test Project", "nunit", &["Test"], "TestProject", &["alpha"]),
    ];
    templates.extend(packs::offered(&templates));
    Arc::new(TemplateCatalog {
        sdk: "10".into(),
        families: families::of(templates),
        solution_formats: vec!["slnx".into(), "sln".into()],
        solution_format: "slnx".into(),
    })
}

fn draft() -> Draft {
    let mut draft = Draft::new(std::env::temp_dir(), None);
    draft.show_catalog(fixture(), true);
    draft
}

#[test]
fn project_name_follows_until_the_person_types_it() {
    let mut d = draft();
    assert_eq!(d.solution_name(), "ConsoleApp1");
    assert_eq!(d.project_name(), "ConsoleApp1");
    d.set_solution_name("Greeter".into());
    assert_eq!(d.project_name(), "Greeter");
    d.set_project_name("Greeter.Core".into());
    d.set_solution_name("Renamed".into());
    assert_eq!(d.project_name(), "Greeter.Core");
    d.search("nunit");
    assert_eq!(d.solution_name(), "Renamed");
    assert_eq!(d.project_name(), "Greeter.Core");
}

#[test]
fn switching_types_keeps_only_valid_person_values() {
    let mut d = draft();
    d.set_option("UseProgramMain", "true".into());
    d.set_option("Mode", "zeta".into());
    d.search("xunit");
    assert_eq!(d.option_value("UseProgramMain"), Some("true"));
    assert_eq!(d.option_value("Mode"), Some("zeta"));
    d.search("nunit");
    assert_eq!(d.option_value("Mode"), Some("alpha"));
    d.search("console");
    assert_eq!(d.option_value("Mode"), Some("zeta"));
}

#[test]
fn searching_a_type_keeps_family_history() {
    let mut d = draft();
    d.search("nunit");
    assert_eq!(d.chosen().unwrap().short_name(), "nunit");
    d.search("");
    d.search("console");
    d.search("");
    let family = d.families.iter().position(|f| f.name == "Unit Test").unwrap();
    d.choose_family(family);
    assert_eq!(d.chosen().unwrap().short_name(), "nunit");
}

#[test]
fn clearing_no_matches_recovers_the_choice() {
    let mut d = draft();
    d.search("nunit");
    d.search("does not exist");
    assert!(d.chosen().is_none());
    assert!(d.request().is_err());
    d.search("");
    assert_eq!(d.chosen().unwrap().short_name(), "nunit");
}

#[test]
fn refresh_preserves_names_and_choices() {
    let mut d = draft();
    d.search("nunit");
    d.set_solution_name("My.Tests".into());
    d.set_framework("net9.0");
    d.set_option("UseProgramMain", "true".into());
    d.show_catalog(fixture(), true);
    assert_eq!(d.solution_name(), "My.Tests");
    assert_eq!(d.chosen().unwrap().short_name(), "nunit");
    assert_eq!(d.framework.as_deref(), Some("net9.0"));
    assert_eq!(d.option_value("UseProgramMain"), Some("true"));
}

#[test]
fn solution_template_hides_project_inputs() {
    let mut d = draft();
    d.search("avalonia.xplat");
    assert!(d.chosen().unwrap().is_solution);
    assert!(!d.asks_project_name());
    assert!(d.request().is_err());
    let target =
        rsharp_core::templates::ProjectTarget { solution: "/Shop.slnx".into(), folder: None, projects: vec![] };
    let mut adding = Draft::new(std::env::temp_dir(), Some(target));
    adding.show_catalog(fixture(), true);
    assert!(adding.families.iter().all(|f| f.types.iter().all(|t| !t.is_solution)));
}

#[test]
fn stale_catalog_cannot_create() {
    let mut d = Draft::new(std::env::temp_dir(), None);
    d.show_catalog(fixture(), false);
    assert!(d.request().is_err());
    d.show_catalog(fixture(), true);
    assert!(d.request().is_ok());
    d.authoritative = false;
    assert!(d.request().is_err());
}

#[test]
fn a_fresh_dialog_uses_the_sdks_preferred_solution_format() {
    assert_eq!(draft().request().unwrap().solution_format, "slnx");
}

#[test]
fn compatible_language_and_framework_survive_and_unavailable_ones_fall_back() {
    let mut loaded = (*fixture()).clone();
    let console = &mut loaded.families.iter_mut().find(|f| f.name == "Console").unwrap().types[0];
    let mut fsharp = console.variants[0].clone();
    fsharp.language = Some("F#".into());
    fsharp.frameworks = vec!["net8.0".into()];
    fsharp.default_framework = Some("net8.0".into());
    console.variants.push(fsharp);
    let mut d = draft();
    d.show_catalog(Arc::new(loaded), true);
    d.set_language("F#");
    assert_eq!(d.framework.as_deref(), Some("net8.0"));
    d.search("xunit");
    assert_eq!(d.language.as_deref(), Some("C#"));
    assert_eq!(d.framework.as_deref(), Some("net10.0"));
}

#[test]
fn retained_disabled_values_still_control_other_options_but_are_not_emitted() {
    let mut d = draft();
    d.set_option("Mode", "alpha".into());
    let mut loaded = (*fixture()).clone();
    let options = &mut loaded.families.iter_mut().find(|f| f.name == "Console").unwrap().types[0].variants[0].options;
    options[1].enabled_when = Some("false".into());
    options[0].enabled_when = Some("Mode == alpha".into());
    d.show_catalog(Arc::new(loaded), true);
    d.set_option("UseProgramMain", "true".into());
    let args = rsharp_core::templates::create::arguments(&d.request().unwrap(), None).unwrap();
    assert!(args.iter().any(|a| a == "--UseProgramMain"));
    assert!(!args.iter().any(|a| a == "--Mode"));
}

#[test]
fn default_names_skip_collisions_and_late_work_keeps_person_names() {
    let mut d = draft();
    d.set_occupied_names(vec!["ConsoleApp1".into(), "ConsoleApp2".into()]);
    assert_eq!(d.solution_name(), "ConsoleApp3");
    d.set_solution_name("Typed".into());
    d.set_occupied_names(vec!["ConsoleApp3".into(), "Typed".into()]);
    assert_eq!(d.solution_name(), "Typed");
    let target = rsharp_core::templates::ProjectTarget {
        solution: "/Shop.slnx".into(),
        folder: None,
        projects: vec!["CONSOLEAPP1".into()],
    };
    let mut adding = Draft::new(std::env::temp_dir(), Some(target));
    adding.show_catalog(fixture(), true);
    assert_eq!(adding.project_name(), "ConsoleApp2");
}

#[test]
fn install_refresh_keeps_the_short_name_and_makes_it_creatable() {
    let mut d = draft();
    d.search("avalonia.app");
    assert!(d.chosen().unwrap().pack.is_some());
    let mut loaded = (*fixture()).clone();
    let app = loaded.families.iter_mut().flat_map(|f| &mut f.types).find(|t| t.short_name() == "avalonia.app").unwrap();
    app.pack = None;
    d.show_catalog(Arc::new(loaded), true);
    assert_eq!(d.chosen().unwrap().short_name(), "avalonia.app");
    assert!(d.request().is_ok());
}
