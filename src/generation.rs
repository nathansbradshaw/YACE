//! Recoverable generation jobs shared by the desktop app and headless commands.
use crate::{
    editor::Store,
    mapping::Transcription,
    models,
    pipeline::{self, Build, IsolationOptions},
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    Audio { path: PathBuf },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub source: Source,
    pub bpm: f64,
    pub title: String,
    pub artist: String,
    pub instruments: Vec<String>,
    #[serde(default)]
    pub guitar_source: crate::mapping::GuitarSource,
    #[serde(default)]
    pub keyboard_sound: crate::keyboard_detection::KeyboardSound,
    #[serde(default)]
    pub piano_quality: crate::keyboard_detection::PianoQuality,
    /// Omitted legacy jobs retain their English recognition policy.
    #[serde(default)]
    pub vocal_language: crate::lyric_language::Language,
    pub model_dir: PathBuf,
    #[serde(default)]
    pub fetch_models: bool,
    #[serde(default)]
    pub full_mix_drums: bool,
    #[serde(default)]
    pub standard_drums: bool,
    #[serde(default)]
    pub lyrics: bool,
    #[serde(default)]
    pub song_info: Option<crate::song_import::SongInfo>,
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        TempoMap::constant(self.bpm)?;
        ensure!(
            !self.title.trim().is_empty() && !self.artist.trim().is_empty(),
            "Enter the song title and artist"
        );
        ensure!(
            !self.title.contains(['\n', '\r']) && !self.artist.contains(['\n', '\r']),
            "Title and artist must each be one line"
        );
        ensure!(
            !self.instruments.is_empty()
                && self.instruments.iter().all(|s| [
                    "guitar",
                    "bass",
                    "drums",
                    "vocals",
                    "piano",
                    "pro_piano"
                ]
                .contains(&s.as_str())),
            "Select guitar, bass, drums, Piano (Keys), Pro Piano (Pro Keys) and/or vocals"
        );
        ensure!(
            self.instruments.iter().any(|s| s != "vocals"),
            "Choose an instrumental track too; standalone vocals use the vocals generate command"
        );
        ensure!(
            !self.full_mix_drums || self.instruments.iter().all(|s| s == "drums"),
            "Full-mix transcription supports drums only"
        );
        if !cfg!(feature = "separation") {
            ensure!(
                self.full_mix_drums
                    || (crate::keyboard_detection::high_quality_piano(
                        &self.instruments,
                        self.keyboard_sound,
                        self.piano_quality
                    ) && self
                        .instruments
                        .iter()
                        .all(|s| matches!(s.as_str(), "piano" | "pro_piano"))),
                "This build has no standard separator. Use a full build, higher-quality Piano only, or drums-only from the full mix."
            );
        }
        match &self.source {
            Source::Audio { path } => {
                ensure!(path.is_file(), "Choose an audio file");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub version: u32,
    pub state: State,
    pub stage: String,
    pub detail: String,
    pub request_sha256: String,
    pub error: Option<String>,
}

pub fn default_model_dir() -> PathBuf {
    model_dir_for(
        std::env::current_exe().ok().as_deref(),
        std::env::current_dir().ok().as_deref(),
        models::user_model_cache(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn model_dir_for(
    executable: Option<&Path>,
    working_directory: Option<&Path>,
    user_cache: Option<PathBuf>,
    repository: &Path,
) -> PathBuf {
    if let Some(exe) = executable
        && exe
            .parent()
            .is_some_and(|path| path.file_name().is_some_and(|name| name == "MacOS"))
        && let Some(contents) = exe.parent().and_then(Path::parent)
        && contents.file_name().is_some_and(|name| name == "Contents")
        && contents.parent().is_some_and(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    {
        let bundled = contents.join("Resources/models");
        if models::assets()
            .iter()
            .all(|asset| bundled.join(&asset.filename).is_file())
        {
            return bundled;
        }
        // Downloads belong outside the signed app. A partial older bundle
        // must not become a writable model directory either.
        return user_cache.unwrap_or_else(|| std::env::temp_dir().join("YACE/models"));
    }
    if let Some(directory) = working_directory {
        let local = directory.join("models");
        if local.join("manifest.json").is_file() {
            return local;
        }
    }
    if let Some(executable) = executable
        && let Some(repository) = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
    {
        let local = repository.join("models");
        if local.join("manifest.json").is_file() {
            return local;
        }
    }
    repository.join("models")
}

#[cfg(test)]
mod model_directory_tests {
    use super::*;

    #[test]
    fn empty_and_partial_app_bundles_use_the_user_cache() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("YACE.app/Contents/MacOS/chart-engine");
        let bundled = root.path().join("YACE.app/Contents/Resources/models");
        let cache = root.path().join("user/models");
        fs::create_dir_all(&bundled).unwrap();
        let select = || {
            model_dir_for(
                Some(&executable),
                Some(root.path()),
                Some(cache.clone()),
                root.path(),
            )
        };
        assert_eq!(select(), cache);
        fs::write(bundled.join(&models::assets()[0].filename), b"fixture").unwrap();
        assert_eq!(select(), cache);
        assert_eq!(fs::read_dir(&bundled).unwrap().count(), 1);
        assert!(!cache.exists(), "path selection must not write files");
    }

    #[test]
    fn a_complete_private_bundle_keeps_its_existing_model_location() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("YACE.app/Contents/MacOS/chart-engine");
        let bundled = root.path().join("YACE.app/Contents/Resources/models");
        fs::create_dir_all(&bundled).unwrap();
        for asset in models::assets() {
            fs::write(bundled.join(asset.filename), b"location fixture").unwrap();
        }
        assert_eq!(
            model_dir_for(
                Some(&executable),
                None,
                Some(root.path().join("cache")),
                root.path()
            ),
            bundled
        );
        assert!(
            models::verified(&bundled, "drums").is_err(),
            "location selection must not replace checksum verification"
        );
    }

    #[test]
    fn command_line_builds_still_find_repository_models() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        fs::create_dir_all(repository.join("models")).unwrap();
        fs::write(repository.join("models/manifest.json"), b"[]").unwrap();
        let executable = repository.join("target/release/chart-engine");
        assert_eq!(
            model_dir_for(
                Some(&executable),
                Some(root.path()),
                Some(root.path().join("cache")),
                root.path()
            ),
            repository.join("models")
        );
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut stage =
        tempfile::NamedTempFile::new_in(path.parent().context("JSON output needs a parent")?)?;
    serde_json::to_writer_pretty(&mut stage, value)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    stage.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Publish a recovery archive only after it is complete.
/// A partial final file would block the next retry.
fn preserve_analysis_backup(path: &Path, previous: &[u8]) -> Result<()> {
    let check_existing = || -> Result<()> {
        ensure!(
            fs::read(path)? == previous,
            "Bass analysis history changed; current analysis retained"
        );
        Ok(())
    };
    if path.exists() {
        return check_existing();
    }
    let mut stage =
        tempfile::NamedTempFile::new_in(path.parent().context("Analysis backup needs a parent")?)?;
    stage.write_all(previous)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    match stage.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => check_existing(),
        Err(error) => Err(error.error.into()),
    }
}

#[cfg(test)]
mod analysis_backup_tests {
    use super::*;

    #[test]
    fn recovery_archive_is_idempotent_and_never_overwrites_existing_history() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("bass-original.json");
        let previous = b"complete original events, including source evidence";
        preserve_analysis_backup(&archive, previous).unwrap();
        assert_eq!(fs::read(&archive).unwrap(), previous);
        preserve_analysis_backup(&archive, previous).unwrap();
        assert!(preserve_analysis_backup(&archive, b"different events").is_err());
        assert_eq!(fs::read(&archive).unwrap(), previous);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_backup_publication_keeps_current_events_and_existing_history() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("events.json");
        fs::write(&current, b"current events").unwrap();
        let obstructed = directory.path().join("backup.json");
        fs::create_dir(&obstructed).unwrap();
        assert!(preserve_analysis_backup(&obstructed, b"old events").is_err());
        assert!(obstructed.is_dir());
        assert_eq!(fs::read(current).unwrap(), b"current events");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}

fn validate_vocal_cache(
    report: &crate::vocal_generation::Generation,
    audio_sha256: &str,
    stem_sha256: &str,
    duration: f64,
    timing: &TempoMap,
) -> Result<()> {
    ensure!(
        report.audio_sha256.eq_ignore_ascii_case(audio_sha256)
            && report.vocal_stem_sha256.eq_ignore_ascii_case(stem_sha256)
            && (report.duration_seconds - duration).abs() <= 1. / 44100.,
        "Saved vocal proposals describe a different recording, vocal stem or sample clock; the proposal is retained. Regenerate against the current inputs."
    );
    ensure!(
        report.timing.as_ref() == Some(timing),
        "Saved vocal proposals have a different or unknown tempo/offset map. Reuse source-time words, then regenerate notes for this job; the existing proposal is retained."
    );
    ensure!(
        report.algorithm == crate::vocal_generation::ALGORITHM,
        "Saved vocal proposals use an earlier pitch algorithm; regenerate into a new proposal before reuse"
    );
    report.policy.validate()?;
    ensure!(
        serde_json::to_value(&report.policy)?
            == serde_json::to_value(crate::vocal_generation::Policy::default())?,
        "Saved vocal proposals use different pitch/recovery settings; retain the proposal and regenerate for the current job settings"
    );
    if let Some(evidence) = &report.vocal_pitch {
        crate::vocal_generation::validate_vocal_frame_evidence(
            evidence,
            audio_sha256,
            stem_sha256,
            duration,
        )?;
    }
    let neural = report.neural_pitch.as_ref().context(
        "Saved vocal proposals do not include the requested neural pitch analysis; regenerate with neural pitch assistance",
    )?;
    let asset = models::assets()
        .into_iter()
        .find(|asset| asset.id == "pitch")
        .context("Pitch model is absent from the model catalogue")?;
    ensure!(
        neural.audio_sha256.eq_ignore_ascii_case(audio_sha256)
            && neural.vocal_stem_sha256.eq_ignore_ascii_case(stem_sha256)
            && (neural.duration_seconds - duration).abs() <= 1. / 44100.
            && neural.model_sha256 == asset.sha256
            && neural.implementation_sha256
                == crate::vocal_generation::neural_pitch_implementation_hash(),
        "Saved neural vocal pitch evidence is stale for this source or implementation"
    );
    Ok(())
}

pub fn status(root: &Path) -> Result<Status> {
    Ok(serde_json::from_slice(&fs::read(root.join("job.json"))?)?)
}

pub fn create(mut request: Request, output: &Path) -> Result<PathBuf> {
    request.validate()?;
    let Source::Audio { path } = &mut request.source;
    *path = path.canonicalize()?;
    // Preserve an absolute model location when the worker is launched from Finder.
    if request.model_dir.is_relative() {
        request.model_dir = std::env::current_dir()?.join(&request.model_dir);
    }
    pipeline::destination(output, None)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    write_json(&stage.path().join("request.json"), &request)?;
    write_json(
        &stage.path().join("job.json"),
        &Status {
            version: 1,
            state: State::Queued,
            stage: "queued".into(),
            detail: "Ready to generate".into(),
            request_sha256: models::hash_file(&stage.path().join("request.json"))?,
            error: None,
        },
    )?;
    pipeline::publish(stage.path(), output)?;
    Ok(output.canonicalize()?)
}

#[derive(Serialize, Deserialize)]
struct Input {
    filename: String,
    sha256: String,
    metadata: pipeline::Metadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original: Option<(String, String)>,
}

fn update(
    root: &Path,
    state: State,
    stage: &str,
    detail: impl Into<String>,
    error: Option<String>,
) -> Result<()> {
    let mut current = status(root)?;
    current.state = state;
    current.stage = stage.into();
    current.detail = detail.into();
    current.error = error;
    write_json(&root.join("job.json"), &current)
}

/// Only called after the child has exited; the same lock prevents cancelling a
/// still-running headless worker or racing another window's recovery operation.
pub fn mark_interrupted(root: &Path, cancelled: bool) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("worker.lock"))?;
    lock.try_lock().context("Generation is still running")?;
    let current = status(root)?;
    if current.state != State::Complete {
        update(
            root,
            if cancelled {
                State::Cancelled
            } else {
                State::Failed
            },
            "stopped",
            "Saved inputs are retained. Resume this job to continue.",
            (!cancelled).then(|| {
                "The generation worker stopped unexpectedly. See worker.log for details.".into()
            }),
        )?;
    }
    Ok(())
}

pub fn run(root: &Path) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("worker.lock"))?;
    lock.try_lock()
        .context("This generation job is already running")?;
    let current = status(root)?;
    ensure!(current.version == 1, "Unsupported job version");
    ensure!(
        current.request_sha256 == models::hash_file(&root.join("request.json"))?,
        "Generation settings changed on disk. Create a new job instead of resuming."
    );
    if current.state == State::Complete {
        Store::new(&root.join("project")).load()?;
        return Ok(());
    }
    let result = run_inner(root);
    if let Err(error) = &result {
        let current = status(root)?;
        update(
            root,
            State::Failed,
            &current.stage,
            "Generation stopped. Inputs and completed stages are retained for retry.",
            Some(format!("{error:#}")),
        )?;
    }
    result
}

