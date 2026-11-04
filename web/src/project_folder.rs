//! Bridge between a selected desktop project folder and the WASM working set.
use anyhow::{Context, Result, ensure};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    File, FileSystemCreateWritableOptions, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetDirectoryOptions, FileSystemGetFileOptions, FileSystemHandle,
    FileSystemWritableFileStream,
};

pub type Directory = FileSystemDirectoryHandle;
type Snapshot = BTreeMap<PathBuf, Arc<Vec<u8>>>;
type DiskFiles = BTreeMap<PathBuf, File>;
thread_local! {
    static OPEN: RefCell<BTreeMap<PathBuf, Rc<Folder>>> = RefCell::default();
}

fn error(value: JsValue) -> anyhow::Error {
    if let Some(error) = value.dyn_ref::<web_sys::DomException>() {
        anyhow::anyhow!("{}", error.message())
    } else {
        anyhow::anyhow!("{value:?}")
    }
}

fn exception(value: &JsValue, name: &str) -> bool {
    value
        .dyn_ref::<web_sys::DomException>()
        .is_some_and(|e| e.name() == name)
}

pub fn supported() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::get(&window, &"showDirectoryPicker".into())
            .is_ok_and(|method| method.is_function())
    })
}

// Call the picker from the click handler, before an await consumes activation.
pub fn pick() -> Result<js_sys::Promise<Directory>> {
    let window = web_sys::window().context("No browser window")?;
    let method = js_sys::Reflect::get(&window, &"showDirectoryPicker".into())
        .map_err(error)?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| anyhow::anyhow!("This browser cannot save directly to a folder"))?;
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"mode".into(), &"readwrite".into()).map_err(error)?;
    Ok(method
        .call1(&window, &options)
        .map_err(error)?
        .unchecked_into())
}

pub async fn selected(picker: js_sys::Promise<Directory>) -> Result<Option<Directory>> {
    match JsFuture::from(picker).await {
        Ok(handle) => Ok(Some(handle)),
        Err(value) if exception(&value, "AbortError") => Ok(None),
        Err(value) => Err(error(value)).context("Could not open the project folder"),
    }
}

async fn directory(parent: &Directory, path: &Path, create: bool) -> Result<Option<Directory>> {
    crate::archive_codec::relative_name(path)?;
    let options = FileSystemGetDirectoryOptions::new();
    options.set_create(create);
    let mut parent = parent.clone();
    for component in path {
        let name = component.to_str().context("Invalid folder name")?;
        match JsFuture::from(parent.get_directory_handle_with_options(name, &options)).await {
            Ok(handle) => parent = handle.unchecked_into(),
            Err(value) if !create && exception(&value, "NotFoundError") => return Ok(None),
            Err(value) => return Err(error(value)).with_context(|| format!("Folder {name}")),
        }
    }
    Ok(Some(parent))
}

