//! Relocate an old generation wrapper inside an unpublished migration copy.
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

fn song_asset(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "notes.chart"
            | "notes.mid"
            | "song.ini"
            | "album.png"
            | "album.jpg"
            | "album.jpeg"
            | "background.png"
            | "background.jpg"
            | "highway.png"
            | "video.mp4"
            | "video.webm"
            | "video.avi"
    ) {
        return true;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ogg", "opus", "mp3", "wav", "flac", "m4a", "aac", "aiff"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

pub(super) fn reorganize(stage: &Path) -> Result<()> {
    let job = stage.join(".yace/jobs/generation");
    if !stage.join("job.json").is_file() {
        ensure!(
            job.join("job.json").is_file() && job.join("song-layout.json").is_file(),
            "No legacy generation job to relocate"
        );
        return Ok(());
    }
    let status: crate::generation::Status =
        serde_json::from_slice(&fs::read(stage.join("job.json"))?)?;
    ensure!(status.version == 1, "Unsupported generation job version");
    ensure!(
        status.state == crate::generation::State::Complete,
        "Finish or cancel and resolve the incomplete generation job before migration"
    );
    ensure!(
        stage.join("request.json").is_file(),
        "Generation job has no saved request"
    );
    ensure!(
        !job.exists(),
        "Generation migration destination already exists"
    );
    let project = stage.join(".yace/project");
    fs::create_dir_all(job.parent().context("Generation job needs a parent")?)?;
    if stage.join("project").is_dir() {
        ensure!(
            !project.exists(),
            "Editing project migration destination already exists"
        );
        fs::rename(stage.join("project"), &project)?;
    }
    ensure!(
        project.join("project.json").is_file(),
        "Generation job has no saved editing project"
    );
    fs::create_dir(&job)?;
    let entries = fs::read_dir(stage)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let name = entry.file_name();
        if name == ".yace" {
            continue;
        }
        let kind = entry.file_type()?;
        ensure!(!kind.is_symlink(), "Generation wrapper contains a symlink");
        if kind.is_file() && song_asset(&entry.path()) {
            continue;
        }
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Generation wrapper contains an unsupported file type"
        );
        let target = job.join(&name);
        ensure!(
            !target.exists(),
            "Generation migration would replace an existing file"
        );
        fs::rename(entry.path(), target)?;
    }
    if job.join("song-layout.json").exists() {
        let previous = job.join("legacy-song-layout.json");
        ensure!(
            !previous.exists(),
            "Legacy song layout marker already has a recovery copy"
        );
        fs::rename(job.join("song-layout.json"), previous)?;
    }
    crate::generation::write_json(
        &job.join("song-layout.json"),
        &serde_json::json!({"version":1,"layout":"standard_song_folder","migrated_legacy_job":true}),
    )?;
    Ok(())
}

/// Freeze the newly exported current chart as this completed job's publication.
/// The caller writes all other migration metadata before invoking this function.
pub(super) fn receipt(stage: &Path) -> Result<()> {
    let job = stage.join(".yace/jobs/generation");
    ensure!(
        job.join("job.json").is_file() && job.join("song-layout.json").is_file(),
        "No relocated generation job to finalize"
    );
    if job.join("publication.json").is_file() {
        return crate::generation::record_publication(&job, stage);
    }
    let game = job.join("game-song");
    if game.exists() {
        let previous = job.join("legacy-game-song");
        ensure!(
            !previous.exists(),
            "Previous generated game export already has a recovery copy"
        );
        fs::rename(&game, previous)?;
    }
    let snapshot = tempfile::Builder::new()
        .prefix(".migration-export-")
        .tempdir_in(&job)?;
    crate::pipeline::copy_song_source(stage, snapshot.path())?;
    crate::pipeline::publish(snapshot.path(), &game)?;
    crate::generation::record_publication(&job, stage)
}

#[cfg(test)]
mod tests {
    use super::*;
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }

    #[test]
    fn relocation_preserves_requests_and_recovery_then_publishes_the_current_project_export() {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("Song");
        fs::create_dir_all(song.join("project")).unwrap();
        let store = fixture::fixture(&song.join("project"));
        let request = br#"{"source":{"type":"youtube","url":"https://example.invalid/original"}}"#;
        fs::write(song.join("request.json"), request).unwrap();
        crate::generation::write_json(
            &song.join("job.json"),
            &crate::generation::Status {
                version: 1,
                state: crate::generation::State::Complete,
                stage: "complete".into(),
                detail: "Saved draft".into(),
                request_sha256: crate::models::hash_file(&song.join("request.json")).unwrap(),
                error: None,
            },
        )
        .unwrap();
        fs::create_dir(song.join("input")).unwrap();
        fs::write(song.join("input/source.wav"), b"preserved import").unwrap();
        fs::write(song.join("events.json"), b"exact historical event bytes").unwrap();
        fs::create_dir(song.join("game-song")).unwrap();
        fs::write(song.join("game-song/notes.chart"), b"old generated chart").unwrap();
        fs::create_dir(song.join("clean-copy")).unwrap();
        fs::write(
            song.join("clean-copy/receipt.json"),
            b"old cleaning receipt",
        )
        .unwrap();
        let before = fs::read(store.root.join("project.json")).unwrap();
        reorganize(&song).unwrap();
        let job = song.join(".yace/jobs/generation");
        assert_eq!(fs::read(job.join("request.json")).unwrap(), request);
        assert_eq!(
            fs::read(job.join("events.json")).unwrap(),
            b"exact historical event bytes"
        );
        assert_eq!(
            fs::read(song.join(".yace/project/project.json")).unwrap(),
            before
        );
        assert_eq!(
            fs::read(job.join("clean-copy/receipt.json")).unwrap(),
            b"old cleaning receipt"
        );
        let source = song.join(".yace/project/source");
        fs::copy(source.join("notes.chart"), song.join("notes.chart")).unwrap();
        fs::copy(source.join("song.ogg"), song.join("song.ogg")).unwrap();
        fs::write(
            song.join("song.ini"),
            "[song]\nname = Current project export\n",
        )
        .unwrap();
        receipt(&song).unwrap();
        assert_eq!(
            fs::read(job.join("legacy-game-song/notes.chart")).unwrap(),
            b"old generated chart"
        );
        assert_eq!(
            fs::read(job.join("game-song/notes.chart")).unwrap(),
            fs::read(song.join("notes.chart")).unwrap()
        );
        crate::generation::run(&song).unwrap();
        reorganize(&song).unwrap();
        receipt(&song).unwrap();
        assert_eq!(fs::read(job.join("request.json")).unwrap(), request);
    }
}
