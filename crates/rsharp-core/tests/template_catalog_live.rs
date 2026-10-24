use rsharp_core::templates::{TemplateEnvironment, cache::Catalogs};
use std::io::Read;
use std::process::Stdio;

#[test]
fn installed_catalog_matches_dotnet_new_list_in_an_isolated_hive() {
    let mut environment = match TemplateEnvironment::discover() {
        Ok(e) => e,
        Err(why) => {
            eprintln!("SKIP template catalog: {why}");
            return;
        }
    };
    let root = std::env::temp_dir().join(format!("rsharp-template-live-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    environment.cli_home = root.join("home");
    environment.hive = Some(root.join("hive"));
    environment.nuget_packages = Some(root.join("packages"));
    let store = Catalogs::new(environment.clone());
    let catalog = store.load(&root).unwrap();
    assert!(catalog.templates().iter().any(|t| t.short_name() == "console" && t.pack.is_none()));
    let mut unmatched: Vec<_> = catalog.templates().into_iter().filter(|t| t.pack.is_none()).collect();
    for kind in ["project", "solution"] {
        let mut command =
            environment.new_command(&["list".into(), "--type".into(), kind.into(), "--columns-all".into()], &root);
        command.env("DOTNET_CLI_UI_LANGUAGE", "en-US").stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = rsharp_core::process::spawn(command).unwrap();
        let mut output = String::new();
        child.process.stdout.take().unwrap().read_to_string(&mut output).unwrap();
        let status = child.wait().unwrap();
        if kind == "solution" && status.code() == Some(103) {
            continue;
        }
        assert!(status.success(), "dotnet new list failed: {output}");
        let lines: Vec<_> = output.lines().collect();
        let rule = lines.iter().position(|l| l.starts_with("---")).expect("dotnet new list wrote no table");
        let columns: Vec<_> = regex::Regex::new("-+").unwrap().find_iter(lines[rule]).map(|m| m.start()).collect();
        let cell = |line: &str, column: usize| -> String {
            let start = columns[column];
            if start >= line.len() {
                return String::new();
            }
            line[start..columns.get(column + 1).copied().unwrap_or(line.len()).min(line.len())].trim().into()
        };
        let headers: Vec<_> = (0..columns.len()).map(|c| cell(lines[rule - 1], c)).collect();
        let short = headers.iter().position(|h| h == "Short Name").unwrap();
        let name = headers.iter().position(|h| h == "Template Name").unwrap();
        let language = headers.iter().position(|h| h == "Language").unwrap();
        for line in lines[rule + 1..].iter().filter(|l| !l.trim().is_empty()) {
            let shorts = cell(line, short);
            if cfg!(not(windows)) && (line.contains("WPF") || line.contains("WinForms")) {
                continue;
            }
            let shorts: Vec<_> = shorts.split(',').collect();
            let label = cell(line, name);
            let languages: Vec<_> = cell(line, language)
                .split(',')
                .filter(|l| !l.is_empty())
                .map(|l| l.trim_matches(['[', ']']).to_string())
                .collect();
            let found = unmatched.iter().position(|t| {
                t.short_names == shorts
                    && t.languages() == languages
                    && (label.strip_suffix("...").map_or(t.name == label, |prefix| t.name.starts_with(prefix)))
            });
            let index = found.unwrap_or_else(|| panic!("catalog omitted or changed metadata for {line}"));
            unmatched.remove(index);
        }
    }
    assert!(
        unmatched.is_empty(),
        "catalog listed templates the CLI did not report: {:?}",
        unmatched.iter().map(|t| &t.short_names).collect::<Vec<_>>()
    );
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
