//! Rescue committed editor state without loading or validating a damaged store.
use crate::{editor::Project, storage};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub revision: u64,
    pub source_error: Option<String>,
    pub missing_checkpoints: Vec<String>,
    pub checkpoint_errors: Vec<String>,
    pub missing_stems: Vec<String>,
    pub explanation: String,
}

/// Stage independent bytes; never repair, replace or acknowledge the live store.
/// A partial copy is still useful when audio or history cannot be recovered.
pub fn prepare(root: &Path, project: &Project, output: &Path) -> Result<Report> {
    anyhow::ensure!(
        !output.starts_with(root) && !root.starts_with(output),
        "Recovery needs an independent staging folder"
    );
    anyhow::ensure!(
        !storage::exists(output),
        "Recovery staging folder already exists"
    );
    let mut files = storage::list(root)?
        .into_iter()
        .map(|(path, bytes)| {
            Ok((
                crate::archive_codec::relative_name(path.strip_prefix(root)?)?,
                bytes,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    // The committed document contains authored edits. An analysis preview must
    // never replace it, and recording context must agree when it is reopened.
    let mut project = project.clone();
    if let Some(basis) = &mut project.recording {
        basis.review = project.review.clone();
        basis.duration = project.duration;
    }
    let bytes = serde_json::to_vec_pretty(&project)?;
    let previous = files.insert("project.json".into(), std::sync::Arc::new(bytes));
    let mut report = Report {
        revision: project.revision,
        source_error: crate::project_portability::validate_sources(&files, &project).err().map(|e| format!("{e:#}")),
        missing_checkpoints: Vec::new(),
        checkpoint_errors: Vec::new(),
        missing_stems: Vec::new(),
        explanation: "Recovered from the open editor, including all available workspace files. The original workspace and browser save were not changed. Open this ZIP as a saved project if its source and history are complete. Missing source audio or checkpoints cannot be recreated from the visible chart; keep this copy and restore those files from an earlier backup before reopening. Analysis previews are excluded. See project.json for your committed notes, lyrics, timing, review decisions and history.".into(),
    };
    let mut contents = vec![project.content.clone()];
    for id in project
        .undo
        .iter()
        .chain(&project.redo)
        .collect::<BTreeSet<_>>()
    {
        let name = format!("checkpoints/{id}.json");
        if let Some(bytes) = files.get(&name) {
            match crate::project_recording::Checkpoint::decode(bytes) {
                Ok(checkpoint) => contents.push(checkpoint.content),
                Err(error) => report.checkpoint_errors.push(format!("{name}: {error:#}")),
            }
        } else {
            report.missing_checkpoints.push(name);
        }
    }
    for asset in contents.iter().flat_map(|content| {
        content
            .stems
            .iter()
            .chain(content.keyboard_stems.iter())
            .flat_map(|m| &m.stems)
    }) {
        let path = if asset.path.is_absolute() {
            asset.path.clone()
        } else {
            root.join(&asset.path)
        };
        if let Ok(bytes) = storage::shared(&path) {
            if !path.starts_with(root) {
                let ext = path.extension().unwrap_or_default().to_string_lossy();
                files.insert(format!("assets/stems/{}.{ext}", asset.sha256), bytes);
            }
        } else {
            report
                .missing_stems
                .push(path.to_string_lossy().into_owned());
        }
    }
    // Reserve a fresh report directory so a previous recovery remains intact.
    let mut directory = "yace-recovery".to_string();
    while files
        .keys()
        .any(|name| name == &directory || name.starts_with(&format!("{directory}/")))
    {
        directory.push('-');
    }
    if let Some(previous) = previous {
        files.insert(format!("{directory}/previous-project.json"), previous);
    }
    files.insert(
        format!("{directory}/report.json"),
        std::sync::Arc::new(serde_json::to_vec_pretty(&report)?),
    );
    for (name, bytes) in files {
        storage::write_shared(output.join(name), bytes)?;
    }
    Ok(report)
}
