//! Stem-only work never enters chart transcription or replaces authored notes.
use super::state::Studio;
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::path::PathBuf;

fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::generation_progress::now_ms()
    }
}

#[component]
pub fn StemGeneration(mut studio: Studio) -> Element {
    let mut detail = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut complete = use_signal(|| false);
    let mut now = use_signal(now_ms);
    let mut started = use_signal(now_ms);
    let mut updated = use_signal(now_ms);
    let mut rate = use_signal(crate::generation_feedback::Tracker::default);
    let mut recording = use_signal(String::new);
    use_future(move || async move {
        loop {
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(1000).await;
            if (studio.stem_generation_busy)() {
                now.set(now_ms());
            }
        }
    });
    let start = move |_| {
        if (studio.stem_generation_busy)() || (studio.generation_busy)() {
            return;
        }
        studio.stem_generation_busy.set(true);
        complete.set(false);
        let stamp = now_ms();
        now.set(stamp);
        started.set(stamp);
        updated.set(stamp);
        rate.set(Default::default());
        error.set(None);
        detail.set("Preparing stem separation…".into());
        spawn(async move {
            let result = async {
                let store = (studio.store)().context("Open a chart first")?;
                let doc = studio.snapshot().context("Open a chart first")?;
                recording.set(doc.title.clone());
                ensure!(doc.has_recording(), "Attach a recording first");
                let audio = store.audio_path(&doc)?;
                let path = separate(audio, store.root.clone(), detail,rate,updated).await?;
                let current = studio.snapshot().context("Reopen the original chart to attach the generated stems")?;
                ensure!(studio.store.read().as_ref().is_some_and(|active| active.root == store.root)
                    && current.audio_sha256 == doc.audio_sha256
                    && current.content.stems == doc.content.stems,
                    "The recording or attached stems changed. Generated audio was retained at {}. Attach it to the original chart when ready.", path.display());
                attach(studio, path).await?;
                Ok::<_, anyhow::Error>(())
            }.await;
            studio.stem_generation_busy.set(false);
            match result {
                Ok(()) => {
                    complete.set(true);
                    detail.set("Stems ready · chart notes kept".into());
                    studio
                        .status
                        .set("Generated stems attached · Undo is available".into());
                }
                Err(problem) => {
                    error.set(Some(format!("{problem:#}")));
                    detail.set("Stem generation could not finish".into());
                }
            }
        });
    };
    let busy = (studio.stem_generation_busy)();
    if !(studio.stem_generation_open)() {
        return if busy {
            rsx! {button {class:"generation-running",onclick:move |_|studio.stem_generation_open.set(true),"Generating stems…"}}
        } else {
            rsx! {}
        };
    }
    let has_stems = studio
        .doc
        .read()
        .as_ref()
        .is_some_and(|doc| doc.content.stems.is_some());
    let can_start = !busy
        && !(studio.generation_busy)()
        && studio
            .doc
            .read()
            .as_ref()
            .is_some_and(|doc| doc.has_recording())
        && studio.analysis_preview.read().is_none();
    rsx! {super::sheet::ToolSheet {
        id:"generate-stems",title:"Generate stems",description:"Separate this recording into guitar, bass, drums, vocals, piano and other audio.",close_label:"Close stem generation",class:"generation-sheet",
        onclose:move |_|studio.stem_generation_open.set(false),
        actions:rsx!{button{class:"action-primary",disabled:!can_start,onclick:start,if busy{"Generating stems…"}else if complete(){"Generate again"}else{"Generate stems"}}},
        p{class:"field-hint","Generates audio only. Your chart notes and tempo map stay in place. Missing separation models are downloaded locally."}
        if has_stems && !complete(){p{class:"field-hint","The new stems replace the current primary stems only after successful generation. Undo restores the previous attachment."}}
        if busy {super::generation_status::GenerationStatus{title:"Generating audio stems",parts:vec!["Guitar".into(),"Bass".into(),"Drums".into(),"Vocals".into(),"Piano".into(),"Other audio".into()],recording:recording(),step:"Separating audio",activity:detail(),elapsed_ms:now().saturating_sub(started()),feedback:rate.read().feedback(now()),quiet_for:now().saturating_sub(updated())/1000}}
        else if complete(){p{role:"status","{detail}"}}
        if (studio.generation_busy)(){p{class:"field-hint","Wait for the current part analysis to finish."}}
        if let Some(problem)=error(){p{role:"alert","{problem}"}}
    }}
}

