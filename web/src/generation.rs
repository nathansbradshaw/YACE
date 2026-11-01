//! Browser working-set operations for the shared generation job protocol.
pub use crate::generation_protocol::{Request, Source, State, Status};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::storage::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
pub fn status(root: &Path) -> Result<Status> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("job.json"),
    )?)?)
}
pub fn read_request(root: &Path) -> Result<Request> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("request.json"),
    )?)?)
}
pub fn project_path(root: &Path) -> PathBuf {
    crate::song_layout::project_for_job(root)
}
pub fn create_job(mut request: Request, root: &Path) -> Result<PathBuf> {
    request.validate()?;
    ensure!(
        !crate::storage::exists(root),
        "Choose a new generation job; saved results are preserved"
    );
    let Source::Audio { path } = &request.source;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Missing recording filename"))?;
    let target = root.join("input").join(filename);
    crate::storage::copy(path, &target)?;
    request.source = Source::Audio { path: target };
    write_json(&root.join("request.json"), &request)?;
    write_json(
        &root.join("job.json"),
        &Status {
            version: 1,
            state: State::Queued,
            stage: "queued".into(),
            detail: "Local browser job created".into(),
            request_sha256: crate::models::hash_file(&root.join("request.json"))?,
            error: None,
        },
    )?;
    Ok(root.to_owned())
}
