//! Native project ZIPs preserve source bytes and publish only complete copies.
use crate::{
    archive_codec,
    editor::Store,
    models,
    project_portability::{self, Files},
};
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path};
fn collect(root: &Path, folder: &Path, files: &mut Files) -> Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "Project backups cannot include symbolic links"
        );
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() && !entry.file_name().to_string_lossy().ends_with(".lock") {
            let name = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(name, fs::read(path)?);
        }
    }
    Ok(())
}
pub fn backup(store: &Store, output: &Path) -> Result<()> {
    let before = models::hash_file(&store.root.join("project.json"))?;
    let doc = store.load()?;
    let mut files = Files::new();
    collect(&store.root, &store.root, &mut files)?;
    project_portability::validate_sources(&files, &doc)?;
    use sha2::{Digest, Sha256};
    let mut present: std::collections::BTreeSet<_> = files
        .values()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .collect();
    for (path, hash) in project_portability::referenced_stems(&files)? {
        if path.is_file() {
            ensure!(
                models::hash_file(&path)?.eq_ignore_ascii_case(&hash),
                "Attached stem checksum differs during backup"
            );
            if !present.insert(hash.to_ascii_lowercase()) {
                continue;
            }
            let ext = path.extension().unwrap_or_default().to_string_lossy();
            files
                .entry(format!("assets/stems/{hash}.{ext}"))
                .or_insert(fs::read(path)?);
        }
    }
    ensure!(
        models::hash_file(&store.root.join("project.json"))? == before,
        "Project changed during backup; try again"
    );
    let bytes = archive_codec::encode(
        files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice())),
    )?;
    let parent = output.parent().context("Choose a backup location")?;
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    stage.write_all(&bytes)?;
    stage.as_file().sync_all()?;
    stage
        .persist_noclobber(output)
        .map_err(|e| e.error)
        .context("Choose a new backup filename; existing files are preserved")?;
    Ok(())
}
pub fn import(path: &Path, output: &Path) -> Result<Store> {
    crate::pipeline::destination(output, None)?;
    let parent = output
        .parent()
        .context("Choose a project location")?
        .canonicalize()?;
    let output = parent.join(output.file_name().context("Invalid project name")?);
    let files =
        archive_codec::strip_wrapper(archive_codec::decode(&fs::read(path)?)?, "project.json")?;
    let mut files: Files = files.into_iter().collect();
    project_portability::relocate(&mut files, &output)?;
    let stage = tempfile::tempdir_in(parent)?;
    for (name, bytes) in files {
        let path = stage.path().join(name);
        fs::create_dir_all(path.parent().context("Invalid project path")?)?;
        crate::storage::write_new(path, &bytes)?;
    }
    Store::new(stage.path()).load()?;
    crate::pipeline::publish(stage.path(), &output)?;
    Ok(Store::new(&output))
}
pub fn import_song(path: &Path, output: &Path) -> Result<Store> {
    let source = unpack_song(path)?;
    Store::create(source.path(), None, output)
}
/// Keep the staged ZIP alive while the UI asks which chart source to edit.
pub fn unpack_song(path: &Path) -> Result<tempfile::TempDir> {
    let files = archive_codec::decode(&fs::read(path)?)?;
    let marker = if files
        .iter()
        .any(|(name, _)| name == "notes.chart" || name.ends_with("/notes.chart"))
    {
        "notes.chart"
    } else {
        "notes.mid"
    };
    let files = archive_codec::strip_wrapper(files, marker)?;
    let source = tempfile::tempdir()?;
    for (name, bytes) in files {
        let path = source.path().join(name);
        fs::create_dir_all(path.parent().context("Invalid song path")?)?;
        crate::storage::write_new(path, &bytes)?;
    }
    Ok(source)
}
