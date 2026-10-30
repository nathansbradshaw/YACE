//! Use the operating system's picker through the browser's standard file input.
#[cfg(not(feature = "browser"))]
pub use rfd::*;
#[cfg(not(feature = "browser"))]
pub async fn download(_path: &std::path::Path) {}
pub fn output_parent() -> Option<std::path::PathBuf> {
    #[cfg(feature = "browser")]
    {
        dioxus::dioxus_core::Runtime::try_current()
            .and_then(|_| dioxus::prelude::try_consume_context::<crate::browser::BrowserContext>())
            .map(|context| context.root)
    }
    #[cfg(not(feature = "browser"))]
    {
        None
    }
}

#[cfg(feature = "browser")]
mod web {
    use crate::browser::BrowserContext;
    use anyhow::{Context, Result};
    use dioxus::prelude::*;
    use std::path::{Path, PathBuf};

    #[derive(Clone, Copy)]
    pub struct Dialogs(Signal<Option<String>>);
    pub fn provide_browser_dialogs() {
        use_context_provider(|| Dialogs(Signal::new(None)));
    }
    #[derive(Default)]
    pub struct AsyncFileDialog {
        title: String,
        name: String,
        accept: Vec<String>,
    }
    #[derive(Clone)]
    pub struct FileHandle(PathBuf);
    impl FileHandle {
        pub fn path(&self) -> &Path {
            &self.0
        }
        pub fn file_name(&self) -> String {
            self.0
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        }
    }
    impl AsyncFileDialog {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn set_title(mut self, title: impl Into<String>) -> Self {
            self.title = title.into();
            self
        }
        pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
            self.name = name.into();
            self
        }
        pub fn set_directory(self, _directory: impl AsRef<Path>) -> Self {
            self
        }
        pub fn add_filter(
            mut self,
            _name: impl Into<String>,
            extensions: &[impl AsRef<str>],
        ) -> Self {
            self.accept.extend(
                extensions
                    .iter()
                    .map(|value| format!(".{}", value.as_ref())),
            );
            self
        }
        pub async fn pick_file(self) -> Option<FileHandle> {
            self.pick("file").await
        }
        pub async fn pick_folder(self) -> Option<FileHandle> {
            self.pick("folder").await
        }
        async fn pick(self, mode: &str) -> Option<FileHandle> {
            let mut dialogs = consume_context::<Dialogs>();
            let script = format!(
                "return await window.__yaceTakePick({},{});",
                serde_json::to_string(mode).unwrap(),
                serde_json::to_string(&self.accept.join(",")).unwrap()
            );
            let result = async {
                let value = dioxus_document::eval(&script).await?;
                if value.is_null() {
                    return Ok(None);
                }
                if let Some(error) = value["error"].as_str() {
                    anyhow::bail!("{error}");
                }
                let path = value[if mode == "folder" {
                    "directory"
                } else {
                    "path"
                }]
                .as_str()
                .context("The file picker returned no selection")?;
                Ok::<_, anyhow::Error>(Some(FileHandle(path.into())))
            }
            .await;
            match result {
                Ok(path) => path,
                Err(error) => {
                    dialogs.0.set(Some(format!("{}: {error:#}", self.title)));
                    None
                }
            }
        }
        pub async fn save_file(self) -> Option<FileHandle> {
            let context = consume_context::<BrowserContext>();
            let mut dialogs = consume_context::<Dialogs>();
            let result = (|| -> Result<_> {
                let category = if self.title.to_lowercase().contains("project") {
                    "projects"
                } else {
                    "exports"
                };
                let parent = context.root.join(category);
                std::fs::create_dir_all(&parent)?;
                let destination = tempfile::tempdir_in(parent)?.keep();
                let name = self.name.replace(['/', '\\', '\0'], "-");
                let name = name.trim().trim_matches('.');
                let name = if name.is_empty() { "YACE export" } else { name };
                Ok(FileHandle(context.resolve(&destination.join(name))?))
            })();
            match result {
                Ok(path) => Some(path),
                Err(error) => {
                    dialogs
                        .0
                        .set(Some(format!("Cannot prepare export: {error:#}")));
                    None
                }
            }
        }
    }
    #[component]
    pub fn BrowserFiles() -> Element {
        let mut dialogs = use_context::<Dialogs>();
        rsx! {if let Some(error)=dialogs.0.read().as_ref(){div{class:"studio-error",role:"alert","{error}" button{onclick:move |_|dialogs.0.set(None),"Dismiss"}}}}
    }
    pub async fn download(path: &Path) {
        let context = consume_context::<BrowserContext>();
        let mut dialogs = consume_context::<Dialogs>();
        let source = path.to_owned();
        let root = context.root.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<_> {
            if source.is_dir() {
                let parent = root.join(".downloads");
                std::fs::create_dir_all(&parent)?;
                let temp = tempfile::tempdir_in(parent)?;
                let name = format!(
                    "{}.zip",
                    source.file_name().unwrap_or_default().to_string_lossy()
                );
                let output = temp.path().join(name);
                crate::browser::archive_folder(&source, &output)?;
                let _ = temp.keep();
                Ok(output)
            } else {
                Ok(source)
            }
        })
        .await;
        let result=async {
            let path=result??;let url=context.register(&path)?;
            let name=path.file_name().unwrap_or_default().to_string_lossy();
            dioxus_document::eval(&format!("const link=document.createElement('a');link.href={};link.download={};const dialogs=document.querySelectorAll('dialog[open]');(dialogs[dialogs.length-1]||document.body).append(link);link.click();link.remove();",serde_json::to_string(&url)?,serde_json::to_string(&name)?)).await?;
            Ok::<_,anyhow::Error>(())
        }.await;
        if let Err(error) = result {
            dialogs.0.set(Some(format!(
                "Download failed: {error:#}. The saved export is retained in the workspace."
            )));
        }
    }
}
#[cfg(feature = "browser")]
pub use web::*;
