//! IndexedDB transactions preserve the source, checkpoints and revision together.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{IdbDatabase, IdbRequest, IdbTransaction, IdbTransactionMode};
type Sender = Rc<RefCell<Option<futures_channel::oneshot::Sender<Result<JsValue, JsValue>>>>>;
fn error(value: JsValue) -> anyhow::Error {
    anyhow::anyhow!("Browser storage: {value:?}")
}
struct RequestGuard {
    request: IdbRequest,
    _callbacks: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.request.set_onsuccess(None);
        self.request.set_onerror(None);
    }
}
async fn request(request: IdbRequest) -> Result<JsValue> {
    let (tx, rx) = futures_channel::oneshot::channel();
    let tx: Sender = Rc::new(RefCell::new(Some(tx)));
    let source = request.clone();
    let success_tx = tx.clone();
    let success = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Some(tx) = success_tx.borrow_mut().take() {
            let _ = tx.send(source.result());
        }
    });
    let source = request.clone();
    let failure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(Err(source
                .error()
                .ok()
                .flatten()
                .map(JsValue::from)
                .unwrap_or_else(|| "IndexedDB request failed".into())));
        }
    });
    request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
    request.set_onerror(Some(failure.as_ref().unchecked_ref()));
    let _guard = RequestGuard {
        request,
        _callbacks: vec![success, failure],
    };
    rx.await
        .context("Storage request interrupted")?
        .map_err(error)
}
async fn open() -> Result<IdbDatabase> {
    let factory = web_sys::window()
        .context("No browser window")?
        .indexed_db()
        .map_err(error)?
        .context("IndexedDB is unavailable")?;
    let request_open = factory
        .open_with_u32("YACE-static-projects", 1)
        .map_err(error)?;
    let source = request_open.clone();
    let upgrade = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        if let Ok(value) = source.result()
            && let Ok(db) = value.dyn_into::<IdbDatabase>()
            && !db.object_store_names().contains("files")
        {
            let _ = db.create_object_store("files");
        }
    });
    request_open.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let result = request(request_open.clone().unchecked_into()).await;
    request_open.set_onupgradeneeded(None);
    result?.dyn_into().map_err(error)
}
struct TransactionGuard {
    tx: IdbTransaction,
    _callbacks: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    rx: Option<futures_channel::oneshot::Receiver<Result<(), String>>>,
    finished: bool,
}
impl TransactionGuard {
    fn new(tx: IdbTransaction) -> Self {
        let (sender, rx) = futures_channel::oneshot::channel();
        let sender = Rc::new(RefCell::new(Some(sender)));
        let success = sender.clone();
        let done = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if let Some(sender) = success.borrow_mut().take() {
                let _ = sender.send(Ok(()));
            }
        });
        let fail = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if let Some(sender) = sender.borrow_mut().take() {
                let _=sender.send(Err("The browser could not commit the project. Export a project ZIP to preserve your work.".into()));
            }
        });
        tx.set_oncomplete(Some(done.as_ref().unchecked_ref()));
        tx.set_onabort(Some(fail.as_ref().unchecked_ref()));
        tx.set_onerror(Some(fail.as_ref().unchecked_ref()));
        Self {
            tx,
            _callbacks: vec![done, fail],
            rx: Some(rx),
            finished: false,
        }
    }
    async fn finish(&mut self) -> Result<()> {
        let result = self
            .rx
            .take()
            .context("Transaction already awaited")?
            .await
            .context("Storage transaction interrupted")?
            .map_err(anyhow::Error::msg);
        self.finished = true;
        result
    }
}
impl Drop for TransactionGuard {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.tx.abort();
        }
        self.tx.set_oncomplete(None);
        self.tx.set_onabort(None);
        self.tx.set_onerror(None);
    }
}
struct Database(IdbDatabase);
impl Drop for Database {
    fn drop(&mut self) {
        self.0.close();
    }
}
pub async fn save(root: &Path, revision: u64, expected: Option<u64>) -> Result<()> {
    save_snapshot(root, revision, expected, "last").await
}
pub async fn save_job(root: &Path, revision: u64, expected: Option<u64>) -> Result<()> {
    save_snapshot(root, revision, expected, "last-generation").await
}
pub async fn save_stepmania(root: &Path, revision: u64, expected: Option<u64>) -> Result<()> {
    save_snapshot(root, revision, expected, "last-stepmania").await
}
pub async fn restore_stepmania() -> Result<Option<(PathBuf, u64)>> {
    let db = Database(open().await?);
    let tx = db.0.transaction_with_str("files").map_err(error)?;
    let store = tx.object_store("files").map_err(error)?;
    let Some(root) = request(store.get(&"last-stepmania".into()).map_err(error)?)
        .await?
        .as_string()
    else {
        return Ok(None);
    };
    load(Path::new(&root)).await
}
async fn save_snapshot(
    root: &Path,
    revision: u64,
    expected: Option<u64>,
    last_key: &str,
) -> Result<()> {
    let db = Database(open().await?);
    let tx =
        db.0.transaction_with_str_and_mode("files", IdbTransactionMode::Readwrite)
            .map_err(error)?;
    let mut transaction = TransactionGuard::new(tx.clone());
    let store = tx.object_store("files").map_err(error)?;
    let revision_key = JsValue::from_str(&format!("revision:{}", root.display()));
    let existing = request(store.get(&revision_key).map_err(error)?)
        .await?
        .as_f64()
        .map(|v| v as u64);
    if existing != expected {
        let _ = tx.abort();
        anyhow::bail!(
            "This project changed in another tab. Export a project ZIP, then reopen the saved project before continuing."
        );
    }
    let files = crate::storage::list(root)?;
    let dirty = crate::storage::pending(root)?;
    let paths: Vec<_> = files
        .iter()
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect();
    let mut hashes = std::collections::BTreeMap::new();
    let mut written = std::collections::BTreeSet::new();
    for (path, bytes) in &dirty {
        let audio = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ["wav", "ogg", "opus", "mp3", "flac", "m4a"]
                    .contains(&ext.to_ascii_lowercase().as_str())
            });
        if !audio || bytes.len() < 65536 {
            // Mutable documents keep their existing replace-in-place encoding.
            // Content-address only large audio, so repeated edits do not leave
            // an orphaned copy of every historical project JSON document.
            store
                .put_with_key(
                    &js_sys::Uint8Array::from(bytes.as_slice()),
                    &format!("file:{}", path.display()).into(),
                )
                .map_err(error)?;
            continue;
        }
        let hash = hashes
            .entry(std::sync::Arc::as_ptr(bytes))
            .or_insert_with(|| crate::models::hash_shared(bytes));
        if written.insert(hash.clone()) {
            store
                .put_with_key(
                    &js_sys::Uint8Array::from(bytes.as_slice()),
                    &format!("blob:{hash}").into(),
                )
                .map_err(error)?;
        }
        store
            .put_with_key(
                &format!("sha256:{hash}").into(),
                &format!("file:{}", path.display()).into(),
            )
            .map_err(error)?;
    }
    store
        .put_with_key(
            &serde_json::to_string(&paths)?.into(),
            &format!("manifest:{}", root.display()).into(),
        )
        .map_err(error)?;
    store
        .put_with_key(&JsValue::from_f64(revision as f64), &revision_key)
        .map_err(error)?;
    store
        .put_with_key(&root.to_string_lossy().as_ref().into(), &last_key.into())
        .map_err(error)?;
    transaction.finish().await?;
    crate::storage::acknowledge(dirty);
    Ok(())
}
pub async fn restore() -> Result<Option<(PathBuf, u64)>> {
    let db = Database(open().await?);
    let tx = db.0.transaction_with_str("files").map_err(error)?;
    let store = tx.object_store("files").map_err(error)?;
    let Some(root) = request(store.get(&"last".into()).map_err(error)?)
        .await?
        .as_string()
    else {
        return Ok(None);
    };
    load(Path::new(&root)).await
}

