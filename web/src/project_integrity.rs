//! Refuse incomplete browser snapshots before replacing a durable manifest.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn validate(
    root: &Path,
    revision: u64,
    files: &[(PathBuf, Arc<Vec<u8>>)],
    cached: Option<Arc<crate::editor::Project>>,
) -> Result<()> {
    let files: BTreeMap<_, _> = files
        .iter()
        .map(|(path, bytes)| (path.as_path(), bytes))
        .collect();
    let required = |path: &Path| -> Result<()> {
        ensure!(
            files.get(path).is_some_and(|bytes| !bytes.is_empty()),
            "Project snapshot is incomplete: {}. Keep this tab open and download a recovery copy; the previous browser save was preserved.",
            path.display()
        );
        Ok(())
    };
    let manifest = root.join("project.json");
    required(&manifest)?;
    let doc = match cached {
        Some(doc) => doc,
        None => Arc::new(
            serde_json::from_slice(files[manifest.as_path()])
                .context("Saved project document is invalid")?,
        ),
    };
    ensure!(
        doc.revision == revision,
        "Project changed while saving; retry with the current revision"
    );
    required(&root.join("source/notes.chart"))?;
    if doc.has_recording() {
        required(&root.join("source").join(&doc.audio_filename))?;
    }
    for hash in doc.undo.iter().chain(&doc.redo) {
        required(&root.join("checkpoints").join(format!("{hash}.json")))?;
    }
    Ok(())
}
