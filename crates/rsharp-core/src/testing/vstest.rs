use super::protocol::{Encoding, Host, STOP_LIMIT, array, text};
use super::*;
use serde_json::{Value, json};
use std::time::Instant;

const SETTINGS: &str = "<RunSettings><RunConfiguration><DesignMode>true</DesignMode></RunConfiguration></RunSettings>";

fn property<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.get(name).or_else(|| {
        array(value, "Properties").iter().find_map(|property| {
            let key = &property["Key"];
            let id = key.as_str().or_else(|| text(key, "Id"))?;
            (id == name || id.strip_prefix("TestCase.") == Some(name) || id.strip_prefix("TestResult.") == Some(name))
                .then(|| &property["Value"])
        })
    })
}

pub(super) fn item(assembly: &TestAssembly, case: &Value) -> Option<TestItem> {
    let name = property(case, "FullyQualifiedName")?.as_str()?;
    let display = property(case, "DisplayName").and_then(Value::as_str).unwrap_or(name);
    let traits = property(case, "TestObject.Traits")
        .and_then(Value::as_array)
        .map(|traits| super::platform::traits(traits))
        .unwrap_or_default();
    Some(TestItem {
        project: assembly.project.clone(),
        fully_qualified_name: name.to_string(),
        display_name: display.to_string(),
        framework: Some(assembly.framework.clone()),
        row: TestItem::row_of(name, display),
        path: property(case, "CodeFilePath")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(|path| source_path(assembly.path.parent().unwrap_or(Path::new("")), path)),
        line: property(case, "LineNumber").and_then(Value::as_u64).unwrap_or(0) as usize,
        traits,
    })
}

pub(super) fn uid(case: &Value) -> String {
    property(case, "Id").and_then(Value::as_str).unwrap_or_default().to_string()
}

fn case_key(case: &Value) -> (String, String) {
    (uid(case), property(case, "Source").and_then(Value::as_str).unwrap_or_default().to_string())
}

fn duration(value: &str) -> Duration {
    let mut parts = value.rsplit(':');
    let seconds = parts.next().and_then(|part| part.parse::<f64>().ok()).unwrap_or(0.0);
    let minutes = parts.next().and_then(|part| part.parse::<f64>().ok()).unwrap_or(0.0);
    let hours = parts.next().unwrap_or("0");
    let (days, hours) =
        hours.split_once('.').map_or((0.0, hours), |(days, hours)| (days.parse().unwrap_or(0.0), hours));
    Duration::try_from_secs_f64(
        days * 86400.0 + hours.parse::<f64>().unwrap_or(0.0) * 3600.0 + minutes * 60.0 + seconds,
    )
    .unwrap_or_default()
}

pub(super) fn outcome(mut test: TestItem, result: &Value) -> TestOutcome {
    let state = match property(result, "Outcome") {
        Some(Value::Number(number)) => match number.as_u64() {
            Some(1) => TestState::Passed,
            Some(2) => TestState::Failed,
            Some(3) => TestState::Skipped,
            _ => TestState::NotRun,
        },
        Some(Value::String(state)) => match state.as_str() {
            "Passed" => TestState::Passed,
            "Failed" => TestState::Failed,
            "Skipped" => TestState::Skipped,
            _ => TestState::NotRun,
        },
        _ => TestState::NotRun,
    };
    if let Some(display) = property(result, "DisplayName").and_then(Value::as_str).filter(|name| !name.is_empty()) {
        test.row = test.row.or_else(|| TestItem::row_of(&test.fully_qualified_name, display));
        test.display_name = display.to_string();
    }
    let output =
        array(result, "Messages").iter().filter_map(|message| text(message, "Text")).collect::<Vec<_>>().join("\n");
    let mut attachments = Vec::new();
    for set in array(result, "Attachments") {
        for attachment in array(set, "Attachments") {
            if let Some(uri) = text(attachment, "Uri") {
                attachments.push(TestAttachment::of(text(attachment, "Description").unwrap_or_default(), uri));
            }
        }
    }
    TestOutcome {
        test,
        state,
        duration: property(result, "Duration").and_then(Value::as_str).map(duration).unwrap_or_default(),
        message: property(result, "ErrorMessage").and_then(Value::as_str).map(str::to_string),
        stack_trace: property(result, "ErrorStackTrace").and_then(Value::as_str).map(str::to_string),
        output: (!output.is_empty()).then_some(output),
        attachments,
    }
}