async fn file_handle(
    root: &Directory,
    path: &Path,
    create: bool,
) -> Result<Option<FileSystemFileHandle>> {
    crate::archive_codec::relative_name(path)?;
    let parent = path.parent().context("Invalid project file")?;
    let folder = if parent.as_os_str().is_empty() {
        root.clone()
    } else if let Some(folder) = directory(root, parent, create).await? {
        folder
    } else {
        return Ok(None);
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid project filename")?;
    let options = FileSystemGetFileOptions::new();
    options.set_create(create);
    match JsFuture::from(folder.get_file_handle_with_options(name, &options)).await {
        Ok(handle) => Ok(Some(handle.unchecked_into())),
        Err(value) if !create && exception(&value, "NotFoundError") => Ok(None),
        Err(value) => Err(error(value)).with_context(|| format!("File {}", path.display())),
    }
}

async fn file(handle: &FileSystemFileHandle) -> Result<File> {
    Ok(JsFuture::from(handle.get_file())
        .await
        .map_err(error)?
        .unchecked_into())
}

async fn contents(root: &Directory) -> Result<DiskFiles> {
    let mut files = BTreeMap::new();
    let mut stack = vec![(PathBuf::new(), root.clone())];
    let mut count = 0;
    while let Some((path, folder)) = stack.pop() {
        let entries = folder.values();
        loop {
            let next = JsFuture::from(entries.next().map_err(error)?)
                .await
                .map_err(error)?;
            if js_sys::Reflect::get(&next, &"done".into())
                .map_err(error)?
                .is_truthy()
            {
                break;
            }
            let handle: FileSystemHandle = js_sys::Reflect::get(&next, &"value".into())
                .map_err(error)?
                .unchecked_into();
            let child = path.join(handle.name());
            crate::archive_codec::relative_name(&child)?;
            count += 1;
            ensure!(
                count <= crate::compact_backup::MAX_FILES,
                "The project folder has too many entries"
            );
            if let Some(folder) = handle.dyn_ref::<Directory>() {
                stack.push((child, folder.clone()));
            } else if let Some(handle) = handle.dyn_ref::<FileSystemFileHandle>()
                && !session_file(&child)
            {
                files.insert(child, file(handle).await?);
            }
        }
    }
    Ok(files)
}

fn session_file(path: &Path) -> bool {
    path.parent().is_some_and(|p| p.as_os_str().is_empty())
        && path.to_str().is_some_and(|name| {
            matches!(
                name,
                "editor.lock" | "window.lock" | "worker.lock" | "vocal-worker.lock"
            )
        })
}

async fn mount(files: &DiskFiles, root: &Path) -> Result<Snapshot> {
    let total: f64 = files.values().map(|file| file.size()).sum();
    ensure!(
        total <= crate::archive_import::available_budget() as f64,
        "This project exceeds the memory available to the browser editor"
    );
    let mut snapshot = BTreeMap::new();
    for (path, file) in files {
        let bytes = Arc::new(crate::files::read_file(file).await?);
        crate::storage::write_shared(root.join(path), bytes.clone())?;
        snapshot.insert(path.clone(), bytes);
    }
    Ok(snapshot)
}

struct Folder {
    directory: Directory,
    manifest: &'static str,
    previous: RefCell<Snapshot>,
    files: RefCell<DiskFiles>,
    saving: futures_util::lock::Mutex<()>,
}

impl Folder {
    async fn verify(&self, path: &Path, current: Option<&FileSystemFileHandle>) -> Result<()> {
        let expected = self.files.borrow().get(path).cloned();
        let same = match (expected, current) {
            (None, None) => true,
            (Some(expected), Some(current)) => {
                let current = file(current).await?;
                current.size() == expected.size()
                    && current.last_modified() == expected.last_modified()
                    && (path != Path::new(self.manifest)
                        || crate::files::read_file(&current).await?
                            == *self.previous.borrow()[path])
            }
            _ => false,
        };
        ensure!(
            same,
            "{} changed outside YACE. Download this project to keep your edits, then reopen the folder.",
            path.display()
        );
        Ok(())
    }

    async fn write(&self, path: &Path, bytes: Arc<Vec<u8>>) -> Result<()> {
        let existing = file_handle(&self.directory, path, false).await?;
        self.verify(path, existing.as_ref()).await?;
        let handle = match existing {
            Some(handle) => handle,
            None => {
                let handle = file_handle(&self.directory, path, true)
                    .await?
                    .context("Could not create file")?;
                // Creating a handle creates an empty file. Retain that baseline for retries.
                let empty = file(&handle).await?;
                ensure!(
                    empty.size() == 0.,
                    "{} appeared in the folder while saving",
                    path.display()
                );
                self.files.borrow_mut().insert(path.to_owned(), empty);
                self.previous
                    .borrow_mut()
                    .insert(path.to_owned(), Arc::default());
                handle
            }
        };
        let options = FileSystemCreateWritableOptions::new();
        js_sys::Reflect::set(&options, &"mode".into(), &"exclusive".into()).map_err(error)?;
        let stream: FileSystemWritableFileStream =
            JsFuture::from(handle.create_writable_with_options(&options))
                .await
                .map_err(error)?
                .unchecked_into();
        let result = async {
            self.verify(path, Some(&handle)).await?;
            // Write bounded chunks so a stem doesn't need a second full-size JS array.
            for chunk in bytes.chunks(4 * 1024 * 1024) {
                let array = js_sys::Uint8Array::from(chunk);
                JsFuture::from(stream.write_with_js_u8_array(&array).map_err(error)?)
                    .await
                    .map_err(error)?;
            }
            JsFuture::from(stream.close()).await.map_err(error)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            let _ = JsFuture::from(stream.abort()).await;
            return Err(error);
        }
        let saved = file(&handle).await?;
        self.files.borrow_mut().insert(path.to_owned(), saved);
        self.previous.borrow_mut().insert(path.to_owned(), bytes);
        Ok(())
    }

    async fn save(&self, root: &Path) -> Result<()> {
        let _saving = self.saving.lock().await;
        let manifest = Path::new(self.manifest);
        let existing = file_handle(&self.directory, manifest, false).await?;
        self.verify(manifest, existing.as_ref()).await?;
        let changes: Vec<_> = crate::storage::list(root)?
            .into_iter()
            .map(|(path, bytes)| Ok((path.strip_prefix(root)?.to_owned(), bytes)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|(path, bytes)| {
                !session_file(path)
                    && self.previous.borrow().get(path).is_none_or(|old| {
                        !Arc::ptr_eq(old, bytes) && old.as_slice() != bytes.as_slice()
                    })
            })
            .collect();
        // The document is the commit point. Publish source assets and history first.
        for (path, bytes) in changes.iter().filter(|(path, _)| path != manifest) {
            self.write(path, bytes.clone()).await?;
        }
        if let Some((_, bytes)) = changes.iter().find(|(path, _)| path == manifest) {
            self.write(manifest, bytes.clone()).await?;
        }
        Ok(())
    }
}

pub fn linked(root: &Path) -> bool {
    OPEN.with(|open| open.borrow().contains_key(root))
}

pub fn forget(root: &Path) {
    OPEN.with(|open| open.borrow_mut().retain(|path, _| !path.starts_with(root)));
}

pub async fn save(root: &Path) -> Result<bool> {
    let folder = OPEN.with(|open| open.borrow().get(root).cloned());
    let Some(folder) = folder else {
        return Ok(false);
    };
    folder
        .save(root)
        .await
        .context("Could not save to the project folder")?;
    Ok(true)
}

fn link(
    root: &Path,
    directory: Directory,
    manifest: &'static str,
    files: DiskFiles,
    previous: Snapshot,
) {
    OPEN.with(|open| {
        open.borrow_mut().insert(
            root.to_owned(),
            Rc::new(Folder {
                directory,
                manifest,
                previous: RefCell::new(previous),
                files: RefCell::new(files),
                saving: futures_util::lock::Mutex::new(()),
            }),
        )
    });
}

pub async fn attach_new(root: &Path, selected: Directory, manifest: &'static str) -> Result<()> {
    let iterator = selected.values();
    let next = JsFuture::from(iterator.next().map_err(error)?)
        .await
        .map_err(error)?;
    ensure!(
        js_sys::Reflect::get(&next, &"done".into())
            .map_err(error)?
            .is_truthy(),
        "Choose an empty folder for this project"
    );
    link(root, selected, manifest, DiskFiles::new(), Snapshot::new());
    if let Err(error) = save(root).await {
        forget(root);
        return Err(error);
    }
    Ok(())
}

/// Open either a standalone project or the editor directory inside a song folder.
pub async fn open(root: &Path, selected: Directory) -> Result<&'static str> {
    let project = Path::new("project.json");
    let dance = Path::new("stepmania-project.json");
    let (folder, manifest, nested) = if file_handle(&selected, project, false).await?.is_some() {
        (selected.clone(), "project.json", false)
    } else if file_handle(&selected, dance, false).await?.is_some() {
        (selected.clone(), "stepmania-project.json", false)
    } else {
        let folder = directory(&selected, Path::new(".yace/project"), false)
            .await?
            .context(
                "Choose a YACE project folder. For an unedited song, use Open charting folder.",
            )?;
        ensure!(
            file_handle(&folder, project, false).await?.is_some(),
            "This folder does not contain a saved YACE project"
        );
        (folder, "project.json", true)
    };
    let files = contents(&folder).await?;
    let previous = mount(&files, root).await?;
    if nested {
        import_song_stems(root, &selected, &previous).await?;
    }
    link(root, folder, manifest, files, previous);
    Ok(manifest)
}

async fn import_song_stems(root: &Path, song: &Directory, project: &Snapshot) -> Result<()> {
    let named = project
        .iter()
        .map(|(path, bytes)| Ok((crate::archive_codec::relative_name(path)?, bytes.clone())))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut needed: std::collections::BTreeSet<_> =
        crate::project_portability::referenced_stems(&named)?
            .into_values()
            .collect();
    if needed.is_empty() {
        return Ok(());
    }
    for (path, bytes) in project {
        if crate::export_options::is_audio(path) {
            needed.remove(&crate::models::hash_shared(bytes));
        }
    }
    if needed.is_empty() {
        return Ok(());
    }
    let Some(audio) = directory(song, Path::new(".yace/audio"), false).await? else {
        return Ok(());
    };
    for (path, file) in contents(&audio).await? {
        if !crate::export_options::is_audio(&path) {
            continue;
        }
        ensure!(
            file.size() <= crate::archive_import::available_budget() as f64,
            "A song stem exceeds the memory available to the browser editor"
        );
        let bytes = Arc::new(crate::files::read_file(&file).await?);
        let hash = crate::models::hash_shared(&bytes);
        if needed.remove(&hash) {
            let ext = path.extension().unwrap_or_default().to_string_lossy();
            crate::storage::write_shared(root.join(format!("assets/stems/{hash}.{ext}")), bytes)?;
        }
        if needed.is_empty() {
            break;
        }
    }
    Ok(())
}
