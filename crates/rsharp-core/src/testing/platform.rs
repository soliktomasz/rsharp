use super::protocol::{Encoding, Host, STOP_LIMIT, array, text};
use super::*;
use serde_json::{Value, json};
use std::time::Instant;

/// The build emits this serialized AssemblyMetadataAttribute, including its Boolean value.
pub(super) fn is_application(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let key = b"Microsoft.Testing.Platform.Application";
    let mut marker = vec![1, 0, key.len() as u8];
    marker.extend_from_slice(key);
    if let Some(at) = bytes.windows(marker.len()).position(|part| part == marker) {
        let value = &bytes[at + marker.len()..];
        return value.len() >= 5 && value[0] == 4 && value[1..5].eq_ignore_ascii_case(b"true");
    }
    // Pre-1.8 platform applications have an entry point and a runtime config, but no metadata marker.
    let deps = path.with_extension("deps.json");
    let Some(deps) = std::fs::read(deps).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else {
        return false;
    };
    has_entry_point(&bytes)
        && path.with_extension("runtimeconfig.json").is_file()
        && deps.get("libraries").and_then(Value::as_object).is_some_and(|libraries| {
            libraries.keys().any(|name| {
                name.strip_prefix("Microsoft.Testing.Platform/1.")
                    .and_then(|version| version.split('.').next())
                    .and_then(|minor| minor.parse::<u32>().ok())
                    .is_some_and(|minor| minor < 8)
            })
        })
}

fn has_entry_point(image: &[u8]) -> bool {
    let read16 = |at: usize| Some(u16::from_le_bytes(image.get(at..at.checked_add(2)?)?.try_into().ok()?));
    let read32 = |at: usize| Some(u32::from_le_bytes(image.get(at..at.checked_add(4)?)?.try_into().ok()?));
    (|| -> Option<u32> {
        let pe = read32(0x3c)? as usize;
        if image.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let count = read16(pe.checked_add(6)?)? as usize;
        let optional = pe.checked_add(24)?;
        let sections = optional.checked_add(read16(pe.checked_add(20)?)? as usize)?;
        let directories = match read16(optional)? {
            0x10b => 96,
            0x20b => 112,
            _ => return None,
        };
        let cli_directory = optional.checked_add(directories + 14 * 8)?;
        if cli_directory.checked_add(8)? > sections {
            return None;
        }
        let cli = read32(cli_directory)?;
        for index in 0..count {
            let section = sections.checked_add(index * 40)?;
            let address = read32(section.checked_add(12)?)?;
            let size = read32(section.checked_add(8)?)?.max(read32(section.checked_add(16)?)?);
            if cli >= address && cli - address < size {
                let offset = (read32(section.checked_add(20)?)? as usize).checked_add((cli - address) as usize)?;
                return read32(offset.checked_add(20)?);
            }
        }
        None
    })()
    .is_some_and(|token| token != 0)
}

pub(super) fn item(assembly: &TestAssembly, node: &Value) -> Option<TestItem> {
    text(node, "uid").filter(|uid| !uid.is_empty())?;
    if text(node, "node-type") == Some("group") {
        return None;
    }
    let display = text(node, "display-name").or_else(|| text(node, "uid"))?;
    let mut class = text(node, "location.type").unwrap_or_default().to_string();
    if let Some(namespace) = text(node, "location.namespace").filter(|namespace| !namespace.is_empty())
        && !class.is_empty()
        && !class.starts_with(&format!("{namespace}."))
    {
        class = format!("{namespace}.{class}");
    }
    let name = match (class.as_str(), text(node, "location.method")) {
        (class, Some(method)) if !class.is_empty() && !method.is_empty() => {
            format!("{class}.{}", TestItem::method_of(method))
        }
        _ => text(node, "vstest.TestCase.FullyQualifiedName").unwrap_or(display).to_string(),
    };
    let traits = traits(array(node, "traits"));
    Some(TestItem {
        project: assembly.project.clone(),
        fully_qualified_name: name.clone(),
        display_name: display.to_string(),
        framework: Some(assembly.framework.clone()),
        row: TestItem::row_of(&name, display),
        path: text(node, "location.file")
            .filter(|path| !path.is_empty())
            .map(|path| source_path(assembly.path.parent().unwrap_or(Path::new("")), path)),
        line: node.get("location.line-start").and_then(Value::as_u64).unwrap_or(0) as usize,
        traits,
    })
}

