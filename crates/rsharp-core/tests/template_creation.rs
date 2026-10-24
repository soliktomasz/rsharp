use rsharp_core::templates::{
    CreateRequest, ProjectTarget, ProjectTemplate, TemplateCatalog, TemplateFamily, TemplateOption, TemplateOptionKind,
    TemplateVariant, create,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn take_out_accepts_utf16_and_preserves_unchanged_bytes() {
    let f = Fixture::new();
    for encoding in ["utf-16le", "utf-16be", "latin1"] {
        let charset = rsharp_core::encodings::Charset::from_editorconfig(encoding).unwrap();
        let bytes = rsharp_core::encodings::encode("// café\r\nclass C {}\r\n", charset).unwrap();
        let file = f.root.join("Program.cs");
        std::fs::write(&file, &bytes).unwrap();
        assert!(rsharp_core::templates::developer_tools::take_out(&f.root).unwrap().is_empty());
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "unchanged {encoding} source was rewritten");
        let bytes = rsharp_core::encodings::encode("// café\r\n    .WithDeveloperTools()\r\n", charset).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        rsharp_core::templates::developer_tools::take_out(&f.root).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), rsharp_core::encodings::encode("// café\r\n", charset).unwrap());
    }
}

struct Fixture {
    root: PathBuf,
    catalog: TemplateCatalog,
    request: CreateRequest,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rsharp-create-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let option = |name: &str, kind, default: &str, enabled: Option<&str>| TemplateOption {
            name: name.into(),
            option: name.into(),
            label: name.into(),
            description: None,
            kind,
            default: default.into(),
            choices: vec![],
            inverted: false,
            required: name == "Count",
            enabled_when: enabled.map(str::to_string),
        };
        let template = ProjectTemplate {
            name: "Console App".into(),
            short_names: vec!["console".into()],
            default_name: "ConsoleApp".into(),
            variants: vec![TemplateVariant {
                identity: "Console".into(),
                language: Some("C#".into()),
                framework_option: Some("framework".into()),
                frameworks: vec!["net10.0".into()],
                default_framework: Some("net10.0".into()),
                skip_restore_option: Some("no-restore".into()),
                options: vec![
                    option("Feature", TemplateOptionKind::Bool, "false", None),
                    option("Count", TemplateOptionKind::Integer, "1", Some("Feature")),
                    option("Ratio", TemplateOptionKind::Float, "1", None),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let catalog = TemplateCatalog {
            sdk: "10.0.100".into(),
            families: vec![TemplateFamily { name: "Console".into(), types: vec![template.clone()] }],
            solution_formats: vec!["slnx".into(), "sln".into()],
            solution_format: "slnx".into(),
        };
        let request = CreateRequest {
            target: None,
            template,
            language: Some("C#".into()),
            framework: Some("net10.0".into()),
            solution_name: "Greeter".into(),
            project_name: "Greeter".into(),
            directory: root.clone(),
            same_directory: false,
            solution_format: "slnx".into(),
            values: BTreeMap::new(),
        };
        Self { root, catalog, request }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn invalid_requests_write_nothing() {
    let mut f = Fixture::new();
    for (name, why) in [
        ("", "Solution name cannot be empty"),
        ("Bad:Name", "Solution name contains invalid characters"),
        ("../outside", "Solution name contains invalid characters"),
        ("..", "Solution name contains invalid characters"),
        ("a\0b", "Solution name contains invalid characters"),
    ] {
        f.request.solution_name = name.into();
        assert_eq!(create::plan(&f.request, &f.catalog).unwrap_err(), why);
    }
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
    f.request.solution_name = "Greeter".into();
    f.request.target =
        Some(ProjectTarget { solution: f.root.join("Shop.slnx"), folder: None, projects: vec!["GREETER".into()] });
    assert_eq!(create::plan(&f.request, &f.catalog).unwrap_err(), "Project Greeter already exists in the solution");
    f.request.target = None;
    std::fs::create_dir(f.root.join("Greeter")).unwrap();
    assert!(create::plan(&f.request, &f.catalog).is_ok());
    std::fs::write(f.root.join("Greeter/notes.txt"), b"kept\r\n").unwrap();
    assert!(create::plan(&f.request, &f.catalog).unwrap_err().ends_with("Greeter is not empty"));
    assert_eq!(std::fs::read(f.root.join("Greeter/notes.txt")).unwrap(), b"kept\r\n");
}

#[test]
fn output_layout_matches_passenger() {
    let mut f = Fixture::new();
    let plan = create::plan(&f.request, &f.catalog).unwrap();
    assert_eq!(plan.root, f.root.join("Greeter"));
    assert_eq!(plan.solution, Some(f.root.join("Greeter/Greeter.slnx")));
    assert_eq!(plan.project_folder, f.root.join("Greeter/Greeter"));
    f.request.same_directory = true;
    assert_eq!(create::plan(&f.request, &f.catalog).unwrap().project_folder, f.root.join("Greeter"));
    f.request.template.is_solution = true;
    f.request.project_name.clear();
    assert_eq!(create::plan(&f.request, &f.catalog).unwrap().project_folder, f.root.join("Greeter"));
    f.request.template.is_solution = false;
    f.request.project_name = "Greeter.Core".into();
    f.request.target =
        Some(ProjectTarget { solution: f.root.join("Shop.slnx"), folder: Some("/src/".into()), projects: vec![] });
    assert_eq!(create::plan(&f.request, &f.catalog).unwrap().project_folder, f.root.join("Greeter.Core"));
    assert!(create::plan(&f.request, &f.catalog).unwrap().solution.is_none());
}

#[test]
fn arguments_are_literal_and_only_changed_options() {
    let mut f = Fixture::new();
    f.request.project_name = "Hello; $HOME Ω".into();
    f.request.values.insert("Count".into(), "3".into());
    let display: Vec<_> =
        create::arguments(&f.request, None).unwrap().into_iter().map(|s| s.into_string().unwrap()).collect();
    assert_eq!(display, ["console", "--name", "Hello; $HOME Ω", "--language", "C#", "--framework", "net10.0"]);
    f.request.values.insert("Feature".into(), "true".into());
    let args: Vec<_> =
        create::arguments(&f.request, Some(&f.root)).unwrap().into_iter().map(|s| s.into_string().unwrap()).collect();
    assert!(args.windows(2).any(|a| a == ["--Count", "3"]));
    assert!(args.windows(2).any(|a| a == ["--no-restore", "true"]));
    assert!(!args.iter().any(|a| a == "--force"));
}

#[test]
fn required_and_numeric_enabled_options_are_checked_before_writing() {
    let mut f = Fixture::new();
    f.request.values.insert("Feature".into(), "true".into());
    for (value, why) in [("", "Count is needed"), ("1.5", "Count: please enter an integer number")] {
        f.request.values.insert("Count".into(), value.into());
        assert_eq!(create::plan(&f.request, &f.catalog).unwrap_err(), why);
    }
    f.request.values.insert("Feature".into(), "false".into());
    assert!(create::plan(&f.request, &f.catalog).is_ok());
    f.request.values.insert("Ratio".into(), "not a number".into());
    assert_eq!(create::plan(&f.request, &f.catalog).unwrap_err(), "Ratio: please enter a float number");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}

#[test]
fn take_out_preserves_every_unrelated_byte() {
    let f = Fixture::new();
    let project = b"\xEF\xBB\xBF<Project>\r\n  <!-- kept -->\r\n  <PackageReference Include=\"AvaloniaUI.DiagnosticsSupport\" Version=\"1\" />\r\n  <PackageReference Include=\"Avalonia.Desktop\" Version=\"12\" />\r\n</Project>\r\n";
    let code =
        b"\xEF\xBB\xBFclass Program\r\n{\r\n#if DEBUG\r\n    .WithDeveloperTools()\r\n#endif\r\n    // kept\r\n}\r\n";
    std::fs::write(f.root.join("App.csproj"), project).unwrap();
    std::fs::write(f.root.join("Program.cs"), code).unwrap();
    std::fs::create_dir(f.root.join("obj")).unwrap();
    std::fs::write(f.root.join("obj/Program.cs"), code).unwrap();
    let changed = rsharp_core::templates::developer_tools::take_out(&f.root).unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!(std::fs::read(f.root.join("App.csproj")).unwrap(), b"\xEF\xBB\xBF<Project>\r\n  <!-- kept -->\r\n  <PackageReference Include=\"Avalonia.Desktop\" Version=\"12\" />\r\n</Project>\r\n");
    assert_eq!(
        std::fs::read(f.root.join("Program.cs")).unwrap(),
        b"\xEF\xBB\xBFclass Program\r\n{\r\n    // kept\r\n}\r\n"
    );
    assert_eq!(std::fs::read(f.root.join("obj/Program.cs")).unwrap(), code);
}
