use rsharp_core::templates::{TemplateOptionKind, TemplatePlatform, catalog};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const HOST: &str = r#"{
  // An IDE asks only for visible symbols, in the host's order.
  "symbolInfo": [
    { "id": "UseProgramMain", "isVisible": true, "name": {"text":"Do not use _top-level statements"} },
    { "id": "Mode", "isVisible": true },
    { "id": "Count", "isVisible": true },
    { "id": "Feature", "isVisible": true, "invertBoolean": true },
  ],
  "disableHttpsSymbol": "NoHttps",
}"#;

struct Fixture {
    root: PathBuf,
    all: Vec<Value>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rsharp-template-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("mount/console/.template.config")).unwrap();
        std::fs::write(root.join("mount/console/.template.config/ide.host.json"), HOST).unwrap();
        let mut all = Vec::new();
        for (lang, order) in [("VB", 100), ("F#", 100), ("C#", 100), ("C#", 50)] {
            all.push(json!({
                "Identity":format!("Console.{lang}.{order}"), "GroupIdentity":"Console", "Name":"Console App",
                "ShortNameList":["console"], "DefaultName":"ConsoleApp1", "Author":"Microsoft",
                "Classifications":["Common","Console"], "Precedence":order,
                "TagsCollection":{"type":"project","language":lang},
                "MountPointUri":root.join("mount"), "ConfigPlace":"/console/.template.config/template.json",
                "HostData":r#"{"symbolInfo":{"UseProgramMain":{"longName":"use-program-main"},"skipRestore":{"longName":"no-restore"}}}"#,
                "Parameters":[
                    {"Name":"Framework","DataType":"choice","DefaultValue":if order == 100 {"net10.0"} else {"net8.0"},
                     "Choices":if order == 100 {json!({"net10.0":{},"net9.0":{}})} else {json!({"net8.0":{}})}},
                    {"Name":"UseProgramMain","DataType":"bool","DefaultValue":"false"},
                    {"Name":"Mode","DataType":"choice","DefaultValue":"zeta","Choices":{"zeta":{"DisplayName":"Zed"},"alpha":{}}},
                    {"Name":"Count","DataType":"integer","DefaultValue":"1","Precedence":{"PrecedenceDefinition":2,"IsRequired":true,"IsEnabledCondition":"Feature"}},
                    {"Name":"Feature","DataType":"bool","DefaultValue":"true"},
                    {"Name":"NoHttps","DataType":"bool","DefaultValue":"false"},
                    {"Name":"skipRestore","DataType":"bool","DefaultValue":"false"},
                    {"Name":"Internal","DataType":"string","DefaultValue":"hidden"}
                ]
            }));
        }
        all.push(json!({"Identity":"Solution","Name":"Solution File","ShortNameList":["sln"],"Precedence":100,
            "TagsCollection":{"type":"solution"},
            "Parameters":[{"Name":"Format","DataType":"choice","DefaultValue":"slnx","Choices":{"sln":{},"slnx":{}}}]}));
        Self { root, all }
    }

    fn read(&self, platform: TemplatePlatform) -> rsharp_core::templates::TemplateCatalog {
        let path = self.root.join("templatecache.json");
        // The hand-written choice order matters independently of serde_json's map ordering.
        let text = serde_json::to_string(&json!({"TemplateInfo":self.all})).unwrap().replace(
            r#""Choices":{"alpha":{},"zeta":{"DisplayName":"Zed"}}"#,
            r#""Choices":{"zeta":{"DisplayName":"Zed"},"alpha":{}}"#,
        );
        std::fs::write(&path, text).unwrap();
        catalog::parse_cache(&path, "10.0.100", platform).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn metadata_matches_passenger() {
    let fixture = Fixture::new();
    let catalog = fixture.read(TemplatePlatform::MacOS);
    let console = catalog.templates().into_iter().find(|t| t.short_name() == "console").unwrap();
    assert_eq!(console.default_name, "ConsoleApp");
    assert_eq!(console.languages(), ["C#", "F#", "VB"]);
    assert_eq!(console.frameworks(Some("C#")), ["net10.0", "net9.0", "net8.0"]);
    let variant = console.variant(Some("C#"), Some("net8.0"));
    assert_eq!(variant.identity, "Console.C#.50");
    assert_eq!(variant.skip_restore_option.as_deref(), Some("no-restore"));
    assert_eq!(variant.options[0].label, "Do not use top-level statements");
    assert_eq!(variant.options[0].option, "use-program-main");
    assert_eq!(variant.options[2].kind, TemplateOptionKind::Integer);
    assert!(variant.options[2].required);
    assert_eq!(variant.options[2].enabled_when.as_deref(), Some("Feature"));
    assert!(variant.options[3].inverted);
    assert_eq!(variant.options[4].label, "Configure for HTTPS");
    assert!(!variant.options.iter().any(|o| ["skipRestore", "Framework", "Internal"].contains(&o.name.as_str())));
    assert_eq!(catalog.solution_formats, ["slnx", "sln"]);
    assert_eq!(catalog.solution_format, "slnx");
}

#[test]
fn host_json_keeps_choice_order() {
    let catalog = Fixture::new().read(TemplatePlatform::Linux);
    let console = catalog.templates().into_iter().find(|t| t.short_name() == "console").unwrap();
    let option = &console.variant(Some("C#"), None).options[1];
    assert_eq!(option.choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>(), ["zeta", "alpha"]);
    assert_eq!(option.choices[0].label, "Zed");
}

#[test]
fn host_metadata_reads_from_a_folder_and_a_zip() {
    let mut fixture = Fixture::new();
    let before = fixture.read(TemplatePlatform::Linux);
    let bytes = futures::executor::block_on(async {
        let mut zip = async_zip::base::write::ZipFileWriter::new(Vec::<u8>::new());
        zip.write_entry_whole(
            async_zip::ZipEntryBuilder::new(
                "console/.template.config/ide.host.json".into(),
                async_zip::Compression::Deflate,
            ),
            HOST.as_bytes(),
        )
        .await
        .unwrap();
        zip.close().await.unwrap()
    });
    let path = fixture.root.join("templates.nupkg");
    std::fs::write(&path, bytes).unwrap();
    for template in &mut fixture.all[..4] {
        template["MountPointUri"] = json!(rsharp_core::paths::to_uri(&path));
    }
    assert_eq!(before, fixture.read(TemplatePlatform::Linux));
}

#[test]
fn old_sdks_offer_only_sln() {
    let mut fixture = Fixture::new();
    fixture.all.last_mut().unwrap()["Parameters"] = json!([]);
    let catalog = fixture.read(TemplatePlatform::MacOS);
    assert_eq!(catalog.solution_formats, ["sln"]);
    assert_eq!(catalog.solution_format, "sln");
}

#[test]
fn platform_and_framework_constraints_match_passenger() {
    let mut fixture = Fixture::new();
    for (identity, class, framework, constraints) in [
        ("Wpf", "WPF", "net10.0-windows", json!([])),
        ("Legacy", "Library", "net48", json!([])),
        ("Mac", "Other", "net10.0", json!([{"Type":"os","Args":"[\"OSX\"]"}])),
    ] {
        fixture.all.push(
            json!({"Identity":identity,"Name":identity,"ShortNameList":[identity],"Classifications":[class],
            "TagsCollection":{"type":"project","language":"C#"},"Constraints":constraints,
            "Parameters":[{"Name":"Framework","DataType":"choice","Choices":{framework:{}}}]}),
        );
    }
    let mac = fixture.read(TemplatePlatform::MacOS);
    assert!(mac.templates().iter().any(|t| t.name == "Mac"));
    assert!(!mac.templates().iter().any(|t| ["Wpf", "Legacy"].contains(&t.name.as_str())));
    let windows = fixture.read(TemplatePlatform::Windows);
    assert!(windows.templates().iter().any(|t| t.name == "Wpf"));
    assert!(!windows.templates().iter().any(|t| ["Mac", "Legacy"].contains(&t.name.as_str())));
}

#[test]
fn absent_or_invalid_ide_host_falls_back_but_corrupt_cache_is_an_error() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("mount/console/.template.config/ide.host.json"), "broken").unwrap();
    let catalog = fixture.read(TemplatePlatform::Linux);
    let console = catalog.templates().into_iter().find(|t| t.short_name() == "console").unwrap();
    assert!(console.variants[0].options.iter().any(|o| o.name == "Internal"));
    let cache = fixture.root.join("broken.json");
    std::fs::write(&cache, "{").unwrap();
    assert!(catalog::parse_cache(&cache, "10.0.100", TemplatePlatform::Linux).is_err());
}

#[test]
fn cache_and_ide_host_utf8_boms_are_read_like_dotnet() {
    let fixture = Fixture::new();
    let mut host = b"\xEF\xBB\xBF".to_vec();
    host.extend_from_slice(HOST.as_bytes());
    std::fs::write(fixture.root.join("mount/console/.template.config/ide.host.json"), host).unwrap();
    let expected = fixture.read(TemplatePlatform::Linux);
    let console = expected.templates().into_iter().find(|t| t.short_name() == "console").unwrap();
    assert_eq!(console.variants[0].options[0].label, "Do not use top-level statements");
    let path = fixture.root.join("templatecache.json");
    let mut marked = b"\xEF\xBB\xBF".to_vec();
    marked.extend(std::fs::read(&path).unwrap());
    std::fs::write(&path, marked).unwrap();
    assert_eq!(catalog::parse_cache(&path, "10.0.100", TemplatePlatform::Linux).unwrap(), expected);
}
