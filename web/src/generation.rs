//! Browser working-set operations for the shared generation job protocol.
pub use crate::generation_protocol::{Request, Source, State, Status};
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Separate,
    Keyboard,
    Transcribe,
    Vocals,
    Build,
}
impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Separate => "Separate vocals and instruments",
            Self::Keyboard => "Separate higher-quality piano",
            Self::Transcribe => "Detect instrument parts",
            Self::Vocals => "Generate vocals and lyrics",
            Self::Build => "Build review project",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Input {
    pub version: u32,
    pub audio_sha256: String,
    pub metadata: crate::pipeline::Metadata,
    pub implementation_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<crate::vocal_generation::LyricSource>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub frames: Option<u64>,
    pub instruments: std::collections::BTreeMap<String, usize>,
    pub vocal_notes: usize,
    pub vocal_issues: usize,
}
pub fn summary(root: &Path) -> Result<Summary> {
    if crate::storage::is_file(root.join("summary.json")) {
        Ok(serde_json::from_slice(&crate::storage::read(
            root.join("summary.json"),
        )?)?)
    } else {
        Ok(Summary::default())
    }
}
pub fn implementation_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for bytes in [
        include_bytes!("generation.rs").as_slice(),
        include_bytes!("transcription.rs"),
        include_bytes!("vocal_inference.rs"),
        include_bytes!("recognition.rs"),
        include_bytes!("acoustic_alignment.rs"),
        include_bytes!("../../src/vocal_alignment.rs"),
        include_bytes!("../../vendor/practice-acoustic/src/lib.rs"),
        include_bytes!("../runtime/yace-recognition.wasm"),
        include_bytes!("../../src/clean_transcript.rs"),
        include_bytes!("onnx.rs"),
        include_bytes!("synth.rs"),
        include_bytes!("keyboard_separation.rs"),
        include_bytes!("../../src/keyboard_separation_dsp.rs"),
        include_bytes!("../../src/keyboard_separation_asset.rs"),
        include_bytes!("../../src/muscriptor.rs"),
        include_bytes!("../../src/lyric_language.rs"),
        include_bytes!("audio_decode.rs"),
        include_bytes!("audio_resample.rs"),
        include_bytes!("../../models/manifest.json"),
        include_bytes!("../Cargo.lock"),
        include_bytes!("../../src/models/postprocessing.rs"),
        include_bytes!("../../src/generation_build.rs"),
        include_bytes!("../../src/vocal_generation.rs"),
        include_bytes!("../../src/vocal_pitch.rs"),
        include_bytes!("../../src/piano_model.rs"),
        include_bytes!("../../src/keyboard_detection.rs"),
        include_bytes!("../../src/keyboard_audio.rs"),
        include_bytes!("../../src/bass_analysis.rs"),
        include_bytes!("../../src/refine.rs"),
        include_bytes!("../../src/mapping.rs"),
        include_bytes!("../../src/difficulty_links.rs"),
        include_bytes!("../../src/generated_rules.rs"),
    ] {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.update(crate::separation::implementation_hash());
    format!("{:x}", hash.finalize())
}
pub fn freeze(root: &Path, metadata: crate::pipeline::Metadata) -> Result<()> {
    let request = read_request(root)?;
    let Source::Audio { path } = &request.source;
    ensure!(
        metadata.name == request.title
            && metadata.artist == request.artist
            && metadata.duration.is_finite()
            && metadata.duration > 0.,
        "Generation metadata differs from the frozen source settings"
    );
    write_json(
        &root.join("browser-job-input.json"),
        &Input {
            version: 1,
            audio_sha256: crate::models::hash_file(path)?,
            metadata,
            implementation_sha256: implementation_hash(),
            lyrics: crate::storage::is_file(root.join("supplied-lyrics.json"))
                .then(|| -> Result<_> {
                    Ok(serde_json::from_slice(&crate::storage::read(
                        root.join("supplied-lyrics.json"),
                    )?)?)
                })
                .transpose()?,
        },
    )
}
pub fn reuse_stems(root: &Path, manifest: &crate::stem_review::StemManifest) -> Result<bool> {
    let input = input(root)?;
    let request = read_request(root)?;
    let expected = if hq_only(&request) {
        crate::keyboard_separation::MODEL_ID
    } else {
        "separation"
    };
    let pinned = crate::models::assets()
        .into_iter()
        .find(|asset| asset.id == expected)
        .context("Missing separator identity")?;
    if manifest.ids() != crate::stem_review::StemId::ALL.into_iter().collect()
        || !manifest
            .generation
            .as_ref()
            .is_some_and(|model| model.model_id == expected && model.model_sha256 == pinned.sha256)
    {
        return Ok(false);
    }
    manifest.validate_binding(&input.audio_sha256, input.metadata.duration)?;
    let mut copied = manifest.clone();
    for stem in &mut copied.stems {
        let path = root.join("stems").join(format!("{}.wav", stem.id.as_str()));
        crate::storage::copy(&stem.path, &path)?;
        stem.path = path;
    }
    write_json(&root.join("supplied-isolation.json"), &copied)?;
    Ok(true)
}
pub fn input(root: &Path) -> Result<Input> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("browser-job-input.json"),
    )?)?)
}
pub fn validate(root: &Path) -> Result<(Request, Input)> {
    let (request, input) = validate_saved(root)?;
    ensure!(
        !hq(&request),
        "Higher quality piano separation is currently desktop-only: its checkpoint failed local browser runtime verification. Standard piano and Synth run locally in this browser"
    );
    ensure!(
        input.implementation_sha256 == implementation_hash(),
        "This saved job uses an earlier generator. Its completed results remain available; start a new job to generate with the current engine"
    );
    Ok((request, input))
}
/// A newer engine must not make retained completed drafts inaccessible.
pub fn validate_saved(root: &Path) -> Result<(Request, Input)> {
    let request = read_request(root)?;
    request.validate()?;
    let state = status(root)?;
    let input = input(root)?;
    ensure!(
        state.version == 1
            && state.request_sha256 == crate::models::hash_file(&root.join("request.json"))?,
        "Saved generation settings changed; retained results were kept"
    );
    ensure!(
        input.version == 1,
        "Unsupported saved generation input version"
    );
    let Source::Audio { path } = &request.source;
    ensure!(
        crate::storage::canonicalize(path)?.starts_with(crate::storage::canonicalize(root)?)
            && crate::models::hash_file(path)? == input.audio_sha256,
        "Saved generation recording changed"
    );
    ensure!(
        request.title == input.metadata.name && request.artist == input.metadata.artist,
        "Saved source metadata changed"
    );
    if let Some(lyrics) = &input.lyrics {
        ensure!(
            !lyrics.source.trim().is_empty() && !lyrics.text.trim().is_empty(),
            "Frozen lyrics need text and provenance"
        );
    }
    Ok((request, input))
}
pub fn isolation(root: &Path) -> Result<crate::stem_review::StemManifest> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("isolation.json"),
    )?)?)
}
pub fn next(root: &Path) -> Result<Option<Stage>> {
    let (request, _) = validate(root)?;
    if !request.full_mix_drums && !crate::storage::is_file(root.join("isolation.json")) {
        return Ok(Some(Stage::Separate));
    }
    if hq(&request)
        && !hq_only(&request)
        && !crate::storage::is_file(root.join("keyboard-isolation.json"))
    {
        return Ok(Some(Stage::Keyboard));
    }
    if !crate::storage::is_file(root.join("transcription.json")) {
        return Ok(Some(Stage::Transcribe));
    }
    if request.instruments.iter().any(|part| part == "vocals")
        && !crate::storage::is_file(root.join("vocals/generation.json"))
    {
        return Ok(Some(Stage::Vocals));
    }
    if status(root)?.state != State::Complete
        || !crate::storage::is_file(project_path(root).join("project.json"))
    {
        return Ok(Some(Stage::Build));
    }
    Ok(None)
}
fn hq(request: &Request) -> bool {
    crate::keyboard_detection::high_quality_piano(
        &request.instruments,
        request.keyboard_sound,
        request.piano_quality,
    )
}
fn hq_only(request: &Request) -> bool {
    hq(request)
        && request
            .instruments
            .iter()
            .all(|part| matches!(part.as_str(), "piano" | "pro_piano"))
}
pub fn audio_paths(root: &Path, stage: Stage) -> Result<Vec<PathBuf>> {
    let request = read_request(root)?;
    let Source::Audio { path } = request.source;
    let mut paths = vec![path];
    if stage == Stage::Separate && crate::storage::is_file(root.join("supplied-isolation.json")) {
        let manifest: crate::stem_review::StemManifest =
            serde_json::from_slice(&crate::storage::read(root.join("supplied-isolation.json"))?)?;
        paths.extend(manifest.stems.into_iter().map(|stem| stem.path));
    }
    if stage != Stage::Separate && !request.full_mix_drums {
        paths.extend(
            isolation(root)?
                .stems
                .into_iter()
                .filter(|stem| match stage {
                    Stage::Separate => false,
                    Stage::Keyboard => false,
                    Stage::Build => true,
                    Stage::Vocals => stem.id == crate::stem_review::StemId::Vocals,
                    Stage::Transcribe => request.instruments.iter().any(|part| {
                        let name = if part == "guitar" {
                            request.guitar_source.stem().as_str()
                        } else if part == "pro_piano" {
                            "piano"
                        } else {
                            part.as_str()
                        };
                        name == stem.id.as_str()
                            || ((part == "piano" || part == "pro_piano")
                                && ["drums", "bass", "vocals"].contains(&stem.id.as_str()))
                    }),
                })
                .map(|stem| stem.path),
        );
    }
    if matches!(stage, Stage::Transcribe | Stage::Build)
        && crate::storage::is_file(root.join("keyboard-isolation.json"))
    {
        let manifest: crate::stem_review::StemManifest =
            serde_json::from_slice(&crate::storage::read(root.join("keyboard-isolation.json"))?)?;
        paths.extend(
            manifest
                .stems
                .into_iter()
                .filter(|stem| {
                    stage == Stage::Build
                        || ["piano", "drums", "bass", "vocals"].contains(&stem.id.as_str())
                })
                .map(|stem| stem.path),
        );
    }
    Ok(paths)
}
pub fn update(
    root: &Path,
    state: State,
    stage: Stage,
    detail: String,
    error: Option<String>,
) -> Result<()> {
    let mut status = status(root)?;
    status.state = state;
    status.stage = format!("{stage:?}").to_ascii_lowercase();
    status.detail = detail;
    status.error = error;
    write_json(&root.join("job.json"), &status)
}
pub async fn checkpoint(root: &Path, expected: Option<u64>) -> Result<u64> {
    let revision = expected.map_or(0, |revision| revision + 1);
    crate::persistence::save_job(root, revision, expected).await?;
    Ok(revision)
}