/// Reopen any durable project, without loading other songs into memory.
pub async fn load(root: &Path) -> Result<Option<(PathBuf, u64)>> {
    let root = root.to_string_lossy();
    let db = Database(open().await?);
    let tx = db.0.transaction_with_str("files").map_err(error)?;
    let store = tx.object_store("files").map_err(error)?;
    let manifest = request(
        store
            .get(&format!("manifest:{root}").into())
            .map_err(error)?,
    )
    .await?
    .as_string();
    let Some(manifest) = manifest else {
        return Ok(None);
    };
    let paths: Vec<String> = serde_json::from_str(&manifest)?;
    ensure!(paths.len() <= 10000, "Saved project has too many files");
    let revision = request(
        store
            .get(&format!("revision:{root}").into())
            .map_err(error)?,
    )
    .await?
    .as_f64()
    .context("Saved project revision is missing")? as u64;
    let mut loaded = Vec::with_capacity(paths.len());
    let mut blocks = std::collections::BTreeMap::new();
    for path in paths {
        ensure!(
            Path::new(&path).starts_with(root.as_ref()),
            "Saved project path is invalid"
        );
        let value = request(store.get(&format!("file:{path}").into()).map_err(error)?).await?;
        ensure!(
            !value.is_undefined(),
            "Saved project file is missing: {path}"
        );
        let bytes = read_bytes(&store, value, &mut blocks).await?;
        loaded.push((PathBuf::from(path), bytes));
    }
    // Publish only a complete snapshot. A missing/quota-corrupt file must not
    // partially overwrite the working copy currently offered for ZIP recovery.
    for (path, bytes) in loaded {
        crate::storage::write_shared(path, bytes)?;
    }
    crate::storage::acknowledge(crate::storage::pending(Path::new(root.as_ref()))?);
    Ok(Some((PathBuf::from(root.as_ref()), revision)))
}

