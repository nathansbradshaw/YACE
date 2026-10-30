use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub fn source_key(project: &crate::editor::Project) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &project.audio_sha256,
            &project.content.stems
        ))?)
    ))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KitAudition {
    pub source_key: String,
    pub kit: PathBuf,
    pub part: String,
    pub label: String,
    pub audio_sha256: String,
    pub frames: u64,
}
