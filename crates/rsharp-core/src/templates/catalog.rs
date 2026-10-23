//! Reads the SDK cache and each template's IDE host; packages stay zipped and open only for this read.

use super::*;
use async_zip::base::read::seek::ZipFileReader;
use futures::io::{AllowStdIo, AsyncReadExt, BufReader};
use serde_json_lenient::Value;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};

const NOT_SHOWN: &[&str] = &[
    "name",
    "type",
    "language",
    "Framework",
    "TargetFrameworkOverride",
    "skipRestore",
    "langVersion",
    "HostIdentifier",
];

pub fn parse_cache(path: &Path, sdk: &str, platform: TemplatePlatform) -> Result<TemplateCatalog, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: Value = serde_json_lenient::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let all = prop(&value, "TemplateInfo").as_array().ok_or("The template cache has no TemplateInfo array")?;
    let mut hosts = HostFiles::default();
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for template in all {
        if !matches!(tag(template, "type"), Some("project" | "solution")) || !usable(template, platform)? {
            continue;
        }
        let Some(id) = text(template, "GroupIdentity").or_else(|| text(template, "Identity")) else { continue };
        if let Some((_, group)) = groups.iter_mut().find(|(key, _)| key == id) {
            group.push(template);
        } else {
            groups.push((id.to_string(), vec![template]));
        }
    }
    let mut templates = Vec::new();
    for (_, mut group) in groups {
        group.sort_by_key(|v| {
            (std::cmp::Reverse(number(v, "Precedence")), language_order(tag(v, "language").unwrap_or("")))
        });
        let mut variants = Vec::new();
        let mut newest = None;
        for entry in group {
            if let Some(v) = variant(entry, platform, &mut hosts)? {
                newest.get_or_insert(entry);
                variants.push(v);
            }
        }
        let Some(newest) = newest else { continue };
        let short_names = strings(prop(newest, "ShortNameList"));
        if short_names.is_empty() {
            continue;
        }
        let name = text(newest, "Name").or_else(|| text(newest, "Identity")).unwrap_or("").to_string();
        let base = text(newest, "DefaultName").unwrap_or("").trim_end_matches(|c: char| c.is_ascii_digit());
        let default_name =
            if base.is_empty() { name.chars().filter(|c| c.is_alphanumeric()).collect() } else { base.to_string() };
        templates.push(ProjectTemplate {
            family: name.clone(),
            type_name: name.clone(),
            name,
            short_names,
            author: text(newest, "Author").unwrap_or("").into(),
            classifications: strings(prop(newest, "Classifications")),
            default_name,
            variants,
            is_solution: tag(newest, "type") == Some("solution"),
            pack: None,
        });
    }
    let (solution_formats, solution_format) = formats(all);
    templates.extend(packs::offered(&templates));
    Ok(TemplateCatalog { sdk: sdk.into(), families: families::of(templates), solution_formats, solution_format })
}

fn prop<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .as_object()
        .and_then(|o| o.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, v)| v))
        .unwrap_or(&Value::Null)
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    prop(v, key).as_str()
}
fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    prop(v, key).as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}
fn boolean(v: &Value, key: &str) -> bool {
    let p = prop(v, key);
    p.as_bool().unwrap_or_else(|| p.as_str().is_some_and(|v| v.eq_ignore_ascii_case("true")))
}
fn number(v: &Value, key: &str) -> i64 {
    let p = prop(v, key);
    p.as_i64().or_else(|| p.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}
fn scalar(v: &Value, key: &str) -> Option<String> {
    match prop(v, key) {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(prop(v, key).to_string()),
        _ => None,
    }
}
fn tag<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    text(prop(v, "TagsCollection"), key)
}
fn plain(s: &str) -> String {
    regex::Regex::new("_(_?)").unwrap().replace_all(s, "$1").into_owned()
}
fn shown(name: &str) -> bool {
    !NOT_SHOWN.iter().any(|n| n.eq_ignore_ascii_case(name))
}