fn run_inner(root: &Path) -> Result<()> {
    let request: Request = serde_json::from_slice(&fs::read(root.join("request.json"))?)?;
    // Use the saved input on retry. The original file may have moved or been deleted.
    TempoMap::constant(request.bpm)?;
    update(
        root,
        State::Running,
        "import",
        "Importing the recording",
        None,
    )?;
    let input_dir = root.join("input");
    if !input_dir.exists() {
        let stage = tempfile::tempdir_in(root)?;
        let mut filename = match &request.source {
            Source::Audio { path } => {
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .context("Audio file needs an extension")?;
                ensure!(
                    ext.bytes().all(|b| b.is_ascii_alphanumeric()) && ext.len() <= 10,
                    "Unsupported audio file extension"
                );
                let filename = format!("source.{}", ext.to_ascii_lowercase());
                let before = models::hash_file(path)?;
                fs::copy(path, stage.path().join(&filename))?;
                ensure!(
                    models::hash_file(&stage.path().join(&filename))? == before
                        && models::hash_file(path)? == before,
                    "The audio file changed during import. Retry after it finishes saving."
                );
                filename
            }
        };
        let original = if !matches!(
            Path::new(&filename).extension().and_then(|s| s.to_str()),
            Some("mp3" | "ogg" | "opus" | "wav")
        ) {
            update(
                root,
                State::Running,
                "decode",
                "Preparing game-compatible audio · the original import is retained",
                None,
            )?;
            let original = (
                filename.clone(),
                models::hash_file(&stage.path().join(&filename))?,
            );
            let encoded = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(stage.path().join(&filename))
                .args([
                    "-map",
                    "0:a:0",
                    "-vn",
                    "-c:a",
                    "pcm_s16le",
                    "-ar",
                    "44100",
                    "-ac",
                    "2",
                    "-n",
                ])
                .arg(stage.path().join("source.wav"))
                .output()
                .context("FFmpeg is required to import this audio format")?;
            ensure!(
                encoded.status.success(),
                "Cannot prepare imported audio: {}",
                String::from_utf8_lossy(&encoded.stderr)
            );
            filename = "source.wav".into();
            Some(original)
        } else {
            None
        };
        let audio = stage.path().join(&filename);
        let metadata = pipeline::metadata(&audio)?;
        write_json(
            &stage.path().join("source.json"),
            &Input {
                filename,
                sha256: models::hash_file(&audio)?,
                metadata,
                original,
            },
        )?;
        if let Some(artwork) = request
            .song_info
            .as_ref()
            .and_then(|info| info.artwork.as_ref())
        {
            let name = if artwork.extension().is_some_and(|ext| ext == "png") {
                "album.png"
            } else {
                "album.jpg"
            };
            fs::copy(artwork, stage.path().join(name))
                .context("Imported artwork is unavailable; import the recording again")?;
        }
        pipeline::publish(stage.path(), &input_dir)?;
    }
    let input: Input = serde_json::from_slice(&fs::read(input_dir.join("source.json"))?)?;
    ensure!(
        Path::new(&input.filename)
            .file_name()
            .is_some_and(|n| n == input.filename.as_str()),
        "Invalid imported source filename"
    );
    let audio = input_dir.join(&input.filename);
    ensure!(
        models::hash_file(&audio)? == input.sha256,
        "Imported source changed. Start a new job with the new recording."
    );
    let events_path = root.join("events.json");
    if !events_path.exists() && !root.join("song").exists() {
        update(
            root,
            State::Running,
            "models",
            "Verifying local transcription models",
            None,
        )?;
        let mut wanted = crate::keyboard_detection::required_models_with_quality(
            &request.instruments,
            request.keyboard_sound,
            request.piano_quality,
            request.full_mix_drums,
        );
        if request.fetch_models
            && request
                .instruments
                .iter()
                .any(|instrument| instrument == "vocals")
        {
            wanted.push("vocal-pitch");
        }
        if request.fetch_models
            && !request.full_mix_drums
            && request
                .instruments
                .iter()
                .any(|instrument| instrument == "drums")
        {
            wanted.push("drum-articulation");
        }
        if request.fetch_models {
            let standard: Vec<_> = wanted
                .iter()
                .copied()
                .filter(|id| *id != crate::keyboard_separation::MODEL_ID)
                .collect();
            models::fetch_required(&request.model_dir, &standard)?;
            if wanted.contains(&crate::keyboard_separation::MODEL_ID) {
                crate::keyboard_separation::fetch(&request.model_dir)?;
            }
        }
        for id in wanted {
            if id == crate::keyboard_separation::MODEL_ID {
                crate::keyboard_separation::verified(&request.model_dir)?;
            } else {
                models::verified(&request.model_dir, id)?;
            }
        }
        let manifest = if !request.full_mix_drums {
            update(
                root,
                State::Running,
                "separation",
                "Separating instruments · existing matching stems are reused",
                None,
            )?;
            let plan = pipeline::prepare_keyboard_isolation(
                &audio,
                &request.model_dir,
                &root
                    .parent()
                    .context("Job needs a parent folder")?
                    .join(".chart-engine-stem-cache"),
                &request.instruments,
                request.keyboard_sound,
                request.piano_quality,
            )?;
            if let Some(analysis) = &plan.keyboard_analysis {
                write_json(&root.join("keyboard-analysis-isolation.json"), analysis)?;
            }
            let path = root.join("isolation.json");
            write_json(&path, &plan.primary)?;
            Some(path)
        } else {
            None
        };
        update(
            root,
            State::Running,
            "transcription",
            format!("Transcribing {}", request.instruments.join(", ")),
            None,
        )?;
        let keyboard_analysis_path = root.join("keyboard-analysis-isolation.json");
        let transcription = pipeline::transcribe_with_isolation(
            &audio,
            &request.model_dir,
            IsolationOptions {
                guitar_source: request.guitar_source,
                manifest: manifest.as_deref(),
                full_mix_drums: request.full_mix_drums,
                keyboard_sound: request.keyboard_sound,
                piano_quality: request.piano_quality,
                keyboard_analysis: keyboard_analysis_path
                    .is_file()
                    .then_some(keyboard_analysis_path.as_path()),
                ..Default::default()
            },
            &request
                .instruments
                .iter()
                .filter(|s| s.as_str() != "vocals")
                .cloned()
                .collect::<Vec<_>>(),
        )?;
        write_json(&events_path, &transcription)?;
    }
    if !root.join("song").exists() {
        update(
            root,
            State::Running,
            "chart",
            "Arranging difficulties and exporting the chart draft",
            None,
        )?;
        let event_bytes = fs::read(&events_path)?;
        let mut transcription: Transcription = serde_json::from_slice(&event_bytes)?;
        ensure!(
            !request.instruments.iter().any(|i| i == "guitar")
                || transcription.guitar_source == request.guitar_source,
            "Saved Guitar candidates use a different sound source; retain them and start a new generation job"
        );
        if request
            .instruments
            .iter()
            .any(|instrument| instrument == "bass")
        {
            let manifest = transcription
                .isolation
                .as_ref()
                .context("Resumed bass analysis requires source-bound isolated audio")?;
            manifest.validate_source(
                &audio,
                &input.sha256,
                pipeline::metadata(&audio)?.duration,
            )?;
            let stem = manifest
                .stems
                .iter()
                .find(|stem| stem.id == crate::stem_review::StemId::Bass)
                .context("Resumed bass analysis needs a bound bass stem")?
                .path
                .clone();
            let decoder_current = crate::bass_analysis::decoder_is_current(&transcription, &stem)?;
            if !decoder_current || !crate::bass_analysis::is_current(&transcription, &stem)? {
                let previous = event_bytes;
                let mut refreshed = transcription.clone();
                if decoder_current {
                    crate::bass_analysis::analyze(&mut refreshed, &stem)?;
                } else {
                    crate::bass_analysis::infer(
                        &mut refreshed,
                        &stem,
                        &models::verified(&request.model_dir, "pitch")?,
                    )?;
                }
                let history = root.join("analysis-history");
                fs::create_dir_all(&history)?;
                let backup =
                    history.join(format!("bass-{:x}.json", sha2::Sha256::digest(&previous)));
                preserve_analysis_backup(&backup, &previous)?;
                ensure!(
                    fs::read(&events_path)? == previous,
                    "Bass events changed during refresh; previous analysis retained"
                );
                write_json(&events_path, &refreshed)?;
                transcription = refreshed;
            }
        }
        let mut meta = input.metadata;
        meta.name = request.title.trim().into();
        meta.artist = request.artist.trim().into();
        if let Some(info) = &request.song_info {
            meta.album = info.album.clone();
            meta.year = info.year.clone();
            meta.genre = info.genre.clone();
        }
        pipeline::build_with_lyrics(
            Build {
                audio: &audio,
                output: &root.join("song"),
                existing: None,
                timing: Some(TempoMap::constant(request.bpm)?),
                transcription: &transcription,
                meta,
                pro: !request.standard_drums,
                lookup_lyrics: (request.lyrics
                    || request.instruments.iter().any(|s| s == "vocals"))
                    && request
                        .song_info
                        .as_ref()
                        .is_none_or(|info| !info.lyrics_checked || info.lyrics.is_some()),
            },
            request
                .song_info
                .as_ref()
                .and_then(|info| info.lyrics.as_ref()),
        )?;
    }
    for name in ["album.jpg", "album.png"] {
        if input_dir.join(name).is_file() && !root.join("song").join(name).exists() {
            fs::copy(input_dir.join(name), root.join("song").join(name))?;
        }
    }
    if let Some(info) = &request.song_info {
        write_json(&root.join("song/import-information.json"), info)?;
    }
    if request.instruments.iter().any(|s| s == "vocals") {
        update(
            root,
            State::Running,
            "vocals",
            "Estimating sung pitches and aligning lyrics",
            None,
        )?;
        let vocal_dir = root.join("vocals");
        let audio_hash = models::hash_file(&audio)?;
        let result = if vocal_dir.exists() {
            let saved_language = vocal_dir.join("recognition-language.json");
            let previous_language: crate::lyric_language::Language = if saved_language.is_file() {
                serde_json::from_slice(&fs::read(saved_language)?)?
            } else {
                crate::lyric_language::Language::English
            };
            ensure!(
                previous_language == request.vocal_language,
                "Saved vocals use a different lyric language; retain them and start a new generation job"
            );
            let result: crate::vocal_generation::Generation =
                serde_json::from_slice(&fs::read(vocal_dir.join("generation.json"))?)?;
            let events: Transcription = serde_json::from_slice(&fs::read(&events_path)?)?;
            let isolation = events
                .isolation
                .as_ref()
                .context("Cached vocal proposals require the current source-bound stems")?;
            isolation.validate_binding(&audio_hash, pipeline::metadata(&audio)?.duration)?;
            let stem = isolation
                .stems
                .iter()
                .find(|stem| stem.id == crate::stem_review::StemId::Vocals)
                .context("Cached vocal proposals require a vocal stem")?;
            ensure!(
                models::hash_file(&stem.path)?.eq_ignore_ascii_case(&stem.sha256),
                "The vocal stem changed after transcription; cached proposals were retained"
            );
            validate_vocal_cache(
                &result,
                &audio_hash,
                &stem.sha256,
                isolation.frames as f64 / isolation.sample_rate as f64,
                &TempoMap::constant(request.bpm)?,
            )?;
            if request.fetch_models || models::verified(&request.model_dir, "vocal-pitch").is_ok() {
                ensure!(
                    result.vocal_pitch.is_some(),
                    "Saved vocal proposals lack the available/requested optional vocal pitch model; retain the proposal and regenerate into a new job before reuse"
                );
            }
            result
        } else {
            let lyrics_path = root.join("song/chart-engine-lyrics.json");
            let value: Option<serde_json::Value> = lyrics_path
                .exists()
                .then(|| -> Result<_> { Ok(serde_json::from_slice(&fs::read(&lyrics_path)?)?) })
                .transpose()?;
            let lyrics = value.as_ref().and_then(|v| {
                v["plainLyrics"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|text| crate::vocal_generation::LyricSource {
                        text: text.into(),
                        synced_lyrics: v["syncedLyrics"]
                            .as_str()
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_owned),
                        source: v["id"].as_u64().map_or_else(
                            || "LRCLIB matched recording".into(),
                            |id| format!("https://lrclib.net/api/get/{id}"),
                        ),
                    })
            });
            crate::vocal_workflow::generate(
                crate::vocal_workflow::Request {
                    audio: &audio,
                    manifest: &root.join("isolation.json"),
                    timing: &TempoMap::constant(request.bpm)?,
                    words: None,
                    lyrics: lyrics.as_ref(),
                    recognize: true,
                    language: request.vocal_language,
                    neural_pitch: true,
                    model_dir: &request.model_dir,
                    download_model: request.fetch_models,
                },
                &vocal_dir,
                std::sync::Arc::new(crate::clean_recognition::Control::default()),
            )?
        };
        // The .chart remains the working master. The editor exports a single
        // MIDI containing its current instrument notes plus this vocal document.
        write_json(
            &root.join("song/chart-engine-vocal-generation.json"),
            &result,
        )?;
        if let Some(chart) = &result.chart {
            chart.validate()?;
            let instrument_chart = crate::chart::Chart::parse(fs::read_to_string(
                pipeline::editable_chart_file(&root.join("song"))?,
            )?)?;
            crate::game_export::song_with_vocals(&instrument_chart, chart)?;
            write_json(&root.join("song/chart-engine-vocals.json"), chart)?;
        }
    }
    update(
        root,
        State::Running,
        "editor",
        "Preparing the editable review project",
        None,
    )?;
    if !root.join("project").exists() {
        Store::create(&root.join("song"), None, &root.join("project"))?;
    }
    let store = Store::new(&root.join("project"));
    let mut project = store.load()?;
    let vocal_report = root.join("song/chart-engine-vocal-generation.json");
    if project.content.vocal_review.is_none() && vocal_report.is_file() {
        let report: crate::vocal_generation::Generation =
            serde_json::from_slice(&fs::read(&vocal_report)?)?;
        // A reviewer may have edited the project before this retry.
        // Attach diagnostics only if it still matches the generated document.
        if let Some(document) = report.chart.as_ref().filter(|document| {
            report.timing.is_some() && project.content.vocals.as_ref() == Some(*document)
        }) {
            store.apply(crate::editor::Request {
                expected_revision: project.revision,
                actor: crate::editor::Actor {
                    kind: crate::editor::ActorKind::Ai,
                    name: "YACE vocal diagnostics".into(),
                },
                action: crate::editor::Action::SetVocalGeneration {
                    document: document.clone(),
                    review: crate::vocal_review::VocalReviewMap::from_generation(&report)?,
                },
            })?;
            project = store.load()?;
        }
    }
    if (project.content.vocals.is_some()
        || project
            .content
            .tracks
            .keys()
            .any(|t| crate::keys::is_pro(t)))
        && !root.join("game-song").exists()
    {
        update(
            root,
            State::Running,
            "game_export",
            "Exporting instruments for the game",
            None,
        )?;
        store.apply(crate::editor::Request {
            expected_revision: project.revision,
            actor: crate::editor::Actor {
                kind: crate::editor::ActorKind::Ai,
                name: "YACE generation".into(),
            },
            action: crate::editor::Action::Export {
                output: root.join("game-song"),
            },
        })?;
    }
    update(
        root,
        State::Complete,
        "complete",
        "Draft ready. Listen, review and edit before exporting.",
        None,
    )
}

