//! File operations use the same Rust controls in desktop and browser builds.
#[cfg(not(feature = "browser"))]
pub use rfd::*;
#[cfg(not(feature = "browser"))]
pub async fn download(_path: &std::path::Path) {}

#[cfg(feature = "browser")]
mod web {
    use crate::browser::BrowserContext;
    use anyhow::{Result, ensure};
    use dioxus::prelude::*;
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
        rc::Rc,
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        File,
        Folder,
        Save,
    }
    struct Request {
        title: String,
        name: String,
        mode: Mode,
        reply: RefCell<Option<tokio::sync::oneshot::Sender<PathBuf>>>,
    }
    #[derive(Clone, Copy)]
    pub struct Dialogs(Signal<Option<Rc<Request>>>, Signal<Option<String>>);
    pub fn provide_browser_dialogs() {
        use_context_provider(|| Dialogs(Signal::new(None), Signal::new(None)));
    }
    #[derive(Default)]
    pub struct AsyncFileDialog {
        title: String,
        name: String,
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
    impl AsRef<Path> for FileHandle {
        fn as_ref(&self) -> &Path {
            self.path()
        }
    }
    impl std::ops::Deref for FileHandle {
        type Target = Path;
        fn deref(&self) -> &Path {
            self.path()
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
        pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl AsRef<str>]) -> Self {
            self
        }
        pub async fn pick_file(self) -> Option<FileHandle> {
            self.pick(Mode::File).await
        }
        pub async fn pick_folder(self) -> Option<FileHandle> {
            self.pick(Mode::Folder).await
        }
        pub async fn save_file(self) -> Option<FileHandle> {
            self.pick(Mode::Save).await
        }
        async fn pick(self, mode: Mode) -> Option<FileHandle> {
            let mut dialogs = consume_context::<Dialogs>();
            let (tx, rx) = tokio::sync::oneshot::channel();
            dialogs.0.set(Some(Rc::new(Request {
                title: if self.title.is_empty() {
                    match mode {
                        Mode::File => "Choose file",
                        Mode::Folder => "Choose folder",
                        Mode::Save => "Save as",
                    }
                    .into()
                } else {
                    self.title
                },
                name: self.name,
                mode,
                reply: RefCell::new(Some(tx)),
            })));
            rx.await.ok().map(FileHandle)
        }
    }
    fn finish(mut dialogs: Dialogs, path: Option<PathBuf>) {
        if let Some(request) = dialogs.0.peek().as_ref()
            && let Some(reply) = request.reply.borrow_mut().take()
            && let Some(path) = path
        {
            let _ = reply.send(path);
        }
        dialogs.0.set(None);
    }
    #[component]
    pub fn BrowserFiles() -> Element {
        let mut dialogs = use_context::<Dialogs>();
        let request = dialogs.0.read().clone();
        let Some(request) = request else {
            return rsx! {if let Some(error)=dialogs.1.read().as_ref(){div{class:"studio-error",role:"alert","{error}" button{onclick:move |_|dialogs.1.set(None),"Dismiss"}}}};
        };
        let key = format!("{:p}", Rc::as_ptr(&request));
        rsx! { FileChooser {key:"{key}",request,dialogs} }
    }
    #[derive(Props, Clone)]
    struct ChooserProps {
        request: Rc<Request>,
        dialogs: Dialogs,
    }
    impl PartialEq for ChooserProps {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.request, &other.request)
        }
    }
    #[component]
    fn FileChooser(props: ChooserProps) -> Element {
        let context = use_context::<BrowserContext>();
        let mut folder = use_signal(|| context.root.clone());
        let mut name = use_signal(|| props.request.name.clone());
        let mut error = use_signal(String::new);
        let current = folder();
        let entries = std::fs::read_dir(&current)
            .map(|items| {
                let mut entries: Vec<_> = items
                    .filter_map(|item| item.ok())
                    .filter(|item| !item.file_name().to_string_lossy().starts_with('.'))
                    .filter_map(|item| {
                        let path = item.path();
                        context.resolve(&path).ok().map(|_| {
                            (
                                path.is_dir(),
                                item.file_name().to_string_lossy().into_owned(),
                                path,
                            )
                        })
                    })
                    .collect();
                entries.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
                });
                entries
            })
            .unwrap_or_default();
        let mode = props.request.mode;
        let dialogs = props.dialogs;
        let title = props.request.title.clone();
        let parent = current
            .parent()
            .filter(|p| p.starts_with(&context.root))
            .map(Path::to_owned);
        let relative = current
            .strip_prefix(&context.root)
            .unwrap_or(&current)
            .display()
            .to_string();
        let new_context = context.clone();
        let save_context = context.clone();
        let css = r#"
