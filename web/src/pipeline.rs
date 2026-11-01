//! Only portable song metadata is shared with native chart export.
pub use crate::generation_build::{Build, build, build_with_lyrics, with_generated_rule_reviews,lyric_events};
pub use crate::recording_metadata::Metadata;
pub use crate::song_files::{
    chart_file, copy_song_source, editable_chart_file, organize_song_stage, validate_existing,
};
pub use crate::song_metadata::*;
use anyhow::Result;
use std::path::Path;
pub fn publish(stage: &Path, output: &Path) -> Result<()> {
    anyhow::ensure!(
        !crate::storage::exists(output),
        "Output appeared during processing; refusing to overwrite"
    );
    crate::storage::rename(stage, output)?;
    Ok(())
}
pub fn lyrics(_meta: &Metadata) -> Result<Option<serde_json::Value>> {
    Ok(None)
}
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
