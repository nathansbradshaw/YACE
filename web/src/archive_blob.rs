//! A seekable ZIP sink backed by bounded browser buffers, not one WASM Vec.
use std::io::{self, Seek, SeekFrom, Write};
use wasm_bindgen::JsValue;

const CHUNK: usize = 4 * 1024 * 1024;
#[derive(Default)]
pub struct Writer {
    chunks: Vec<js_sys::Uint8Array>,
    position: u64,
    length: u64,
    reported: u64,
}
impl Write for Writer {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let length = bytes.len();
        let end = self
            .position
            .checked_add(length as u64)
            .ok_or_else(|| io::Error::other("Archive position overflow"))?;
        if end > 3 * 1024 * 1024 * 1024 {
            return Err(io::Error::other("Archive exceeds 3 GiB"));
        }
        while !bytes.is_empty() {
            let index = self.position as usize / CHUNK;
            let offset = self.position as usize % CHUNK;
            while self.chunks.len() <= index {
                self.chunks
                    .push(js_sys::Uint8Array::new_with_length(CHUNK as u32));
            }
            let take = bytes.len().min(CHUNK - offset);
            self.chunks[index]
                .subarray(offset as u32, (offset + take) as u32)
                .copy_from(&bytes[..take]);
            self.position += take as u64;
            bytes = &bytes[take..];
        }
        self.length = self.length.max(self.position);
        if self.length / (32 * 1024 * 1024) > self.reported {
            self.reported = self.length / (32 * 1024 * 1024);
            crate::worker::progress(format!(
                "Preparing archive download · {} MiB",
                self.length / (1024 * 1024)
            ));
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Writer {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(value) => i128::from(self.length) + i128::from(value),
            SeekFrom::Current(value) => i128::from(self.position) + i128::from(value),
        };
        if !(0..=3 * 1024 * 1024 * 1024).contains(&next) {
            return Err(io::Error::other("Invalid archive seek"));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
impl Writer {
    pub fn finish(self) -> anyhow::Result<web_sys::Blob> {
        let parts = js_sys::Array::new();
        for (index, chunk) in self.chunks.into_iter().enumerate() {
            let take = (self.length as usize - index * CHUNK).min(CHUNK);
            parts.push(&chunk.subarray(0, take as u32));
        }
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("application/zip");
        web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &options).map_err(
            |error: JsValue| anyhow::anyhow!("Could not prepare archive download: {error:?}"),
        )
    }
}
