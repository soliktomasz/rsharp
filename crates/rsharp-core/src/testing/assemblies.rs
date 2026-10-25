use super::*;
use crate::project::EvaluatedProject;
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST: AtomicU64 = AtomicU64::new(0);

/// Asks MSBuild's project-reference targets for each framework's actual TargetPath, including imported output settings.
pub fn resolve_assemblies(
    dotnet: &Path,
    folder: &Path,
    projects: &[EvaluatedProject],
    cancel: &AtomicBool,
) -> Result<Vec<TestAssembly>, String> {
    if canceled(cancel) {
        return Err("Test assembly resolution stopped".into());
    }
    let projects: Vec<_> = projects.iter().collect();
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let request = std::env::temp_dir().join(format!(
        "rsharp-tests-{}-{}.proj",
        std::process::id(),
        REQUEST.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&request).map_err(|e| e.to_string())?;
    struct Remove(PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _remove = Remove(request.clone());
    std::io::Write::write_all(&mut file, request_xml(&projects).as_bytes()).map_err(|e| e.to_string())?;
    drop(file);
    let arguments = [
        "msbuild",
        request.to_str().ok_or("The test assembly request path is not UTF-8")?,
        "-t:RSharpTestAssemblies",
        "-nologo",
        "-tl:off",
        "-v:m",
        "-nodeReuse:false",
        "-clp:NoSummary;ForceNoAlign",
    ];
    let command = crate::dotnet::command(dotnet, arguments, folder);
    let mut output = String::new();
    let result = crate::build::run(command, cancel, |text, _| {
        if output.len() < 4 * 1024 * 1024 {
            output.push_str(text);
        }
    })
    .map_err(|e| format!("MSBuild could not resolve test assemblies: {e}"))?;
    if result.is_none() || canceled(cancel) {
        return Err("Test assembly resolution stopped".into());
    }
    let mut found = Vec::new();
    for line in output.lines() {
        let fields: Vec<_> = line.trim().split('\t').collect();
        if fields.len() != 5 || fields[0] != "rsharp-test-assembly" {
            continue;
        }
        let Some(project) = fields[1].parse::<usize>().ok().and_then(|index| projects.get(index)) else {
            continue;
        };
        if fields[2].is_empty() || fields[4].is_empty() {
            continue;
        }
        let index = fields[1].parse::<usize>().unwrap();
        let framework_index = fields[3].split(';').position(|framework| framework.trim() == fields[2]).unwrap_or(0);
        found.push((
            index,
            framework_index,
            TestAssembly {
                project: project.path.clone(),
                framework: fields[2].to_string(),
                path: source_path(folder, fields[4]),
            },
        ));
    }
    if found.is_empty() && result != Some(0) {
        return Err(format!("MSBuild could not say where the test projects build:\n{}", output.trim()));
    }
    found.sort_by_key(|(project, framework, _)| (*project, *framework));
    Ok(found.into_iter().map(|(_, _, assembly)| assembly).collect())
}

fn request_xml(projects: &[&EvaluatedProject]) -> String {
    let mut xml = String::from("<Project><ItemGroup>");
    for (index, project) in projects.iter().enumerate() {
        let escaped: String = project
            .path
            .to_string_lossy()
            .chars()
            .map(|c| {
                if matches!(c, '%' | ';' | '$' | '@' | '\'' | '*' | '?' | '(' | ')') {
                    format!("%{:02X}", c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect();
        xml.push_str(&format!(
            "<RSharpTestProject Include=\"{}\" RSharpIndex=\"{index}\" />",
            quick_xml::escape::escape(&escaped)
        ));
    }
    xml.push_str(r#"</ItemGroup><Target Name="RSharpTestAssemblies">
<MSBuild Projects="@(RSharpTestProject)" Targets="GetTargetFrameworks" Condition="'%(RSharpTestProject.RSharpIndex)' != ''" SkipNonexistentProjects="true" ContinueOnError="true">
<Output TaskParameter="TargetOutputs" ItemName="_RSharpProject" /></MSBuild>
<ItemGroup>
<_RSharpFramework Include="%(_RSharpProject.TargetFrameworks)" RSharpProject="%(_RSharpProject.Identity)" RSharpIndex="%(_RSharpProject.RSharpIndex)" RSharpSingle="%(_RSharpProject.HasSingleTargetFramework)" RSharpFrameworks="%(_RSharpProject.TargetFrameworks)" />
<_RSharpBuild Include="@(_RSharpFramework->'%(RSharpProject)')" RSharpFramework="%(Identity)" AdditionalProperties="TargetFramework=%(Identity)" Condition="'%(RSharpSingle)' != 'true'" />
<_RSharpBuild Include="@(_RSharpFramework->'%(RSharpProject)')" RSharpFramework="%(Identity)" Condition="'%(RSharpSingle)' == 'true'" />
</ItemGroup>
<MSBuild Projects="@(_RSharpBuild)" Targets="GetTargetPath" Condition="'%(_RSharpBuild.RSharpIndex)/%(_RSharpBuild.RSharpFramework)' != ''" ContinueOnError="true">
<Output TaskParameter="TargetOutputs" ItemName="_RSharpAssembly" /></MSBuild>
<Message Importance="high" Text="rsharp-test-assembly%09%(_RSharpAssembly.RSharpIndex)%09%(_RSharpAssembly.RSharpFramework)%09%(_RSharpAssembly.RSharpFrameworks)%09%(_RSharpAssembly.FullPath)" />
</Target></Project>"#);
    xml
}
