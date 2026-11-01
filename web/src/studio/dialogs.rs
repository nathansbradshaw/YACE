//! Portable dialog adapter for shared review/vocal file controls.
use dioxus::prelude::WritableExt;
use std::path::{Path, PathBuf};
pub fn output_parent() -> Option<PathBuf> {
    Some(crate::files::unique_root())
}
#[derive(Default)]
pub struct AsyncFileDialog {
    name: String,
    accept: String,
}
pub struct FileHandle(PathBuf);
impl FileHandle {
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl AsyncFileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
    pub fn add_filter(mut self, _title: impl Into<String>, extensions: &[impl AsRef<str>]) -> Self {
        self.accept = extensions
            .iter()
            .map(|s| format!(".{}", s.as_ref()))
            .collect::<Vec<_>>()
            .join(",");
        self
    }
    pub async fn pick_file(self) -> Option<FileHandle> {
        self.pick(false).await
    }
    pub async fn pick_folder(self) -> Option<FileHandle> {
        self.pick(true).await
    }
    async fn pick(self, folder: bool) -> Option<FileHandle> {
        let result = async {
            let Some(files) =
                crate::files::pick(false, if folder { ".zip" } else { &self.accept }).await?
            else {
                return Ok(None);
            };
            let files = if folder {
                crate::files::unpack(files)?
            } else {
                files
            };
            let first = files
                .first()
                .ok_or_else(|| anyhow::anyhow!("The selected archive has no files"))?
                .0
                .clone();
            let root = crate::files::unique_root();
            crate::files::mount(&root, files)?;
            Ok::<_, anyhow::Error>(Some(FileHandle(if folder {
                crate::files::imported_root(&root)?
            } else {
                root.join(first)
            })))
        }
        .await;
        match result {
            Ok(file) => file,
            Err(error) => {
                report(error);
                None
            }
        }
    }
    pub async fn save_file(self) -> Option<FileHandle> {
        Some(FileHandle(crate::files::unique_root().join(
            if self.name.is_empty() {
                "export.json"
            } else {
                &self.name
            },
        )))
    }
}
pub async fn download(path: &Path) {
    let result = (|| -> anyhow::Result<()> {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if crate::storage::is_file(path) {
            crate::files::download(&name, &crate::storage::shared(path)?)
        } else {
            anyhow::ensure!(
                !crate::storage::list(path)?.is_empty(),
                "No export files were produced"
            );
            crate::files::download(&format!("{name}.zip"), &crate::files::archive(path)?)
        }
    })();
    if let Err(error) = result {
        report(error);
    }
}
fn report(error: anyhow::Error) {
    if let Some(mut studio) = dioxus::prelude::try_consume_context::<super::state::Studio>() {
        studio
            .error
            .set(Some(format!("Local file operation failed: {error:#}")));
    }
}
