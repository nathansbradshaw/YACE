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
pub async fn prepare_project_drum_parts(
    store: &crate::editor::Store,
    project: &crate::editor::Project,
    model_dir: &Path,
    progress: &mut impl FnMut(String),
) -> Result<DrumPartSet> {
    let audio = store.audio_path(project)?;
    ensure!(
        crate::models::hash_file(&audio)? == project.audio_sha256,
        "Project recording changed"
    );
    let manifest = project
        .content
        .stems
        .as_ref()
        .context("Attach aligned drum audio first")?;
    manifest.validate_binding(&project.audio_sha256, crate::models::duration(&audio)?)?;
    let drums = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Drums)
        .context("No drum stem is attached")?;
    let hash = crate::models::hash_file(&drums.path)?;
    ensure!(hash == drums.sha256, "Attached drum audio changed");
    ensure!(
        crate::models::format(&drums.path)? == (44100, 2),
        "Drum audio must be 44.1 kHz stereo"
    );
    let samples = crate::models::decode(&drums.path, 44100, 2)?;
    ensure!(
        samples.len() as u64 == manifest.frames * 2,
        "Drum source and chart clock differ"
    );
    let analysis = store.root.join("analysis");
    crate::storage::create_dir_all(&analysis)?;
    let unique = crate::files::unique_root();
    let stage = analysis.join(format!(
        "kit-{}",
        unique
            .file_name()
            .context("Missing kit cache identity")?
            .to_string_lossy()
    ));
    crate::storage::create_dir(&stage)?;
    let directory = stage.join("parts");
    crate::storage::create_dir(&directory)?;
    let mut receipt =
        crate::larsnet::separate_to_directory(&samples, model_dir, &directory, progress).await?;
    ensure!(
        receipt.frames == manifest.frames,
        "Separated kit changed recording length"
    );
    receipt.source_audio_sha256 = Some(hash.clone());
    let mut parts = Vec::new();
    for output in &receipt.outputs {
        let label = match output.part.as_str() {
            "kick" => "Kick",
            "snare" => "Snare",
            "toms" => "Toms",
            "hihat" => "Hi-hat",
            "cymbals" => "Cymbals",
            _ => anyhow::bail!("Unexpected separated kit output"),
        };
        parts.push(DrumPartFile {
            id: output.part.clone(),
            label: label.into(),
            filename: output.filename.clone(),
            sha256: output
                .wav_sha256
                .clone()
                .context("Separated audio hash missing")?,
            frames: output.frames,
        });
    }
    ensure!(
        crate::models::hash_file(&drums.path)? == hash,
        "Drum source changed during separation"
    );
    crate::storage::write(
        directory.join(crate::larsnet::RECEIPT_FILENAME),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    let set = DrumPartSet {
        portable_audio: std::collections::BTreeMap::new(),
        source_key: kit_source_key(project)?,
        original_audio_sha256: project.audio_sha256.clone(),
        drum_audio_sha256: hash,
        directory,
        parts,
        receipt,
    };
    crate::storage::write(
        stage.join("kit-parts.json"),
        serde_json::to_vec_pretty(&set)?,
    )?;
    // Use the same reader as restored desktop/browser caches before publishing.
    ensure!(
        common::load_project_drum_parts(store, project)?.is_some(),
        "Separated kit failed cache validation"
    );
    Ok(set)
}
#[derive(Clone)]
struct Prepared {
    root: PathBuf,
    selected: KitAudition,
    set: DrumPartSet,
    files: Vec<(PathBuf, Arc<Vec<u8>>)>,
}
thread_local! {static PREPARED:std::cell::RefCell<Option<Prepared>>=std::cell::RefCell::default();}
pub(crate) fn forget_root(root: &Path) {
    PREPARED.with(|cache| {
        let clear = cache
            .borrow()
            .as_ref()
            .is_some_and(|prepared| prepared.root.starts_with(root));
        if clear {
            *cache.borrow_mut() = None;
        }
    });
}
pub(crate) fn remember_selection(
    store: &crate::editor::Store,
    project: &crate::editor::Project,
    selected: KitAudition,
) -> Result<()> {
    ensure!(
        selected.source_key == kit_source_key(project)?,
        "Kit source changed during preparation"
    );
    let set: DrumPartSet = serde_json::from_slice(&crate::storage::read(&selected.kit)?)?;
    ensure!(
        kit_descriptor(&set, &selected.part)? == selected,
        "Prepared kit descriptor differs from its validated set"
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
            set,
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
            prepared.root == store.root && prepared.selected.source_key == kit_source_key(project)?,
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
        kit_descriptor(&prepared.set, part)
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