#[cfg(not(target_arch = "wasm32"))]
async fn separate(
    audio: PathBuf,
    project: PathBuf,
    mut detail: Signal<String>,
    mut rate: Signal<crate::generation_feedback::Tracker>,
    mut updated: Signal<u64>,
) -> Result<PathBuf> {
    detail.set("Separating instrument audio · this can take several minutes…".into());
    let reports = tempfile::tempdir()?;
    let report_root = reports.path().to_owned();
    let task = tokio::task::spawn_blocking(move || {
        let _scope = crate::generation_progress::begin(&report_root);
        crate::generation_progress::stage("separation", "Preparing stem separation…");
        let models = crate::generation::default_model_dir();
        crate::models::fetch_required(&models, &["separation"])?;
        let manifest = crate::pipeline::cached_separation(
            &audio,
            &models,
            &crate::song_layout::project_data(&project).join("cache/separation"),
        )?;
        let folder = crate::song_layout::project_data(&project).join("jobs/stems");
        std::fs::create_dir_all(&folder)?;
        let path = folder.join(format!("{}.json", crate::generation_progress::now_ms()));
        crate::generation::write_json(&path, &manifest)?;
        Ok(path)
    });
    loop {
        if let Ok(bytes) = tokio::fs::read(reports.path().join("progress.json")).await
            && let Ok(progress) =
                serde_json::from_slice::<crate::generation_progress::Progress>(&bytes)
        {
            rate.with_mut(|rate| {
                rate.observe(
                    "stems",
                    &progress.detail,
                    progress.completed.zip(progress.total),
                    progress.updated_ms,
                )
            });
            updated.set(progress.updated_ms);
            detail.set(progress.detail);
        }
        if task.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    }
    task.await?
}

#[cfg(target_arch = "wasm32")]
async fn separate(
    audio: PathBuf,
    project: PathBuf,
    detail: Signal<String>,
    rate: Signal<crate::generation_feedback::Tracker>,
    updated: Signal<u64>,
) -> Result<PathBuf> {
    let output = project
        .join("assets/stems")
        .join(crate::storage::unique_id());
    let progress = std::rc::Rc::new(move |message: String| {
        let mut detail = detail;
        let mut rate = rate;
        let mut updated = updated;
        let stamp = now_ms();
        rate.with_mut(|rate| rate.observe("stems", &message, None, stamp));
        updated.set(stamp);
        detail.set(message)
    });
    let result = crate::worker::run_with_progress::<crate::stem_review::StemManifest>(
        crate::worker::Task::Separate {
            audio,
            output: output.clone(),
        },
        std::slice::from_ref(&output),
        progress,
    )
    .await;
    match result {
        Ok(manifest) => {
            let path = output.join("manifest.json");
            crate::generation::write_json(&path, &manifest)?;
            Ok(path)
        }
        Err(error) => {
            crate::storage::clear(&output);
            Err(error)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn attach(studio: Studio, path: PathBuf) -> Result<()> {
    studio.attach_stems_manifest(path).await;
    if let Some(error) = (studio.error)() {
        anyhow::bail!(error)
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn attach(studio: Studio, path: PathBuf) -> Result<()> {
    studio.dispatch(crate::editor::Action::AttachStems { manifest: path });
    if let Some(error) = (studio.error)() {
        anyhow::bail!(error)
    }
    Ok(())
}
