use super::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST: AtomicU64 = AtomicU64::new(1);
pub(super) fn next_request() -> u64 {
    REQUEST.fetch_add(1, Ordering::Relaxed)
}

struct Found {
    assembly: TestAssembly,
    test: TestItem,
    node: Value,
    platform: bool,
}

/// A folder's two runners, with opaque discovery identities kept for selected runs.
pub struct TestRunners {
    dotnet: PathBuf,
    folder: PathBuf,
    found: HashMap<String, Found>,
}

impl TestRunners {
    pub fn new(dotnet: PathBuf, folder: PathBuf) -> Self {
        Self { dotnet, folder, found: HashMap::new() }
    }

    pub fn discover(
        &mut self,
        assemblies: &[TestAssembly],
        cancel: &AtomicBool,
        event: Events<'_>,
    ) -> Result<Vec<TestItem>, String> {
        let mut all = Vec::new();
        let mut errors = Vec::new();
        for assembly in assemblies {
            if canceled(cancel) {
                return Err("Test discovery stopped".into());
            }
            self.found.retain(|_, found| {
                found.assembly.path != assembly.path || found.assembly.framework != assembly.framework
            });
            let platform = super::platform::is_application(&assembly.path);
            let result = if platform {
                super::platform::discover(&self.dotnet, assembly, cancel, event)
            } else {
                super::vstest::discover(&self.dotnet, &self.folder, assembly, cancel, event)
            };
            let nodes = match result {
                Ok(nodes) => nodes,
                Err(error) => {
                    if canceled(cancel) {
                        return Err(error);
                    }
                    event(TestEvent::Log(error.clone()));
                    errors.push(error);
                    continue;
                }
            };
            let mut seen = HashSet::new();
            let mut batch = Vec::new();
            for (mut test, node) in nodes {
                let uid = if platform {
                    node["uid"].as_str().unwrap_or_default().to_string()
                } else {
                    super::vstest::uid(&node)
                };
                if !seen.insert(uid) {
                    continue;
                }
                let row = test.row.as_deref().unwrap_or(&test.display_name).to_string();
                let mut count = 2;
                while self.found.contains_key(&test.key()) {
                    test.row = Some(format!("{row} ({count})"));
                    count += 1;
                }
                self.found.insert(test.key(), Found { assembly: assembly.clone(), test: test.clone(), node, platform });
                batch.push(test);
            }
            if !batch.is_empty() {
                event(TestEvent::Discovered(batch.clone()));
                all.extend(batch);
            }
        }
        if all.is_empty() && !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        Ok(all)
    }

    pub fn run(&mut self, tests: &[TestItem], cancel: &AtomicBool, event: Events<'_>) -> Result<RunSummary, String> {
        let mut groups: Vec<(TestAssembly, bool, Vec<(TestItem, Value)>)> = Vec::new();
        let mut seen = HashSet::new();
        for test in tests {
            let Some(found) = self.found.get(&test.key()) else {
                return Err(format!("{} must be discovered in its built assembly before running", test.display_name));
            };
            if !seen.insert(test.key()) {
                continue;
            }
            let index = groups.iter().position(|(assembly, _, _)| assembly == &found.assembly).unwrap_or_else(|| {
                groups.push((found.assembly.clone(), found.platform, Vec::new()));
                groups.len() - 1
            });
            groups[index].2.push((found.test.clone(), found.node.clone()));
        }
        let mut summary = RunSummary::default();
        for (assembly, platform, tests) in groups {
            if canceled(cancel) {
                summary.canceled = true;
                break;
            }
            let part = if platform {
                super::platform::run(&self.dotnet, &assembly, &tests, cancel, event)
            } else {
                super::vstest::run(&self.dotnet, &self.folder, &tests, cancel, event)
            };
            summary.elapsed += part.elapsed;
            summary.canceled |= part.canceled;
            summary.aborted |= part.aborted;
            summary.error = summary.error.or(part.error);
            summary.outcomes.extend(part.outcomes);
            if summary.canceled {
                break;
            }
        }
        Ok(summary)
    }
}
