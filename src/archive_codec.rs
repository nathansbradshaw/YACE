//! One bounded ZIP contract for both clients. Extraction never trusts ZIP paths.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read, Seek, Write},
    path::{Component, Path},
};
pub type Files = Vec<(String, Vec<u8>)>;
pub type SharedFiles = Vec<(String, std::sync::Arc<Vec<u8>>)>;
/// Convert native path separators, never the bytes of an authored filename.
/// ZIP entry names must be representable without lossy Unicode conversion or
/// interpreting a literal backslash as a different directory structure.
pub fn relative_name(path: &Path) -> Result<String> {
    let mut names = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            anyhow::bail!("Archive paths must stay inside the project");
        };
        let name = name.to_str().with_context(|| {
            format!("Archive filename is not valid UTF-8 at {path:?}; source files retained")
        })?;
        ensure!(
            !name.contains('\\'),
            "Archive filename contains a literal backslash at {path:?}; source files retained"
        );
        names.push(name);
    }
    ensure!(!names.is_empty(), "Archive path is empty");
    Ok(names.join("/"))
}
#[cfg(test)]
#[path = "archive_codec/path_tests.rs"]
mod path_tests;
/// Retain repeated immutable media once when a game ZIP also carries its
/// editable recovery copy. Limits apply to both logical and unique bytes.
pub fn decode_shared(bytes: &[u8]) -> Result<SharedFiles> {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, sync::Arc};
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(
        !archive.is_empty() && archive.len() <= 4096,
        "Invalid archive entry count"
    );
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut blocks = BTreeMap::<String, Arc<Vec<u8>>>::new();
    let (mut total, mut unique) = (0u64, 0u64);
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        ensure!(!entry.is_symlink(), "Archive symlinks are unsupported");
        let path = entry
            .enclosed_name()
            .context("Archive path leaves the project")?;
        if entry.is_dir() {
            continue;
        }
        let name = path.to_string_lossy().replace('\\', "/");
        ensure!(
            Path::new(&name)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "Invalid archive path"
        );
        ensure!(names.insert(name.clone()), "Duplicate archive path");
        total = total
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            total <= 3 * 1024 * 1024 * 1024 && entry.size() <= 1024 * 1024 * 1024,
            "Expanded archive exceeds browser limits"
        );
        if name.starts_with("__MACOSX/") {
            continue;
        }
        let size = entry.size();
        let mut bytes = Vec::with_capacity(size as usize);
        let mut limited = entry.take(size);
        limited.read_to_end(&mut bytes)?;
        let mut entry = limited.into_inner();
        let mut extra = [0u8; 1];
        ensure!(
            bytes.len() as u64 == size && entry.read(&mut extra)? == 0,
            "Archive entry size differs from its directory"
        );
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let bytes = if let Some(existing) = blocks.get(&hash) {
            existing.clone()
        } else {
            unique = unique
                .checked_add(bytes.len() as u64)
                .context("Archive size overflow")?;
            ensure!(
                unique <= 1024 * 1024 * 1024,
                "Unique archive data exceeds 1 GiB"
            );
            let bytes = Arc::new(bytes);
            blocks.insert(hash, bytes.clone());
            bytes
        };
        files.push((name, bytes));
    }
    ensure!(!files.is_empty(), "Archive contains no files");
    Ok(files)
}
pub fn decode(bytes: &[u8]) -> Result<Files> {
    Ok(decode_shared(bytes)?
        .into_iter()
        .map(|(name, bytes)| (name, std::sync::Arc::unwrap_or_clone(bytes)))
        .collect())
}
pub fn encode<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<Vec<u8>> {
    Ok(encode_into(files, Cursor::new(Vec::new()))?.into_inner())
}
/// Browser writers can keep archive chunks outside the bounded WASM heap.
pub fn encode_into<'a, W: Write + Seek>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    writer: W,
) -> Result<W> {
    let mut zip = zip::ZipWriter::new(writer);
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for (name, bytes) in files {
        ensure!(
            names.len() < 4096 && names.insert(name),
            "Invalid or duplicate archive entry"
        );
        ensure!(
            !name.contains('\\')
                && Path::new(name)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "Invalid archive path"
        );
        total = total
            .checked_add(bytes.len() as u64)
            .context("Archive size overflow")?;
        ensure!(total <= 3 * 1024 * 1024 * 1024, "Archive exceeds 3 GiB");
        let extension = Path::new(name).extension().and_then(|s| s.to_str());
        let text = extension.is_some_and(|s| matches!(s, "json" | "chart" | "ini" | "txt"));
        let wave = extension.is_some_and(|s| s.eq_ignore_ascii_case("wav"));
        let mut options =
            zip::write::SimpleFileOptions::default().compression_method(if text || wave {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            });
        if wave {
            options = options.compression_level(Some(3));
        }
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
    }
    ensure!(!names.is_empty(), "No files to export");
    Ok(zip.finish()?)
}
pub fn strip_wrapper(mut files: Files, marker: &str) -> Result<Files> {
    for _ in 0..32 {
        if files.iter().any(|(name, _)| name == marker) {
            return Ok(files);
        }
        let first = files
            .first()
            .and_then(|(name, _)| name.split_once('/').map(|(folder, _)| folder.to_owned()))
            .context("Archive does not contain the expected project")?;
        let prefix = format!("{first}/");
        ensure!(
            files.iter().all(|(name, _)| name.starts_with(&prefix)),
            "Archive does not contain one project folder"
        );
        for (name, _) in &mut files {
            *name = name.strip_prefix(&prefix).unwrap().to_owned();
        }
    }
    anyhow::bail!("Archive folders are nested too deeply")
}
