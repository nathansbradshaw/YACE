//! Explicit destination for a newly created project. Never infer browser storage.
use anyhow::{Result, bail};
use std::path::Path;

#[derive(Clone, Default)]
pub enum Choice {
    #[default]
    Unchosen,
    Browser,
    Folder(crate::project_folder::Directory),
}

impl Choice {
    pub fn ready(&self) -> bool {
        !matches!(self, Self::Unchosen)
    }

    /// Save the prepared project before publishing it into the editor. A failed
    /// folder write must not turn the user's choice into a browser-only project.
    pub async fn save(&self, root: &Path) -> Result<()> {
        match self {
            Self::Unchosen => bail!("Choose where to save this project before creating it"),
            Self::Browser => Ok(()),
            Self::Folder(directory) => {
                crate::project_folder::attach_new(root, directory.clone(), "project.json").await
            }
        }
    }
}
