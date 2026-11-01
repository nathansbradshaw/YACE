//! Only portable song metadata is shared with native chart export.
pub use crate::song_files::{
    chart_file, copy_song_source, editable_chart_file, organize_song_stage, validate_existing,
};
pub use crate::song_metadata::*;
use anyhow::Result;
use std::path::Path;
pub fn copy_folder(from: &Path, to: &Path) -> Result<()> {
    for (path, _) in crate::storage::list(from)? {
        crate::storage::copy(&path, to.join(path.strip_prefix(from)?))?;
    }
    Ok(())
}
pub fn destination(path: &Path, _source: Option<&Path>) -> Result<()> {
    anyhow::ensure!(
        !crate::storage::exists(path),
        "Choose a new output folder; existing files are preserved"
    );
    Ok(())
}