pub(super) fn traits(values: &[Value]) -> Vec<(String, String)> {
    let mut traits = Vec::new();
    for value in values {
        if let Some(key) = text(value, "Key").or_else(|| text(value, "key")) {
            traits.push((
                key.to_string(),
                text(value, "Value").or_else(|| text(value, "value")).unwrap_or_default().to_string(),
            ));
        } else if let Some(properties) = value.as_object() {
            for (key, value) in properties {
                traits.push(match value.as_str().filter(|value| !value.is_empty()) {
                    Some(value) => (key.clone(), value.to_string()),
                    None => ("Category".into(), key.clone()),
                });
            }
        }
    }
    traits
}

pub(super) fn outcome(mut test: TestItem, node: &Value) -> Option<TestOutcome> {
    if text(node, "node-type") == Some("group")
        || node.get("retry.is-superseded").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let state = match text(node, "execution-state")? {
        "passed" => TestState::Passed,
        "failed" | "error" | "timed-out" => TestState::Failed,
        "skipped" => TestState::Skipped,
        _ => return None,
    };
    if let Some(display) = text(node, "display-name") {
        test.row = test.row.or_else(|| TestItem::row_of(&test.fully_qualified_name, display));
        test.display_name = display.to_string();
    }
    let output = [text(node, "standardOutput"), text(node, "standardError")]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let mut attachments = Vec::new();
    for at in 0.. {
        let Some(uri) = text(node, &format!("attachments.{at}.uri")) else {
            break;
        };
        attachments
            .push(TestAttachment::of(text(node, &format!("attachments.{at}.display-name")).unwrap_or_default(), uri));
    }
    Some(TestOutcome {
        test,
        state,
        duration: Duration::try_from_secs_f64(
            node.get("time.duration-ms").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0,
        )
        .unwrap_or_default(),
        message: text(node, "error.message").map(str::to_string),
        stack_trace: text(node, "error.stacktrace").map(str::to_string),
        output: (!output.is_empty()).then_some(output),
        attachments,
    })
}

