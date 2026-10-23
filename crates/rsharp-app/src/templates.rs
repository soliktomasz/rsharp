//! The UI's shared template service. Discovery and preload never run on a window's executor.

use gpui::{App, Global};
use rsharp_core::templates::{TemplateEnvironment, cache::Catalogs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type SharedCatalogs = Arc<Mutex<Option<Catalogs>>>;

pub struct Templates {
    pub catalogs: SharedCatalogs,
    pub enabled: bool,
    preloaded: bool,
}
impl Global for Templates {}

pub fn init(cx: &mut App) {
    cx.set_global(Templates { catalogs: Arc::new(Mutex::new(None)), enabled: true, preloaded: false });
}

/// Worker-only: a failed discovery is not retained, so installing dotnet then Retry can recover.
pub fn service(shared: &SharedCatalogs) -> Result<Catalogs, String> {
    let mut slot = shared.lock().unwrap();
    if let Some(service) = slot.as_ref() {
        return Ok(service.clone());
    }
    let service = Catalogs::new(TemplateEnvironment::discover()?);
    *slot = Some(service.clone());
    Ok(service)
}

pub fn shared(cx: &App) -> SharedCatalogs {
    cx.global::<Templates>().catalogs.clone()
}

pub fn shutdown(cx: &App) -> Option<Catalogs> {
    cx.global::<Templates>().catalogs.try_lock().ok().and_then(|slot| slot.clone())
}

/// Worker-only: beside the last solution, as Passenger's BesideLast, otherwise the person's home.
pub fn beside_last(last: Option<&Path>) -> PathBuf {
    let parent =
        last.and_then(|last| if last.is_file() { last.parent().and_then(Path::parent) } else { last.parent() });
    parent.filter(|p| p.is_dir()).map(Path::to_path_buf).unwrap_or_else(rsharp_core::programs::home)
}

pub fn preload(last: Option<PathBuf>, cx: &mut App) {
    let templates = cx.global_mut::<Templates>();
    if !templates.enabled || templates.preloaded {
        return;
    }
    templates.preloaded = true;
    let shared = templates.catalogs.clone();
    cx.background_executor()
        .spawn(async move {
            let read = service(&shared).and_then(|s| s.load(&beside_last(last.as_deref())));
            if let Err(why) = read {
                log::warn!("New Solution's templates could not be read ahead: {why}");
            }
        })
        .detach();
}