fn usable(v: &Value, platform: TemplatePlatform) -> Result<bool, String> {
    if platform != TemplatePlatform::Windows
        && strings(prop(v, "Classifications")).iter().any(|c| matches!(c.as_str(), "WPF" | "WinForms"))
    {
        return Ok(false);
    }
    for constraint in array(v, "Constraints") {
        if text(constraint, "Type") != Some("os") {
            continue;
        }
        let Some(args) = text(constraint, "Args") else { continue };
        let parsed: Value = if args.trim_start().starts_with(['[', '"']) {
            serde_json_lenient::from_str(args).map_err(|e| format!("Invalid template OS constraint: {e}"))?
        } else {
            Value::String(args.into())
        };
        let systems = if let Some(s) = parsed.as_str() { vec![s.to_string()] } else { strings(&parsed) };
        if !systems.iter().any(|s| match platform {
            TemplatePlatform::MacOS => ["OSX", "macOS"].iter().any(|n| s.eq_ignore_ascii_case(n)),
            TemplatePlatform::Windows => s.eq_ignore_ascii_case("Windows"),
            TemplatePlatform::Linux => s.eq_ignore_ascii_case("Linux"),
        }) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn targetable(framework: &str, platform: TemplatePlatform) -> bool {
    let lower = framework.to_ascii_lowercase();
    !(lower.strip_prefix("net").is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        || platform != TemplatePlatform::Windows && lower.ends_with("-windows"))
}

fn choices(parameter: &Value) -> Vec<TemplateChoice> {
    prop(parameter, "Choices")
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(name, value)| TemplateChoice {
                    value: name.clone(),
                    label: text(value, "DisplayName").filter(|s| !s.trim().is_empty()).unwrap_or(name).into(),
                    description: text(value, "Description").map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn long_name(cli: &Value, name: &str) -> String {
    text(prop(prop(cli, "symbolInfo"), name), "longName").filter(|s| !s.is_empty()).unwrap_or(name).to_string()
}

fn option(
    parameter: &Value,
    cli: &Value,
    inverted: bool,
    label: Option<&str>,
    description: Option<&str>,
) -> Option<TemplateOption> {
    let name = text(parameter, "Name")?;
    if text(parameter, "Type").is_some_and(|t| t != "parameter") {
        return None;
    }
    let precedence = prop(parameter, "Precedence");
    if precedence.is_object() && matches!(number(precedence, "PrecedenceDefinition"), 3 | 5) {
        return None;
    }
    let kind = match text(parameter, "DataType").unwrap_or("string").to_ascii_lowercase().as_str() {
        "bool" | "boolean" => TemplateOptionKind::Bool,
        "choice" => TemplateOptionKind::Choice,
        "int" | "integer" => TemplateOptionKind::Integer,
        "float" => TemplateOptionKind::Float,
        _ => TemplateOptionKind::Text,
    };
    let choices = choices(parameter);
    if kind == TemplateOptionKind::Choice && choices.is_empty() {
        return None;
    }
    let default = scalar(parameter, "DefaultValue").unwrap_or_else(|| match kind {
        TemplateOptionKind::Bool => "false".into(),
        TemplateOptionKind::Choice => choices[0].value.clone(),
        _ => String::new(),
    });
    Some(TemplateOption {
        name: name.into(),
        option: long_name(cli, name),
        label: plain(
            label.or_else(|| text(parameter, "DisplayName")).or_else(|| text(parameter, "Description")).unwrap_or(name),
        ),
        description: Some(plain(description.or_else(|| text(parameter, "Description")).unwrap_or(""))),
        kind,
        default,
        choices,
        inverted: inverted && kind == TemplateOptionKind::Bool,
        required: boolean(precedence, "IsRequired"),
        enabled_when: text(precedence, "IsEnabledCondition").map(str::to_string),
    })
}

fn variant(v: &Value, platform: TemplatePlatform, hosts: &mut HostFiles) -> Result<Option<TemplateVariant>, String> {
    let parameters = array(v, "Parameters");
    let find = |name: &str| parameters.iter().find(|p| text(p, "Name") == Some(name));
    let cli: Value = text(v, "HostData")
        .map(serde_json_lenient::from_str)
        .transpose()
        .map_err(|e| format!("Invalid CLI host: {e}"))?
        .unwrap_or(Value::Null);
    let mut frameworks = Vec::new();
    let mut framework_option = None;
    let mut default_framework = None;
    if let Some(framework) = find("Framework") {
        framework_option = Some(long_name(&cli, "Framework"));
        let offered = choices(framework);
        frameworks = offered.iter().filter(|c| targetable(&c.value, platform)).map(|c| c.value.clone()).collect();
        if !offered.is_empty() && frameworks.is_empty() {
            return Ok(None);
        }
        default_framework = scalar(framework, "DefaultValue")
            .filter(|v| frameworks.contains(v))
            .or_else(|| frameworks.first().cloned());
    }
    let mut options = Vec::new();
    if let Some(host) = hosts.read(v) {
        for symbol in array(&host, "symbolInfo") {
            if let Some(id) = text(symbol, "id").filter(|id| shown(id))
                && boolean(symbol, "isVisible")
                && let Some(p) = find(id)
                && let Some(o) = option(
                    p,
                    &cli,
                    boolean(symbol, "invertBoolean"),
                    text(prop(symbol, "name"), "text"),
                    text(prop(symbol, "description"), "text"),
                )
            {
                options.push(o);
            }
        }
        if let Some(https) = text(&host, "disableHttpsSymbol")
            && !options.iter().any(|o| o.name == https)
            && let Some(p) = find(https)
            && let Some(o) = option(p, &cli, true, Some("Configure for HTTPS"), None)
        {
            options.push(o);
        }
    } else {
        for p in parameters {
            if let Some(name) = text(p, "Name").filter(|name| shown(name))
                && !boolean(prop(prop(&cli, "symbolInfo"), name), "isHidden")
                && let Some(o) = option(p, &cli, false, None, None)
            {
                options.push(o);
            }
        }
    }
    let identity = text(v, "Identity").ok_or("A template has no Identity")?.to_string();
    Ok(Some(TemplateVariant {
        identity,
        language: tag(v, "language").map(str::to_string),
        precedence: number(v, "Precedence"),
        description: text(v, "Description").unwrap_or("").into(),
        framework_option,
        frameworks,
        default_framework,
        options,
        skip_restore_option: find("skipRestore").map(|_| long_name(&cli, "skipRestore")),
    }))
}

fn formats(all: &[Value]) -> (Vec<String>, String) {
    let parameter = all
        .iter()
        .filter(|t| strings(prop(t, "ShortNameList")).iter().any(|s| s == "sln"))
        .max_by_key(|t| number(t, "Precedence"))
        .and_then(|t| array(t, "Parameters").iter().find(|p| text(p, "Name") == Some("Format")));
    let mut formats: Vec<String> = parameter.map(choices).unwrap_or_default().into_iter().map(|c| c.value).collect();
    if formats.is_empty() {
        return (vec!["sln".into()], "sln".into());
    }
    let default = parameter
        .and_then(|p| scalar(p, "DefaultValue"))
        .filter(|v| formats.contains(v))
        .unwrap_or_else(|| formats[0].clone());
    formats.sort_by_key(|f| if *f == default { 0 } else { 1 });
    (formats, default)
}

type Archive = ZipFileReader<BufReader<AllowStdIo<File>>>;

#[derive(Default)]
struct HostFiles {
    archives: HashMap<PathBuf, Option<Archive>>,
}

impl HostFiles {
    fn read(&mut self, template: &Value) -> Option<Value> {
        let mount = text(template, "MountPointUri")?;
        let mount = crate::paths::from_uri(mount).unwrap_or_else(|| PathBuf::from(mount));
        let place = Path::new(text(template, "ConfigPlace")?.trim_start_matches('/'));
        if place.components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        let entry = place.parent()?.join("ide.host.json");
        let bytes = if mount.is_dir() {
            let file = mount.join(&entry);
            if !crate::paths::canonical(&file).starts_with(crate::paths::canonical(&mount)) {
                return None;
            }
            std::fs::read(file).ok()?
        } else {
            let archive = self
                .archives
                .entry(mount.clone())
                .or_insert_with(|| {
                    let file = File::open(&mount).ok()?;
                    futures::executor::block_on(ZipFileReader::new(BufReader::new(AllowStdIo::new(file)))).ok()
                })
                .as_mut()?;
            let name = entry.to_string_lossy().replace('\\', "/");
            let index =
                archive.file().entries().iter().position(|e| e.filename().as_str().ok() == Some(name.as_str()))?;
            futures::executor::block_on(async {
                let mut reader = archive.reader_without_entry(index).await.ok()?;
                let mut bytes = Vec::new();
                reader.read_to_end(&mut bytes).await.ok()?;
                Some(bytes)
            })?
        };
        serde_json_lenient::from_slice(&bytes).ok()
    }
}
