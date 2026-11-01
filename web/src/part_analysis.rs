//! Reuse read-only analysis and saved kit validation without inference runtimes.
#[path = "../../src/part_analysis.rs"]
mod common;
pub use crate::kit_audition::KitAudition;
pub use crate::kit_audition::source_key as kit_source_key;
use anyhow::{Context, Result, ensure};
pub use common::select_kit_audition as select_kit_on_worker;
pub use common::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone)]
struct Prepared {
    root: PathBuf,
    selected: KitAudition,
    files: Vec<(PathBuf, Arc<Vec<u8>>)>,
}
thread_local! {static PREPARED:std::cell::RefCell<Option<Prepared>>=std::cell::RefCell::default();}
pub(crate) fn remember_selection(
    store: &crate::editor::Store,
    project: &crate::editor::Project,
    selected: KitAudition,
) -> Result<()> {
    ensure!(
        selected.source_key == kit_source_key(project)?,
        "Kit source changed during preparation"
    );
    let mut files = crate::storage::list(&store.root.join("analysis"))?;
    let audio = store.audio_path(project)?;
    files.push((audio.clone(), crate::storage::shared(audio)?));
    for asset in project.content.stems.iter().flat_map(|m| &m.stems) {
        files.push((asset.path.clone(), crate::storage::shared(&asset.path)?));
    }
    PREPARED.with(|cache| {
        *cache.borrow_mut() = Some(Prepared {
            root: store.root.clone(),
            selected,
            files,
        })
    });
    Ok(())
}
pub fn select_kit_audition(
    store: &crate::editor::Store,
    project: &crate::editor::Project,
    part: &str,
) -> Result<KitAudition> {
    PREPARED.with(|cache| {
        let cache = cache.borrow();
        let prepared = cache
            .as_ref()
            .context("Prepare the saved kit on a local worker first")?;
        ensure!(
            prepared.root == store.root
                && prepared.selected.part == part
                && prepared.selected.source_key == kit_source_key(project)?,
            "Prepared kit belongs to another source or component"
        );
        ensure!(
            prepared
                .files
                .iter()
                .all(|(path, bytes)| crate::storage::shared(path)
                    .is_ok_and(|current| Arc::ptr_eq(&current, bytes))),
            "Kit source files changed after validation"
        );
        Ok(prepared.selected.clone())
    })
}
pub fn prepared_kit_path(
    store: &crate::editor::Store,
    project: &crate::editor::Project,
    selected: &KitAudition,
) -> Result<PathBuf> {
    ensure!(
        select_kit_audition(store, project, &selected.part)? == *selected,
        "Prepared kit descriptor changed"
    );
    let cache = selected.kit.parent().context("Missing kit folder")?;
    ensure!(
        cache.parent() == Some(store.root.join("analysis").as_path())
            && selected.kit.file_name() == Some(std::ffi::OsStr::new("kit-parts.json")),
        "Kit cache must belong to this project"
    );
    let path = cache.join("parts").join(format!("{}.wav", selected.part));
    ensure!(
        Path::new(&selected.part).components().count() == 1,
        "Invalid kit component"
    );
    Ok(path)
}
