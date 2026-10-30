//! Portable dialog adapter for shared review/vocal file controls.
use std::path::{Path, PathBuf};
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
        let files = crate::files::pick(false, if folder { ".zip" } else { &self.accept })
            .await
            .ok()??;
        let files = if folder {
            crate::files::unpack(files).ok()?
        } else {
            files
        };
        let first = files.first()?.0.clone();
        let root = crate::files::unique_root();
        crate::files::mount(&root, files).ok()?;
        Some(FileHandle(if folder {
            crate::files::imported_root(&root).ok()?
        } else {
            root.join(first)
        }))
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
    if let Ok(bytes) = crate::storage::shared(path) {
        let _ = crate::files::download(
            &path.file_name().unwrap_or_default().to_string_lossy(),
            &bytes,
        );
    }
}
