//! Dance downloads use the same bounded ZIP worker as band project backups.
use anyhow::Result;
use std::path::Path;

struct Staging(std::path::PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        crate::storage::clear(&self.0);
    }
}

pub async fn prepare(
    files: crate::stepmania::bundle::Files,
    parent: Option<&Path>,
    output: &Path,
) -> Result<()> {
    let staging = Staging(output.with_extension("input"));
    if let Some(parent) = parent {
        for (path, bytes) in crate::storage::list(parent)? {
            crate::storage::write_shared(staging.0.join(path.strip_prefix(parent)?), bytes)?;
        }
    }
    let root = if parent.is_some() {
        staging.0.join("dance")
    } else {
        staging.0.clone()
    };
    for (name, bytes) in files {
        crate::storage::write_owned(root.join(name), bytes)?;
    }
    crate::worker::run::<serde_json::Value>(
        crate::worker::Task::Archive {
            root: staging.0.clone(),
            output: output.to_owned(),
        },
        std::slice::from_ref(&staging.0),
    )
    .await?;
    Ok(())
}
