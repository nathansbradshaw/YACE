//! CPU work runs in another WASM instance. Only selected local bytes cross the
//! worker boundary; neither input files nor processing results use a service.
use crate::{
    audio_decode::Pcm,
    clean::{CleanPlan, StemAlignment},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(thread_local_v2, js_namespace = ["import", "meta"], js_name = url)]
    static MODULE_URL: String;
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Task {
    VocalSectionAudition {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        chart: crate::vocals::VocalChart,
        range: crate::editor::TimeRange,
    },
    StepMania {
        audio: PathBuf,
        project: Box<crate::stepmania::Project>,
        request: crate::stepmania::GenerationRequest,
    },
    CompareVocalPitch {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
        range: Option<crate::editor::TimeRange>,
    },
    ExportChartOnly {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        output: PathBuf,
        actor: crate::editor::Actor,
        options: crate::export_options::Options,
    },
    PrepareRecording {
        source: PathBuf,
        root: PathBuf,
    },
    PrepareGameChannels {
        root: PathBuf,
    },
    GenerationStage {
        root: PathBuf,
        stage: crate::generation::Stage,
    },
    Vocals {
        audio: PathBuf,
        manifest: crate::stem_review::StemManifest,
        timing: crate::timing::TempoMap,
        output: PathBuf,
    },
    CacheModel {
        path: PathBuf,
    },
    RecognizeWords {
        audio: PathBuf,
        stem: Option<(PathBuf, StemAlignment)>,
        language: crate::lyric_language::Language,
    },
    BuildDraft {
        audio: PathBuf,
        events: Box<crate::mapping::Transcription>,
        #[serde(default)]
        vocals: Option<Box<crate::vocal_generation::Generation>>,
        timing: crate::timing::TempoMap,
        meta: crate::pipeline::Metadata,
        pro: bool,
        output: PathBuf,
    },
    Transcribe {
        audio: PathBuf,
        manifest: crate::stem_review::StemManifest,
        wanted: Vec<String>,
        #[serde(default)]
        guitar_source: crate::mapping::GuitarSource,
        output: PathBuf,
    },
    Archive {
        root: PathBuf,
        output: PathBuf,
    },
    Separate {
        audio: PathBuf,
        output: PathBuf,
    },
    InspectRecording {
        audio: PathBuf,
        output: PathBuf,
    },
    Backup {
        root: PathBuf,
        output: PathBuf,
        #[serde(default)]
        options: crate::compact_backup::Options,
    },
    CompactArchive {
        root: PathBuf,
        output: PathBuf,
        options: crate::compact_backup::Options,
    },
    Tempo {
        audio: PathBuf,
    },
    Export {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
        output: PathBuf,
        actor: crate::editor::Actor,
        options: crate::export_options::Options,
    },
    Clean {
        audio: PathBuf,
        plan: CleanPlan,
        stem: Option<PathBuf>,
        alignment: Option<StemAlignment>,
        output: PathBuf,
    },
    ValidateStems {
        audio: PathBuf,
        manifest: crate::stem_review::StemManifest,
    },
    Analyse {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
    },
    Relocate {
        root: PathBuf,
    },
    LoadKit {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
    },
    SplitKit {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
        model_dir: PathBuf,
    },
    SelectKit {
        root: PathBuf,
        project: Box<crate::editor::Project>,
        audio: PathBuf,
        part: String,
    },
}
impl Task {
    fn audio(&self) -> Vec<PathBuf> {
        match self {
            Self::VocalSectionAudition { root, project, .. } => crate::editor::Store::new(root)
                .audio_path(project)
                .ok()
                .into_iter()
                .chain(project.content.stems.iter().flat_map(|m| {
                    m.stems
                        .iter()
                        .filter(|s| s.id == crate::stem_review::StemId::Vocals)
                        .map(|s| s.path.clone())
                }))
                .collect(),
            Self::StepMania { audio, .. } => vec![audio.clone()],
            Self::CompareVocalPitch { audio, project, .. } => std::iter::once(audio.clone())
                .chain(project.content.stems.iter().flat_map(|m| {
                    m.stems
                        .iter()
                        .filter(|s| s.id == crate::stem_review::StemId::Vocals)
                        .map(|s| s.path.clone())
                }))
                .collect(),
            Self::ExportChartOnly { .. } => Vec::new(),
            Self::PrepareRecording { source, .. } => vec![source.clone()],
            Self::PrepareGameChannels { root } => {
                crate::stem_review::conventional::decode_inputs(root).unwrap_or_default()
            }
            Self::RecognizeWords { audio, stem, .. } => std::iter::once(audio.clone())
                .chain(stem.iter().map(|(path, _)| path.clone()))
                .collect(),
            Self::GenerationStage { root, stage } => {
                crate::generation::audio_paths(root, *stage).unwrap_or_default()
            }
            Self::Vocals {
                audio, manifest, ..
            } => std::iter::once(audio.clone())
                .chain(
                    manifest
                        .stems
                        .iter()
                        .filter(|s| s.id == crate::stem_review::StemId::Vocals)
                        .map(|s| s.path.clone()),
                )
                .collect(),
            Self::BuildDraft { audio, events, .. } => std::iter::once(audio.clone())
                .chain(
                    events
                        .isolation
                        .iter()
                        .flat_map(|m| m.stems.iter().map(|s| s.path.clone())),
                )
                .collect(),
            Self::Transcribe {
                audio,
                manifest,
                wanted,
                guitar_source,
                ..
            } => std::iter::once(audio.clone())
                .chain(
                    manifest
                        .stems
                        .iter()
                        .filter(|stem| {
                            wanted.iter().any(|name| {
                                (name == "guitar" && stem.id == guitar_source.stem())
                                    || (name != "guitar" && name == stem.id.as_str())
                                    || ((name == "piano" || name == "pro_piano")
                                        && ["piano", "drums", "bass", "vocals"]
                                            .contains(&stem.id.as_str()))
                            })
                        })
                        .map(|stem| stem.path.clone()),
                )
                .collect(),
            Self::Archive { .. } | Self::CacheModel { .. } => Vec::new(),
            Self::Separate { audio, .. } => vec![audio.clone()],
            Self::Backup { .. } | Self::CompactArchive { .. } => Vec::new(),
            Self::Tempo { audio } | Self::InspectRecording { audio, .. } => vec![audio.clone()],
            Self::Relocate { .. } | Self::LoadKit { .. } | Self::SelectKit { .. } => Vec::new(),
            Self::Clean { audio, stem, .. } => {
                std::iter::once(audio.clone()).chain(stem.clone()).collect()
            }
            Self::ValidateStems { audio, manifest } => std::iter::once(audio.clone())
                .chain(manifest.stems.iter().map(|s| s.path.clone()))
                .collect(),
            Self::SplitKit { audio, project, .. } => std::iter::once(audio.clone())
                .chain(
                    project
                        .content
                        .stems
                        .iter()
                        .flat_map(|m| &m.stems)
                        .filter(|asset| asset.id == crate::stem_review::StemId::Drums)
                        .map(|asset| asset.path.clone()),
                )
                .collect(),
            Self::Analyse { audio, project, .. } | Self::Export { audio, project, .. } => {
                std::iter::once(audio.clone())
                    .chain(
                        project
                            .content
                            .stems
                            .iter()
                            .chain(project.content.keyboard_stems.iter())
                            .flat_map(|m| m.stems.iter().map(|s| s.path.clone())),
                    )
                    .collect()
            }
        }
    }
    fn execute(&self) -> Result<serde_json::Value> {
        match self {
            Self::VocalSectionAudition {
                root,
                project,
                chart,
                range,
            } => Ok(serde_json::to_value(crate::vocal_section_audio::prepare(
                project,
                &crate::editor::Store::new(root),
                chart,
                *range,
            )?)?),
            Self::StepMania {
                audio,
                project,
                request,
            } => {
                let samples = crate::models::decode(audio, 11025, 1)?;
                let mut project = project.as_ref().clone();
                project.generate(
                    project.revision,
                    "human".into(),
                    request,
                    &samples,
                    11025,
                    || false,
                )?;
                Ok(serde_json::to_value(project)?)
            }
            Self::ExportChartOnly {
                root,
                project,
                output,
                actor,
                options,
            } => {
                ensure!(
                    !project.has_recording() || !options.include_audio,
                    "Use recording-aware export for a recorded project"
                );
                crate::editor::Store::new(root)
                    .write_export_with_options(project, output, actor, options)?;
                Ok(serde_json::Value::Null)
            }
            Self::PrepareRecording { source, root } => Ok(serde_json::to_value(
                crate::project_recording::prepare_recording(source, root)?,
            )?),
            Self::SplitKit { .. } | Self::CompareVocalPitch { .. } => {
                anyhow::bail!("Split kit must run on the asynchronous inference worker")
            }
            Self::PrepareGameChannels { root } => Ok(serde_json::to_value(
                crate::stem_review::conventional::prepare(root)?,
            )?),
            Self::BuildDraft {
                audio,
                events,
                vocals,
                timing,
                meta,
                pro,
                output,
            } => {
                if let Some(manifest) = &events.isolation {
                    drop(manifest.load(audio)?);
                    crate::models::remember_stems(manifest)?;
                }
                let report = crate::pipeline::build(crate::pipeline::Build {
                    audio,
                    output,
                    existing: None,
                    timing: Some(timing.clone()),
                    transcription: events,
                    meta: meta.clone(),
                    pro: *pro,
                    lookup_lyrics: false,
                })?;
                if let Some(manifest) = &events.isolation {
                    let mut portable = crate::stem_review::game_audio::retain_in(output, manifest)?;
                    for stem in &mut portable.stems {
                        stem.path = stem
                            .path
                            .strip_prefix(output.join(".yace/audio"))?
                            .to_owned();
                    }
                    crate::storage::write(
                        crate::song_layout::path(output, "chart-engine-isolation.json"),
                        serde_json::to_vec_pretty(&portable)?,
                    )?;
                }
                if let Some(vocals) = vocals {
                    ensure!(
                        vocals.audio_sha256 == crate::models::hash_file(audio)?
                            && vocals.timing.as_ref() == Some(timing),
                        "Vocal draft belongs to another source or timing map"
                    );
                    crate::storage::write(
                        crate::song_layout::path(output, "chart-engine-vocal-generation.json"),
                        serde_json::to_vec_pretty(vocals)?,
                    )?;
                    if let Some(chart) = &vocals.chart {
                        chart.validate()?;
                        crate::storage::write(
                            crate::song_layout::path(output, "chart-engine-vocals.json"),
                            serde_json::to_vec_pretty(chart)?,
                        )?;
                    }
                }
                Ok(report)
            }
            Self::Separate { .. }
            | Self::Transcribe { .. }
            | Self::CacheModel { .. }
            | Self::GenerationStage { .. }
            | Self::Vocals { .. } => {
                anyhow::bail!("Separation requires the asynchronous GPU worker")
            }
            Self::RecognizeWords { .. } => {
                anyhow::bail!("Recognition requires the asynchronous local worker")
            }
            Self::Archive { .. } | Self::CompactArchive { .. } => Ok(serde_json::Value::Null),
            Self::InspectRecording { audio, output } => Ok(serde_json::to_value(
                crate::song_import::local(audio, output)?,
            )?),
            Self::Backup { root, .. } => {
                crate::editor::Store::new(root).load()?;
                Ok(serde_json::Value::Null)
            }
            Self::Tempo { audio } => Ok(serde_json::to_value(crate::tempo_analysis::analyze(
                audio,
            )?)?),
            Self::Export {
                root,
                project,
                output,
                actor,
                options,
                ..
            } => {
                for manifest in project
                    .content
                    .stems
                    .iter()
                    .chain(project.content.keyboard_stems.iter())
                {
                    crate::models::validate_stems(
                        manifest,
                        &crate::editor::Store::new(root).audio_path(project)?,
                    )?;
                }
                crate::editor::Store::new(root)
                    .write_export_with_options(project, output, actor, options)?;
                Ok(serde_json::Value::Null)
            }
            Self::Clean {
                audio,
                plan,
                stem,
                alignment,
                output,
            } => Ok(serde_json::to_value(crate::clean::render(
                crate::clean::RenderRequest {
                    audio,
                    plan,
                    vocal_stem: stem.as_deref(),
                    stem_alignment: alignment.as_ref(),
                    output,
                },
            )?)?),
            Self::ValidateStems { audio, manifest } => {
                manifest.load(audio)?;
                Ok(serde_json::to_value(manifest)?)
            }
            Self::Analyse { root, project, .. } => Ok(serde_json::to_value(
                crate::part_analysis::analyze_project(&crate::editor::Store::new(root), project)?,
            )?),
            Self::Relocate { root } => {
                let original = crate::storage::list(root)?;
                let mut files: BTreeMap<String, std::sync::Arc<Vec<u8>>> = original
                    .iter()
                    .map(|(path, bytes)| {
                        Ok((
                            path.strip_prefix(root)?
                                .to_string_lossy()
                                .replace('\\', "/"),
                            bytes.clone(),
                        ))
                    })
                    .collect::<Result<_>>()?;
                crate::project_portability::relocate(&mut files, root)?;
                let mut changed = Vec::new();
                for (name, bytes) in files {
                    let path = root.join(name);
                    if crate::storage::shared(&path)
                        .is_ok_and(|old| old.as_slice() == bytes.as_slice())
                    {
                        continue;
                    }
                    crate::storage::write(&path, bytes.as_slice())?;
                    changed.push(path);
                }
                Ok(serde_json::to_value(changed)?)
            }
            Self::LoadKit { root, project, .. } => Ok(serde_json::to_value(
                crate::part_analysis::load_project_drum_parts(
                    &crate::editor::Store::new(root),
                    project,
                )?,
            )?),
            Self::SelectKit {
                root,
                project,
                part,
                ..
            } => Ok(serde_json::to_value(
                crate::part_analysis::select_kit_on_worker(
                    &crate::editor::Store::new(root),
                    project,
                    part,
                )?,
            )?),
        }
    }
}
fn js_error(error: JsValue) -> anyhow::Error {
    anyhow::anyhow!("Browser worker: {error:?}")
}
fn field(value: &JsValue, key: &str) -> Result<JsValue> {
    js_sys::Reflect::get(value, &key.into()).map_err(js_error)
}
fn set(value: &JsValue, key: &str, field: &JsValue) -> Result<()> {
    js_sys::Reflect::set(value, &key.into(), field).map_err(js_error)?;
    Ok(())
}
fn message(kind: &str) -> Result<JsValue> {
    let value: JsValue = js_sys::Object::new().into();
    set(&value, "kind", &kind.into())?;
    Ok(value)
}
fn buffers(message: &JsValue) -> Result<js_sys::Array> {
    let buffers = js_sys::Array::new();
    for key in ["files", "decoded"] {
        let rows = field(message, key)?;
        if rows.is_undefined() {
            continue;
        }
        for row in js_sys::Array::from(&rows).iter() {
            let data = js_sys::Array::from(&row).get(1);
            if data.is_string() {
                continue;
            }
            buffers.push(&field(&data, "buffer")?);
        }
    }
    Ok(buffers)
}
fn pcm_message(path: &Path, pcm: &Pcm) -> js_sys::Array {
    let row = js_sys::Array::new();
    row.push(&path.to_string_lossy().as_ref().into());
    row.push(&js_sys::Float32Array::from(pcm.stereo.as_slice()));
    row.push(&pcm.rate.into());
    row.push(&(pcm.channels as u32).into());
    row
}
async fn process(input: JsValue) -> Result<JsValue> {
    let task: Task = serde_json::from_str(
        &field(&input, "task")?
            .as_string()
            .context("Missing worker task")?,
    )?;
    let bounded = matches!(
        task,
        Task::PrepareGameChannels { .. } | Task::PrepareRecording { .. }
    );
    for row in js_sys::Array::from(&field(&input, "files")?).iter() {
        let row = js_sys::Array::from(&row);
        let path = row.get(0).as_string().context("Missing worker path")?;
        if let Some(source) = row.get(1).as_string() {
            crate::storage::copy(source, path)?;
        } else {
            crate::storage::write_owned(path, js_sys::Uint8Array::new(&row.get(1)).to_vec())?;
        }
    }
    for row in js_sys::Array::from(&field(&input, "decoded")?).iter() {
        let row = js_sys::Array::from(&row);
        let path = PathBuf::from(row.get(0).as_string().context("Missing decoded path")?);
        if bounded {
            ensure!(
                crate::models::decoded_bytes()
                    .saturating_add(js_sys::Float32Array::new(&row.get(1)).length() as usize * 4)
                    <= 1024 * 1024 * 1024,
                "Game channels exceed the browser decoded-audio budget"
            );
        }
        crate::models::remember(
            &path,
            Pcm {
                stereo: js_sys::Float32Array::new(&row.get(1)).to_vec(),
                rate: row.get(2).as_f64().context("Missing rate")? as u32,
                channels: row.get(3).as_f64().context("Missing channels")? as usize,
            },
        )?;
    }
    if let Task::GenerationStage { root, stage } = &task {
        crate::generation::audio_paths(root, *stage)?;
    }
    // Keep the original allocations alive while processing. Unchanged audio
    // copied into the resulting project can refer back to these inputs rather
    // than crossing the worker boundary a second time.
    let input_audio: Vec<_> = task
        .audio()
        .into_iter()
        .map(|path| {
            let bytes = crate::storage::shared(&path)?;
            Ok((path, bytes))
        })
        .collect::<Result<_>>()?;
    crate::storage::acknowledge(crate::storage::pending(Path::new("/"))?);
    drop(input);
    let decode = if let Task::Export { audio, project, .. } = &task {
        std::iter::once(audio.clone())
            .chain(
                project
                    .content
                    .stems
                    .iter()
                    .chain(project.content.keyboard_stems.iter())
                    .flat_map(|manifest| &manifest.stems)
                    .filter(|asset| {
                        !asset
                            .path
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
                    })
                    .map(|asset| asset.path.clone()),
            )
            .collect()
    } else {
        task.audio()
    };
    for path in decode {
        if crate::models::format(&path).is_ok() {
            continue;
        }
        match crate::audio_decode::decode(crate::storage::shared(&path)?.as_slice()).await {
            Ok(pcm) => {
                if bounded {
                    ensure!(
                        crate::models::decoded_bytes().saturating_add(pcm.stereo.len() * 4)
                            <= 1024 * 1024 * 1024,
                        "Game channels exceed the browser decoded-audio budget"
                    );
                }
                crate::models::remember(&path, pcm)?;
            }
            Err(_) => {
                let response = message("decode")?;
                set(&response, "path", &path.to_string_lossy().as_ref().into())?;
                return Ok(response);
            }
        }
    }
    if let Task::Export { audio, project, .. } = &task {
        for manifest in project
            .content
            .stems
            .iter()
            .chain(project.content.keyboard_stems.iter())
        {
            progress("Checking separated audio before export…".into());
            manifest.validate_binding(
                &crate::models::hash_file(audio)?,
                crate::models::duration(audio)?,
            )?;
            for asset in &manifest.stems {
                ensure!(
                    asset.path != *audio && crate::models::hash_file(&asset.path)? == asset.sha256,
                    "Separated audio changed before export"
                );
                // Validate one WAV at a time. Export copies these encoded bytes
                // unchanged, so retaining six decoded PCM copies is unnecessary.
                let pcm =
                    crate::audio_decode::decode(&crate::storage::shared(&asset.path)?).await?;
                if manifest.conventional.is_some() {
                    manifest.prepare_channel(pcm.samples(44100, 2)?)?;
                } else {
                    ensure!(
                        pcm.rate == 44100
                            && pcm.channels == 2
                            && pcm.stereo.len() as u64 == manifest.frames * 2,
                        "Separated audio format or source clock changed"
                    );
                }
            }
            crate::models::remember_stems(manifest)?;
        }
    }
    let result = if let Task::SplitKit {
        root,
        project,
        model_dir,
        ..
    } = &task
    {
        let store = crate::editor::Store::new(root);
        serde_json::to_value(
            crate::part_analysis::prepare_project_drum_parts(
                &store,
                project,
                model_dir,
                &mut progress,
            )
            .await?,
        )?
    } else if let Task::GenerationStage { root, stage } = &task {
        serde_json::to_value(crate::generation::run_stage(root, *stage, &mut progress).await?)?
    } else if let Task::Separate { audio, output } = &task {
        serde_json::to_value(crate::separation::run(audio, output, &mut progress).await?)?
    } else if let Task::Transcribe {
        audio,
        manifest,
        wanted,
        guitar_source,
        output,
    } = &task
    {
        let events = crate::transcription::instruments(
            audio,
            manifest,
            wanted,
            *guitar_source,
            &mut progress,
        )
        .await?;
        crate::storage::write(
            output.join("transcription.json"),
            serde_json::to_vec_pretty(&events)?,
        )?;
        serde_json::to_value(events)?
    } else if let Task::CompareVocalPitch {
        root,
        project,
        audio,
        range,
    } = &task
    {
        let store = crate::editor::Store::new(root);
        let timing = project.timing(&store)?;
        let manifest = project
            .content
            .stems
            .as_ref()
            .context("Attach a vocal stem before comparing pitches")?;
        let generation =
            crate::vocal_inference::pitch_comparison(audio, manifest, &timing, &mut progress)
                .await?;
        serde_json::to_value(crate::vocal_pitch_analysis::propose(
            project,
            &timing,
            &generation,
            *range,
        )?)?
    } else if let Task::Vocals {
        audio,
        manifest,
        timing,
        output,
    } = &task
    {
        let result =
            crate::vocal_inference::generate(audio, manifest, timing, &mut progress).await?;
        crate::storage::write(
            output.join("vocal-generation.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        serde_json::to_value(result)?
    } else if let Task::RecognizeWords {
        audio,
        stem,
        language,
    } = &task
    {
        serde_json::to_value(
            crate::recognition::analyze(
                audio,
                stem.as_ref()
                    .map(|(path, binding)| (path.as_path(), binding)),
                *language,
                &mut progress,
            )
            .await?,
        )?
    } else if let Task::CacheModel { path } = &task {
        progress("Verifying the selected model checkpoint…".into());
        serde_json::to_value(crate::model_assets::import(crate::storage::read(path)?).await?)?
    } else {
        task.execute()?
    };
    let response = message("done")?;
    set(&response, "value", &serde_json::to_string(&result)?.into())?;
    let files = js_sys::Array::new();
    let archive = match &task {
        Task::Clean { output, .. }
        | Task::Export { output, .. }
        | Task::ExportChartOnly { output, .. } => {
            Some((output.clone(), output.with_extension("zip")))
        }
        Task::Backup { root, output, .. }
        | Task::CompactArchive { root, output, .. }
        | Task::Archive { root, output } => Some((root.clone(), output.clone())),
        _ => None,
    };
    if let Some((folder, output)) = archive {
        progress("Preparing archive download in bounded chunks…".into());
        let blob =
            if let Task::Backup { options, .. } | Task::CompactArchive { options, .. } = &task {
                crate::files::compact_archive_blob(&folder, *options)?
            } else {
                crate::files::archive_blob(&folder)?
            };
        let row = js_sys::Array::new();
        row.push(&output.to_string_lossy().as_ref().into());
        row.push(&blob);
        let downloads = js_sys::Array::new();
        downloads.push(&row);
        set(&response, "downloads", &downloads)?;
    }
    if matches!(task, Task::Relocate { .. }) {
        for path in serde_json::from_value::<Vec<PathBuf>>(result.clone())? {
            let row = js_sys::Array::new();
            row.push(&path.to_string_lossy().as_ref().into());
            row.push(&js_sys::Uint8Array::from(
                crate::storage::shared(path)?.as_slice(),
            ));
            files.push(&row);
        }
    }
    if let Task::GenerationStage { root: output, .. }
    | Task::SplitKit { root: output, .. }
    | Task::PrepareGameChannels { root: output }
    | Task::PrepareRecording { root: output, .. }
    | Task::Vocals { output, .. }
    | Task::BuildDraft { output, .. }
    | Task::InspectRecording { output, .. }
    | Task::Separate { output, .. }
    | Task::Transcribe { output, .. } = &task
    {
        let mut aliases: BTreeMap<_, JsValue> = input_audio
            .iter()
            .map(|(path, bytes)| {
                (
                    std::sync::Arc::as_ptr(bytes),
                    JsValue::from(path.to_string_lossy().as_ref()),
                )
            })
            .collect();
        let changed = if matches!(
            task,
            Task::GenerationStage { .. }
                | Task::PrepareGameChannels { .. }
                | Task::PrepareRecording { .. }
        ) {
            crate::storage::pending(output)?
        } else {
            crate::storage::list(output)?
        };
        for (path, bytes) in changed {
            let row = js_sys::Array::new();
            row.push(&path.to_string_lossy().as_ref().into());
            if let Some(source) = aliases.get(&std::sync::Arc::as_ptr(&bytes)) {
                row.push(source);
            } else {
                row.push(&js_sys::Uint8Array::from(bytes.as_slice()));
                aliases.insert(
                    std::sync::Arc::as_ptr(&bytes),
                    JsValue::from(path.to_string_lossy().as_ref()),
                );
            }
            files.push(&row);
        }
    }
    set(&response, "files", &files)?;
    Ok(response)
}
pub(crate) fn progress(label: String) {
    let result = (|| -> Result<()> {
        let value = message("progress")?;
        set(&value, "label", &label.into())?;
        js_sys::global()
            .unchecked_into::<web_sys::DedicatedWorkerGlobalScope>()
            .post_message(&value)
            .map_err(js_error)?;
        Ok(())
    })();
    if let Err(error) = result {
        web_sys::console::error_1(&format!("{error:#}").into());
    }
}

fn remember_verified_manifest(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
) -> Result<()> {
    // These identities were checked against complete decoded clocks and hashes
    // by this dedicated worker before its atomic result was transferred.
    crate::models::remember_verified_hash(
        &crate::storage::shared(audio)?,
        &manifest.source_audio_sha256,
    );
    for asset in &manifest.stems {
        crate::models::remember_verified_hash(&crate::storage::shared(&asset.path)?, &asset.sha256);
    }
    crate::models::remember_stems(manifest)
}
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_worker() {
    let scope = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
    let target = scope.clone();
    let handler =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let target = target.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let result = if field(&event.data(), "kind")
                    .ok()
                    .and_then(|v| v.as_string())
                    .as_deref()
                    == Some("import-archive")
                {
                    crate::archive_import::process(&event.data(), &target)
                } else {
                    process(event.data()).await
                };
                let response = match result {
                    Ok(value) => value,
                    Err(error) => {
                        let value = message("error").unwrap();
                        set(&value, "error", &format!("{error:#}").into()).unwrap();
                        value
                    }
                };
                if let Ok(transfer) = buffers(&response) {
                    let _ = target.post_message_with_transfer(&response, &transfer);
                }
            });
        });
    scope.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    handler.forget();
    let _ = scope.post_message(&message("ready").unwrap());
}
struct Running(web_sys::Worker);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.set_onmessage(None);
        self.0.set_onerror(None);
        self.0.terminate();
    }
}
async fn exchange(input: &JsValue, progress: Rc<dyn Fn(String)>) -> Result<JsValue> {
    exchange_with_entries(input, progress, None).await
}
type EntrySink = Rc<dyn Fn(JsValue) -> Result<()>>;
pub(crate) async fn exchange_with_entries(
    input: &JsValue,
    progress: Rc<dyn Fn(String)>,
    entries: Option<EntrySink>,
) -> Result<JsValue> {
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let url = MODULE_URL
        .with(|base| web_sys::Url::new_with_base("./worker.js", base))
        .map_err(js_error)?;
    let worker =
        Running(web_sys::Worker::new_with_options(&url.href(), &options).map_err(js_error)?);
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let errors = sender.clone();
    let target = worker.0.clone();
    let activity = Rc::new(std::cell::Cell::new(js_sys::Date::now()));
    let last_activity = activity.clone();
    let input = input.clone();
    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            activity.set(js_sys::Date::now());
            if field(&event.data(), "kind")
                .ok()
                .and_then(|v| v.as_string())
                .as_deref()
                == Some("ready")
            {
                let sent = buffers(&input)
                    .map_err(|e| JsValue::from_str(&e.to_string()))
                    .and_then(|transfer| target.post_message_with_transfer(&input, &transfer));
                if let Err(error) = sent
                    && let Some(sender) = sender.borrow_mut().take()
                {
                    let _ = sender.send(Err(js_error(error)));
                }
            } else if field(&event.data(), "kind")
                .ok()
                .and_then(|kind| kind.as_string())
                .as_deref()
                == Some("progress")
            {
                if let Ok(label) = field(&event.data(), "label")
                    && let Some(label) = label.as_string()
                {
                    progress(label);
                }
            } else if field(&event.data(), "kind")
                .ok()
                .and_then(|v| v.as_string())
                .as_deref()
                == Some("archive-entry")
            {
                let result = entries
                    .as_ref()
                    .context("Unexpected archive entry")
                    .and_then(|sink| sink(event.data()));
                if let Err(error) = result
                    && let Some(sender) = sender.borrow_mut().take()
                {
                    let _ = sender.send(Err(error));
                }
            } else if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(Ok(event.data()));
            }
        });
    let onerror = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(
        move |event: web_sys::ErrorEvent| {
            if let Some(sender) = errors.borrow_mut().take() {
                let _ = sender.send(Err(anyhow::anyhow!(
                    "Could not run local processing worker: {}",
                    js_sys::Reflect::get(event.as_ref(),&JsValue::from_str("message")).ok().and_then(|message|message.as_string()).filter(|message|!message.trim().is_empty()).unwrap_or_else(||"The worker could not load. Reload YACE and try again; your chart is unchanged.".into())
                )));
            }
        },
    );
    worker
        .0
        .set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    worker.0.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    // Long inference jobs remain valid while making progress; a stalled worker
    // still has a bounded idle wait and can always be cancelled by its caller.
    let timeout = async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            if js_sys::Date::now() - last_activity.get() > 300_000. {
                break;
            }
        }
    };
    futures_util::pin_mut!(receiver, timeout);
    let result = match futures_util::future::select(receiver, timeout).await {
        futures_util::future::Either::Left((result, _)) => {
            result.context("Local processing worker stopped")?
        }
        futures_util::future::Either::Right(_) => Err(anyhow::anyhow!(
            "Local processing did not finish; existing chart and audio were kept"
        )),
    };
    drop(worker);
    result
}
pub async fn run<T: serde::de::DeserializeOwned>(task: Task, roots: &[PathBuf]) -> Result<T> {
    run_with_progress(task, roots, Rc::new(|_| {})).await
}
pub async fn run_with_progress<T: serde::de::DeserializeOwned>(
    task: Task,
    roots: &[PathBuf],
    progress: Rc<dyn Fn(String)>,
) -> Result<T> {
    let bound_audio = task.audio();
    let mut selected = BTreeMap::new();
    for root in roots {
        for (path, bytes) in crate::storage::list(root)? {
            if let Task::Backup { root, options, .. } | Task::CompactArchive { root, options, .. } =
                &task
            {
                let name = crate::archive_codec::relative_name(path.strip_prefix(root)?)?;
                if !options.include_generated_stems && crate::compact_backup::generated(&name) {
                    continue;
                }
            }
            if matches!(task, Task::GenerationStage { .. } | Task::RecognizeWords { .. } | Task::CompareVocalPitch { .. } | Task::VocalSectionAudition { .. })
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        ["wav", "ogg", "opus", "mp3", "flac", "m4a"]
                            .contains(&extension.to_ascii_lowercase().as_str())
                    })
                && !bound_audio.contains(&path)
                // All six encoded assets are needed to validate the source
                // manifest, even when this stage decodes only one part.
                && !(matches!(task,Task::GenerationStage{..}) && (path.starts_with(root.join("stems"))||path.starts_with(root.join("keyboard-stems"))))
            {
                continue;
            }
            selected.insert(path, bytes);
        }
    }
    if let Task::Backup { root, options, .. } = &task {
        let files = selected
            .iter()
            .map(|(path, bytes)| {
                Ok((
                    crate::archive_codec::relative_name(path.strip_prefix(root)?)?,
                    bytes.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let doc = crate::editor::Store::new(root).load()?;
        crate::project_portability::validate_sources(&files, &doc)?;
        for (path, hash) in crate::project_portability::referenced_stems(&files)? {
            if options.include_generated_stems
                && let Ok(bytes) = crate::storage::shared(&path)
            {
                ensure!(
                    crate::models::hash_shared(&bytes).eq_ignore_ascii_case(&hash),
                    "Attached stem checksum differs during backup"
                );
                if !path.starts_with(root) {
                    let ext = path.extension().unwrap_or_default().to_string_lossy();
                    selected.insert(root.join(format!("assets/stems/{hash}.{ext}")), bytes);
                }
            }
        }
    }
    for path in &bound_audio {
        selected.insert(path.clone(), crate::storage::shared(path)?);
    }
    if let Task::LoadKit { audio, project, .. } | Task::SelectKit { audio, project, .. } = &task {
        for path in std::iter::once(audio.clone()).chain(
            project
                .content
                .stems
                .iter()
                .flat_map(|m| m.stems.iter().map(|s| s.path.clone())),
        ) {
            selected.insert(path.clone(), crate::storage::shared(path)?);
        }
    }
    let mut overrides: BTreeMap<PathBuf, Pcm> = BTreeMap::new();
    let mut supplied = std::collections::BTreeSet::new();
    loop {
        let files = js_sys::Array::new();
        let mut aliases = std::collections::BTreeMap::new();
        for (path, bytes) in &selected {
            let row = js_sys::Array::new();
            row.push(&path.to_string_lossy().as_ref().into());
            if let Some(source) = aliases.get(&std::sync::Arc::as_ptr(bytes)) {
                row.push(source);
            } else {
                row.push(&js_sys::Uint8Array::from(bytes.as_slice()));
                aliases.insert(
                    std::sync::Arc::as_ptr(bytes),
                    JsValue::from(path.to_string_lossy().as_ref()),
                );
            }
            files.push(&row);
        }
        let decoded = js_sys::Array::new();
        for (path, pcm) in &overrides {
            decoded.push(&pcm_message(path, pcm));
        }
        let input = js_sys::Object::new();
        set(&input, "task", &serde_json::to_string(&task)?.into())?;
        set(&input, "files", &files)?;
        set(&input, "decoded", &decoded)?;
        let response = exchange(&input, progress.clone()).await?;
        match field(&response, "kind")?.as_string().as_deref() {
            Some("done") => {
                for path in &bound_audio {
                    let original = selected.get(path).context("Missing bound worker input")?;
                    ensure!(
                        crate::storage::shared(path)
                            .is_ok_and(|current| std::sync::Arc::ptr_eq(&current, original)),
                        "Input audio changed during local processing; current files and chart were kept"
                    );
                }
                if let Task::LoadKit {
                    root,
                    project,
                    audio,
                }
                | Task::SelectKit {
                    root,
                    project,
                    audio,
                    ..
                } = &task
                {
                    for (path, bytes) in &selected {
                        if path == audio
                            || path.starts_with(root.join("analysis"))
                            || project
                                .content
                                .stems
                                .iter()
                                .flat_map(|m| &m.stems)
                                .any(|asset| asset.path == *path)
                        {
                            ensure!(
                                crate::storage::shared(path)
                                    .is_ok_and(|current| std::sync::Arc::ptr_eq(&current, bytes)),
                                "Kit input changed during validation; retry with the current source"
                            );
                        }
                    }
                }
                let result = serde_json::from_str(
                    &field(&response, "value")?
                        .as_string()
                        .context("Worker returned no result")?,
                )?;
                // Publish complete results only after successful processing.
                let downloads = field(&response, "downloads")?;
                if !downloads.is_undefined() {
                    for row in js_sys::Array::from(&downloads).iter() {
                        let row = js_sys::Array::from(&row);
                        let path = row.get(0).as_string().context("Missing download path")?;
                        let blob = row.get(1).dyn_into::<web_sys::Blob>().map_err(js_error)?;
                        crate::files::retain_download(path.into(), blob);
                    }
                }
                let mut published = std::collections::BTreeSet::new();
                for row in js_sys::Array::from(&field(&response, "files")?).iter() {
                    let row = js_sys::Array::from(&row);
                    let path = row.get(0).as_string().context("Missing output path")?;
                    ensure!(
                        !published.contains(&path),
                        "Worker returned duplicate output paths"
                    );
                    if let Some(source) = row.get(1).as_string() {
                        ensure!(
                            published.contains(&source)
                                || bound_audio
                                    .iter()
                                    .any(|path| path.to_string_lossy() == source
                                        && selected.contains_key(path)),
                            "Worker returned an invalid file alias"
                        );
                        crate::storage::copy(source, &path)?;
                    } else {
                        crate::storage::write_owned(
                            &path,
                            js_sys::Uint8Array::new(&row.get(1)).to_vec(),
                        )?;
                    }
                    published.insert(path);
                }
                match &task {
                    Task::Separate { audio, .. } | Task::ValidateStems { audio, .. } => {
                        let manifest = serde_json::from_str::<crate::stem_review::StemManifest>(
                            &field(&response, "value")?
                                .as_string()
                                .context("Missing verified stem result")?,
                        )?;
                        remember_verified_manifest(audio, &manifest)?;
                    }
                    Task::GenerationStage {
                        root,
                        stage: crate::generation::Stage::Separate,
                    } => {
                        let crate::generation::Source::Audio { path } =
                            crate::generation::read_request(root)?.source;
                        remember_verified_manifest(&path, &crate::generation::isolation(root)?)?;
                    }
                    Task::GenerationStage {
                        root,
                        stage: crate::generation::Stage::Build,
                    } => {
                        let store =
                            crate::editor::Store::new(&crate::generation::project_path(root));
                        let doc = store.load()?;
                        let audio = store.audio_path(&doc)?;
                        for manifest in doc
                            .content
                            .stems
                            .iter()
                            .chain(doc.content.keyboard_stems.iter())
                        {
                            remember_verified_manifest(&audio, manifest)?;
                        }
                    }
                    _ => {}
                }
                return Ok(result);
            }
            Some("decode") => {
                let path = PathBuf::from(
                    field(&response, "path")?
                        .as_string()
                        .context("Missing decoder request")?,
                );
                ensure!(
                    bound_audio.contains(&path) && supplied.insert(path.clone()),
                    "Worker requested invalid or repeated decoding"
                );
                let pcm =
                    crate::studio::decode_local(crate::storage::shared(&path)?.as_slice()).await?;
                overrides.insert(path, pcm);
            }
            Some("error") => anyhow::bail!(
                "{}",
                field(&response, "error")?
                    .as_string()
                    .unwrap_or_else(|| "Local processing failed".into())
            ),
            _ => anyhow::bail!("Invalid local worker response"),
        }
    }
}
