use rsharp_core::templates::{
    ProjectTemplate, TemplateCatalog, TemplateOption, TemplateOptionKind, TemplateVariant, families, packs,
};
use std::sync::Arc;

pub fn fixture() -> Arc<TemplateCatalog> {
    let template = |name: &str, short: &str, tag: &str, base: &str| ProjectTemplate {
        name: name.into(),
        short_names: vec![short.into()],
        classifications: vec![tag.into()],
        default_name: base.into(),
        variants: vec![TemplateVariant {
            identity: short.into(),
            language: Some("C#".into()),
            frameworks: vec!["net10.0".into(), "net9.0".into()],
            default_framework: Some("net10.0".into()),
            description: format!("Create {name}."),
            options: vec![TemplateOption {
                name: "UseProgramMain".into(),
                option: "use-program-main".into(),
                label: "Do not use top-level statements".into(),
                description: None,
                kind: TemplateOptionKind::Bool,
                default: "false".into(),
                choices: vec![],
                inverted: false,
                required: false,
                enabled_when: None,
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut templates = vec![
        template("Console App", "console", "Console", "ConsoleApp"),
        template("Class Library", "classlib", "Library", "ClassLibrary"),
        template("xUnit Test Project", "xunit", "Test", "TestProject"),
        template("NUnit Test Project", "nunit", "Test", "TestProject"),
    ];
    templates.extend(packs::offered(&templates));
    Arc::new(TemplateCatalog {
        sdk: "10".into(),
        families: families::of(templates),
        solution_formats: vec!["slnx".into(), "sln".into()],
        solution_format: "slnx".into(),
    })
}
