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
    compact: Option<crate::compact_backup::Options>,
) -> Result<()> {
    let staging = Staging(output.with_extension("input"));
    if let Some(parent) = parent {
        for (path, bytes) in crate::storage::list(parent)? {
            crate::storage::write_shared(staging.0.join(path.strip_prefix(parent)?), bytes)?;
        }
        let originals = crate::storage::list(parent)?
            .into_iter()
            .map(|(path, bytes)| {
                Ok((
                    crate::archive_codec::relative_name(path.strip_prefix(parent)?)?,
                    bytes,
                ))
            })
            .collect::<anyhow::Result<std::collections::BTreeMap<_, _>>>()?;
        for (path, hash) in crate::project_portability::referenced_stems(&originals)? {
            if compact.is_some_and(|options| options.include_generated_stems)
                && !path.starts_with(parent)
                && let Ok(bytes) = crate::storage::shared(&path)
            {
                anyhow::ensure!(
                    crate::models::hash_shared(&bytes).eq_ignore_ascii_case(&hash),
                    "Attached stem checksum differs during backup"
                );
                let ext = path.extension().unwrap_or_default().to_string_lossy();
                crate::storage::write_shared(
                    staging.0.join(format!("assets/stems/{hash}.{ext}")),
                    bytes,
                )?;
            }
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
        if let Some(options) = compact {
            crate::worker::Task::CompactArchive {
                root: staging.0.clone(),
                output: output.to_owned(),
                options,
            }
        } else {
            crate::worker::Task::Archive {
                root: staging.0.clone(),
                output: output.to_owned(),
            }
        },
        std::slice::from_ref(&staging.0),
    )
    .await?;
    Ok(())
}
