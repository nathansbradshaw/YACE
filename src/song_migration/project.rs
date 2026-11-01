//! Relocate editing state without regenerating its notes or discarding undo.
use crate::{
    editor::Project,
    models,
    stem_review::{StemId, StemManifest},
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn ordinary_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut prefix = PathBuf::new();
    for component in absolute.components() {
        prefix.push(component.as_os_str());
        ensure!(
            !fs::symlink_metadata(&prefix)?.file_type().is_symlink(),
            "Migration refuses symlink {}",
            prefix.display()
        );
    }
    Ok(absolute.canonicalize()?)
}

fn ordinary_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "Migration refuses symlink {}",
            entry.path().display()
        );
        if kind.is_dir() {
            ordinary_tree(&entry.path())?;
        } else {
            ensure!(
                kind.is_file(),
                "Unsupported migration entry {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

struct Retainer {
    old_project: PathBuf,
    audio: PathBuf,
    audio_hash: String,
    duration: f64,
    retained: BTreeMap<(StemId, PathBuf, String), PathBuf>,
}

impl Retainer {
    fn content(&mut self, content: &mut Value) -> Result<()> {
        for field in ["stems", "keyboard_stems"] {
            let Some(value) = content.get_mut(field).filter(|value| !value.is_null()) else {
                continue;
            };
            let before = value.clone();
            let mut manifest: StemManifest = serde_json::from_value(before.clone())?;
            for asset in &mut manifest.stems {
                let path = if asset.path.is_absolute() {
                    asset.path.clone()
                } else {
                    self.old_project.join(&asset.path)
                };
                asset.path =
                    ordinary_path(&path).context("A referenced stem cannot be relocated")?;
            }
            manifest.validate_binding(&self.audio_hash, self.duration)?;
            for asset in &mut manifest.stems {
                let key = (asset.id, asset.path.clone(), asset.sha256.clone());
                if let Some(relative) = self.retained.get(&key) {
                    asset.path = relative.clone();
                    continue;
                }
                ensure!(
                    models::hash_file(&asset.path)? == asset.sha256,
                    "{} stem changed before migration",
                    asset.id.as_str()
                );
                let extension = asset
                    .path
                    .extension()
                    .and_then(|value| value.to_str())
                    .context("Referenced stem needs an extension")?;
                ensure!(
                    !extension.is_empty()
                        && extension.len() <= 10
                        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()),
                    "Unsupported stem filename"
                );
                let folder = self.audio.join("stems").join(&asset.sha256);
                fs::create_dir_all(&folder)?;
                let name = format!("{}.{}", asset.id.as_str(), extension.to_ascii_lowercase());
                let target = folder.join(&name);
                if !target.exists() {
                    let mut stage = tempfile::NamedTempFile::new_in(&folder)?;
                    std::io::copy(&mut fs::File::open(&asset.path)?, stage.as_file_mut())?;
                    stage.as_file().sync_all()?;
                    ensure!(
                        models::hash_file(stage.path())? == asset.sha256,
                        "Stem changed while copying migration assets"
                    );
                    stage
                        .persist_noclobber(&target)
                        .map_err(|error| error.error)?;
                }
                ensure!(
                    fs::symlink_metadata(&target)?.is_file()
                        && models::hash_file(&target)? == asset.sha256,
                    "Retained stem checksum mismatch"
                );
                let relative = Path::new("../audio/stems").join(&asset.sha256).join(name);
                self.retained.insert(key, relative.clone());
                asset.path = relative;
            }
            let updated = serde_json::to_value(manifest)?;
            if updated != before {
                *value = updated;
            }
        }
        Ok(())
    }
}

/// `project` is a complete staged editing copy at `new_song/.yace/project`.
/// The caller owns backups, locks, final publication, and visible song files.
pub(super) fn prepare(
    project: &Path,
    old_project: &Path,
    old_song: &Path,
    new_song: &Path,
) -> Result<()> {
    prepare_with_audio_recovery(project, old_project, old_song, new_song, None)
}

/// An optional source is accepted only for repairing a mismatched audio copy
/// whose exact original bytes are still independently hash-bound by the project.
pub(super) fn prepare_with_audio_recovery(
    project: &Path,
    old_project: &Path,
    old_song: &Path,
    new_song: &Path,
    recovery: Option<&Path>,
) -> Result<()> {
    let project = ordinary_path(project)?;
    let old_project = ordinary_path(old_project)?;
    let old_song = ordinary_path(old_song)?;
    let new_song = ordinary_path(new_song)?;
    ensure!(
        project == new_song.join(".yace/project"),
        "Unexpected staged project location"
    );
    ensure!(
        project != old_project && !project.starts_with(&old_project),
        "Migration must prepare a separate project copy"
    );
    ordinary_tree(&project)?;
    ordinary_tree(&old_project)?;
    let audio = new_song.join(".yace/audio");
    if audio.exists() {
        ordinary_path(&audio)?;
        ordinary_tree(&audio)?;
    }
    let original_bytes = fs::read(old_project.join("project.json"))?;
    let original: Project = serde_json::from_slice(&original_bytes)?;
    ensure!(original.version == 1, "Unsupported editing project version");
    ensure!(
        original.kit_audition.is_none(),
        "Turn off the active drum-kit audition before migrating this project"
    );
    ensure!(
        fs::read(project.join("project.json"))? == original_bytes,
        "Editing state changed while staging migration"
    );
    ensure!(
        matches!(
            Path::new(&original.audio_filename)
                .components()
                .collect::<Vec<_>>()
                .as_slice(),
            [Component::Normal(_)]
        ),
        "Invalid project recording filename"
    );
    let chart = Path::new("source/notes.chart");
    ensure!(
        models::hash_file(&old_project.join(chart))? == original.source_chart_sha256
            && models::hash_file(&project.join(chart))? == original.source_chart_sha256,
        "Immutable project source chart changed during migration"
    );
    let audio_file = Path::new("source").join(&original.audio_filename);
    let old_audio = old_project.join(&audio_file);
    let staged_audio = project.join(&audio_file);
    let old_audio_hash = models::hash_file(&old_audio)?;
    ensure!(
        models::hash_file(&staged_audio)? == old_audio_hash,
        "Staged source audio no longer matches its original project copy"
    );
    if old_audio_hash != original.audio_sha256 {
        let recovery = recovery.context(
            "Immutable project source audio changed; an exact verified recovery is required",
        )?;
        let recovery = ordinary_path(recovery)?;
        ensure!(
            fs::symlink_metadata(&recovery)?.is_file()
                && models::hash_file(&recovery)? == original.audio_sha256,
            "Recovery audio does not match the project's saved source checksum"
        );
        let mut replacement = tempfile::NamedTempFile::new_in(
            staged_audio
                .parent()
                .context("Source audio needs a parent")?,
        )?;
        std::io::copy(&mut fs::File::open(&recovery)?, replacement.as_file_mut())?;
        replacement.as_file().sync_all()?;
        ensure!(
            models::hash_file(replacement.path())? == original.audio_sha256,
            "Recovery audio changed during copying"
        );
        ensure!(
            models::hash_file(&old_audio)? == old_audio_hash
                && models::hash_file(&staged_audio)? == old_audio_hash,
            "Project audio changed while preparing its verified recovery"
        );
        replacement
            .persist(&staged_audio)
            .map_err(|error| error.error)?;
    }
    ensure!(
        models::hash_file(&staged_audio)? == original.audio_sha256,
        "Immutable project source audio changed during migration"
    );

    // Validate every retained checkpoint before changing staged references.
    let checkpoints = project.join("checkpoints");
    let mut saved = Vec::new();
    for entry in fs::read_dir(&checkpoints)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        ensure!(entry.file_type()?.is_file(), "Invalid checkpoint entry");
        let id = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .context("Invalid checkpoint filename")?
            .to_owned();
        ensure!(
            id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "Invalid checkpoint identity"
        );
        let bytes = fs::read(entry.path())?;
        ensure!(
            hash(&bytes) == id
                && fs::read(old_project.join("checkpoints").join(entry.file_name()))? == bytes,
            "Checkpoint checksum mismatch during migration"
        );
        let value: Value = serde_json::from_slice(&bytes)?;
        let _: crate::editor::Content = serde_json::from_value(value.clone())?;
        saved.push((id, bytes, value));
    }
    let known: BTreeSet<_> = saved.iter().map(|(id, _, _)| id.clone()).collect();
    ensure!(
        original
            .undo
            .iter()
            .chain(&original.redo)
            .all(|id| known.contains(id)),
        "Editing project is missing referenced undo checkpoints"
    );

    let mut retained = Retainer {
        old_project: old_project.clone(),
        audio,
        audio_hash: original.audio_sha256.clone(),
        duration: original.duration,
        retained: BTreeMap::new(),
    };
    let mut document: Value = serde_json::from_slice(&original_bytes)?;
    retained.content(
        document
            .get_mut("content")
            .context("Missing editing content")?,
    )?;
    document["original_folder"] = serde_json::to_value(&old_song)?;
    let mut identities = BTreeMap::new();
    for (old_id, old_bytes, mut content) in saved {
        let before = content.clone();
        retained.content(&mut content)?;
        let bytes = if content == before {
            old_bytes
        } else {
            serde_json::to_vec(&content)?
        };
        let id = hash(&bytes);
        let target = checkpoints.join(format!("{id}.json"));
        if target.exists() {
            ensure!(
                fs::read(&target)? == bytes,
                "Checkpoint migration collision"
            );
        } else {
            fs::write(&target, bytes)?;
        }
        identities.insert(old_id, id);
    }
    for field in ["undo", "redo"] {
        for value in document
            .get_mut(field)
            .and_then(Value::as_array_mut)
            .context("Invalid undo history")?
        {
            let old = value.as_str().context("Invalid checkpoint reference")?;
            *value = Value::String(
                identities
                    .get(old)
                    .context("Missing migrated checkpoint")?
                    .clone(),
            );
        }
    }
    // Original checkpoint bytes remain in the caller's backup, never under a
    // checksum filename that no longer describes their relocated content.
    let current: BTreeSet<_> = identities.values().cloned().collect();
    for old in identities.keys().filter(|old| !current.contains(*old)) {
        fs::remove_file(checkpoints.join(format!("{old}.json")))?;
    }
    ensure!(
        fs::read(old_project.join("project.json"))? == original_bytes,
        "Editing state changed during migration; staged copy was not published"
    );
    fs::write(
        project.join("project.json"),
        serde_json::to_vec_pretty(&document)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Action, Actor, ActorKind, Request, Store};
    use crate::stem_review::{AlignmentEvidence, StemAsset};
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()));
            } else {
                fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }
    fn setup(root: &Path) -> (PathBuf, PathBuf, PathBuf, String) {
        let old = root.join("old-project");
        fs::create_dir(&old).unwrap();
        let store = fixture::fixture(&old);
        let mut doc = store.load().unwrap();
        let stem = root.join("external-drum.wav");
        fs::write(
            &stem,
            b"Known stem bytes; this test verifies relocation, not decoding",
        )
        .unwrap();
        doc.content.stems = Some(StemManifest {
            version: 1,
            source_audio_sha256: doc.audio_sha256.clone(),
            sample_rate: 44100,
            channels: 2,
            frames: 4 * 44100,
            alignment: AlignmentEvidence {
                method: "migration fixture".into(),
                details: "Unchanged synthetic asset identity".into(),
            },
            stems: vec![StemAsset {
                id: StemId::Drums,
                path: stem,
                sha256: models::hash_file(&root.join("external-drum.wav")).unwrap(),
            }],
            generation: None,
        });
        let mut older = doc.content.clone();
        older.tracks.get_mut("ExpertSingle").unwrap().pop();
        let bytes = serde_json::to_vec(&older).unwrap();
        let id = hash(&bytes);
        fs::write(old.join("checkpoints").join(format!("{id}.json")), bytes).unwrap();
        doc.undo = vec![id.clone()];
        doc.revision = 2;
        fs::write(
            old.join("project.json"),
            serde_json::to_vec_pretty(&doc).unwrap(),
        )
        .unwrap();
        let song = root.join("Song");
        fs::create_dir(&song).unwrap();
        let staged = root.join("Stage");
        copy_tree(&old, &staged.join(".yace/project"));
        (old, song, staged, id)
    }
    #[test]
    fn relocated_assets_and_content_addressed_undo_survive_removing_original_locations() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, old_id) = setup(&root);
        let original_bytes = fs::read(old.join("project.json")).unwrap();
        let source_bytes = fs::read(old.join("source/notes.chart")).unwrap();
        prepare(&staged.join(".yace/project"), &old, &song, &staged).unwrap();
        assert_eq!(fs::read(old.join("project.json")).unwrap(), original_bytes);
        let store = Store::new(&staged.join(".yace/project"));
        let doc = store.load().unwrap();
        assert_eq!(doc.revision, 2);
        assert_eq!(doc.audit.len(), 0);
        assert_ne!(doc.undo[0], old_id);
        assert_eq!(
            models::hash_file(
                &store
                    .root
                    .join("checkpoints")
                    .join(format!("{}.json", doc.undo[0]))
            )
            .unwrap(),
            doc.undo[0]
        );
        assert_eq!(
            fs::read(store.root.join("source/notes.chart")).unwrap(),
            source_bytes
        );
        fs::remove_dir_all(&old).unwrap();
        fs::remove_file(root.join("external-drum.wav")).unwrap();
        let moved = root.join("Moved");
        fs::rename(&staged, &moved).unwrap();
        let store = Store::new(&moved.join(".yace/project"));
        let count = store.load().unwrap().content.tracks["ExpertSingle"].len();
        for action in [Action::Undo, Action::Redo] {
            store
                .apply(Request {
                    expected_revision: store.load().unwrap().revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: "Migration test".into(),
                    },
                    action,
                })
                .unwrap();
            let doc = store.load().unwrap();
            let stem = &doc.content.stems.as_ref().unwrap().stems[0];
            assert_eq!(models::hash_file(&stem.path).unwrap(), stem.sha256);
        }
        assert_eq!(
            store.load().unwrap().content.tracks["ExpertSingle"].len(),
            count
        );
    }
    #[test]
    fn corrupt_checkpoint_is_rejected_before_stems_or_project_state_are_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, id) = setup(&root);
        fs::write(
            old.join("checkpoints").join(format!("{id}.json")),
            b"changed",
        )
        .unwrap();
        fs::write(
            staged
                .join(".yace/project/checkpoints")
                .join(format!("{id}.json")),
            b"changed",
        )
        .unwrap();
        let before = fs::read(staged.join(".yace/project/project.json")).unwrap();
        assert!(prepare(&staged.join(".yace/project"), &old, &song, &staged).is_err());
        assert_eq!(
            fs::read(staged.join(".yace/project/project.json")).unwrap(),
            before
        );
        assert!(!staged.join(".yace/audio/stems").exists());
    }

    #[test]
    fn verified_recovery_repairs_only_the_staged_audio_and_keeps_saved_identity_and_history() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, _) = setup(&root);
        let source = Path::new("source/song.ogg");
        let original_audio = fs::read(old.join(source)).unwrap();
        let recovery = root.join("input-recording.ogg");
        fs::write(&recovery, &original_audio).unwrap();
        let changed_audio = b"A different encoding of the recording";
        fs::write(old.join(source), changed_audio).unwrap();
        fs::write(staged.join(".yace/project").join(source), changed_audio).unwrap();
        let original_document = fs::read(old.join("project.json")).unwrap();
        let before: Project = serde_json::from_slice(&original_document).unwrap();
        assert!(prepare(&staged.join(".yace/project"), &old, &song, &staged).is_err());
        assert_eq!(
            fs::read(staged.join(".yace/project").join(source)).unwrap(),
            changed_audio
        );
        prepare_with_audio_recovery(
            &staged.join(".yace/project"),
            &old,
            &song,
            &staged,
            Some(&recovery),
        )
        .unwrap();
        let store = Store::new(&staged.join(".yace/project"));
        let restored = store.load().unwrap();
        assert_eq!(restored.audio_sha256, before.audio_sha256);
        assert_eq!(restored.source_chart_sha256, before.source_chart_sha256);
        assert_eq!(restored.revision, before.revision);
        assert_eq!(restored.content.tracks, before.content.tracks);
        assert_eq!(restored.undo.len(), before.undo.len());
        assert_eq!(restored.redo.len(), before.redo.len());
        assert_eq!(fs::read(store.root.join(source)).unwrap(), original_audio);
        assert_eq!(fs::read(old.join(source)).unwrap(), changed_audio);
        assert_eq!(
            fs::read(old.join("project.json")).unwrap(),
            original_document
        );
        assert_eq!(fs::read(recovery).unwrap(), original_audio);
        store
            .apply(Request {
                expected_revision: restored.revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Recovered audio undo".into(),
                },
                action: Action::Undo,
            })
            .unwrap();
        assert_eq!(
            store.load().unwrap().content.tracks["ExpertSingle"].len() + 1,
            restored.content.tracks["ExpertSingle"].len()
        );
    }

    #[test]
    fn wrong_recovery_or_changed_staging_audio_cannot_replace_staged_source_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, _) = setup(&root);
        let source = Path::new("source/song.ogg");
        let correct = fs::read(old.join(source)).unwrap();
        let recovery = root.join("recovery.ogg");
        fs::write(&recovery, b"Not the bound recording").unwrap();
        let changed = b"Original encoding was replaced";
        fs::write(old.join(source), changed).unwrap();
        let staged_audio = staged.join(".yace/project").join(source);
        fs::write(&staged_audio, changed).unwrap();
        let document = fs::read(staged.join(".yace/project/project.json")).unwrap();
        let error = prepare_with_audio_recovery(
            &staged.join(".yace/project"),
            &old,
            &song,
            &staged,
            Some(&recovery),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Recovery audio does not match"));
        assert_eq!(fs::read(&staged_audio).unwrap(), changed);
        assert_eq!(
            fs::read(staged.join(".yace/project/project.json")).unwrap(),
            document
        );
        fs::write(&recovery, correct).unwrap();
        fs::write(&staged_audio, b"A separate staged edit").unwrap();
        let error = prepare_with_audio_recovery(
            &staged.join(".yace/project"),
            &old,
            &song,
            &staged,
            Some(&recovery),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Staged source audio no longer matches")
        );
        assert_eq!(fs::read(&staged_audio).unwrap(), b"A separate staged edit");
        assert_eq!(fs::read(old.join(source)).unwrap(), changed);
        assert!(!staged.join(".yace/audio/stems").exists());
    }

    #[test]
    fn active_kit_audition_is_explicitly_refused_without_discarding_its_preference() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, _) = setup(&root);
        let mut value: Value =
            serde_json::from_slice(&fs::read(old.join("project.json")).unwrap()).unwrap();
        value["kit_audition"] = serde_json::to_value(crate::kit_audition::KitAudition {
            source_key: "fixture source".into(),
            kit: old.join("analysis/kit-fixture/kit-parts.json"),
            part: "kick".into(),
            label: "Kick".into(),
            audio_sha256: "0".repeat(64),
            frames: 4 * 44100,
        })
        .unwrap();
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(old.join("project.json"), &bytes).unwrap();
        fs::write(staged.join(".yace/project/project.json"), &bytes).unwrap();
        let error = prepare(&staged.join(".yace/project"), &old, &song, &staged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Turn off the active drum-kit audition")
        );
        assert_eq!(fs::read(old.join("project.json")).unwrap(), bytes);
        assert_eq!(
            fs::read(staged.join(".yace/project/project.json")).unwrap(),
            bytes
        );
    }

    #[cfg(unix)]
    #[test]
    fn externally_referenced_symlinks_are_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let (old, song, staged, _) = setup(&root);
        let link = root.join("linked-drum.wav");
        std::os::unix::fs::symlink(root.join("external-drum.wav"), &link).unwrap();
        let mut value: Value =
            serde_json::from_slice(&fs::read(old.join("project.json")).unwrap()).unwrap();
        value["content"]["stems"]["stems"][0]["path"] = serde_json::to_value(link).unwrap();
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(old.join("project.json"), &bytes).unwrap();
        fs::write(staged.join(".yace/project/project.json"), &bytes).unwrap();
        let error = prepare(&staged.join(".yace/project"), &old, &song, &staged).unwrap_err();
        assert!(format!("{error:#}").contains("symlink"));
        assert!(!staged.join(".yace/audio/stems").exists());
    }
}
