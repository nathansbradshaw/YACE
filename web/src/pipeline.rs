//! Only portable song metadata is shared with native chart export.
pub use crate::song_metadata::*;
use anyhow::Result;
use std::path::Path;
pub fn copy_folder(from: &Path, to: &Path) -> Result<()> {
    for (path, _) in crate::storage::list(from)? {
        crate::storage::copy(&path, to.join(path.strip_prefix(from)?))?;
    }
    Ok(())
}
