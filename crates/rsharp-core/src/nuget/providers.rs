//! NuGet's credential providers (Azure Artifacts' and the rest), found where the NuGet client finds them and asked
//! through its plugin protocol, each run through `process::spawn`. A provider is asked without a dialog, as restore
//! asks one in the background; what it gives is kept in memory for the session, never written or logged.

use super::secret::Credentials;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const STEP: Duration = Duration::from_secs(20);
const ANSWER: Duration = Duration::from_secs(60);

/// Credentials for a feed from the first provider that gives any; a retry asks again, past what was given.
pub fn credentials(feed: &str, retry: bool) -> Option<Credentials> {
    static GIVEN: Mutex<Option<HashMap<String, Credentials>>> = Mutex::new(None);
    if !retry && let Some(known) = GIVEN.lock().unwrap().get_or_insert_default().get(feed) {
        return Some(known.clone());
    }
    for plugin in find() {
        let name = plugin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match command_for(&plugin).and_then(|command| ask(command, &name, feed, retry)) {
            Ok(Some(given)) => {
                GIVEN.lock().unwrap().get_or_insert_default().insert(feed.to_string(), given.clone());
                return Some(given);
            }
            Ok(None) => {}
            Err(why) => log::info!("NuGet: the credential provider {name} gave nothing for {feed}: {why}"),
        }
    }
    None
}

/// The providers, as NuGet finds them: the variable's paths alone when it is set, else each folder's assembly under
/// ~/.nuget/plugins/netcore and the nuget-plugin-* programs on PATH.
pub fn find() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(named) = var("NUGET_NETCORE_PLUGIN_PATHS").or_else(|| var("NUGET_PLUGIN_PATHS")) {
        return std::env::split_paths(&named).filter(|p| p.is_file()).collect();
    }
    let mut found = Vec::new();
    let home = var("NUGET_HOME").map(PathBuf::from).unwrap_or_else(|| crate::programs::home().join(".nuget"));
    let mut folders: Vec<_> = std::fs::read_dir(home.join("plugins/netcore"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let name = folder.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let assembly = folder.join(format!("{name}.dll"));
        if assembly.is_file() {
            found.push(assembly);
        }
    }
    for place in std::env::split_paths(&var("PATH").unwrap_or_default()) {
        let mut programs: Vec<_> = std::fs::read_dir(&place)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("nuget-plugin-")) && runs(p))
            .collect();
        programs.sort();
        for program in programs {
            if !found.contains(&program) {
                found.push(program);
            }
        }
    }
    found
}

fn runs(program: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        program.is_file() && std::fs::metadata(program).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        program.is_file() && program.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    }
}

fn command_for(plugin: &Path) -> Result<Command, String> {
    let mut command = if plugin.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
        let mut dotnet = Command::new(crate::programs::dotnet().ok_or("dotnet is not installed")?);
        dotnet.arg(plugin);
        dotnet
    } else {
        Command::new(plugin)
    };
    command.arg("-Plugin");
    if let Some(folder) = plugin.parent() {
        command.current_dir(folder);
    }
    Ok(command)
}

enum Said {
    Answer(Value),
    Fault(String),
}

