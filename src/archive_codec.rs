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
    decode_reader(Cursor::new(bytes))
}
pub enum Decoded {
    Bytes(Vec<u8>),
    Alias(String),
}
/// Read just one entry at a time. A sink can transfer each allocation out of a
/// worker before the next entry is inflated; no complete encoded ZIP is needed.
pub fn decode_into<R: Read + Seek>(
    reader: R,
    mut sink: impl FnMut(String, Decoded) -> Result<()>,
) -> Result<()> {
    decode_into_limited(
        reader,
        crate::compact_backup::MAX_FILE,
        crate::compact_backup::MAX_TOTAL,
        &mut sink,
    )
}
pub fn decode_into_limited<R: Read + Seek>(
    reader: R,
    max_entry: u64,
    max_unique: u64,
    mut sink: impl FnMut(String, Decoded) -> Result<()>,
) -> Result<()> {
    use crate::compact_backup as backup;
    use std::collections::BTreeMap;
    let mut archive = zip::ZipArchive::new(reader)?;
    ensure!(
        !archive.is_empty() && archive.len() <= backup::MAX_FILES + 1,
        "Invalid archive entry count"
    );
    let manifest = backup::manifest(&mut archive)?;
    let mut pending = BTreeMap::<String, Vec<&backup::Entry>>::new();
    if let Some(manifest) = &manifest {
        for entry in &manifest.files {
            pending.entry(entry.blob.clone()).or_default().push(entry);
        }
    }
    let mut names = BTreeSet::new();
    let mut blocks = BTreeMap::<String, String>::new();
    let (mut total, mut unique) = (0u64, 0u64);
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        ensure!(!entry.is_symlink(), "Archive symlinks are unsupported");
        ensure!(
            !entry.name()?.contains('\\'),
            "ZIP filename contains a backslash; source archive retained"
        );
        let path = entry
            .enclosed_name()
            .context("Archive path leaves the project")?;
        if entry.is_dir() {
            continue;
        }
        let name = relative_name(&path)?;
        ensure!(names.insert(name.clone()), "Duplicate archive path");
        total = total
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            total <= backup::MAX_TOTAL && entry.size() <= max_entry.min(backup::MAX_FILE),
            "Expanded archive exceeds supported limits"
        );
        if name.starts_with("__MACOSX/") {
            continue;
        }
        if manifest.is_some() && name == backup::MANIFEST {
            continue;
        }
        let descriptions = if manifest.is_some() {
            Some(pending.remove(&name).context("Unexpected backup asset")?)
        } else {
            None
        };
        let size = entry.size();
        if let Some(descriptions) = &descriptions {
            ensure!(
                descriptions[0].bytes <= max_entry
                    && descriptions[0]
                        .packed_bytes
                        .unwrap_or(descriptions[0].bytes)
                        == size,
                "Backup asset length exceeds limits or differs"
            );
        }
        let mut bytes = Vec::with_capacity(size as usize);
        let mut limited = entry.take(size);
        limited.read_to_end(&mut bytes)?;
        let mut entry = limited.into_inner();
        let mut extra = [0u8; 1];
        ensure!(
            bytes.len() as u64 == size && entry.read(&mut extra)? == 0,
            "Archive entry size differs from its directory"
        );
        if let Some(descriptions) = &descriptions {
            bytes = backup::restore_entry(bytes, descriptions[0])?;
        }
        let hash = backup::digest(&bytes);
        let destination = descriptions
            .as_ref()
            .map_or(name.as_str(), |entries| entries[0].path.as_str())
            .to_owned();
        if let Some(descriptions) = &descriptions {
            ensure!(
                descriptions[0]
                    .restored_sha256
                    .as_ref()
                    .unwrap_or(&descriptions[0].sha256)
                    == &hash,
                "Backup asset checksum differs"
            );
        }
        if let Some(existing) = blocks.get(&hash) {
            sink(destination.clone(), Decoded::Alias(existing.clone()))?;
        } else {
            unique = unique
                .checked_add(bytes.len() as u64)
                .context("Archive size overflow")?;
            ensure!(
                unique <= max_unique.min(backup::MAX_TOTAL),
                "Unique archive data exceeds supported limits"
            );
            blocks.insert(hash, destination.clone());
            sink(destination.clone(), Decoded::Bytes(bytes))?;
        }
        if let Some(descriptions) = descriptions {
            for entry in descriptions.into_iter().skip(1) {
                sink(entry.path.clone(), Decoded::Alias(destination.clone()))?;
            }
        }
    }
    ensure!(
        !blocks.is_empty() && pending.is_empty(),
        "Archive has no files or is missing backup assets"
    );
    if let Some(manifest) = manifest
        && !manifest.transcodes.is_empty()
    {
        sink(
            backup::TRANSCODES.into(),
            Decoded::Bytes(serde_json::to_vec(&manifest.transcodes)?),
        )?;
    }
    Ok(())
}
pub fn decode_reader<R: Read + Seek>(reader: R) -> Result<SharedFiles> {
    use std::{collections::BTreeMap, sync::Arc};
    let mut files = BTreeMap::<String, Arc<Vec<u8>>>::new();
    let mut ordered = Vec::new();
    decode_into(reader, |name, data| {
        let bytes = match data {
            Decoded::Bytes(bytes) => Arc::new(bytes),
            Decoded::Alias(source) => files.get(&source).context("Missing archive alias")?.clone(),
        };
        ensure!(
            files.insert(name.clone(), bytes.clone()).is_none(),
            "Duplicate restored archive path"
        );
        ordered.push((name, bytes));
        Ok(())
    })?;
    Ok(ordered)
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
pub fn strip_wrapper(files: Files, marker: &str) -> Result<Files> {
    strip_wrapper_data(files, marker)
}
pub fn strip_wrapper_shared(files: SharedFiles, marker: &str) -> Result<SharedFiles> {
    strip_wrapper_data(files, marker)
}
fn strip_wrapper_data<B>(mut files: Vec<(String, B)>, marker: &str) -> Result<Vec<(String, B)>> {
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