async fn read_bytes(
    store: &web_sys::IdbObjectStore,
    value: JsValue,
    blocks: &mut std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
) -> Result<std::sync::Arc<Vec<u8>>> {
    if let Some(reference) = value.as_string() {
        let hash = reference
            .strip_prefix("sha256:")
            .context("Unknown saved file encoding")?;
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid saved content identity"
        );
        if let Some(bytes) = blocks.get(hash) {
            return Ok(bytes.clone());
        }
        let value = request(store.get(&format!("blob:{hash}").into()).map_err(error)?).await?;
        ensure!(
            !value.is_undefined() && !value.is_string(),
            "Saved file content is missing"
        );
        let bytes = std::sync::Arc::new(js_sys::Uint8Array::new(&value).to_vec());
        ensure!(
            format!("{:x}", Sha256::digest(bytes.as_slice())) == hash,
            "Saved file checksum differs; existing working copy kept"
        );
        crate::models::remember_verified_hash(&bytes, hash);
        blocks.insert(hash.into(), bytes.clone());
        Ok(bytes)
    } else {
        Ok(std::sync::Arc::new(
            js_sys::Uint8Array::new(&value).to_vec(),
        ))
    }
}

/// Only the small project documents are read for the chooser, never audio.
pub async fn projects() -> Result<Vec<crate::chart_library::Entry>> {
    let db = Database(open().await?);
    let tx = db.0.transaction_with_str("files").map_err(error)?;
    let store = tx.object_store("files").map_err(error)?;
    let keys = request(store.get_all_keys().map_err(error)?).await?;
    let mut entries = Vec::new();
    let mut blocks = std::collections::BTreeMap::new();
    for key in js_sys::Array::from(&keys)
        .iter()
        .filter_map(|key| key.as_string())
    {
        let Some(root) = key.strip_prefix("manifest:") else {
            continue;
        };
        let value = request(
            store
                .get(&format!("file:{root}/project.json").into())
                .map_err(error)?,
        )
        .await?;
        if value.is_undefined() {
            continue;
        }
        let bytes = read_bytes(&store, value, &mut blocks).await?;
        let doc: crate::editor::Project = serde_json::from_slice(&bytes)?;
        let metadata = request(
            store
                .get(&format!("file:{root}/source/song.ini").into())
                .map_err(error)?,
        )
        .await?;
        let fields = if metadata.is_undefined() {
            std::collections::BTreeMap::new()
        } else {
            crate::pipeline::ini_fields(std::str::from_utf8(
                &read_bytes(&store, metadata, &mut blocks).await?,
            )?)
        };
        let title = fields
            .get("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&doc.title);
        let title = fields
            .get("artist")
            .filter(|s| !s.trim().is_empty())
            .map_or_else(|| title.clone(), |artist| format!("{artist} — {title}"));
        let seconds = doc.duration.round() as u64;
        entries.push(crate::chart_library::Entry {
            path: root.into(),
            title: format!(
                "{title} · {}:{:02} · revision {}",
                seconds / 60,
                seconds % 60,
                doc.revision
            ),
            kind: crate::chart_library::Kind::Project,
        });
    }
    entries.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    Ok(entries)
}

/// Only frozen request metadata is read; choosing a job loads its audio later.
pub async fn jobs() -> Result<Vec<(PathBuf, String)>> {
    let db = Database(open().await?);
    let tx = db.0.transaction_with_str("files").map_err(error)?;
    let store = tx.object_store("files").map_err(error)?;
    let keys = request(store.get_all_keys().map_err(error)?).await?;
    let mut rows = Vec::new();
    let mut blocks = std::collections::BTreeMap::new();
    for key in js_sys::Array::from(&keys)
        .iter()
        .filter_map(|key| key.as_string())
    {
        let Some(root) = key.strip_prefix("manifest:") else {
            continue;
        };
        let marker = request(
            store
                .get(&format!("file:{root}/browser-job-input.json").into())
                .map_err(error)?,
        )
        .await?;
        if marker.is_undefined() {
            continue;
        }
        let value = request(
            store
                .get(&format!("file:{root}/request.json").into())
                .map_err(error)?,
        )
        .await?;
        ensure!(!value.is_undefined(), "Saved generation request is missing");
        let bytes = read_bytes(&store, value, &mut blocks).await?;
        let job: crate::generation::Request = serde_json::from_slice(&bytes)?;
        rows.push((
            PathBuf::from(root),
            format!("{} — {}", job.artist, job.title),
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(rows)
}
