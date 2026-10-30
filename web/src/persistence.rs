//! IndexedDB transactions preserve the source, checkpoints and revision together.
use anyhow::{Context, Result, ensure};
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
        if let Ok(value) = source.result() {
            if let Ok(db) = value.dyn_into::<IdbDatabase>() {
                if !db.object_store_names().contains("files") {
                    let _ = db.create_object_store("files");
                }
            }
        }
    });
    request_open.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let result = request(request_open.clone().unchecked_into()).await;
    request_open.set_onupgradeneeded(None);
    result?.dyn_into().map_err(|value| error(value))
}
struct TransactionGuard {
    tx: IdbTransaction,
    _callbacks: Vec<Closure<dyn FnMut(web_sys::Event)>>,
    rx: Option<futures_channel::oneshot::Receiver<Result<(), String>>>,
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
        }
    }
    async fn finish(&mut self) -> Result<()> {
        self.rx
            .take()
            .context("Transaction already awaited")?
            .await
            .context("Storage transaction interrupted")?
            .map_err(anyhow::Error::msg)
    }
}
impl Drop for TransactionGuard {
    fn drop(&mut self) {
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
    let files = crate::storage::list(root);
    let dirty = crate::storage::pending(root);
    let paths: Vec<_> = files
        .iter()
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect();
    for (path, bytes) in &dirty {
        store
            .put_with_key(
                &js_sys::Uint8Array::from(bytes.as_slice()),
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
        .put_with_key(&root.to_string_lossy().as_ref().into(), &"last".into())
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
    let manifest = request(
        store
            .get(&format!("manifest:{root}").into())
            .map_err(error)?,
    )
    .await?
    .as_string()
    .context("Saved project manifest is missing")?;
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
    for path in paths {
        ensure!(
            Path::new(&path).starts_with(&root),
            "Saved project path is invalid"
        );
        let value = request(store.get(&format!("file:{path}").into()).map_err(error)?).await?;
        ensure!(
            !value.is_undefined(),
            "Saved project file is missing: {path}"
        );
        crate::storage::write(path, js_sys::Uint8Array::new(&value).to_vec())?;
    }
    Ok(Some((root.into(), revision)))
}
