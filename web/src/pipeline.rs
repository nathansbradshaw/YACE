//! Only portable song metadata is shared with native chart export.
pub use crate::song_metadata::*;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
pub fn audio_file(folder: &Path) -> Result<PathBuf> {
    [
        "song.ogg",
        "song.mp3",
        "song.wav",
        "song.flac",
        "song.opus",
        "song.m4a",
    ]
    .iter()
    .map(|name| folder.join(name))
    .find(|path| crate::storage::is_file(path))
    .context("The song needs its recording (song.ogg, .mp3, .wav, .flac, .opus or .m4a)")
}
pub fn copy_folder(from: &Path, to: &Path) -> Result<()> {
    for (path, _) in crate::storage::list(from) {
        crate::storage::copy(&path, to.join(path.strip_prefix(from)?))?;
    }
    Ok(())
}
