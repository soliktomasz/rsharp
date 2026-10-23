//! One catalog read per SDK/home/hive, with the last successful data kept while sources refresh.

use super::{TemplateCatalog, TemplateEnvironment, TemplatePlatform, catalog, command};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Condvar, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Clone)]
pub struct Catalogs {
    inner: Arc<Inner>,
}
struct Inner {
    environment: TemplateEnvironment,
    state: Mutex<State>,
    changed: Condvar,
    watchers: Mutex<Option<Vec<notify::RecommendedWatcher>>>,
    callbacks: Mutex<HashMap<u64, Arc<dyn Fn() + Send + Sync>>>,
    next_callback: AtomicU64,
    cancel: AtomicBool,
}
#[derive(Default)]
struct State {
    generation: u64,
    catalogs: HashMap<String, (u64, Arc<TemplateCatalog>)>,
    loading: HashSet<(String, u64)>,
    sdks: HashMap<PathBuf, String>,
    recent: VecDeque<PathBuf>,
    last: Option<String>,
}
pub struct CatalogSubscription {
    inner: Weak<Inner>,
    id: u64,
}
impl Drop for CatalogSubscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.callbacks.lock().unwrap().remove(&self.id);
        }
    }
}

impl Catalogs {
    pub fn new(environment: TemplateEnvironment) -> Self {
        Self {
            inner: Arc::new(Inner {
                environment,
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
                watchers: Mutex::new(None),
                callbacks: Mutex::new(HashMap::new()),
                next_callback: AtomicU64::new(0),
                cancel: AtomicBool::new(false),
            }),
        }
    }
    pub fn environment(&self) -> &TemplateEnvironment {
        &self.inner.environment
    }
    pub fn subscribe(&self, changed: impl Fn() + Send + Sync + 'static) -> CatalogSubscription {
        let id = self.inner.next_callback.fetch_add(1, Ordering::Relaxed);
        self.inner.callbacks.lock().unwrap().insert(id, Arc::new(changed));
        CatalogSubscription { inner: Arc::downgrade(&self.inner), id }
    }
    pub fn ready(&self, folder: &Path) -> Option<Arc<TemplateCatalog>> {
        let state = self.inner.state.lock().unwrap();
        let sdk = state.sdks.get(folder).or(state.last.as_ref())?;
        state.catalogs.get(sdk).map(|(_, c)| c.clone())
    }
    pub fn forget(&self) {
        let mut state = self.inner.state.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.sdks.clear();
        self.inner.changed.notify_all();
    }
    pub fn shutdown(&self) {
        self.inner.cancel.store(true, Ordering::Release);
        self.inner.watchers.lock().unwrap().take();
        self.inner.changed.notify_all();
    }
    fn sdk(&self, folder: &Path) -> Result<String, String> {
        let read = command::run(
            self.inner.environment.command(&["--version".into()], folder),
            Duration::from_secs(60),
            &self.inner.cancel,
            &mut |_| {},
        )?;
        if !read.status.success() || read.stdout.trim().is_empty() {
            return Err(format!("dotnet --version failed: {}", read.said()));
        }
        Ok(read.stdout.trim().into())
    }
    pub fn last_listed(&self, folder: &Path) -> Result<Option<Arc<TemplateCatalog>>, String> {
        let sdk = self.sdk(folder)?;
        if let Some((_, catalog)) = self.inner.state.lock().unwrap().catalogs.get(&sdk) {
            return Ok(Some(catalog.clone()));
        }
        if self.cache_file(&sdk).is_none() {
            return Ok(None);
        }
        self.read_cache(&sdk).map(|c| Some(Arc::new(c)))
    }
    pub fn load(&self, folder: &Path) -> Result<Arc<TemplateCatalog>, String> {
        let sdk = self.sdk(folder)?;
        let read = self.load_sdk(folder, &sdk, &mut || {
            let listed = command::run(
                self.inner.environment.new_command(&["list".into(), "--type".into(), "project".into()], folder),
                Duration::from_secs(180),
                &self.inner.cancel,
                &mut |_| {},
            )?;
            if !listed.status.success() {
                return Err(format!("dotnet new list failed: {}", listed.said()));
            }
            self.read_cache(&sdk)
        })?;
        self.watch();
        Ok(read)
    }
    fn load_sdk(
        &self,
        folder: &Path,
        sdk: &str,
        read: &mut impl FnMut() -> Result<TemplateCatalog, String>,
    ) -> Result<Arc<TemplateCatalog>, String> {
        loop {
            let mut state = self.inner.state.lock().unwrap();
            if self.inner.cancel.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            state.recent.retain(|p| p != folder);
            state.recent.push_front(folder.to_path_buf());
            state.recent.truncate(4);
            state.sdks.insert(folder.to_path_buf(), sdk.to_string());
            state.last = Some(sdk.into());
            let generation = state.generation;
            if let Some((seen, catalog)) = state.catalogs.get(sdk)
                && *seen == generation
            {
                return Ok(catalog.clone());
            }
            let key = (sdk.to_string(), generation);
            if state.loading.contains(&key) {
                drop(self.inner.changed.wait(state).unwrap());
                continue;
            }
            state.loading.insert(key.clone());
            drop(state);
            let result = read().map(Arc::new);
            let mut state = self.inner.state.lock().unwrap();
            state.loading.remove(&key);
            if let Ok(catalog) = &result
                && state.generation == generation
            {
                state.catalogs.insert(sdk.into(), (generation, catalog.clone()));
            }
            let stale = state.generation != generation;
            drop(state);
            self.inner.changed.notify_all();
            if stale {
                continue;
            }
            return result;
        }
    }
    fn cache_file(&self, sdk: &str) -> Option<PathBuf> {
        let env = &self.inner.environment;
        let root = env.hive.clone().unwrap_or_else(|| env.cli_home.join(".templateengine"));
        [
            root.join("dotnetcli").join(sdk).join("templatecache.json"),
            root.join("dotnetcli").join(format!("v{sdk}")).join("templatecache.json"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    }
    fn read_cache(&self, sdk: &str) -> Result<TemplateCatalog, String> {
        let mut result = Err(format!("dotnet new keeps no templates for SDK {sdk}"));
        for attempt in 0..4 {
            if self.inner.cancel.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(200));
            }
            result = self
                .cache_file(sdk)
                .ok_or_else(|| format!("dotnet new keeps no templatecache.json for SDK {sdk}"))
                .and_then(|path| catalog::parse_cache(&path, sdk, TemplatePlatform::current()));
            if result.is_ok() {
                break;
            }
        }
        result
    }
    pub fn install(
        &self,
        package: &str,
        folder: &Path,
        cancel: &AtomicBool,
        output: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        let installed = command::run(
            self.inner.environment.new_command(&["install".into(), package.into()], folder),
            Duration::from_secs(600),
            cancel,
            output,
        )?;
        if !installed.status.success() {
            return Err(format!("Installation of {package} has failed: {}", installed.said()));
        }
        self.forget();
        Ok(())
    }
    fn watch(&self) {
        use notify::Watcher as _;
        let mut watching = self.inner.watchers.lock().unwrap();
        if watching.is_some() {
            return;
        }
        let env = &self.inner.environment;
        let home = env.hive.clone().unwrap_or_else(|| env.cli_home.join(".templateengine"));
        let dotnet = crate::paths::canonical(&env.dotnet);
        let mut sources = vec![home.clone()];
        if let Some(root) = dotnet.parent() {
            sources.extend(["sdk", "templates", "template-packs"].iter().map(|p| root.join(p)));
        }
        let (sender, events) = std::sync::mpsc::sync_channel(1);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event
                && !matches!(event.kind, notify::EventKind::Access(_))
                && event.paths.iter().any(|p| !p.starts_with(&home) || *p == home.join("packages.json"))
            {
                let _ = sender.try_send(());
            }
        });
        let mut watchers = Vec::new();
        if let Ok(mut watcher) = watcher {
            for source in sources.into_iter().filter(|p| p.is_dir()) {
                if let Err(why) = watcher.watch(&source, notify::RecursiveMode::NonRecursive) {
                    log::warn!("Template sources {} are not watched: {why}", source.display());
                }
            }
            watchers.push(watcher);
        }
        *watching = Some(watchers);
        drop(watching);
        let weak = Arc::downgrade(&self.inner);
        let _ = std::thread::Builder::new().name("template sources".into()).spawn(move || {
            while events.recv().is_ok() {
                loop {
                    match events.recv_timeout(Duration::from_millis(500)) {
                        Ok(()) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        Err(_) => return,
                    }
                }
                let Some(inner) = weak.upgrade() else { break };
                let store = Catalogs { inner };
                if store.inner.cancel.load(Ordering::Acquire) {
                    break;
                }
                let folders = store.inner.state.lock().unwrap().recent.clone();
                store.forget();
                for folder in folders.iter().rev().filter(|p| p.is_dir()) {
                    if let Err(why) = store.load(folder) {
                        log::warn!("Templates could not be refreshed: {why}");
                    }
                }
                let callbacks: Vec<_> = store.inner.callbacks.lock().unwrap().values().cloned().collect();
                for changed in callbacks {
                    changed();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn store(name: &str) -> Catalogs {
        Catalogs::new(TemplateEnvironment {
            dotnet: "dotnet".into(),
            cli_home: std::env::temp_dir().join(format!("rsharp-catalog-{name}-{}", std::process::id())),
            hive: None,
            nuget_packages: None,
        })
    }
    fn catalog(sdk: &str) -> TemplateCatalog {
        TemplateCatalog {
            sdk: sdk.into(),
            families: vec![],
            solution_formats: vec!["sln".into()],
            solution_format: "sln".into(),
        }
    }

    #[test]
    fn one_read_is_shared_per_sdk_home_and_hive() {
        let store = store("shared");
        let reads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        store
                            .load_sdk(Path::new("/folder"), "10", &mut || {
                                reads.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(30));
                                Ok(catalog("10"))
                            })
                            .unwrap()
                    })
                })
                .collect();
            let catalogs: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
            assert!(catalogs.iter().all(|c| Arc::ptr_eq(c, &catalogs[0])));
        });
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        let other = store.load_sdk(Path::new("/other"), "11", &mut || Ok(catalog("11"))).unwrap();
        assert_eq!(other.sdk, "11");
        assert_eq!(store.ready(Path::new("/folder")).unwrap().sdk, "10");
        let separate = super::tests::store("other-home");
        let own = separate.load_sdk(Path::new("/folder"), "10", &mut || Ok(catalog("10"))).unwrap();
        assert!(!Arc::ptr_eq(&own, &store.ready(Path::new("/folder")).unwrap()));
    }

    #[test]
    fn failed_reads_can_retry_and_forget_keeps_only_preliminary_data() {
        let store = store("retry");
        assert!(store.load_sdk(Path::new("/folder"), "10", &mut || Err("broken cache".into())).is_err());
        let first = store.load_sdk(Path::new("/folder"), "10", &mut || Ok(catalog("10"))).unwrap();
        store.forget();
        assert!(Arc::ptr_eq(&first, &store.ready(Path::new("/folder")).unwrap()));
        let after = store.load_sdk(Path::new("/folder"), "10", &mut || Ok(catalog("10"))).unwrap();
        assert!(!Arc::ptr_eq(&first, &after));
    }

    #[test]
    fn source_refresh_remembers_only_four_recent_contexts() {
        let store = store("recent");
        for n in 0..6 {
            store.load_sdk(&PathBuf::from(format!("/folder{n}")), "10", &mut || Ok(catalog("10"))).unwrap();
        }
        let state = store.inner.state.lock().unwrap();
        assert_eq!(
            state.recent.iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>(),
            ["/folder5", "/folder4", "/folder3", "/folder2"]
        );
    }

    #[test]
    fn a_cache_rewritten_while_read_gets_a_bounded_retry_and_v_prefix_is_found() {
        let store = store("rewritten");
        let path = store.inner.environment.cli_home.join(".templateengine/dotnetcli/v10/templatecache.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{").unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(40));
                std::fs::write(&path, r#"{"TemplateInfo":[]}"#).unwrap();
            });
            assert_eq!(store.read_cache("10").unwrap().sdk, "10");
        });
        let _ = std::fs::remove_dir_all(&store.inner.environment.cli_home);
    }

    #[test]
    fn source_bursts_notify_once_and_unsubscribed_dialogs_hear_nothing() {
        let store = store("watch");
        let source = store.inner.environment.cli_home.join(".templateengine");
        std::fs::create_dir_all(&source).unwrap();
        let heard = Arc::new(AtomicUsize::new(0));
        let count = heard.clone();
        let subscription = store.subscribe(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        store.watch();
        for _ in 0..6 {
            std::fs::write(source.join("packages.json"), "{}").unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        let end = std::time::Instant::now() + Duration::from_secs(4);
        while heard.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(heard.load(Ordering::SeqCst), 1);
        drop(subscription);
        std::fs::write(source.join("packages.json"), "[]").unwrap();
        std::thread::sleep(Duration::from_millis(650));
        assert_eq!(heard.load(Ordering::SeqCst), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(source.parent().unwrap());
    }
}