.browser-file-overlay{position:fixed;inset:0;background:#000b;z-index:30000;display:grid;place-items:center;color:var(--text-primary)}
.browser-file-dialog{box-sizing:border-box;width:min(720px,94vw);max-height:85vh;display:flex;flex-direction:column;gap:14px;padding:24px;background:var(--bg-secondary);border:1px solid var(--border-color);border-radius:8px}
.browser-file-list{overflow:auto;min-height:180px;max-height:40vh;display:flex;flex-direction:column;gap:4px}
.browser-file-list button{text-align:left;padding:9px;background:var(--bg-tertiary);color:inherit;border:1px solid var(--border-color);border-radius:3px}
.browser-file-actions{display:flex;gap:10px;align-items:center;flex-wrap:wrap}.browser-file-dialog input{padding:8px;min-width:100px;background:var(--bg-tertiary);color:inherit;border:1px solid var(--border-color)}
.browser-file-actions>.field-button{width:auto;flex:0 0 auto}.browser-file-actions>span{flex:1;overflow-wrap:anywhere}.browser-file-actions>input{flex:1;min-width:120px}
"#;
        rsx! {
            super::super::modal::Modal {id:"browser-files",label:"Workspace files",onclose:move |_|finish(dialogs,None),
                style {"{css}"}
                div {class:"browser-file-dialog",
                    h2 {"{title}"}
                    div {class:"browser-file-actions",
                        button {class:"field-button",disabled:parent.is_none(),onclick:move |_|{if let Some(path)=&parent{folder.set(path.clone())}},"Up"}
                        span {"Workspace / {relative}"}
                        button {class:"field-button",onclick:move |_|finish(dialogs,None),"Cancel"}
                    }
                    div {class:"browser-file-list",
                        for (directory,label,path) in entries {
                            { let open_path=path.clone();let download_path=path.clone();rsx!{
                                div {class:"browser-file-actions",
                                    button {style:"flex:1",disabled:!directory&&mode==Mode::Folder,onclick:move |_|{if directory{folder.set(open_path.clone());}else if mode==Mode::File{finish(dialogs,Some(open_path.clone()));}else{name.set(label.clone());}},if directory{"Folder · "}"{label}"}
                                    button {class:"field-button",onclick:move |_|{let path=download_path.clone();spawn(async move{download(&path).await;});},"Download"}
                                }
                            }}
                        }
                        if mode!=Mode::Save {
                            label {"Upload from your computer" input {r#type:"file",id:"yace-browser-upload",multiple:mode==Mode::Folder,"webkitdirectory":if mode==Mode::Folder{Some("")}else{None},
                                onmounted:move |_|{spawn(upload_input("yace-browser-upload",mode,dialogs,error));}
                            }}
                            if mode==Mode::Folder {label {"Or upload a song ZIP" input {r#type:"file",id:"yace-browser-zip",accept:".zip",onmounted:move |_|{spawn(upload_input("yace-browser-zip",mode,dialogs,error));}}}}
                        }
                    }
                    div {class:"browser-file-actions",
                        input {aria_label:if mode==Mode::Save{"Save name"}else{"New folder name"},value:"{name}",oninput:move|e|name.set(e.value())}
                        button {class:"field-button",onclick:move |_|{
                            let result=(||->Result<()>{let target=new_context.resolve(&folder().join(name()))?;ensure!(!name().trim().is_empty(),"Enter a name");std::fs::create_dir(&target)?;folder.set(target);name.set(String::new());Ok(())})();if let Err(e)=result{error.set(e.to_string());}
                        },"New folder"}
                        if mode==Mode::Folder {button {class:"field-button",onclick:move |_|finish(dialogs,Some(folder())),"Choose this folder"}}
                        if mode==Mode::Save {button {class:"field-button",onclick:move |_|{
                            let result=(||->Result<_>{ensure!(!name().trim().is_empty(),"Enter a name");let path=save_context.resolve(&folder().join(name()))?;ensure!(!path.exists(),"Choose a new name; existing files are preserved");Ok(path)})();match result{Ok(path)=>finish(dialogs,Some(path)),Err(e)=>error.set(e.to_string())}
                        },"Save here"}}
                    }
                    if !error().is_empty(){p{role:"alert","{error}"}}
                }
            }
        }
    }
    async fn upload_input(id: &str, mode: Mode, dialogs: Dialogs, mut error: Signal<String>) {
        let script = format!(
            "const input=document.getElementById({});",
            serde_json::to_string(id).unwrap()
        ) + r#"
input.addEventListener('input',event=>event.stopPropagation());
input.onchange=async(event)=>{event.stopPropagation();try{const form=new FormData();for(const file of input.files){const relative=file.webkitRelativePath?file.webkitRelativePath.split('/').slice(1).join('/'):file.name;form.append('file',file,relative);}input.disabled=true;const response=await fetch('/upload',{method:'POST',body:form});if(!response.ok)throw Error(await response.text());dioxus.send(await response.json());}catch(error){input.disabled=false;dioxus.send({error:String(error)});}};
await new Promise(()=>{});"#;
        let mut upload = dioxus_document::eval(&script);
        while let Ok(value) = upload.recv::<serde_json::Value>().await {
            if let Some(message) = value["error"].as_str() {
                error.set(message.into());
            } else if let Some(path) = value[if mode == Mode::Folder {
                "directory"
            } else {
                "path"
            }]
            .as_str()
            {
                finish(dialogs, Some(PathBuf::from(path)));
                break;
            }
        }
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
            dioxus_document::eval(&format!("const link=document.createElement('a');link.href={};link.download={};link.click();",serde_json::to_string(&url)?,serde_json::to_string(&name)?)).await?;
            Ok::<_,anyhow::Error>(())
        }.await;
        if let Err(error) = result {
            dialogs.1.set(Some(format!(
                "Download failed: {error:#}. The saved export is retained in the workspace."
            )));
        }
    }
}
#[cfg(feature = "browser")]
pub use web::*;