fn start(dotnet: &Path, assembly: &TestAssembly, cancel: &AtomicBool) -> Result<Host, String> {
    let mut host = Host::start(Encoding::Platform, cancel, |port| {
        crate::dotnet::command(
            dotnet,
            [
                "exec".to_string(),
                assembly.path.to_string_lossy().into_owned(),
                "--server".into(),
                "--client-host".into(),
                "127.0.0.1".into(),
                "--client-port".into(),
                port.to_string(),
            ],
            assembly.path.parent().unwrap_or(Path::new(".")),
        )
    })?;
    host.send(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":std::process::id(),"clientInfo":{"name":"RSharp","version":env!("CARGO_PKG_VERSION")},"capabilities":{"testing":{"debuggerProvider":false,"isStateful":true}}}}))?;
    let began = Instant::now();
    loop {
        if canceled(cancel) {
            return Err("Test request stopped".into());
        }
        if began.elapsed() > Duration::from_secs(60) {
            return Err("The test host did not initialize within 60 seconds".into());
        }
        if let Some(message) = host.receive()? {
            if message.get("id") == Some(&json!(1)) && message.get("method").is_none() {
                if let Some(error) = message.get("error") {
                    return Err(format!("The test host refused initialization: {error}"));
                }
                break;
            }
            acknowledge(&mut host, &message)?;
        }
    }
    Ok(host)
}

fn acknowledge(host: &mut Host, message: &Value) -> Result<(), String> {
    if message.get("method").is_some()
        && let Some(id) = message.get("id")
    {
        host.send(&json!({"jsonrpc":"2.0","id":id,"result":null}))?;
    }
    Ok(())
}

/// The response can precede the final update; both must arrive before the process is allowed to exit.
fn request(
    dotnet: &Path,
    assembly: &TestAssembly,
    method: &str,
    mut parameters: Value,
    cancel: &AtomicBool,
    event: Events<'_>,
    changed: &mut dyn FnMut(&Value),
) -> Result<bool, String> {
    let mut host = start(dotnet, assembly, cancel)?;
    let run_id = format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), super::runners::next_request());
    parameters["runId"] = json!(run_id);
    host.send(&json!({"jsonrpc":"2.0","id":2,"method":method,"params":parameters}))?;
    let mut stopped = None;
    let mut answered = None;
    let mut ended = false;
    let result = loop {
        if canceled(cancel) && stopped.is_none() {
            host.send(&json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":2}}))?;
            stopped = Some(Instant::now());
        }
        if stopped.is_some_and(|stopped| stopped.elapsed() >= STOP_LIMIT) {
            break Ok(false);
        }
        if let Some(message) = host.receive()? {
            match text(&message, "method") {
                Some("testing/testUpdates/tests") => {
                    let parameters = &message["params"];
                    if text(parameters, "runId") == Some(run_id.as_str()) {
                        if let Some(changes) = parameters.get("changes").and_then(Value::as_array) {
                            for change in changes {
                                if let Some(node) = change.get("node") {
                                    changed(node);
                                }
                            }
                        } else {
                            ended = true;
                        }
                    }
                }
                Some("client/log" | "client/showMessage") => {
                    if let Some(message) = text(&message["params"], "message") {
                        event(TestEvent::Log(message.to_string()));
                    }
                }
                _ => {}
            }
            acknowledge(&mut host, &message)?;
            if message.get("id") == Some(&json!(2)) && message.get("method").is_none() {
                if let Some(error) = message.get("error") {
                    if stopped.is_some() {
                        break Ok(false);
                    }
                    break Err(format!("The test host refused {method}: {error}"));
                }
                answered = Some(Instant::now());
            }
        }
        if ended && answered.is_some() {
            break Ok(stopped.is_none());
        }
        if answered.is_some_and(|answered| answered.elapsed() >= Duration::from_secs(10)) {
            break Err(format!("The test host answered {method} without ending its updates"));
        }
    };
    let _ = host.send(&json!({"jsonrpc":"2.0","method":"exit","params":{}}));
    host.finish(if stopped.is_some() { STOP_LIMIT } else { Duration::from_secs(10) });
    result
}

pub(super) fn discover(
    dotnet: &Path,
    assembly: &TestAssembly,
    cancel: &AtomicBool,
    event: Events<'_>,
) -> Result<Vec<(TestItem, Value)>, String> {
    let mut nodes = Vec::new();
    let complete = request(dotnet, assembly, "testing/discoverTests", json!({}), cancel, event, &mut |node| {
        if let Some(test) = item(assembly, node) {
            nodes.push((test, node.clone()));
        }
    })?;
    if !complete {
        return Err("Test discovery stopped".into());
    }
    Ok(nodes)
}