fn sdk_console(dotnet: &Path, folder: &Path, cancel: &AtomicBool) -> Result<PathBuf, String> {
    let mut info = String::new();
    let code = crate::build::run(crate::dotnet::command(dotnet, ["--info"], folder), cancel, |text, _| {
        if info.len() < 65536 {
            info.push_str(text);
        }
    })
    .map_err(|e| e.to_string())?;
    if code.is_none() {
        return Err("Test request stopped".into());
    }
    let path = info
        .lines()
        .find_map(|line| {
            line.trim().strip_prefix("Base Path:").map(|path| PathBuf::from(path.trim()).join("vstest.console.dll"))
        })
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("The folder's .NET SDK has no vstest.console: {}", info.trim()))?;
    Ok(path)
}

fn send(host: &mut Host, kind: &str, payload: Value) -> Result<(), String> {
    host.send(&json!({"Version":7,"MessageType":kind,"Payload":payload}))
}

fn start(dotnet: &Path, folder: &Path, cancel: &AtomicBool) -> Result<Host, String> {
    let console = sdk_console(dotnet, folder, cancel)?;
    let mut host = Host::start(Encoding::VsTest, cancel, |port| {
        crate::dotnet::command(
            dotnet,
            [
                console.to_string_lossy().into_owned(),
                format!("/parentprocessid:{}", std::process::id()),
                format!("/port:{port}"),
            ],
            folder,
        )
    })?;
    let began = Instant::now();
    let mut connected = false;
    loop {
        if canceled(cancel) {
            return Err("Test request stopped".into());
        }
        if began.elapsed() > Duration::from_secs(60) {
            return Err("VSTest did not initialize within 60 seconds".into());
        }
        if let Some(message) = host.receive()? {
            match text(&message, "MessageType") {
                Some("TestSession.Connected") => {
                    connected = true;
                    host.send(&json!({"MessageType":"ProtocolVersion","Payload":7}))?;
                }
                Some("ProtocolVersion") if connected => {
                    if message["Payload"].as_u64() != Some(7) {
                        return Err(format!("VSTest does not support protocol 7: {}", message["Payload"]));
                    }
                    send(&mut host, "Extensions.Initialize", json!([]))?;
                    return Ok(host);
                }
                Some("ProtocolError") => return Err(format!("VSTest refused the protocol: {}", message["Payload"])),
                _ => {}
            }
        }
    }
}

fn log(message: &Value, event: Events<'_>) {
    if text(message, "MessageType") == Some("TestSession.Message")
        && let Some(message) = text(&message["Payload"], "Message")
    {
        event(TestEvent::Log(message.to_string()));
    }
}

pub(super) fn discover(
    dotnet: &Path,
    folder: &Path,
    assembly: &TestAssembly,
    cancel: &AtomicBool,
    event: Events<'_>,
) -> Result<Vec<(TestItem, Value)>, String> {
    let mut host = start(dotnet, folder, cancel)?;
    send(
        &mut host,
        "TestDiscovery.Start",
        json!({"Sources":[assembly.path],"RunSettings":SETTINGS,"TestPlatformOptions":{}}),
    )?;
    let mut found = Vec::new();
    let mut stopped = None;
    loop {
        if canceled(cancel) && stopped.is_none() {
            send(&mut host, "TestDiscovery.Cancel", Value::Null)?;
            stopped = Some(Instant::now());
        }
        if stopped.is_some_and(|stopped| stopped.elapsed() >= STOP_LIMIT) {
            return Err("Test discovery stopped".into());
        }
        if let Some(message) = host.receive()? {
            log(&message, event);
            let payload = &message["Payload"];
            let kind = text(&message, "MessageType");
            let cases = match kind {
                Some("TestDiscovery.TestFound") => payload.as_array().map(Vec::as_slice).unwrap_or_default(),
                Some("TestDiscovery.Completed") => array(payload, "LastDiscoveredTests"),
                _ => &[],
            };
            for case in cases {
                if let Some(test) = item(assembly, case) {
                    found.push((test, case.clone()));
                }
            }
            if kind == Some("TestDiscovery.Completed") {
                if canceled(cancel) {
                    return Err("Test discovery stopped".into());
                }
                if payload["IsAborted"].as_bool() == Some(true) {
                    return Err("VSTest discovery was aborted; see the test log".into());
                }
                break;
            }
        }
    }
    let _ = send(&mut host, "TestSession.Terminate", Value::Null);
    host.finish(Duration::from_secs(3));
    Ok(found)
}