#[cfg(test)]
mod vocal_cache_tests {
    use super::*;
    use crate::vocal_generation::{Generation, NeuralPitchEvidence, Policy, Status};

    fn report() -> Generation {
        let model = models::assets()
            .into_iter()
            .find(|asset| asset.id == "pitch")
            .unwrap();
        Generation {
            version: 1,
            audio_sha256: "a".repeat(64),
            vocal_stem_sha256: "b".repeat(64),
            duration_seconds: 5.,
            timing: Some(TempoMap::constant(125.).unwrap()),
            algorithm: crate::vocal_generation::ALGORITHM.into(),
            policy: Policy::default(),
            status: Status::NeedsLyrics,
            chart: None,
            pitch_candidates: vec![],
            pitch_frames: vec![],
            vocal_pitch: None,
            words: None,
            lyrics: None,
            issues: vec![],
            warnings: vec![],
            neural_pitch: Some(NeuralPitchEvidence {
                audio_sha256: "a".repeat(64),
                vocal_stem_sha256: "b".repeat(64),
                duration_seconds: 5.,
                model_sha256: model.sha256,
                implementation_sha256: crate::vocal_generation::neural_pitch_implementation_hash(),
                notes: vec![],
            }),
        }
    }

    fn check(report: &Generation) -> Result<()> {
        validate_vocal_cache(
            report,
            &"a".repeat(64),
            &"b".repeat(64),
            5.,
            &TempoMap::constant(125.)?,
        )
    }