pub(super) fn run(
    dotnet: &Path,
    assembly: &TestAssembly,
    tests: &[(TestItem, Value)],
    cancel: &AtomicBool,
    event: Events<'_>,
) -> RunSummary {
    let began = Instant::now();
    let selected: Vec<_> =
        tests.iter().map(|(_, node)| json!({"uid":node["uid"],"display-name":node["display-name"]})).collect();
    let by_uid: std::collections::HashMap<_, _> =
        tests.iter().filter_map(|(test, node)| text(node, "uid").map(|uid| (uid, test))).collect();
    let mut outcomes = Vec::new();
    let events = std::cell::RefCell::new(event);
    let result = request(
        dotnet,
        assembly,
        "testing/runTests",
        json!({"tests":selected}),
        cancel,
        &mut |message| (**events.borrow_mut())(message),
        &mut |node| {
            if let Some(test) = text(node, "uid").and_then(|uid| by_uid.get(uid).copied())
                && let Some(result) = outcome(test.clone(), node)
            {
                (**events.borrow_mut())(TestEvent::Outcomes(vec![result.clone()]));
                outcomes.push(result);
            }
        },
    );
    let error = result.as_ref().err().cloned().filter(|_| !canceled(cancel));
    RunSummary {
        elapsed: began.elapsed(),
        canceled: canceled(cancel) || matches!(result, Ok(false)),
        aborted: error.is_some(),
        error,
        outcomes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn older_platform_detection_requires_a_managed_entry_point() {
        let mut image = vec![0; 1024];
        image[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        image[64..68].copy_from_slice(b"PE\0\0");
        image[70..72].copy_from_slice(&1u16.to_le_bytes());
        image[84..86].copy_from_slice(&224u16.to_le_bytes());
        image[88..90].copy_from_slice(&0x10bu16.to_le_bytes());
        image[296..300].copy_from_slice(&0x2000u32.to_le_bytes());
        image[320..324].copy_from_slice(&512u32.to_le_bytes());
        image[324..328].copy_from_slice(&0x2000u32.to_le_bytes());
        image[328..332].copy_from_slice(&512u32.to_le_bytes());
        image[332..336].copy_from_slice(&512u32.to_le_bytes());
        assert!(!has_entry_point(&image), "A class library that references old MTP is a VSTest assembly");
        image[532..536].copy_from_slice(&0x06000001u32.to_le_bytes());
        assert!(has_entry_point(&image));
        assert!(!has_entry_point(&image[..400]), "A truncated executable cannot be started");
    }

    #[test]
    fn platform_properties_keep_source_rows_traits_errors_and_attachments() {
        let assembly = TestAssembly {
            project: "/p/tests.csproj".into(),
            framework: "net10.0".into(),
            path: "/p/Tests.dll".into(),
        };
        let node = json!({"uid":"row-id", "display-name":"Space.Tests.Theory(value: 1)", "location.namespace":"Space", "location.type":"Tests", "location.method":"Theory(Int32)", "location.file":"Tests.cs", "location.line-start":12,
            "traits":[{"Key":"Owner","Value":"Me"},{"Smoke":""}], "execution-state":"failed", "error.message":"broken", "error.stacktrace":"at Tests.cs:12", "time.duration-ms":2.5, "standardOutput":"hello", "standardError":"warning", "attachments.0.uri":"file:///tmp/a%20b.png", "attachments.0.display-name":"frame"});
        let test = item(&assembly, &node).unwrap();
        assert_eq!(test.fully_qualified_name, "Space.Tests.Theory");
        assert_eq!(test.row.as_deref(), Some("Theory(value: 1)"));
        assert_eq!(test.path, Some(PathBuf::from("/p/Tests.cs")));
        assert_eq!(test.line, 12);
        assert_eq!(test.traits, [("Owner".into(), "Me".into()), ("Category".into(), "Smoke".into())]);
        let result = outcome(test, &node).unwrap();
        assert_eq!(result.state, TestState::Failed);
        assert_eq!(result.duration, Duration::from_micros(2500));
        assert_eq!(result.output.as_deref(), Some("hello\nwarning"));
        assert_eq!(result.message.as_deref(), Some("broken"));
        assert_eq!(result.attachments[0], TestAttachment::of("frame", "file:///tmp/a%20b.png"));
        let mut retry = node.clone();
        retry["retry.is-superseded"] = json!(true);
        assert!(outcome(result.test.clone(), &retry).is_none());
        retry["retry.is-superseded"] = json!(false);
        retry["execution-state"] = json!("canceled");
        assert!(outcome(result.test, &retry).is_none());
    }
}
