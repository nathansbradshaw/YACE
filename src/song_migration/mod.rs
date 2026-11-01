//! Offline migration: prepare and validate copies, then retain originals before publication.
mod generation;
mod project;

use crate::{chart_library, editor, models, pipeline};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    collections::BTreeMap,
    fs,
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub song: PathBuf,
    pub project: Option<PathBuf>,
    pub generated_wrapper: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub items: Vec<Item>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub backup: PathBuf,
    pub completed: Vec<Value>,
}

/// Inventory only. Song/project boundaries keep immutable snapshots out of the plan.
pub fn plan(roots: &[PathBuf]) -> Result<Plan> {
    let mut items = BTreeMap::new();
    for selected in roots {
        let library = chart_library::discover(selected)?;
        for entry in &library.entries {
            let project = library.project_path(entry)?;
            let wrapper = entry.path.join("job.json").is_file();
            let song = if entry.kind == chart_library::Kind::Project {
                if wrapper {
                    entry.path.clone()
                } else if entry.path.file_name().is_some_and(|n| n == "project")
                    && entry
                        .path
                        .parent()
                        .is_some_and(|p| p.join("job.json").is_file())
                {
                    entry.path.parent().unwrap().to_owned()
                } else if entry.path.ends_with(".yace/project") {
                    entry.path.parent().unwrap().parent().unwrap().to_owned()
                } else {
                    // A standalone project is itself a useful song container.
                    entry.path.clone()
                }
            } else {
                entry.path.clone()
            };
            let wrapper = song.join("job.json").is_file();
            let existing = if wrapper {
                song.join("project")
            } else {
                project
            };
            let item = Item {
                song: song.canonicalize()?,
                project: existing.join("project.json").is_file().then_some(existing),
                generated_wrapper: wrapper,
            };
            items.insert(item.song.clone(), item);
        }
    }
    let items: Vec<_> = items.into_values().collect();
    for (i, item) in items.iter().enumerate() {
        ensure!(
            !items
                .iter()
                .skip(i + 1)
                .any(|other| other.song.starts_with(&item.song)),
            "Selected song folders overlap"
        );
    }
    Ok(Plan { version: 1, items })
}

/// Hash every regular file; refuse symlinks and special files instead of following them.
fn inventory(root: &Path) -> Result<BTreeMap<PathBuf, String>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, String>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Migration refuses symlink {}",
                entry.path().display()
            );
            if kind.is_dir() {
                visit(root, &entry.path(), out)?;
            } else {
                ensure!(
                    kind.is_file(),
                    "Migration refuses special file {}",
                    entry.path().display()
                );
                out.insert(
                    entry.path().strip_prefix(root)?.to_owned(),
                    models::hash_file(&entry.path())?,
                );
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result)?;
    Ok(result)
}

