//! Standard browser file selection and Blob downloads, bound directly from Rust.
use anyhow::{Context, Result, ensure};
use std::{
    cell::RefCell,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::JsFuture;
pub type Files = Vec<(String, Vec<u8>)>;
fn error(value: impl Into<wasm_bindgen::JsValue>) -> anyhow::Error {
    let value = value.into();
    anyhow::anyhow!("Browser: {value:?}")
}
pub async fn pick(folder: bool, accept: &str) -> Result<Option<Files>> {
    let document = web_sys::window()
        .context("No window")?
        .document()
        .context("No document")?;
    let input: web_sys::HtmlInputElement = document
        .create_element("input")
        .map_err(error)?
        .dyn_into()
        .map_err(error)?;
    input.set_type("file");
    input.set_accept(accept);
    input.set_hidden(true);
    if folder {
        input.set_multiple(true);
        input.set_attribute("webkitdirectory", "").map_err(error)?;
    }
    let (tx, rx) = futures_channel::oneshot::channel();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let chosen = tx.clone();
    let input_copy = input.clone();
    let change = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.stop_propagation();
        if let Some(tx) = chosen.borrow_mut().take() {
            let _ = tx.send(input_copy.files());
        }
    });
    let cancel = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.stop_propagation();
        if let Some(tx) = tx.borrow_mut().take() {
            let _ = tx.send(None);
        }
    });
    input
        .add_event_listener_with_callback("change", change.as_ref().unchecked_ref())
        .map_err(error)?;
    input
        .add_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref())
        .map_err(error)?;
    let parent = document
        .query_selector("dialog[open]")
        .map_err(error)?
        .unwrap_or(document.body().context("No body")?.into());
    parent.append_child(&input).map_err(error)?;
    input.click();
    let result = rx.await;
    input.remove();
    input
        .remove_event_listener_with_callback("change", change.as_ref().unchecked_ref())
        .map_err(error)?;
    input
        .remove_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref())
        .map_err(error)?;
    let Some(files) = result.context("File selection interrupted")? else {
        return Ok(None);
    };
    if files.length() == 0 {
        return Ok(None);
    };
    ensure!(files.length() <= 4096, "Choose at most 4096 files");
    let mut result = Vec::new();
    let mut total = 0u64;
    for index in 0..files.length() {
        let file = files.get(index).context("Selected file disappeared")?;
        total += file.size() as u64;
        ensure!(total <= 1024 * 1024 * 1024, "Selected files exceed 1 GiB");
        let relative = js_sys::Reflect::get(&file, &"webkitRelativePath".into())
            .map_err(error)?
            .as_string()
            .unwrap_or_default();
        let name = if relative.is_empty() {
            file.name()
        } else {
            relative
                .split_once('/')
                .map_or(relative.clone(), |(_, rest)| rest.to_owned())
        };
        let buffer = JsFuture::from(file.array_buffer()).await.map_err(error)?;
        result.push((name, js_sys::Uint8Array::new(&buffer).to_vec()));
    }
    Ok(Some(result))
}
pub fn unpack(files: Files) -> Result<Files> {
    if files.len() != 1 || !files[0].0.to_ascii_lowercase().ends_with(".zip") {
        return Ok(files);
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(&files[0].1))?;
    ensure!(
        !zip.is_empty() && zip.len() <= 4096,
        "Invalid archive entry count"
    );
    let mut result = Vec::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        ensure!(!entry.is_symlink(), "Archive symlinks are unsupported");
        let path = entry
            .enclosed_name()
            .context("Archive path leaves the project")?;
        if entry.is_dir() {
            continue;
        }
        total = total
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            total <= 1024 * 1024 * 1024,
            "Expanded archive exceeds 1 GiB"
        );
        let name = path.to_string_lossy().replace('\\', "/");
        if name.starts_with("__MACOSX/") {
            continue;
        }
        let mut bytes = Vec::new();
        entry.take(1024 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 1024 * 1024 * 1024,
            "Archive entry exceeds 1 GiB"
        );
        result.push((name, bytes));
    }
    Ok(result)
}
pub fn mount(root: &Path, files: Files) -> Result<()> {
    for (name, bytes) in files {
        let path = Path::new(&name);
        ensure!(
            !path.is_absolute()
                && path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "Invalid project path"
        );
        let target = root.join(path);
        ensure!(!crate::storage::is_file(&target), "Duplicate project path");
        crate::storage::write(target, bytes)?;
    }
    Ok(())
}
pub fn archive(root: &Path) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in crate::storage::list(root) {
        let name = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let text = path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| matches!(v, "json" | "chart" | "ini" | "txt"));
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default().compression_method(if text {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            }),
        )?;
        zip.write_all(&bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}
pub fn download(name: &str, bytes: &[u8]) -> Result<()> {
    let chunks = js_sys::Array::new();
    chunks.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&chunks, &options).map_err(error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(error)?;
    let document = web_sys::window()
        .context("No window")?
        .document()
        .context("No document")?;
    let link: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(error)?
        .dyn_into()
        .map_err(error)?;
    link.set_href(&url);
    link.set_download(name);
    let parent = document
        .query_selector("dialog[open]")
        .map_err(error)?
        .unwrap_or(document.body().context("No body")?.into());
    parent.append_child(&link).map_err(error)?;
    link.click();
    link.remove();
    wasm_bindgen_futures::spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(30000).await;
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    Ok(())
}
pub fn unique_root() -> PathBuf {
    PathBuf::from(format!(
        "/projects/{:016x}-{:08x}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * u32::MAX as f64) as u32
    ))
}
pub fn imported_root(root: &Path) -> PathBuf {
    let mut root = root.to_owned();
    for _ in 0..32 {
        if ["project.json", "notes.chart", "notes.mid"]
            .iter()
            .any(|name| crate::storage::is_file(root.join(name)))
        {
            break;
        }
        let files = crate::storage::list(&root);
        let first = files
            .first()
            .and_then(|(path, _)| path.strip_prefix(&root).ok())
            .and_then(|path| path.components().next())
            .map(|v| v.as_os_str().to_owned());
        let Some(first) = first else { break };
        let next = root.join(first);
        if files.iter().all(|(path, _)| path.starts_with(&next)) {
            root = next
        } else {
            break;
        }
    }
    root
}