pub(super) fn run(
    dotnet: &Path,
    folder: &Path,
    tests: &[(TestItem, Value)],
    cancel: &AtomicBool,
    event: Events<'_>,
) -> RunSummary {
    let began = Instant::now();
    let mut outcomes = Vec::new();
    let by_case: std::collections::HashMap<_, _> = tests.iter().map(|(test, case)| (case_key(case), test)).collect();
    let mut run_canceled = false;
    let mut aborted = false;
    let mut error = None;
    let result = (|| -> Result<(), String> {
        let mut host = start(dotnet, folder, cancel)?;
        let cases: Vec<_> = tests.iter().map(|(_, case)| case.clone()).collect();
        send(
            &mut host,
            "TestExecution.RunAllWithDefaultHost",
            json!({"TestCases":cases,"RunSettings":SETTINGS,"TestPlatformOptions":{}}),
        )?;
        let mut stopped = None;
        loop {
            if canceled(cancel) && stopped.is_none() {
                send(&mut host, "TestExecution.Cancel", Value::Null)?;
                stopped = Some(Instant::now());
            }
            if stopped.is_some_and(|stopped| stopped.elapsed() >= STOP_LIMIT) {
                run_canceled = true;
                break;
            }
            if let Some(message) = host.receive()? {
                log(&message, event);
                let payload = &message["Payload"];
                let kind = text(&message, "MessageType");
                let changed = if kind == Some("TestExecution.Completed") { &payload["LastRunTests"] } else { payload };
                let mut batch = Vec::new();
                if matches!(kind, Some("TestExecution.StatsChange" | "TestExecution.Completed")) {
                    for result in array(changed, "NewTestResults") {
                        let case = &result["TestCase"];
                        if let Some(test) = by_case.get(&case_key(case)).copied() {
                            batch.push(outcome(test.clone(), result));
                        }
                    }
                    if !batch.is_empty() {
                        event(TestEvent::Outcomes(batch.clone()));
                        outcomes.extend(batch);
                    }
                }
                if kind == Some("TestExecution.Completed") {
                    let args = &payload["TestRunCompleteArgs"];
                    run_canceled = args["IsCanceled"].as_bool().unwrap_or(false);
                    aborted = args["IsAborted"].as_bool().unwrap_or(false);
                    error = args
                        .get("Error")
                        .filter(|value| !value.is_null())
                        .map(|value| text(value, "Message").map(str::to_string).unwrap_or_else(|| value.to_string()));
                    break;
                }
            }
        }
        let _ = send(&mut host, "TestSession.Terminate", Value::Null);
        host.finish(STOP_LIMIT);
        Ok(())
    })();
    if let Err(failure) = result
        && !canceled(cancel)
    {
        aborted = true;
        error = Some(failure);
    }
    RunSummary { elapsed: began.elapsed(), canceled: run_canceled || canceled(cancel), aborted, error, outcomes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vstest_results_retain_data_rows_properties_and_result_files() {
        let assembly = TestAssembly {
            project: "/p/tests.csproj".into(),
            framework: "net10.0".into(),
            path: "/p/Tests.dll".into(),
        };
        let case = json!({"Id":"uid", "FullyQualifiedName":"Space.Tests.Theory(1)", "DisplayName":"Theory(1)", "Source":"/p/Tests.dll", "CodeFilePath":"/p/Tests.cs", "LineNumber":14,
            "Properties":[{"Key":{"Id":"TestObject.Traits"},"Value":[{"Key":"Category","Value":"Smoke"}]}]});
        let test = item(&assembly, &case).unwrap();
        assert_eq!(test.row.as_deref(), Some("Theory(1)"));
        assert_eq!(test.traits, [("Category".into(), "Smoke".into())]);
        let result = json!({"TestCase":case,"Outcome":2,"Duration":"00:00:00.0025000", "DisplayName":"Theory(2)", "ErrorMessage":"failed", "ErrorStackTrace":"stack", "Messages":[{"Category":"StdOutMsgs","Text":"output"}], "Attachments":[{"Attachments":[{"Uri":"file:///tmp/file%20name.txt","Description":"log"}]}]});
        let result = outcome(test, &result);
        assert_eq!(result.state, TestState::Failed);
        assert_eq!(result.test.row.as_deref(), Some("Theory(1)"));
        assert_eq!(result.duration, Duration::from_micros(2500));
        assert_eq!(result.output.as_deref(), Some("output"));
        assert_eq!(result.attachments[0].name, "log");
        assert_eq!(result.attachments[0].path, PathBuf::from("/tmp/file name.txt"));
    }

    #[test]
    fn outcomes_preserve_discovery_rows_and_derive_only_rows_created_during_execution() {
        let test = TestItem {
            fully_qualified_name: "Space.Tests.Theory".into(),
            row: Some("Theory(1) (2)".into()),
            ..Default::default()
        };
        let result = json!({"DisplayName":"Theory(1)","Outcome":1});
        assert_eq!(
            outcome(test.clone(), &result).test.id(),
            test.id(),
            "A duplicate adapter display name merged distinct discovered tests"
        );
        let mut dynamic = test;
        dynamic.row = None;
        assert_eq!(outcome(dynamic, &result).test.row.as_deref(), Some("Theory(1)"));
    }
}