fn clone_folder(from: &Path, to: &Path) -> Result<()> {
    ensure!(!to.exists(), "Staging destination exists");
    // APFS clones are independent backups without duplicating large audio blocks.
    #[cfg(target_os = "macos")]
    {
        let result = Command::new("/bin/cp")
            .arg("-cR")
            .arg(from)
            .arg(to)
            .output()?;
        ensure!(
            result.status.success(),
            "APFS clone failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(!kind.is_symlink(), "Migration refuses symlinks");
            if kind.is_dir() {
                clone_folder(&entry.path(), &to.join(entry.file_name()))?;
            } else {
                ensure!(kind.is_file(), "Unsupported migration file");
                fs::copy(entry.path(), to.join(entry.file_name()))?;
            }
        }
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = File::create_new(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.sync_all()?;
    Ok(())
}

fn strip_stem_paths(value: &mut Value) {
    if let Some(stems) = value.get_mut("stems").and_then(Value::as_object_mut)
        && let Some(assets) = stems.get_mut("stems").and_then(Value::as_array_mut)
    {
        for asset in assets {
            asset.as_object_mut().unwrap().remove("path");
        }
    }
    if let Some(stems) = value
        .get_mut("keyboard_stems")
        .and_then(Value::as_object_mut)
        && let Some(assets) = stems.get_mut("stems").and_then(Value::as_array_mut)
    {
        for asset in assets {
            asset.as_object_mut().unwrap().remove("path");
        }
    }
}

fn fingerprint(path: &Path) -> Result<Value> {
    let store = editor::Store::new(path);
    let doc = store.load()?;
    ensure!(
        models::hash_file(&store.audio_path(&doc)?)? == doc.audio_sha256,
        "Project source audio changed"
    );
    let mut value = serde_json::to_value(&doc)?;
    value.as_object_mut().unwrap().remove("original_folder");
    strip_stem_paths(&mut value["content"]);
    for name in ["undo", "redo"] {
        let ids = value[name].as_array().context("Invalid undo history")?;
        let mut contents = Vec::new();
        for id in ids {
            let id = id.as_str().context("Invalid checkpoint ID")?;
            ensure!(
                id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid checkpoint ID"
            );
            let file = path.join("checkpoints").join(format!("{id}.json"));
            ensure!(models::hash_file(&file)? == id, "Checkpoint changed");
            let content: editor::Content = serde_json::from_slice(&fs::read(file)?)?;
            let mut content = serde_json::to_value(content)?;
            strip_stem_paths(&mut content);
            contents.push(content);
        }
        value[name] = json!(contents);
    }
    Ok(json!({"project":value,"chart_sha256":doc.chart_hash(&store)?}))
}

fn locks(item: &Item) -> Result<Vec<File>> {
    let mut files = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let managed = item.song.join(".yace/project");
    let job = item.song.join(".yace/jobs/generation");
    for root in std::iter::once(&item.song)
        .chain(item.project.iter())
        .chain(std::iter::once(&managed))
        .chain(std::iter::once(&job))
    {
        for name in ["editor.lock", "worker.lock", "window.lock"] {
            let path = root.join(name);
            if path.exists() {
                let path = path.canonicalize()?;
                if !seen.insert(path.clone()) {
                    continue;
                }
                let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
                file.try_lock().with_context(|| {
                    format!(
                        "Close the active project/job at {} before migration",
                        root.display()
                    )
                })?;
                files.push(file);
            }
        }
    }
    Ok(files)
}

/// Originals are moved into the backup only after the replacement passes validation.
/// Each item has a durable journal; failed post-publication checks restore its original.
pub fn apply(plan: &Plan, backup: &Path) -> Result<Report> {
    ensure!(
        plan.version == 1 && !plan.items.is_empty(),
        "Empty or unsupported migration plan"
    );
    let parent = backup
        .parent()
        .context("Backup needs a parent")?
        .canonicalize()?;
    let backup = parent.join(backup.file_name().context("Backup needs a name")?);
    ensure!(
        !plan.items.iter().any(|item| backup.starts_with(&item.song)
            || item
                .project
                .as_ref()
                .is_some_and(|project| backup.starts_with(project))),
        "Backup must be outside song folders"
    );
    fs::create_dir(&backup).context("Backup directory must be new")?;
    write_json(&backup.join("plan.json"), plan)?;
    let mut report = Report {
        version: 1,
        backup: backup.clone(),
        completed: vec![],
    };
    for (index, item) in plan.items.iter().enumerate() {
        eprintln!(
            "Preparing {}/{}: {}",
            index + 1,
            plan.items.len(),
            item.song.display()
        );
        let _locks = locks(item)?;
        let unit = backup.join(format!("{index:03}"));
        fs::create_dir(&unit)?;
        let before = inventory(&item.song)?;
        let external = item.project.as_ref().filter(|p| !p.starts_with(&item.song));
        let external_before = external.map(|p| inventory(p)).transpose()?;
        write_json(&unit.join("original-files.json"), &before)?;
        if let Some(external_before) = &external_before {
            write_json(&unit.join("original-shared-files.json"), external_before)?;
        }
        let expected = item.project.as_ref().map(|p| fingerprint(p)).transpose()?;
        let stage = unit.join("prepared");
        clone_folder(&item.song, &stage)?;
        let managed = stage.join(".yace/project");
        if item.generated_wrapper {
            generation::reorganize(&stage)?;
        } else if stage.join("project.json").is_file() {
            // Standalone editing project: keep its full existing state under .yace.
            let holding = unit.join("project-copy");
            fs::rename(&stage, &holding)?;
            fs::create_dir_all(stage.join(".yace"))?;
            fs::rename(holding, &managed)?;
        } else {
            pipeline::organize_song_stage(&stage)?;
            if let Some(project) = external {
                fs::create_dir_all(stage.join(".yace"))?;
                clone_folder(project, &managed)?;
            }
        }
        if managed.join("project.json").is_file() {
            project::prepare(
                &managed,
                item.project.as_deref().unwrap(),
                &item.song,
                &stage,
            )?;
        } else {
            fs::create_dir_all(stage.join(".yace"))?;
            editor::Store::create(&item.song, None, &managed)?;
        }
        let prepared = fingerprint(&managed)?;
        if let Some(expected) = &expected {
            ensure!(
                &prepared == expected,
                "Migration changed saved edits, settings or undo history for {}",
                item.song.display()
            );
        }
        if item.generated_wrapper || !stage.join("song.ini").is_file() {
            let export = unit.join("export");
            fs::create_dir(&export)?;
            let store = editor::Store::new(&managed);
            store.write_export(
                &store.load()?,
                &export,
                &editor::Actor {
                    kind: editor::ActorKind::Human,
                    name: "YACE layout migration".into(),
                },
            )?;
            merge_export(&export, &stage)?;
        }
        if item.generated_wrapper {
            generation::receipt(&stage)?;
        }
        ensure!(
            pipeline::inspect(&stage)?.error.is_none(),
            "Migrated song failed chart inventory"
        );
        ensure!(
            inventory(&item.song)? == before,
            "Song changed during migration; original retained"
        );
        if let Some(external) = external {
            ensure!(
                Some(inventory(external)?) == external_before,
                "Saved project changed during migration"
            );
        }
        write_json(
            &unit.join("ready.json"),
            &json!({"song":item.song,"project_before":expected,"project_after":prepared}),
        )?;
        let original = unit.join("original");
        fs::rename(&item.song, &original).context("Cannot retain original song in backup")?;
        if let Err(error) = fs::rename(&stage, &item.song) {
            fs::rename(&original, &item.song)
                .context("Publication failed; restore original from backup")?;
            return Err(error.into());
        }
        let verified = (|| -> Result<()> {
            ensure!(
                fingerprint(&item.song.join(".yace/project"))? == prepared,
                "Published project differs from prepared project"
            );
            let library = chart_library::discover(&item.song)?;
            ensure!(
                library.entries.len() == 1,
                "Migrated song must appear exactly once"
            );
            ensure!(
                library.open(&library.entries[0])? == item.song.join(".yace/project"),
                "Library opens a different editing project"
            );
            if item.generated_wrapper {
                crate::generation::run(&item.song)?;
            }
            Ok(())
        })();
        if let Err(error) = verified {
            fs::rename(&item.song, unit.join("failed-replacement"))?;
            fs::rename(&original, &item.song)?;
            return Err(error.context("Post-migration verification failed; original restored"));
        }
        if let Some(external) = external
            && let Err(error) = fs::rename(external, unit.join("original-shared-project"))
        {
            fs::rename(&item.song, unit.join("failed-replacement"))?;
            fs::rename(&original, &item.song)?;
            return Err(anyhow::Error::from(error)
                .context("Shared project backup failed; original song restored"));
        }
        let completed = json!({"song":item.song,"backup":original,"project":item.song.join(".yace/project"),"verified":true});
        write_json(&unit.join("complete.json"), &completed)?;
        report.completed.push(completed);
        eprintln!("Verified {}", item.song.display());
    }
    consolidate_caches(plan, &backup)?;
    write_json(&backup.join("report.json"), &report)?;
    Ok(report)
}

/// An explicit rollback preserves the migrated version too; it never overwrites a folder.
pub fn restore(backup: &Path) -> Result<()> {
    let plan: Plan = serde_json::from_slice(&fs::read(backup.join("plan.json"))?)?;
    ensure!(plan.version == 1, "Unsupported migration backup");
    for (index, item) in plan.items.iter().enumerate().rev() {
        let unit = backup.join(format!("{index:03}"));
        if !unit.join("ready.json").is_file() || unit.join("restored.json").is_file() {
            continue;
        }
        // An interrupted preparation has not moved the original yet.
        if !unit.join("original").is_dir() {
            continue;
        }
        let _locks = locks(item)?;
        let expected: BTreeMap<PathBuf, String> =
            serde_json::from_slice(&fs::read(unit.join("original-files.json"))?)?;
        ensure!(
            inventory(&unit.join("original"))? == expected,
            "Backup changed; refusing restore"
        );
        let current = unit.join("migrated-before-restore");
        ensure!(!current.exists(), "Restore destination exists");
        let shared = item.project.as_ref().filter(|p| !p.starts_with(&item.song));
        let shared_backup = unit.join("original-shared-project");
        if let Some(project) = shared {
            let expected: BTreeMap<PathBuf, String> =
                serde_json::from_slice(&fs::read(unit.join("original-shared-files.json"))?)?;
            if shared_backup.exists() {
                ensure!(
                    !project.exists(),
                    "Shared project destination was recreated; refusing overwrite"
                );
                ensure!(
                    inventory(&shared_backup)? == expected,
                    "Shared project backup changed"
                );
            } else {
                ensure!(
                    inventory(project)? == expected,
                    "Original shared project changed before interrupted migration completed"
                );
            }
        }
        let had_current = item.song.exists();
        if had_current {
            fs::rename(&item.song, &current)?;
        }
        if let Err(error) = fs::rename(unit.join("original"), &item.song) {
            if had_current {
                fs::rename(&current, &item.song)?;
            }
            return Err(error.into());
        }
        if let Some(project) = shared
            && shared_backup.exists()
            && let Err(error) = fs::rename(&shared_backup, project)
        {
            fs::rename(&item.song, unit.join("original"))?;
            if had_current {
                fs::rename(&current, &item.song)?;
            }
            return Err(error.into());
        }
        write_json(
            &unit.join("restored.json"),
            &json!({"song":item.song,"migrated_copy":current}),
        )?;
    }
    restore_caches(backup)?;
    Ok(())
}

// Historical immutable manifests may still name the original absolute cache.
// The small relative alias keeps those witnesses usable; audio bytes live in .yace.
#[cfg(unix)]
fn consolidate_caches(plan: &Plan, backup: &Path) -> Result<()> {
    let parents: std::collections::BTreeSet<_> = plan
        .items
        .iter()
        .filter(|item| item.generated_wrapper)
        .filter_map(|item| item.song.parent().map(Path::to_owned))
        .collect();
    let mut cache_report = Vec::new();
    for (index, parent) in parents.iter().enumerate() {
        let old = parent.join(".chart-engine-stem-cache");
        if !old.exists() || old.is_symlink() {
            continue;
        }
        let target = parent.join(".yace/cache/legacy-separation");
        ensure!(
            !target.exists(),
            "Legacy cache destination exists: {}",
            target.display()
        );
        let unit = backup.join(format!("cache-{index:03}"));
        fs::create_dir(&unit)?;
        let hashes = inventory(&old)?;
        write_json(&unit.join("original-files.json"), &hashes)?;
        let prepared = unit.join("prepared");
        clone_folder(&old, &prepared)?;
        ensure!(
            inventory(&prepared)? == hashes && inventory(&old)? == hashes,
            "Cache changed during migration"
        );
        fs::create_dir_all(target.parent().unwrap())?;
        let original = unit.join("original");
        write_json(
            &unit.join("ready.json"),
            &json!({"old":old,"target":target,"original":original}),
        )?;
        fs::rename(&old, &original)?;
        if let Err(error) = fs::rename(&prepared, &target) {
            fs::rename(&original, &old)?;
            return Err(error.into());
        }
        if let Err(error) = std::os::unix::fs::symlink(".yace/cache/legacy-separation", &old) {
            fs::rename(&target, &prepared)?;
            fs::rename(&original, &old)?;
            return Err(error.into());
        }
        let entry = json!({"old":old,"target":target,"original":original,"unit":unit});
        write_json(&unit.join("complete.json"), &entry)?;
        cache_report.push(entry);
    }
    write_json(&backup.join("caches.json"), &cache_report)?;
    Ok(())
}

#[cfg(unix)]
fn restore_caches(backup: &Path) -> Result<()> {
    for directory in fs::read_dir(backup)? {
        let directory = directory?;
        if !directory.file_type()?.is_dir()
            || !directory
                .file_name()
                .to_string_lossy()
                .starts_with("cache-")
        {
            continue;
        }
        let unit = directory.path();
        if !unit.join("ready.json").is_file()
            || !unit.join("original").is_dir()
            || unit.join("restored.json").is_file()
        {
            continue;
        }
        let entry: Value = serde_json::from_slice(&fs::read(unit.join("ready.json"))?)?;
        let old = Path::new(entry["old"].as_str().context("Missing cache path")?);
        let original = unit.join("original");
        let had_alias = old.symlink_metadata().is_ok();
        if had_alias {
            ensure!(
                fs::read_link(old)? == Path::new(".yace/cache/legacy-separation"),
                "Cache alias changed; refusing restore"
            );
        }
        let expected: BTreeMap<PathBuf, String> =
            serde_json::from_slice(&fs::read(unit.join("original-files.json"))?)?;
        ensure!(
            inventory(&original)? == expected,
            "Original cache backup changed"
        );
        if had_alias {
            fs::rename(old, unit.join("compatibility-alias"))?;
        }
        if let Err(error) = fs::rename(&original, old) {
            if had_alias {
                fs::rename(unit.join("compatibility-alias"), old)?;
            }
            return Err(error.into());
        }
        write_json(&unit.join("restored.json"), &json!({"cache":old}))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn consolidate_caches(_plan: &Plan, _backup: &Path) -> Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn restore_caches(_backup: &Path) -> Result<()> {
    Ok(())
}

fn merge_export(from: &Path, to: &Path) -> Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() && dest.is_dir() {
            merge_export(&entry.path(), &dest)?;
        } else {
            if dest.is_file()
                && entry.file_type()?.is_file()
                && models::hash_file(&dest)? == models::hash_file(&entry.path())?
            {
                continue;
            }
            ensure!(
                !dest.exists(),
                "Export conflicts with preserved file {}",
                dest.display()
            );
            fs::rename(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }

    fn saved_project(root: &Path) -> editor::Store {
        fs::create_dir(root).unwrap();
        let store = fixture::fixture(root);
        fs::write(
            root.join("source/song.ini"),
            "[song]\nname = Migration fixture\nartist = Test\n",
        )
        .unwrap();
        store
    }

    #[test]
    fn migrates_saved_project_with_exact_backup_and_preserved_edit_history() {
        let temp = tempfile::tempdir().unwrap();
        let song = temp.path().canonicalize().unwrap().join("Song");
        let store = saved_project(&song);
        File::create(song.join("editor.lock")).unwrap();
        let mut doc = store.load().unwrap();
        let checkpoint = serde_json::to_vec(&doc.content).unwrap();
        let id = format!("{:x}", sha2::Sha256::digest(&checkpoint));
        fs::write(
            song.join("checkpoints").join(format!("{id}.json")),
            checkpoint,
        )
        .unwrap();
        doc.undo.push(id);
        doc.revision = 5;
        doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
            .note
            .tick = 460;
        fixture::save(&store, &doc);
        let original = inventory(&song).unwrap();
        let expected = fingerprint(&song).unwrap();
        let plan = plan(std::slice::from_ref(&song)).unwrap();
        let backup = temp.path().join("backup");
        let report = apply(&plan, &backup).unwrap();
        assert_eq!(report.completed.len(), 1);
        assert_eq!(inventory(&backup.join("000/original")).unwrap(), original);
        assert_eq!(fingerprint(&song.join(".yace/project")).unwrap(), expected);
        assert!(!song.join("project.json").exists());
        assert!(song.join("notes.chart").is_file());
        let library = chart_library::discover(&song).unwrap();
        assert_eq!(
            library.open(&library.entries[0]).unwrap(),
            song.join(".yace/project")
        );
        fs::remove_file(backup.join("000/complete.json")).unwrap();
        fs::remove_file(backup.join("report.json")).unwrap();
        restore(&backup).unwrap();
        assert_eq!(inventory(&song).unwrap(), original);
        assert!(
            backup
                .join("000/migrated-before-restore/.yace/project/project.json")
                .is_file()
        );
    }

    #[test]
    fn refuses_bad_checkpoint_without_changing_original() {
        let temp = tempfile::tempdir().unwrap();
        let song = temp.path().canonicalize().unwrap().join("Song");
        let store = saved_project(&song);
        let mut doc = store.load().unwrap();
        let id = "a".repeat(64);
        fs::write(song.join("checkpoints").join(format!("{id}.json")), b"{}").unwrap();
        doc.undo.push(id);
        fixture::save(&store, &doc);
        let original = inventory(&song).unwrap();
        let plan = plan(std::slice::from_ref(&song)).unwrap();
        assert!(apply(&plan, &temp.path().join("backup")).is_err());
        assert_eq!(inventory(&song).unwrap(), original);
    }

    #[test]
    fn restore_checks_shared_project_backup_before_changing_published_song() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let song = root.join("Song");
        let shared = root.join("shared-project");
        saved_project(&shared);
        clone_folder(&shared.join("source"), &song).unwrap();
        let original_song = inventory(&song).unwrap();
        let original_shared = inventory(&shared).unwrap();
        let plan = Plan {
            version: 1,
            items: vec![Item {
                song: song.clone(),
                project: Some(shared.clone()),
                generated_wrapper: false,
            }],
        };
        let backup = root.join("backup");
        apply(&plan, &backup).unwrap();
        let published = inventory(&song).unwrap();
        let saved = backup.join("000/original-shared-project/project.json");
        let bytes = fs::read(&saved).unwrap();
        fs::write(&saved, b"damaged backup").unwrap();
        assert!(restore(&backup).is_err());
        assert_eq!(inventory(&song).unwrap(), published);
        fs::write(&saved, bytes).unwrap();
        restore(&backup).unwrap();
        assert_eq!(inventory(&song).unwrap(), original_song);
        assert_eq!(inventory(&shared).unwrap(), original_shared);
    }

    #[test]
    fn plan_is_read_only_and_backup_must_be_outside_song() {
        let temp = tempfile::tempdir().unwrap();
        let song = temp.path().canonicalize().unwrap().join("Song");
        saved_project(&song);
        let original = inventory(&song).unwrap();
        let plan = plan(std::slice::from_ref(&song)).unwrap();
        assert_eq!(inventory(&song).unwrap(), original);
        assert!(apply(&plan, &song.join("backup")).is_err());
        assert_eq!(inventory(&song).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_without_following_it() {
        let temp = tempfile::tempdir().unwrap();
        let song = temp.path().canonicalize().unwrap().join("Song");
        saved_project(&song);
        std::os::unix::fs::symlink(temp.path(), song.join("escape")).unwrap();
        let plan = plan(std::slice::from_ref(&song)).unwrap();
        assert!(apply(&plan, &temp.path().join("backup")).is_err());
        assert!(song.join("project.json").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn legacy_cache_alias_preserves_old_references_and_exact_original_backup() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("Artist");
        let cache = parent.join(".chart-engine-stem-cache/hash");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("vocals.wav"), b"historical exact stem").unwrap();
        let original = inventory(&parent.join(".chart-engine-stem-cache")).unwrap();
        let backup = temp.path().join("backup");
        fs::create_dir(&backup).unwrap();
        let plan = Plan {
            version: 1,
            items: vec![Item {
                song: parent.join("Song"),
                project: None,
                generated_wrapper: true,
            }],
        };
        consolidate_caches(&plan, &backup).unwrap();
        assert!(parent.join(".chart-engine-stem-cache").is_symlink());
        assert_eq!(
            fs::read(cache.join("vocals.wav")).unwrap(),
            b"historical exact stem"
        );
        assert_eq!(
            inventory(&backup.join("cache-000/original")).unwrap(),
            original
        );
        assert!(
            parent
                .join(".yace/cache/legacy-separation/hash/vocals.wav")
                .is_file()
        );
        fs::remove_file(backup.join("cache-000/complete.json")).unwrap();
        fs::remove_file(backup.join("caches.json")).unwrap();
        restore_caches(&backup).unwrap();
        assert!(!parent.join(".chart-engine-stem-cache").is_symlink());
        assert_eq!(
            inventory(&parent.join(".chart-engine-stem-cache")).unwrap(),
            original
        );
    }
}