    #[test]
    fn cached_ticks_cannot_cross_tempo_offset_resolution_or_unknown_maps() {
        let original = report();
        assert!(check(&original).is_ok());
        let mut changed = original.clone();
        changed.timing = Some(TempoMap::constant(175.).unwrap());
        assert!(check(&changed).is_err());
        changed = original.clone();
        changed.timing.as_mut().unwrap().offset = -0.5;
        assert!(check(&changed).is_err());
        changed = original.clone();
        changed.timing.as_mut().unwrap().resolution = 192;
        assert!(check(&changed).is_err());
        changed = original;
        changed.timing = None;
        assert!(check(&changed).is_err());
    }

    #[test]
    fn cached_mode_cannot_claim_neural_analysis_or_reuse_changed_sources() {
        let original = report();
        for changed in 0..7 {
            let mut report = original.clone();
            match changed {
                0 => report.vocal_stem_sha256 = "c".repeat(64),
                1 => report.duration_seconds += 0.01,
                2 => report.algorithm = "older-analysis".into(),
                3 => report.neural_pitch = None,
                4 => report.neural_pitch.as_mut().unwrap().model_sha256 = "c".repeat(64),
                5 => report.neural_pitch.as_mut().unwrap().implementation_sha256 = "c".repeat(64),
                6 => report.policy.minimum_note_seconds = 0.5,
                _ => unreachable!(),
            }
            assert!(
                check(&report).is_err(),
                "Accepted stale cache case {changed}"
            );
        }
    }

