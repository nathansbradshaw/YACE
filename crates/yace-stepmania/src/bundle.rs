//! A portable backup is a versioned document plus immutable source assets.
use crate::{Document, Format, Profile, Project};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};
pub const MANIFEST: &str = "stepmania-project.json";
pub type Files = BTreeMap<String, Vec<u8>>;
/// Choose the shallowest song/document in an archive. A band project owns its
/// nested dance sidecar; a wrapped dance backup owns its immutable source files.
pub fn entry(files: &Files) -> Option<String> {
    let depth = |p: &str| p.matches('/').count();
    let band = files
        .keys()
        .filter(|p| {
            Path::new(p)
                .file_name()
                .is_some_and(|n| n == "project.json" || n == "notes.chart" || n == "notes.mid")
        })
        .map(|p| depth(p))
        .min();
    let chosen = files
        .keys()
        .filter_map(|p| {
            let manifest = Path::new(p).file_name().is_some_and(|n| n == MANIFEST);
            let extension = Path::new(p)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            (manifest
                || extension.eq_ignore_ascii_case("ssc")
                || extension.eq_ignore_ascii_case("sm"))
            .then_some((
                depth(p),
                if manifest {
                    0
                } else if extension.eq_ignore_ascii_case("ssc") {
                    1
                } else {
                    2
                },
                p,
            ))
        })
        .min()?;
    if band.is_some_and(|d| d <= chosen.0) {
        None
    } else {
        Some(chosen.2.clone())
    }
}
pub fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', '\0'])
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub fn candidates(files: &Files) -> Vec<String> {
    files
        .keys()
        .filter(|p| {
            Path::new(p)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("sm") || e.eq_ignore_ascii_case("ssc"))
        })
        .cloned()
        .collect()
}
pub fn validate_files(files: &Files) -> Result<()> {
    ensure!(files.len() <= 4096, "Too many source assets");
    let mut total = 0usize;
    for (name, bytes) in files {
        ensure!(safe_path(name), "Unsafe asset path: {name}");
        total = total
            .checked_add(bytes.len())
            .context("Asset size overflow")?;
    }
    ensure!(total <= 1024 * 1024 * 1024, "Source assets exceed 1 GiB");
    Ok(())
}
pub fn import(files: Files, selected: &str, profile: Profile) -> Result<(Project, Files)> {
    validate_files(&files)?;
    ensure!(safe_path(selected), "Unsafe simfile path");
    let path = Path::new(selected);
    let root = path.parent().unwrap_or(Path::new(""));
    let bytes = files.get(selected).context("Selected simfile is missing")?;
    let format = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ssc"))
    {
        Format::Ssc
    } else {
        Format::Sm
    };
    let mut project = Project::import(String::from_utf8(bytes.clone())?, format, profile)?;
    let mut source = Files::new();
    for (name, bytes) in files {
        if let Ok(relative) = Path::new(&name).strip_prefix(root) {
            source.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
    check_music(&project, &source)?;
    bind(&mut project, &source)?;
    Ok((project, source))
}
pub fn check_music<'a>(
    project: &Project,
    source: &'a Files,
) -> Result<Option<(&'a str, &'a [u8])>> {
    let name = project
        .content
        .document
        .metadata
        .get("MUSIC")
        .map(String::as_str)
        .unwrap_or("");
    if name.is_empty() {
        return Ok(None);
    }
    ensure!(safe_path(name), "MUSIC path leaves the song folder");
    let (key, bytes) = source
        .get_key_value(name)
        .context("The simfile's recording is missing; import its song folder or ZIP")?;
    Ok(Some((key, bytes)))
}
pub fn backup(project: &Project, source: &Files) -> Result<Files> {
    validate_files(source)?;
    let mut project = project.clone();
    if project.assets_sha256.is_empty() {
        bind(&mut project, source)?;
    }
    verify(&project, source)?;
    let mut out = Files::from([(MANIFEST.into(), project.encode()?)]);
    for (name, bytes) in source {
        out.insert(format!("source/{name}"), bytes.clone());
    }
    Ok(out)
}
pub fn restore(files: Files) -> Result<(Project, Files)> {
    validate_files(&files)?;
    let manifest = files
        .keys()
        .find(|p| Path::new(p).file_name().is_some_and(|n| n == MANIFEST))
        .context("No StepMania project manifest")?
        .clone();
    let root = Path::new(&manifest).parent().unwrap_or(Path::new(""));
    let project = Project::decode(&files[&manifest])?;
    let source = files
        .iter()
        .filter_map(|(name, bytes)| {
            Path::new(name)
                .strip_prefix(root.join("source"))
                .ok()
                .map(|p| (p.to_string_lossy().into_owned(), bytes.clone()))
        })
        .collect();
    verify(&project, &source)?;
    check_music(&project, &source)?;
    Ok((project, source))
}
pub fn export(project: &Project, source: &Files, format: Format) -> Result<Files> {
    validate_files(source)?;
    verify(project, source)?;
    check_music(project, source)?;
    let mut out = Files::new();
    for profile in [Profile::Pad, Profile::Keyboard] {
        let mut document = project.content.document.clone();
        document
            .charts
            .retain(|chart| chart.profile.unwrap_or(project.content.profile) == profile);
        if document.charts.is_empty() && profile != project.content.profile {
            continue;
        }
        if document.charts.is_empty() && !project.content.document.charts.is_empty() {
            continue;
        }
        let text = document.export(format)?;
        Document::parse(text.clone(), format)?;
        let prefix = format!("YACE-{}/", profile.name());
        for (name, bytes) in source {
            // An old SSC next to a new SM can take precedence in the engine.
            // Originals belong in the recovery subtree, never beside the export.
            let simfile = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("sm") || e.eq_ignore_ascii_case("ssc"));
            if !simfile {
                out.insert(format!("{prefix}{name}"), bytes.clone());
            }
        }
        out.insert(
            format!("{prefix}notes.{}", format.extension()),
            text.into_bytes(),
        );
        out.insert(
            format!("{prefix}yace-history/{MANIFEST}"),
            project.encode()?,
        );
        for (name, bytes) in source {
            out.insert(format!("{prefix}yace-history/source/{name}"), bytes.clone());
        }
    }
    Ok(out)
}
pub fn bind(project: &mut Project, source: &Files) -> Result<()> {
    validate_files(source)?;
    ensure!(
        project.assets_sha256.is_empty(),
        "Source assets are already bound"
    );
    project.assets_sha256 = source
        .iter()
        .map(|(name, bytes)| (name.clone(), format!("{:x}", Sha256::digest(bytes))))
        .collect();
    Ok(())
}
pub fn verify(project: &Project, source: &Files) -> Result<()> {
    for (name, hash) in &project.assets_sha256 {
        let bytes = source
            .get(name)
            .with_context(|| format!("Missing source asset {name}"))?;
        ensure!(
            &format!("{:x}", Sha256::digest(bytes)) == hash,
            "Source asset changed: {name}"
        );
    }
    Ok(())
}