pub async fn run_stage(
    root: &Path,
    stage: Stage,
    progress: &mut impl FnMut(String),
) -> Result<Status> {
    let (request, input) = validate(root)?;
    let Source::Audio { path: audio } = &request.source;
    ensure!(
        (crate::models::duration(audio)? - input.metadata.duration).abs() <= 1. / 44100.,
        "Saved recording duration changed"
    );
    update(root, State::Running, stage, stage.label().into(), None)?;
    let mut summary = summary(root)?;
    match stage {
        Stage::Separate => {
            let cached = root.join("supplied-isolation.json");
            let manifest = if crate::storage::is_file(&cached) {
                progress("Checking the chart’s source-bound separated audio…".into());
                let manifest: crate::stem_review::StemManifest =
                    serde_json::from_slice(&crate::storage::read(cached)?)?;
                drop(manifest.load(audio)?);
                crate::models::remember_stems(&manifest)?;
                let mut portable = manifest.clone();
                for stem in &mut portable.stems {
                    stem.path = stem.path.strip_prefix(root.join("stems"))?.to_owned();
                }
                write_json(&root.join("stems/stem-manifest.json"), &portable)?;
                manifest
            } else if hq_only(&request) {
                crate::keyboard_separation::run(audio, &root.join("stems"), progress).await?
            } else {
                crate::separation::run(audio, &root.join("stems"), progress).await?
            };
            summary.frames = Some(manifest.frames);
            write_json(&root.join("isolation.json"), &manifest)?;
        }
        Stage::Keyboard => {
            let manifest =
                crate::keyboard_separation::run(audio, &root.join("keyboard-stems"), progress)
                    .await?;
            write_json(&root.join("keyboard-isolation.json"), &manifest)?;
        }
        Stage::Transcribe => {
            let wanted: Vec<_> = request
                .instruments
                .iter()
                .filter(|part| part.as_str() != "vocals")
                .cloned()
                .collect();
            let events = if request.full_mix_drums {
                let (drums, drum_recovered_hits) =
                    crate::transcription::drums(audio, progress).await?;
                crate::mapping::Transcription{drums,drum_recovered_hits,warnings:vec!["Explicit full-mix drum fallback; no isolated source was used. Listening review is required.".into()],..Default::default()}
            } else {
                let manifest = isolation(root)?;
                if wanted.is_empty() {
                    crate::mapping::Transcription {
                        isolation: Some(manifest),
                        ..Default::default()
                    }
                } else {
                    let keyboard: Option<crate::stem_review::StemManifest> =
                        if crate::storage::is_file(root.join("keyboard-isolation.json")) {
                            Some(serde_json::from_slice(&crate::storage::read(
                                root.join("keyboard-isolation.json"),
                            )?)?)
                        } else {
                            None
                        };
                    crate::transcription::instruments_with_sound(
                        audio,
                        &manifest,
                        &wanted,
                        request.guitar_source,
                        request.keyboard_sound,
                        keyboard.as_ref(),
                        progress,
                    )
                    .await?
                }
            };
            events.validate()?;
            summary.instruments = std::collections::BTreeMap::from([
                ("Drums".into(), events.drums.len()),
                ("Guitar".into(), events.guitar.len()),
                ("Bass".into(), events.bass.len()),
                ("Keys".into(), events.piano.len()),
                ("Pro Keys".into(), events.pro_piano.len()),
            ]);
            write_json(&root.join("transcription.json"), &events)?;
        }
        Stage::Vocals => {
            let result = crate::vocal_inference::generate_language(
                audio,
                &isolation(root)?,
                &request.timing()?,
                request.vocal_language,
                request.vocal_analysis_mode,
                Some(&root.join("vocals/recognition.json")),
                input.lyrics.as_ref(),
                progress,
            )
            .await?;
            summary.vocal_notes = result.chart.as_ref().map_or(0, |chart| chart.notes.len());
            summary.vocal_issues = result.issues.len();
            write_json(&root.join("vocals/generation.json"), &result)?;
        }
        Stage::Build => {
            let timing = request.timing()?;
            let events: crate::mapping::Transcription =
                serde_json::from_slice(&crate::storage::read(root.join("transcription.json"))?)?;
            let song = root.join("song");
            ensure!(
                !crate::storage::exists(&song),
                "A built song already exists; keep it and start a new job"
            );
            if let Some(manifest) = &events.isolation {
                drop(manifest.load(audio)?);
                crate::models::remember_stems(manifest)?;
            }
            if let Some(manifest) = &events.keyboard_analysis {
                drop(manifest.load(audio)?);
                crate::models::remember_stems(manifest)?;
            }
            let instrument_parts = request.instruments.iter().any(|part| part != "vocals");
            if instrument_parts {
                crate::pipeline::build(crate::pipeline::Build {
                    audio,
                    output: &song,
                    existing: None,
                    timing: Some(timing.clone()),
                    transcription: &events,
                    meta: input.metadata.clone(),
                    pro: !request.standard_drums,
                    lookup_lyrics: false,
                })?;
            } else {
                let (ini, chart) = crate::song_metadata::blank_song_with_timing(
                    &request.title,
                    &request.artist,
                    &timing,
                )?;
                crate::storage::write(song.join("song.ini"), ini)?;
                crate::storage::write(song.join("notes.chart"), chart)?;
                let extension = audio
                    .extension()
                    .and_then(|value| value.to_str())
                    .context("Missing recording format")?;
                crate::storage::copy(audio, song.join(format!("song.{extension}")))?;
            }
            for (manifest, name) in events
                .isolation
                .as_ref()
                .map(|m| (m, "chart-engine-isolation.json"))
                .into_iter()
                .chain(
                    events
                        .keyboard_analysis
                        .as_ref()
                        .map(|m| (m, "chart-engine-keyboard-isolation.json")),
                )
            {
                let mut manifest = crate::stem_review::game_audio::retain_in(&song, manifest)?;
                for stem in &mut manifest.stems {
                    stem.path = stem.path.strip_prefix(song.join(".yace/audio"))?.to_owned();
                }
                write_json(&crate::song_layout::path(&song, name), &manifest)?;
            }
            let vocal_path = root.join("vocals/generation.json");
            if let Some(lyrics) = &input.lyrics {
                write_json(
                    &crate::song_layout::path(&song, "chart-engine-lyrics.json"),
                    &serde_json::json!({"plainLyrics":lyrics.text,"syncedLyrics":lyrics.synced_lyrics,"source":lyrics.source}),
                )?;
            }
            if crate::storage::is_file(&vocal_path) {
                let report: crate::vocal_generation::Generation =
                    serde_json::from_slice(&crate::storage::read(&vocal_path)?)?;
                ensure!(
                    report.audio_sha256 == input.audio_sha256
                        && report.timing.as_ref() == Some(&timing)
                        && report.policy.analysis_mode == request.vocal_analysis_mode,
                    "Saved vocal draft does not match the frozen source, timing or analysis mode"
                );
                write_json(
                    &crate::song_layout::path(&song, "chart-engine-vocal-generation.json"),
                    &report,
                )?;
                if let Some(chart) = &report.chart {
                    write_json(
                        &crate::song_layout::path(&song, "chart-engine-vocals.json"),
                        chart,
                    )?;
                }
            }
            let store = crate::editor::Store::new(&project_path(root));
            crate::song_files::prepare_editable_source(&song, &store.root.join("source"))?;
            store.initialize(&store.root.join("source"), None, input.metadata.duration)?;
            if crate::storage::is_file(&vocal_path) {
                let report = serde_json::from_slice(&crate::storage::read(vocal_path)?)?;
                crate::vocal_review::attach_generation(&store, store.load()?, &report)?;
            }
        }
    }
    write_json(&root.join("summary.json"), &summary)?;
    update(
        root,
        if stage == Stage::Build {
            State::Complete
        } else {
            State::Running
        },
        stage,
        format!("{} complete", stage.label()),
        None,
    )?;
    status(root)
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::storage::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
pub fn status(root: &Path) -> Result<Status> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("job.json"),
    )?)?)
}
pub fn read_request(root: &Path) -> Result<Request> {
    Ok(serde_json::from_slice(&crate::storage::read(
        root.join("request.json"),
    )?)?)
}
pub fn project_path(root: &Path) -> PathBuf {
    crate::song_layout::project_for_job(root)
}
pub fn create_job(mut request: Request, root: &Path) -> Result<PathBuf> {
    request.validate()?;
    ensure!(
        !hq(&request),
        "Higher quality piano separation is currently desktop-only; choose Standard piano or Synth"
    );
    ensure!(
        !crate::storage::exists(root),
        "Choose a new generation job; saved results are preserved"
    );
    let Source::Audio { path } = &request.source;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Missing recording filename"))?;
    let target = root.join("input").join(filename);
    crate::storage::copy(path, &target)?;
    request.source = Source::Audio { path: target };
    write_json(&root.join("request.json"), &request)?;
    write_json(
        &root.join("job.json"),
        &Status {
            version: 1,
            state: State::Queued,
            stage: "queued".into(),
            detail: "Local browser job created".into(),
            request_sha256: crate::models::hash_file(&root.join("request.json"))?,
            error: None,
        },
    )?;
    Ok(root.to_owned())
}