/// One provider asked: started, its handshake shaken, then whether it authenticates and the feed's credentials.
pub fn ask(mut command: Command, name: &str, feed: &str, retry: bool) -> Result<Option<Credentials>, String> {
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = crate::process::spawn(command).map_err(|e| e.to_string())?;
    let input = Arc::new(Mutex::new(child.process.stdin.take().ok_or("the provider has no input")?));
    let output = child.process.stdout.take().ok_or("the provider has no output")?;
    let (answers, answered) = mpsc::channel::<(String, Said)>();
    let replies = input.clone();
    // The provider's own requests are answered here as NuGet answers them, its answers to ours sent on.
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let Ok(line) = line else { break };
            // A line that is no message (what a provider prints before it speaks the protocol) is passed over.
            let Ok(message) = serde_json::from_str::<Value>(line.trim()) else { continue };
            let id = message["RequestId"].as_str().unwrap_or("").to_string();
            let method = message["Method"].as_str().unwrap_or("").to_string();
            match message["Type"].as_str() {
                Some("Request") => {
                    let payload = match method.as_str() {
                        "Handshake" => json!({"ResponseCode": "Success", "ProtocolVersion": "2.0.0"}),
                        "Log" | "SetLogLevel" => json!({"ResponseCode": "Success"}),
                        _ => json!({"ResponseCode": "NotFound"}),
                    };
                    let reply = json!({"RequestId": id, "Type": "Response", "Method": method, "Payload": payload});
                    let _ = writeln!(replies.lock().unwrap(), "{reply}");
                }
                Some("Response") => {
                    let _ = answers.send((id, Said::Answer(message["Payload"].clone())));
                }
                Some("Fault") => {
                    let why = message["Payload"]["Message"].as_str().unwrap_or("a fault").to_string();
                    let _ = answers.send((id, Said::Fault(why)));
                }
                _ => {}
            }
        }
    });
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let request = |method: &str, payload: Value, limit: Duration| -> Result<Value, String> {
        let id = format!("rsharp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let message = json!({"RequestId": id, "Type": "Request", "Method": method, "Payload": payload});
        writeln!(input.lock().unwrap(), "{message}").map_err(|e| format!("{name} could not be told {method}: {e}"))?;
        let deadline = std::time::Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match answered.recv_timeout(left) {
                Ok((said, Said::Answer(payload))) if said == id => {
                    return match payload["ResponseCode"].as_str() {
                        Some("Success") => Ok(payload),
                        code => Err(format!("{name} answered {method} with {}", code.unwrap_or("nothing"))),
                    };
                }
                Ok((said, Said::Fault(why))) if said == id => {
                    return Err(format!("{name} answered {method} with {why}"));
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!("{name} did not answer {method} in {} seconds", limit.as_secs()));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(format!("{name} ended before it answered")),
            }
        }
    };
    let asked = (|| {
        request("Handshake", json!({"ProtocolVersion": "2.0.0", "MinimumProtocolVersion": "1.0.0"}), STEP)?;
        request(
            "Initialize",
            json!({"ClientVersion": "7.0.0", "Culture": "en-US", "RequestTimeout": "00:01:00"}),
            STEP,
        )?;
        let claims = request("GetOperationClaims", json!({}), STEP)?;
        if !claims["Claims"].as_array().is_some_and(|c| c.iter().any(|c| c == "Authentication")) {
            return Ok(None);
        }
        let given = request(
            "GetAuthenticationCredentials",
            json!({"Uri": feed, "IsRetry": retry, "IsNonInteractive": true, "CanShowDialog": false}),
            ANSWER,
        )?;
        Ok(match (given["Username"].as_str(), given["Password"].as_str()) {
            (Some(username), Some(password)) if !password.is_empty() => {
                Some(Credentials { username: username.into(), password: password.into() })
            }
            _ => None,
        })
    })();
    let _ = writeln!(input.lock().unwrap(), "{}", json!({"RequestId": "close", "Type": "Request", "Method": "Close"}));
    drop(input);
    // A provider that does not end once told to is ended, its whole tree with it.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while child.process.try_wait().ok().flatten().is_none() {
        if std::time::Instant::now() > deadline {
            child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.wait();
    asked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "nuget::providers::tests::stand_in_provider", "--ignored", "--nocapture"])
            .env("RSHARP_PROVIDER_CASE", mode);
        command
    }

    /// A credential provider as NuGet's plugin protocol has one: it shakes hands first, logs, then answers.
    #[test]
    #[ignore = "a stand-in provider, started only by the provider tests"]
    fn stand_in_provider() {
        let mode = std::env::var("RSHARP_PROVIDER_CASE").unwrap();
        let out = |message: Value| {
            println!("{message}");
            std::io::stdout().flush().unwrap();
        };
        out(
            json!({"RequestId": "p1", "Type": "Request", "Method": "Handshake", "Payload": {"ProtocolVersion": "2.0.0"}}),
        );
        let mut input = String::new();
        let stdin = std::io::stdin();
        loop {
            input.clear();
            if stdin.lock().read_line(&mut input).unwrap() == 0 {
                return;
            }
            let Ok(message) = serde_json::from_str::<Value>(&input) else { continue };
            if message["Type"] != "Request" {
                continue;
            }
            let id = message["RequestId"].clone();
            let method = message["Method"].as_str().unwrap().to_string();
            let respond = |payload: Value| {
                out(json!({"RequestId": id, "Type": "Response", "Method": method, "Payload": payload}))
            };
            match method.as_str() {
                "Handshake" => respond(json!({"ResponseCode": "Success", "ProtocolVersion": "2.0.0"})),
                "Initialize" => {
                    out(
                        json!({"RequestId": "p2", "Type": "Request", "Method": "Log", "Payload": {"Message": "ready"}}),
                    );
                    respond(json!({"ResponseCode": "Success"}))
                }
                "GetOperationClaims" if mode == "none" => respond(json!({"ResponseCode": "Success", "Claims": []})),
                "GetOperationClaims" => respond(json!({"ResponseCode": "Success", "Claims": ["Authentication"]})),
                "GetAuthenticationCredentials" => {
                    let user =
                        format!("{}-{}", message["Payload"]["Uri"].as_str().unwrap(), message["Payload"]["IsRetry"]);
                    respond(json!({"ResponseCode": "Success", "Username": user, "Password": "tok"}))
                }
                "Close" => return,
                _ => respond(json!({"ResponseCode": "NotFound"})),
            }
        }
    }

    #[test]
    fn a_provider_gives_the_feeds_credentials_through_the_plugin_protocol() {
        let given = ask(provider("give"), "stand-in", "https://feed.example/index.json", true).unwrap();
        assert_eq!(
            given,
            Some(Credentials { username: "https://feed.example/index.json-true".into(), password: "tok".into() }),
            "the provider was not asked for this feed as a retry"
        );
        assert_eq!(
            ask(provider("none"), "stand-in", "https://x", false).unwrap(),
            None,
            "a provider claiming no authentication was asked anyway"
        );
    }

    #[test]
    fn find_takes_the_variables_paths_alone() {
        let dir = std::env::temp_dir().join(format!("rsharp-providers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let named = dir.join("Provider.dll");
        std::fs::write(&named, "").unwrap();
        // SAFETY: no other test reads this variable.
        unsafe {
            std::env::set_var(
                "NUGET_NETCORE_PLUGIN_PATHS",
                std::env::join_paths([&named, &dir.join("gone.dll")]).unwrap(),
            )
        };
        assert_eq!(find(), [named], "a path the variable names that is not there was offered, or others were added");
        unsafe { std::env::remove_var("NUGET_NETCORE_PLUGIN_PATHS") };
        let _ = std::fs::remove_dir_all(dir);
    }
}
