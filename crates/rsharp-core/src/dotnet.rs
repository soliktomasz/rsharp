//! The .NET CLI environment shared by builds and the project Host.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

pub fn command(dotnet: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>, folder: &Path) -> Command {
    let mut command = Command::new(dotnet);
    command.args(args).current_dir(folder);
    prepare(&mut command);
    command
}

pub fn prepare(command: &mut Command) {
    let mut salt = command
        .get_envs()
        .find(|(key, _)| *key == OsStr::new("MSBUILDNODEHANDSHAKESALT"))
        .map(|(_, value)| value.map(OsStr::to_os_string))
        .unwrap_or_else(|| std::env::var_os("MSBUILDNODEHANDSHAKESALT"))
        .unwrap_or_default();
    salt.push(format!("rsharp-{}", std::process::id()));
    command
        .env("DOTNET_CLI_UI_LANGUAGE", "en")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_FORCE_UTF8_ENCODING", "true")
        .env("DOTNET_CLI_USE_MSBUILD_SERVER", "0")
        .env("MSBUILDUSESERVER", "0")
        .env("MSBUILDNODEHANDSHAKESALT", salt);
    for key in ["MSBUILDENSURESTDOUTFORTASKPROCESSES", "MSBuildSDKsPath", "MSBuildExtensionsPath", "MSBUILD_EXE_PATH"] {
        command.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn commands_use_literal_arguments_and_isolate_cli_and_worker_nodes() {
        let command = command(Path::new("/sdk/dotnet"), ["build", "a b.csproj"], Path::new("/work"));
        assert_eq!(command.get_program(), "/sdk/dotnet");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["build", "a b.csproj"]);
        assert_eq!(command.get_current_dir(), Some(Path::new("/work")));
        let env: BTreeMap<_, _> = command.get_envs().collect();
        for (key, value) in [
            ("DOTNET_CLI_UI_LANGUAGE", "en"),
            ("DOTNET_NOLOGO", "1"),
            ("DOTNET_CLI_FORCE_UTF8_ENCODING", "true"),
            ("DOTNET_CLI_USE_MSBUILD_SERVER", "0"),
            ("MSBUILDUSESERVER", "0"),
        ] {
            assert_eq!(env.get(OsStr::new(key)), Some(&Some(OsStr::new(value))), "{key}");
        }
        for key in
            ["MSBUILDENSURESTDOUTFORTASKPROCESSES", "MSBuildSDKsPath", "MSBuildExtensionsPath", "MSBUILD_EXE_PATH"]
        {
            assert_eq!(env.get(OsStr::new(key)), Some(&None), "{key}");
        }
        assert!(env[OsStr::new("MSBUILDNODEHANDSHAKESALT")].unwrap().to_string_lossy().contains("rsharp-"));
    }

    #[test]
    fn preparing_a_host_preserves_its_existing_node_isolation() {
        let mut command = Command::new("dotnet");
        command.env("MSBUILDNODEHANDSHAKESALT", "existing-server-");
        prepare(&mut command);
        let salt = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("MSBUILDNODEHANDSHAKESALT"))
            .unwrap()
            .1
            .unwrap()
            .to_string_lossy();
        assert_eq!(salt, format!("existing-server-rsharp-{}", std::process::id()));
    }
}
