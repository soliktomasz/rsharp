//! The folder's package sources as restore reads them, from nuget.config files a test writes; the user's and the
//! machine's folders are the test's own, never the person's.

use rsharp_core::nuget::sources::{NUGET_ORG, Place, Places, SourceSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    root: PathBuf,
    places: Places,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rsharp-nuget-sources-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("repo/solution")).unwrap();
        let places = Places { user: root.join("user"), machine: root.join("machine") };
        Self { root: rsharp_core::paths::canonical(&root), places }
    }
    fn solution(&self) -> PathBuf {
        self.root.join("repo/solution")
    }
    fn write(&self, path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn load(&self) -> SourceSet {
        SourceSet::load(&self.solution(), &self.places)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn names(set: &SourceSet) -> Vec<(&str, &str, bool)> {
    set.feeds.iter().map(|f| (f.name.as_str(), f.source.as_str(), f.enabled)).collect()
}

#[test]
fn closer_files_win_and_a_missing_user_file_means_nuget_org() {
    let f = Fixture::new();
    f.write(
        &f.solution().join("nuget.config"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="local" value="packages" />
    <add key="Shared" value="https://solution.example/v3/index.json" protocolVersion="3" />
  </packageSources>
</configuration>"#,
    );
    f.write(
        &f.root.join("repo/NuGet.Config"),
        r#"<configuration><packageSources><add key="shared" value="https://repo.example/index.json" /><add key="repo" value="./feed/../repo-feed" /></packageSources>
<disabledPackageSources><add key="repo" value="true" /></disabledPackageSources></configuration>"#,
    );
    let set = f.load();
    let local = f.solution().join("packages");
    let repo = f.root.join("repo/repo-feed");
    assert_eq!(
        names(&set),
        [
            ("local", local.to_str().unwrap(), true),
            ("Shared", "https://solution.example/v3/index.json", true),
            ("repo", repo.to_str().unwrap(), false),
            ("nuget.org", NUGET_ORG, true),
        ],
        "a closer file's key did not win, a relative folder was not made absolute, or nuget.org was not assumed"
    );
    assert_eq!(set.feeds[1].protocol_version, Some(3));
    assert_eq!(set.files.iter().map(|f| f.place).collect::<Vec<_>>(), [Place::Solution, Place::Folder]);
    assert!(set.files[1].feeds[1].disabled_here);
    assert!(!f.places.user.exists(), "loading the sources wrote the user's NuGet folder");
}

#[test]
fn a_clear_ends_the_files_after_it_and_the_user_file_replaces_nuget_org() {
    let f = Fixture::new();
    f.write(&f.solution().join("nuget.config"), r#"<configuration><packageSources><clear /><add key="only" value="https://only.example/index.json" /></packageSources></configuration>"#);
    assert_eq!(
        names(&f.load()).into_iter().map(|n| n.0).collect::<Vec<_>>(),
        ["only"],
        "nuget.org was assumed although the solution's file clears every feed after it"
    );
    std::fs::remove_file(f.solution().join("nuget.config")).unwrap();
    f.write(&f.places.user.join("NuGet.Config"), r#"<configuration><packageSources><add key="mine" value="https://mine.example/index.json" /></packageSources></configuration>"#);
    f.write(&f.places.machine.join("Corp.config"), r#"<configuration><packageSources><add key="corp" value="https://corp.example/index.json" /></packageSources></configuration>"#);
    assert_eq!(
        names(&f.load()).into_iter().map(|n| n.0).collect::<Vec<_>>(),
        ["mine", "corp"],
        "the user's and the machine's feeds were not read in NuGet's order"
    );
    f.write(
        &f.solution().join("nuget.config"),
        r#"<configuration><packageSources><clear /><add key="only" value="https://only.example/index.json" /></packageSources></configuration>"#,
    );
    let set = f.load();
    assert_eq!(
        names(&set).into_iter().map(|n| n.0).collect::<Vec<_>>(),
        ["only"],
        "<clear /> left the other files' feeds"
    );
    assert!(set.files[0].clears);
    assert_eq!(set.files.len(), 3, "a cleared file is still one in effect for the Sources tab");
}

#[test]
fn environment_variables_expand_and_an_unreadable_file_is_its_own_problem() {
    let f = Fixture::new();
    // SAFETY: no other test reads this variable.
    unsafe { std::env::set_var("RSHARP_TEST_FEED_HOST", "env.example") };
    f.write(&f.solution().join("nuget.config"), r#"<configuration><packageSources><add key="env" value="https://%RSHARP_TEST_FEED_HOST%/index.json" /></packageSources></configuration>"#);
    f.write(&f.root.join("repo/nuget.config"), "<configuration><packageSources>");
    let set = f.load();
    assert_eq!(set.feeds[0].source, "https://env.example/index.json");
    assert!(set.files[1].problem.is_some(), "a broken file did not say why");
    assert!(set.feeds.iter().any(|f| f.name == "nuget.org"), "one broken file took the other feeds with it");
}

#[test]
fn the_mapping_keeps_each_package_to_its_longest_pattern() {
    let f = Fixture::new();
    f.write(
        &f.solution().join("nuget.config"),
        r#"<configuration>
  <packageSources><add key="nuget.org" value="https://api.nuget.org/v3/index.json" /><add key="corp" value="https://corp.example/index.json" /></packageSources>
  <packageSourceMapping>
    <packageSource key="nuget.org"><package pattern="*" /></packageSource>
    <packageSource key="corp"><package pattern="Corp.*" /><package pattern="Newtonsoft.Json" /></packageSource>
  </packageSourceMapping>
</configuration>"#,
    );
    let set = f.load();
    assert!(set.mapped());
    assert!(set.allows("nuget.org", "Serilog") && !set.allows("corp", "Serilog"));
    assert!(set.allows("corp", "corp.logging") && !set.allows("nuget.org", "Corp.Logging"));
    assert!(set.allows("corp", "Newtonsoft.Json") && !set.allows("nuget.org", "newtonsoft.json"));
    let unmapped = Fixture::new();
    assert!(unmapped.load().allows("anything", "Any.Package"), "without a mapping every feed may serve a package");
}
