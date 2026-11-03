//! Seek local Blob slices in a worker; transfer one expanded entry at a time.
use anyhow::{Context, Result, ensure};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    rc::Rc,
    sync::Arc,
};
use wasm_bindgen::{JsCast, JsValue};
const CHUNK: u64 = 1024 * 1024;
pub const MAX_ENCODED: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_RESIDENT: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ENTRY: u64 = 1024 * 1024 * 1024;
fn error(value: JsValue) -> anyhow::Error {
    anyhow::anyhow!("Local archive: {value:?}")
}
fn io_error(value: JsValue) -> io::Error {
    io::Error::other(format!("Local archive: {value:?}"))
}
struct Reader {
    blob: web_sys::Blob,
    reader: web_sys::FileReaderSync,
    position: u64,
    start: u64,
    cache: Vec<u8>,
}
impl Reader {
    fn new(blob: web_sys::Blob) -> Result<Self> {
        ensure!(
            blob.size() <= MAX_ENCODED as f64,
            "ZIP exceeds the 16 GiB archive limit"
        );
        Ok(Self {
            blob,
            reader: web_sys::FileReaderSync::new().map_err(error)?,
            position: 0,
            start: 0,
            cache: Vec::new(),
        })
    }
}
impl Read for Reader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.blob.size() as u64 {
            return Ok(0);
        }
        if self.position < self.start || self.position >= self.start + self.cache.len() as u64 {
            self.start = self.position / CHUNK * CHUNK;
            let slice = self
                .blob
                .slice_with_f64_and_f64(
                    self.start as f64,
                    (self.start + CHUNK).min(self.blob.size() as u64) as f64,
                )
                .map_err(io_error)?;
            let buffer = self.reader.read_as_array_buffer(&slice).map_err(io_error)?;
            self.cache = js_sys::Uint8Array::new(&buffer).to_vec();
        }
        let offset = (self.position - self.start) as usize;
        let count = output.len().min(self.cache.len() - offset);
        output[..count].copy_from_slice(&self.cache[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for Reader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(v) => i128::from(v),
            SeekFrom::Current(v) => i128::from(self.position) + i128::from(v),
            SeekFrom::End(v) => self.blob.size() as i128 + i128::from(v),
        };
        if next < 0 || next > self.blob.size() as i128 {
            return Err(io::Error::other("Invalid ZIP seek"));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
fn field(object: &JsValue, name: &str) -> Result<JsValue> {
    js_sys::Reflect::get(object, &name.into()).map_err(error)
}
fn set(object: &JsValue, name: &str, value: &JsValue) -> Result<()> {
    js_sys::Reflect::set(object, &name.into(), value).map_err(error)?;
    Ok(())
}
pub fn process(input: &JsValue, scope: &web_sys::DedicatedWorkerGlobalScope) -> Result<JsValue> {
    let blob = field(input, "archive")?
        .dyn_into::<web_sys::Blob>()
        .map_err(error)?;
    let mut count = 0;
    let mut unique = 0u64;
    let budget = field(input, "budget")?
        .as_f64()
        .context("Missing import memory budget")? as u64;
    crate::archive_codec::decode_into_limited(
        Reader::new(blob)?,
        MAX_ENTRY,
        budget.min(MAX_RESIDENT),
        |name, data| {
            count += 1;
            let message = js_sys::Object::new();
            set(&message, "kind", &"archive-entry".into())?;
            set(&message, "name", &name.into())?;
            let transfers = js_sys::Array::new();
            match data {
                crate::archive_codec::Decoded::Bytes(bytes) => {
                    unique = unique
                        .checked_add(bytes.len() as u64)
                        .context("ZIP size overflow")?;
                    ensure!(
                        unique <= MAX_RESIDENT,
                        "Project exceeds the browser's 2 GiB expanded-data budget. Use a backup without generated stems or open it on desktop."
                    );
                    let buffer = js_sys::Uint8Array::from(bytes.as_slice());
                    transfers.push(&buffer.buffer());
                    set(&message, "bytes", &buffer)?;
                }
                crate::archive_codec::Decoded::Alias(source) => {
                    set(&message, "alias", &source.into())?
                }
            }
            scope
                .post_message_with_transfer(&message, &transfers)
                .map_err(error)?;
            crate::worker::progress(format!(
                "Importing local archive · {count} files · {} MiB",
                unique / (1024 * 1024)
            ));
            Ok(())
        },
    )?;
    let done = js_sys::Object::new();
    set(&done, "kind", &"done".into())?;
    Ok(done.into())
}
pub async fn read(blob: web_sys::Blob) -> Result<crate::files::Files> {
    ensure!(
        blob.size() <= MAX_ENCODED as f64,
        "ZIP exceeds the 16 GiB archive limit"
    );
    let input = js_sys::Object::new();
    set(&input, "kind", &"import-archive".into())?;
    set(&input, "archive", &blob)?;
    let budget = (3 * 1024 * 1024 * 1024u64)
        .saturating_sub(crate::storage::resident_bytes())
        .min(MAX_RESIDENT);
    ensure!(
        budget > 0,
        "The browser is holding too much project data to import another project. Open the backup on desktop or choose one without generated stems."
    );
    set(&input, "budget", &JsValue::from_f64(budget as f64))?;
    let files = Rc::new(RefCell::new(BTreeMap::<String, Arc<Vec<u8>>>::new()));
    let output = files.clone();
    let response = crate::worker::exchange_with_entries(
        &input,
        Rc::new(move |label| {
            if let Some(mut studio) =
                dioxus::prelude::try_consume_context::<crate::studio::state::Studio>()
            {
                use dioxus::prelude::WritableExt;
                studio.status.set(label);
            }
        }),
        Some(Rc::new(move |message| {
            let name = field(&message, "name")?
                .as_string()
                .context("Missing archive path")?;
            let alias = field(&message, "alias")?.as_string();
            let bytes = if let Some(alias) = alias {
                output
                    .borrow()
                    .get(&alias)
                    .context("Missing archive alias")?
                    .clone()
            } else {
                Arc::new(js_sys::Uint8Array::new(&field(&message, "bytes")?).to_vec())
            };
            ensure!(
                output.borrow_mut().insert(name, bytes).is_none(),
                "Duplicate restored archive path"
            );
            Ok(())
        })),
    )
    .await?;
    ensure!(
        field(&response, "kind")?.as_string().as_deref() == Some("done"),
        "{}",
        field(&response, "error")?
            .as_string()
            .unwrap_or_else(|| "Archive import failed".into())
    );
    Ok(std::mem::take(&mut *files.borrow_mut())
        .into_iter()
        .collect())
}