    #[test]
    fn cached_vocal_frames_cannot_reuse_another_model_source_or_frame_clock() {
        let mut original = report();
        original.vocal_pitch = Some(crate::vocal_pitch::Evidence {
            version: 1,
            model_id: crate::vocal_pitch::MODEL_ID.into(),
            model_revision: crate::vocal_pitch::REVISION.into(),
            model_sha256: crate::vocal_pitch::MODEL_SHA256.into(),
            implementation_sha256: crate::vocal_pitch::implementation_hash(),
            audio_sha256: original.audio_sha256.clone(),
            vocal_stem_sha256: original.vocal_stem_sha256.clone(),
            pcm_sha256: "c".repeat(64),
            duration_seconds: 5.,
            sample_rate: 16000,
            hop_samples: 256,
            frames: (0..312)
                .map(|i| crate::vocal_pitch::Frame {
                    time_seconds: i as f64 * 0.016,
                    frequency_hz: Some(220.),
                    confidence: 0.9,
                    rms: 0.1,
                })
                .collect(),
        });
        assert!(check(&original).is_ok());
        for changed in 0..6 {
            let mut report = original.clone();
            let evidence = report.vocal_pitch.as_mut().unwrap();
            match changed {
                0 => evidence.implementation_sha256 = "0".repeat(64),
                1 => evidence.model_sha256 = "0".repeat(64),
                2 => evidence.vocal_stem_sha256 = "0".repeat(64),
                3 => evidence.frames[1].time_seconds += 0.001,
                4 => evidence.frames[1].confidence = f32::NAN,
                5 => {
                    evidence.frames.pop();
                }
                _ => unreachable!(),
            }
            assert!(
                check(&report).is_err(),
                "Accepted stale vocal frame case {changed}"
            );
        }
    }
}
