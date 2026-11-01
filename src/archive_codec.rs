//! One bounded ZIP contract for both clients. Extraction never trusts ZIP paths.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read, Write},
    path::{Component, Path},
};
pub type Files = Vec<(String, Vec<u8>)>;
pub fn decode(bytes: &[u8]) -> Result<Files> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(
        !archive.is_empty() && archive.len() <= 4096,
        "Invalid archive entry count"
    );
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut names = BTreeSet::new();
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
            total <= 1024 * 1024 * 1024,
            "Expanded archive exceeds 1 GiB"
        );
        if name.starts_with("__MACOSX/") {
            continue;
        }
        let mut bytes = Vec::new();
        entry.take(1024 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 1024 * 1024 * 1024,
            "Archive entry exceeds 1 GiB"
        );
        files.push((name, bytes));
    }
    ensure!(!files.is_empty(), "Archive contains no files");
    Ok(files)
}
pub fn encode<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
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
        ensure!(total <= 1024 * 1024 * 1024, "Archive exceeds 1 GiB");
        let text = Path::new(name)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| matches!(s, "json" | "chart" | "ini" | "txt"));
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default().compression_method(if text {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            }),
        )?;
        zip.write_all(bytes)?;
    }
    ensure!(!names.is_empty(), "No files to export");
    Ok(zip.finish()?.into_inner())
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
