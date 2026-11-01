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

#[derive(Clone, Serialize, Deserialize)]
pub enum Task {
    Transcribe {
        audio: PathBuf,
        manifest: crate::stem_review::StemManifest,
        wanted: Vec<String>,
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
            Self::Transcribe {
                audio,
                manifest,
                wanted,
                ..
            } => std::iter::once(audio.clone())
                .chain(
                    manifest
                        .stems
                        .iter()
                        .filter(|stem| wanted.iter().any(|name| name == stem.id.as_str()))
                        .map(|stem| stem.path.clone()),
                )
                .collect(),
            Self::Archive { .. } => Vec::new(),
            Self::Separate { audio, .. } => vec![audio.clone()],
            Self::Backup { .. } => Vec::new(),
            Self::Tempo { audio } | Self::InspectRecording { audio, .. } => vec![audio.clone()],
            Self::Relocate { .. } | Self::LoadKit { .. } | Self::SelectKit { .. } => Vec::new(),
            Self::Clean { audio, stem, .. } => {
                std::iter::once(audio.clone()).chain(stem.clone()).collect()
            }
            Self::ValidateStems { audio, manifest } => std::iter::once(audio.clone())
                .chain(manifest.stems.iter().map(|s| s.path.clone()))
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
            Self::Separate { .. } | Self::Transcribe { .. } => {
                anyhow::bail!("Separation requires the asynchronous GPU worker")
            }
            Self::Archive { .. } => Ok(serde_json::Value::Null),
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
                ..
            } => {
                for manifest in project
                    .content
                    .stems
                    .iter()
                    .chain(project.content.keyboard_stems.iter())
                {
                    manifest.load(&crate::editor::Store::new(root).audio_path(project)?)?;
                    crate::models::remember_stems(manifest)?;
                }
                crate::editor::Store::new(root).write_export(project, output, actor)?;
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
        crate::models::remember(
            &path,
            Pcm {
                stereo: js_sys::Float32Array::new(&row.get(1)).to_vec(),
                rate: row.get(2).as_f64().context("Missing rate")? as u32,
                channels: row.get(3).as_f64().context("Missing channels")? as usize,
            },
        )?;
    }
    let task: Task = serde_json::from_str(
        &field(&input, "task")?
            .as_string()
            .context("Missing worker task")?,
    )?;
    drop(input);
    for path in task.audio() {
        if crate::models::format(&path).is_ok() {
            continue;
        }
        match crate::audio_decode::decode(crate::storage::shared(&path)?.as_slice()).await {
            Ok(pcm) => crate::models::remember(&path, pcm)?,
            Err(_) => {
                let response = message("decode")?;
                set(&response, "path", &path.to_string_lossy().as_ref().into())?;
                return Ok(response);
            }
        }
    }
    let result = if let Task::Separate { audio, output } = &task {
        struct Progress;
        impl demucs_core::listener::ForwardListener for Progress {
            fn on_event(&mut self, event: demucs_core::listener::ForwardEvent) {
                use demucs_core::listener::ForwardEvent;
                match event {
                    ForwardEvent::ChunkStarted { index, total } => progress(format!(
                        "Separating vocals and instruments · chunk {} of {total}",
                        index + 1
                    )),
                    ForwardEvent::ChunkDone { index, total } => {
                        progress(format!("Separated {} of {total} chunks", index + 1))
                    }
                    _ => {}
                }
            }
        }
        let weights = crate::model_assets::load("separation", &mut progress).await?;
        let model = PathBuf::from("/models/htdemucs_6s.safetensors");
        crate::storage::write_owned(&model, weights)?;
        progress("Preparing the browser GPU and native separation model".into());
        serde_json::to_value(
            crate::separation::separate(audio, &model, output, &mut Progress).await?,
        )?
    } else if let Task::Transcribe {
        audio,
        manifest,
        wanted,
        output,
    } = &task
    {
        let events =
            crate::transcription::instruments(audio, manifest, wanted, &mut progress).await?;
        crate::storage::write(
            output.join("transcription.json"),
            serde_json::to_vec_pretty(&events)?,
        )?;
        serde_json::to_value(events)?
    } else {
        task.execute()?
    };
    let response = message("done")?;
    set(&response, "value", &serde_json::to_string(&result)?.into())?;
    let files = js_sys::Array::new();
    let archive = match &task {
        Task::Clean { output, .. } | Task::Export { output, .. } => {
            Some((output.clone(), output.with_extension("zip")))
        }
        Task::Backup { root, output } | Task::Archive { root, output } => {
            Some((root.clone(), output.clone()))
        }
        _ => None,
    };
    if let Some((folder, output)) = archive {
        let bytes = crate::files::archive(&folder)?;
        let row = js_sys::Array::new();
        row.push(&output.to_string_lossy().as_ref().into());
        row.push(&js_sys::Uint8Array::from(bytes.as_slice()));
        files.push(&row);
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
    if let Task::InspectRecording { output, .. }
    | Task::Separate { output, .. }
    | Task::Transcribe { output, .. } = &task
    {
        for (path, bytes) in crate::storage::list(output)? {
            let row = js_sys::Array::new();
            row.push(&path.to_string_lossy().as_ref().into());
            row.push(&js_sys::Uint8Array::from(bytes.as_slice()));
            files.push(&row);
        }
    }
    set(&response, "files", &files)?;
    Ok(response)
}
fn progress(label: String) {
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
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_worker() {
    let scope = js_sys::global().unchecked_into::<web_sys::DedicatedWorkerGlobalScope>();
    let target = scope.clone();
    let handler =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let target = target.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let response = match process(event.data()).await {
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
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let worker =
        Running(web_sys::Worker::new_with_options("./worker.js", &options).map_err(js_error)?);
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
            } else if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(Ok(event.data()));
            }
        });
    let onerror =
        Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
            if let Some(sender) = errors.borrow_mut().take() {
                let _ = sender.send(Err(anyhow::anyhow!(
                    "Could not run local processing worker: {}",
                    event.message()
                )));
            }
        });
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
    let mut selected = BTreeMap::new();
    for root in roots {
        for (path, bytes) in crate::storage::list(root)? {
            selected.insert(path, bytes);
        }
    }
    for path in task.audio() {
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
                for row in js_sys::Array::from(&field(&response, "files")?).iter() {
                    let row = js_sys::Array::from(&row);
                    crate::storage::write_owned(
                        row.get(0).as_string().context("Missing output path")?,
                        js_sys::Uint8Array::new(&row.get(1)).to_vec(),
                    )?;
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
                    task.audio().contains(&path) && supplied.insert(path.clone()),
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
