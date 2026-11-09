//! Explicit destination for a newly created project. Never infer browser storage.
use anyhow::{Result, bail};
use std::path::Path;

#[derive(Clone)]
pub enum Choice {
    Unchosen,
    PickFolder,
    Browser,
    Folder(crate::project_folder::Directory),
}
impl Default for Choice {
    fn default() -> Self {
        if crate::project_folder::supported() {
            Self::PickFolder
        } else {
            Self::Unchosen
        }
    }
}

impl Choice {
    pub fn ready(&self) -> bool {
        matches!(self, Self::Browser | Self::Folder(_))
    }
    pub fn can_start(&self) -> bool {
        !matches!(self, Self::Unchosen)
    }
    pub fn create_label(&self) -> &'static str {
        if matches!(self, Self::PickFolder) {
            "Choose folder & create chart"
        } else {
            "Create chart"
        }
    }
    /// Begin the picker synchronously in the create click's user activation.
    pub fn resolve(self) -> Result<impl std::future::Future<Output = Result<Option<Self>>>> {
        anyhow::ensure!(self.can_start(), "Choose where to save this project");
        let picker = if matches!(self, Self::PickFolder) {
            Some(crate::project_folder::pick()?)
        } else {
            None
        };
        Ok(async move {
            if let Some(picker) = picker {
                Ok(crate::project_folder::selected(picker)
                    .await?
                    .map(Self::Folder))
            } else {
                Ok(Some(self))
            }
        })
    }

    /// Save the prepared project before publishing it into the editor. A failed
    /// folder write must not turn the user's choice into a browser-only project.
    pub async fn save(&self, root: &Path) -> Result<()> {
        match self {
            Self::Unchosen | Self::PickFolder => {
                bail!("Choose where to save this project before creating it")
            }
            Self::Browser => Ok(()),
            Self::Folder(directory) => {
                crate::project_folder::attach_new(root, directory.clone(), "project.json").await
            }
        }
    }
}
