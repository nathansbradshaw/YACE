//! The same authoring surface is compiled for desktop and static WASM.
use crate::stepmania::{
    self as sm, Action, Chart, Difficulty, Event, Format, GenerationRequest, NoteKind, Profile,
    Project, Request, Timing, TimingKind,
    bundle::{self, Files, MANIFEST},
    project::Foot,
    timing::AudioPosition,
};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::path::PathBuf;
#[path = "dance_preview.rs"]
mod preview;
#[path = "dance_workspace.rs"]
mod workspace;
pub use workspace::{Audition, Inspector, Roll};
#[path = "dance_transport.rs"]
mod transport_backend;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub use transport_backend::{playback, position, seek, set_loop, set_rate, set_volume, transport};
#[derive(Clone, Copy, PartialEq)]
pub struct DanceStudio {
    pub open: Signal<bool>,
    project: Signal<Option<Project>>,
    assets: Signal<Files>,
    root: Signal<Option<PathBuf>>,
    saved: Signal<Option<u64>>,
    saving: Signal<bool>,
    blocked: Signal<bool>,
    error: Signal<Option<String>>,
    status: Signal<String>,
    selected: Signal<String>,
    audio: Signal<String>,
    clock: Signal<f64>,
    motion_epoch: Signal<u64>,
    playing: Signal<bool>,
    playback_intent: Signal<u64>,
    length: Signal<f64>,
    rate: Signal<f64>,
    loop_range: Signal<Option<crate::editor::TimeRange>>,
    busy: Signal<bool>,
    cancel: Signal<bool>,
    pub generation_open: Signal<bool>,
    pub review_open: Signal<bool>,
    pub export_open: Signal<bool>,
    pub after_new: Signal<bool>,
    parent: Signal<Option<PathBuf>>,
    selection: Signal<std::collections::BTreeSet<String>>,
    clipboard: Signal<Vec<sm::Note>>,
    view_row: Signal<i64>,
    cursor: Signal<i64>,
    snap_rows: Signal<i64>,
    note_kind: Signal<NoteKind>,
    epoch: Signal<u64>,
}
pub fn provide() {
    let studio = use_context_provider(|| DanceStudio {
        open: Signal::new(false),
        project: Signal::new(None),
        assets: Signal::new(Files::new()),
        root: Signal::new(None),
        saved: Signal::new(None),
        saving: Signal::new(false),
        blocked: Signal::new(false),
        error: Signal::new(None),
        status: Signal::new("Open a song folder or start from audio.".into()),
        selected: Signal::new(String::new()),
        audio: Signal::new(String::new()),
        clock: Signal::new(0.),
        motion_epoch: Signal::new(0),
        playing: Signal::new(false),
        playback_intent: Signal::new(0),
        length: Signal::new(0.),
        rate: Signal::new(1.),
        loop_range: Signal::new(None),
        busy: Signal::new(false),
        cancel: Signal::new(false),
        generation_open: Signal::new(false),
        review_open: Signal::new(false),
        export_open: Signal::new(false),
        after_new: Signal::new(false),
        parent: Signal::new(None),
        selection: Signal::new(Default::default()),
        clipboard: Signal::new(Vec::new()),
        view_row: Signal::new(0),
        cursor: Signal::new(0),
        snap_rows: Signal::new(12),
        note_kind: Signal::new(NoteKind::Tap),
        epoch: Signal::new(0),
    });
    // Imports, recovery and backups outlive the component that opened them.
    use_hook(move || dioxus::core::provide_root_context(studio));
}
impl DanceStudio {
    fn report(mut self, result: Result<()>) {
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    fn apply(mut self, action: Action) {
        let result = (|| {
            let mut project = self.project.write();
            let project = project.as_mut().context("Open a chart project")?;
            project.apply(Request {
                expected_revision: project.revision,
                actor: "human".into(),
                action,
            })
        })();
        if result.is_ok() {
            let selected = (self.selected)();
            let replacement = self.project.read().as_ref().and_then(|p| {
                (!p.content.document.charts.iter().any(|c| c.id == selected)).then(|| {
                    p.content
                        .document
                        .charts
                        .first()
                        .map_or(String::new(), |c| c.id.clone())
                })
            });
            if let Some(id) = replacement {
                self.selected.set(id);
            }
            let ids: std::collections::BTreeSet<_> = self
                .project
                .read()
                .as_ref()
                .map(|p| {
                    p.content
                        .document
                        .charts
                        .iter()
                        .filter(|c| c.id == (self.selected)())
                        .flat_map(|c| c.notes.iter().map(|n| n.id.clone()))
                        .collect()
                })
                .unwrap_or_default();
            self.selection.write().retain(|id| ids.contains(id));
        }
        self.report(result);
    }
    fn load(
        mut self,
        mut project: Project,
        assets: Files,
        root: Option<PathBuf>,
        saved: Option<u64>,
    ) -> Result<()> {
        ensure!(
            self.project
                .read()
                .as_ref()
                .is_none_or(|p| (self.saved)() == Some(p.revision)),
            "The current chart is still saving. Wait for it to save, or save a project backup before switching."
        );
        project.encode()?;
        if project.assets_sha256.is_empty() {
            bundle::bind(&mut project, &assets)?;
        }
        bundle::verify(&project, &assets)?;
        let audio = bundle::check_music(&project, &assets)?
            .map(|(name, bytes)| audio_url(name, bytes))
            .transpose()?
            .unwrap_or_default();
        if let Some(studio) = try_consume_context::<super::state::Studio>()
            && studio.snapshot().is_some_and(|p| p.transport.playing)
        {
            studio.dispatch(crate::editor::Action::Playback { playing: false });
        }
        pause_preview();
        #[cfg(target_arch = "wasm32")]
        let previous = if self.parent.read().is_none() {
            (self.root)()
        } else {
            None
        };
        let destination = root.unwrap_or_else(unique_root);
        self.selected.set(
            project
                .content
                .document
                .charts
                .first()
                .map_or(String::new(), |c| c.id.clone()),
        );
        self.project.set(Some(project));
        self.open.set(true);
        self.assets.set(assets);
        #[cfg(target_arch = "wasm32")]
        {
            let previous = self.audio.replace(audio);
            if previous.starts_with("blob:") {
                let _ = web_sys::Url::revoke_object_url(&previous);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.audio.set(audio);
        self.clock.set(0.);
        self.motion_epoch.set((self.motion_epoch)().wrapping_add(1));
        self.playing.set(false);
        self.length.set(0.);
        self.rate.set(1.);
        self.loop_range.set(None);
        self.selection.set(Default::default());
        self.view_row.set(0);
        self.cursor.set(0);
        #[cfg(target_arch = "wasm32")]
        if let Some(previous) = previous.filter(|p| p != &destination) {
            crate::storage::clear(&previous);
        }
        self.root.set(Some(destination));
        self.saved.set(saved);
        self.blocked.set(false);
        self.epoch.set((self.epoch)().wrapping_add(1));
        self.error.set(None);
        self.status
            .set("Draft ready. Review synchronization and playability before sharing.".into());
        Ok(())
    }
    async fn save(mut self) {
        if (self.saving)() || (self.blocked)() {
            return;
        }
        let Some(project) = self.project.read().clone() else {
            return;
        };
        if (self.saved)() == Some(project.revision) {
            return;
        }
        let Some(root) = (self.root)() else {
            return;
        };
        let expected = (self.saved)();
        self.saving.set(true);
        let result = async {
            if expected.is_none() {
                crate::storage::create_dir_all(root.join("source"))?;
                for (name, bytes) in self.assets.read().iter() {
                    let path = root.join("source").join(name);
                    crate::storage::create_dir_all(path.parent().unwrap())?;
                    crate::storage::write(path, bytes)?;
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                crate::storage::write(root.join(MANIFEST), project.encode()?)?;
                if self.parent.read().is_some() {
                    let mut studio = consume_context::<super::state::Studio>();
                    ensure!(
                        studio.store.read().as_ref().map(|s| &s.root)
                            == self.parent.read().as_ref(),
                        "Open the parent project before saving these parts"
                    );
                    studio.dirty.set(true);
                    studio.persist_current().await?;
                } else {
                    crate::persistence::save_stepmania(&root, project.revision, expected).await?;
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                crate::stepmania_files::save_document(&root, &project, expected)?;
                let parent = recovery_root();
                std::fs::create_dir_all(&parent)?;
                std::fs::write(parent.join("last"), root.to_string_lossy().as_bytes())?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.saving.set(false);
        if (self.root)().as_ref() == Some(&root) {
            match result {
                Ok(()) => {
                    self.saved.set(Some(project.revision));
                    if let Some(studio) = try_consume_context::<super::state::Studio>() {
                        remember_recent(studio);
                    }
                }
                Err(error) => {
                    self.blocked.set(true);
                    self.error.set(Some(format!("Autosave failed: {error:#}. Save a project ZIP to preserve this working copy.")));
                }
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn restore(self) {
        let result = async {
            let recovered = {
                let last = recovery_root().join("last");
                if last.exists() {
                    let root = PathBuf::from(std::fs::read_to_string(last)?);
                    let p = Project::decode(&std::fs::read(root.join(MANIFEST))?)?;
                    Some((root, p.revision))
                } else {
                    None
                }
            };
            if let Some((root, revision)) = recovered {
                let bytes = crate::storage::read(root.join(MANIFEST))?;
                let project = Project::decode(&bytes)?;
                let assets = crate::stepmania_files::read(&root.join("source"))?;
                if self.project.read().is_none() {
                    self.load(project, assets, Some(root), Some(revision))?;
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.report(result);
    }
    async fn prepare(self, format: Option<Format>) {
        self.prepare_with_options(format, crate::compact_backup::Options::default())
            .await;
    }
    async fn prepare_with_options(
        mut self,
        format: Option<Format>,
        options: crate::compact_backup::Options,
    ) {
        let result = async {
            let project = self.project.read().clone().context("No project")?;
            let files = if let Some(format) = format {
                bundle::export(&project, &self.assets.read(), format)?
            } else {
                bundle::backup(&project, &self.assets.read())?
            };
            #[cfg(not(target_arch = "wasm32"))]
            let parent = format
                .is_none()
                .then(|| self.parent.read().clone())
                .flatten();
            #[cfg(target_arch = "wasm32")]
            {
                let path = unique_root().join(if format.is_some() {
                    "YACE-song.zip"
                } else {
                    "YACE-project.zip"
                });
                let parent = format
                    .is_none()
                    .then(|| self.parent.read().clone())
                    .flatten();
                self.status.set("Preparing ZIP on a local worker…".into());
                crate::dance_archive::prepare(
                    files,
                    parent.as_deref(),
                    &path,
                    format.is_none().then_some(options),
                )
                .await?;
                self.status.set(format!(
                    "Download ready: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let mut studio = consume_context::<super::state::Studio>();
                let (name, kind) = if format.is_none() {
                    ("YACE-project.zip", super::state::DownloadKind::Project)
                } else {
                    studio.song_download_receipt.set(None);
                    ("YACE-song.zip", super::state::DownloadKind::Song)
                };
                if let Some((previous, _, _)) =
                    studio.export_ready.replace(Some((path, name.into(), kind)))
                {
                    crate::storage::clear(&previous);
                }
                self.export_open.set(false);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(file) = super::dialogs::AsyncFileDialog::new()
                    .set_file_name(if format.is_some() {
                        "YACE-song.zip"
                    } else {
                        "YACE-project.zip"
                    })
                    .save_file()
                    .await
                {
                    let output = file.path().to_owned();
                    self.status
                        .set("Preparing compact backup on a local worker…".into());
                    tokio::task::spawn_blocking(move || {
                        let mut files = files;
                        if let Some(parent) = parent {
                            let mut combined = crate::project_files::snapshot(
                                &crate::editor::Store::new(&parent),
                                options,
                            )?;
                            for (name, bytes) in files {
                                combined.insert(format!("dance/{name}"), bytes);
                            }
                            files = combined;
                        }
                        let mut stage = tempfile::NamedTempFile::new_in(
                            output.parent().context("Choose a backup location")?,
                        )?;
                        let entries = files
                            .iter()
                            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()));
                        if format.is_none() {
                            crate::compact_backup::encode_into(entries, &mut stage, options)?;
                        } else {
                            crate::archive_codec::encode_into(entries, &mut stage)?;
                        }
                        stage.as_file().sync_all()?;
                        stage
                            .persist_noclobber(&output)
                            .map_err(|error| error.error)
                            .context("Choose a new filename; existing files are preserved")?;
                        Ok::<_, anyhow::Error>(())
                    })
                    .await??;
                    let status = format!("Saved {}", file.path().display());
                    self.status.set(status.clone());
                    consume_context::<super::state::Studio>().status.set(status);
                    self.export_open.set(false);
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.report(result);
    }
    async fn generate(mut self, request: GenerationRequest) {
        let result=async{ensure!(!(self.busy)(),"Generation is running");let project=self.project.read().clone().context("No project")?;let name={let assets=self.assets.read();bundle::check_music(&project,&assets)?.context("Attach a recording first")?.0.to_owned()};let root=(self.root)().context("No project storage")?;
            self.save().await;ensure!(!(self.blocked)(),"Save the current project before generation");let audio=root.join("source").join(name);let revision=project.revision;let source_hash=project.source_sha256.clone();self.busy.set(true);self.cancel.set(false);self.status.set("Analyzing attacks and arranging draft charts…".into());
            #[cfg(not(target_arch="wasm32"))]
            let result={let cancelled=Arc::new(AtomicBool::new(false));let _cancel_on_drop=CancelOnDrop(cancelled.clone());let flag=cancelled.clone();let mut task=tokio::task::spawn_blocking(move ||{let samples=crate::models::decode(&audio,11025,1)?;let mut project=project;project.generate(revision,"human".into(),&request,&samples,11025,||flag.load(Ordering::Relaxed))?;Ok::<_,anyhow::Error>(project)});loop{tokio::select!{value=&mut task=>break value.context("Generation worker stopped")?,_=tokio::time::sleep(std::time::Duration::from_millis(40))=>{if (self.cancel)(){cancelled.store(true,Ordering::Relaxed);}}}}};
            #[cfg(target_arch="wasm32")]
            let result={let task=crate::worker::run::<Project>(crate::worker::Task::StepMania{audio,project:Box::new(project),request},std::slice::from_ref(&root));let cancellation=async{loop{gloo_timers::future::TimeoutFuture::new(40).await;if (self.cancel)(){break;}}};futures_util::pin_mut!(task,cancellation);match futures_util::future::select(task,cancellation).await{futures_util::future::Either::Left((result,_))=>result,futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Generation cancelled"))}};
            self.busy.set(false);ensure!(!(self.cancel)(),"Generation cancelled; existing charts kept");let candidate=result?;ensure!((self.root)().as_ref()==Some(&root)&&self.project.read().as_ref().is_some_and(|p|p.revision==revision&&p.source_sha256==source_hash),"Chart changed during generation; existing edits kept");self.selected.set(candidate.content.document.charts.last().map_or(String::new(),|c|c.id.clone()));self.project.set(Some(candidate));self.status.set("Generated drafts added. Review timing, patterns and estimated ratings.".into());Ok::<_,anyhow::Error>(())
        }.await;
        self.busy.set(false);
        self.report(result);
    }
}
#[cfg(not(target_arch = "wasm32"))]
struct CancelOnDrop(Arc<AtomicBool>);
#[cfg(not(target_arch = "wasm32"))]
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn recovery_root() -> PathBuf {
    std::env::var_os("YACE_DANCE_RECOVERY_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("yace-stepmania-recovery"))
}
#[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
pub async fn restore_legacy() {
    if let Some(studio) = try_consume_context::<DanceStudio>() {
        studio.restore().await;
    }
}
fn unique_root() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    {
        crate::files::unique_root().join("stepmania")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        recovery_root().join(crate::storage::unique_id())
    }
}
fn audio_url(name: &str, bytes: &[u8]) -> Result<String> {
    let mime = match std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        _ => "audio/wav",
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use base64::Engine;
        Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let parts = js_sys::Array::new();
        parts.push(&js_sys::Uint8Array::from(bytes));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime);
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
            .map_err(|e| anyhow::anyhow!("Audio Blob: {e:?}"))?;
        web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|e| anyhow::anyhow!("Audio URL: {e:?}"))
    }
}
#[component]
pub fn Controller(studio: super::state::Studio) -> Element {
    let mut s = use_context::<DanceStudio>();
    use_future(move || async move {
        #[cfg(all(not(target_arch = "wasm32"), feature = "browser"))]
        {
            s.restore().await;
            if studio.snapshot().is_some() {
                s.open.set(false);
            }
        }
        loop {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(500).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            s.save().await;
        }
    });
    rsx! {transport_backend::AudioEngine {} workspace::Dialogs {} if (s.export_open)(){super::sheet::ToolSheet{id:"chart-export",title:"Export chart",description:"Choose the game format for this recording.",close_label:"Close export",class:"parts-sheet",onclose:move |_|s.export_open.set(false),actions:rsx!{button{onclick:move |_|s.export_open.set(false),"Close"}},
        button{onclick:move |_|async move{s.prepare(Some(Format::Ssc)).await;},"Dance simfiles (.ssc)"}
        button{onclick:move |_|async move{s.prepare(Some(Format::Sm)).await;},"Legacy dance simfiles (.sm)"}
        if band_visible(studio)&&studio.snapshot().is_some(){button{onclick:move |_|{s.export_open.set(false);studio.export_dialog();},"MIDI / .chart formats…"}}
        if let Some(error)=(s.error)(){p{role:"alert","{error}"}}
    }}}
}
fn note_symbol(kind: NoteKind, lane: u8) -> &'static str {
    match kind {
        NoteKind::Mine => "✕",
        NoteKind::Lift => "◇",
        NoteKind::Fake => "○",
        NoteKind::Hold => ["←━", "↓━", "↑━", "→━"][lane as usize],
        NoteKind::Roll => ["←≋", "↓≋", "↑≋", "→≋"][lane as usize],
        NoteKind::Tap => ["←", "↓", "↑", "→"][lane as usize],
    }
}
#[component]
fn SongProperties(s: DanceStudio) -> Element {
    let p = s.project.read();
    let Some(p) = p.as_ref() else {
        return rsx! {};
    };
    let mut title = use_signal(|| {
        p.content
            .document
            .metadata
            .get("TITLE")
            .cloned()
            .unwrap_or_default()
    });
    let mut artist = use_signal(|| {
        p.content
            .document
            .metadata
            .get("ARTIST")
            .cloned()
            .unwrap_or_default()
    });
    let mut first = use_signal(|| (-p.content.document.timing.offset).to_string());
    let mut bpm = use_signal(|| p.content.document.timing.bpm(0.).to_string());
    rsx! {details{onkeydown:move|e|e.stop_propagation(),summary{"Song and synchronization"}label{"Title " input {value:"{title}",oninput:move |e|title.set(e.value())}}label{"Artist " input {value:"{artist}",oninput:move |e|artist.set(e.value())}}label{"Starting BPM " input {r#type:"number",min:"1",step:"any",value:"{bpm}",oninput:move |e|bpm.set(e.value())}}label{"First beat in recording (seconds) " input {r#type:"number",step:"0.001",value:"{first}",oninput:move |e|first.set(e.value())}}button{onclick:move |_|{let result=(||{let mut timing=s.project.read().as_ref().context("No project")?.content.document.timing.clone();timing.offset = -first().parse::<f64>()?;let starting=bpm().parse::<f64>()?;if let Some(event)=timing.events.iter_mut().find(|e|e.kind==TimingKind::Bpms&&e.beat==0.){event.values[0]=starting;}else{timing.events.push(Event::value(TimingKind::Bpms,0.,starting));}timing.validate()?;Ok::<_,anyhow::Error>(timing)})();match result{Ok(timing)=>s.apply(Action::SetSong{title:title(),artist:artist(),timing}),Err(e)=>s.report(Err(e))}},"Apply song details"}p{"Align the first beat by listening. Automatic tempo proposals still need review."}}}
}
#[component]
fn ChartProperties(s: DanceStudio, chart: Chart) -> Element {
    let mut difficulty = use_signal(|| chart.difficulty.clone());
    let mut meter = use_signal(|| chart.meter.to_string());
    let mut description = use_signal(|| chart.description.clone());
    rsx! {div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),label{"Difficulty " select {value:"{difficulty}",onchange:move |e|difficulty.set(e.value()),for d in Difficulty::GENERATED.into_iter().chain([Difficulty::Edit]){option{value:d.name(),selected:d.name()==difficulty(),"{d.name()}"}}}}label{"Meter (estimated for drafts) " input {r#type:"number",min:"1",max:"999",value:"{meter}",oninput:move |e|meter.set(e.value())}}label{"Description " input {value:"{description}",oninput:move |e|description.set(e.value())}}button{onclick:move |_|match meter().parse(){Ok(meter)=>s.apply(Action::SetChart{chart_id:chart.id.clone(),difficulty:difficulty(),meter,description:description()}),Err(e)=>s.report(Err(anyhow::Error::new(e)))},"Apply chart details"}}}
}
#[component]
fn TimingEditor(s: DanceStudio) -> Element {
    let mut local = use_signal(|| false);
    let mut selected = use_signal(|| None::<usize>);
    let mut kind = use_signal(|| TimingKind::Bpms);
    let mut beat = use_signal(|| "0".to_owned());
    let mut first = use_signal(|| "120".to_owned());
    let mut second = use_signal(|| "4".to_owned());
    let mut third = use_signal(|| false);
    let mut label = use_signal(String::new);
    let p = s.project.read();
    let Some(p) = p.as_ref() else {
        return rsx! {};
    };
    let id = (s.selected)();
    let chart = p.content.document.charts.iter().find(|c| c.id == id);
    let t = if local() {
        chart
            .and_then(|c| c.timing.as_ref())
            .unwrap_or(&p.content.document.timing)
    } else {
        &p.content.document.timing
    };
    let timing = t.clone();
    rsx! {details{summary{"Advanced timing"}label{"Offset (seconds) " input {r#type:"number",aria_label:if local(){"Chart offset seconds"}else{"Song offset seconds"},step:"0.001",value:"{timing.offset}",oninput:{let id=id.clone();let timing=timing.clone();move |event:FormEvent|{let result=(||{let mut timing=timing.clone();timing.offset=event.value().parse::<f64>()?;timing.validate()?;Ok::<_,anyhow::Error>(Action::SetTiming{chart_id:local().then(||id.clone()),timing:Some(timing)})})();match result{Ok(action)=>s.apply(action),Err(error)=>s.report(Err(error))}}}}}label{"Scope " select {value:if local(){"chart"}else{"song"},onchange:move |e|{local.set(e.value()=="chart");selected.set(None);},option{value:"song","Song timing"}option{value:"chart","Selected chart timing"}}}
        if local(){p{if chart.is_some_and(|c|c.timing.is_some()){"This chart has its own timing map."}else{"This chart inherits the song map. Editing creates a complete chart override."}}button{onclick:{let id=id.clone();move |_|s.apply(Action::SetTiming{chart_id:Some(id.clone()),timing:None})},"Inherit song timing"}}
        div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),
            label{"Event " select {value:kind().tag(),onchange:move |e|if let Some(k)=TimingKind::parse(&e.value()){kind.set(k);selected.set(None);first.set(if k==TimingKind::Bpms{"120"}else{"1"}.into());},for k in TimingKind::ALL{option{value:k.tag(),"{k.tag()}"}}}}
            label{"Beat " input {r#type:"number",step:"0.020833333",value:"{beat}",oninput:move |e|beat.set(e.value())}}
            if kind()==TimingKind::Labels{label{"Label " input {value:"{label}",oninput:move |e|label.set(e.value())}}}else{label{"Value " input {r#type:"number",step:"any",value:"{first}",oninput:move |e|first.set(e.value())}}}
            if matches!(kind(),TimingKind::Timesignatures|TimingKind::Combos|TimingKind::Speeds){label{if kind()==TimingKind::Speeds{"Transition length "}else if kind()==TimingKind::Timesignatures{"Denominator "}else{"Miss multiplier "}input{r#type:"number",value:"{second}",oninput:move |e|second.set(e.value())}}}
            if kind()==TimingKind::Speeds{label{input{r#type:"checkbox",checked:third(),onchange:move |e|third.set(e.checked())}"Transition uses seconds (otherwise beats)"}}
            button{onclick:{let id=id.clone();let timing=timing.clone();move |_|{let result=(||{let mut timing=timing.clone();let kind=kind();let values=if kind==TimingKind::Labels{Vec::new()}else if kind==TimingKind::Speeds{vec![first().parse()?,second().parse()?,if third(){1.}else{0.}]}else if matches!(kind,TimingKind::Timesignatures|TimingKind::Combos){vec![first().parse()?,second().parse()?]}else{vec![first().parse()?]};let event=Event{kind,beat:beat().parse()?,values,text:label()};event.validate()?;if let Some(index)=selected(){ensure!(index<timing.events.len(),"Timing event changed; select it again");timing.events[index]=event;}else{timing.events.push(event);}timing.validate()?;Ok::<_,anyhow::Error>(Action::SetTiming{chart_id:local().then(||id.clone()),timing:Some(timing)})})();match result{Ok(action)=>{s.apply(action);selected.set(None);},Err(error)=>s.report(Err(error))}}},if selected().is_some(){"Update event"}else{"Add event"}}
            button{onclick:move |_|selected.set(None),"New event"}
        }
        p{"BPM: beats/min · stops/delays: seconds · warps/fakes: beats · speeds/scrolls: multiplier. Stops happen after the note; delays before it."}
        div{class:"step-timing-list",for(index,event)in timing.events.iter().enumerate(){div{button{onclick:{let event=event.clone();move |_|{selected.set(Some(index));kind.set(event.kind);beat.set(event.beat.to_string());first.set(event.values.first().unwrap_or(&1.).to_string());second.set(event.values.get(1).unwrap_or(&4.).to_string());third.set(event.values.get(2)==Some(&1.));label.set(event.text.clone());}},"{event.kind.tag()} · beat {event.beat} · " {if event.kind==TimingKind::Labels{event.text.clone()}else{event.values.iter().map(ToString::to_string).collect::<Vec<_>>().join(" / ")}}}button{onclick:{let timing=timing.clone();let id=id.clone();move |_|{let mut timing=timing.clone();if index>=timing.events.len(){return;}timing.events.remove(index);s.apply(Action::SetTiming{chart_id:local().then(||id.clone()),timing:Some(timing.clone())});selected.set(None);}},"Delete"}}}}
    }}
}
#[cfg(all(test, not(feature = "browser"), not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::RefCell, rc::Rc};
    type Slot = Rc<RefCell<Option<DanceStudio>>>;
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    fn app() -> Element {
        provide();
        let studio = super::super::state::use_studio();
        super::super::difficulty::use_snapshot(studio);
        use_hook(move || dioxus::core::provide_root_context(studio));
        let mut s = consume_context::<DanceStudio>();
        let slot = use_context::<Slot>();
        use_hook(move || {
            let p = Project::blank(
                "UI fixture".into(),
                "YACE".into(),
                String::new(),
                120.,
                0.,
                Profile::Pad,
            )
            .unwrap();
            s.load(p, Files::new(), None, None).unwrap();
            s.open.set(true);
            *slot.borrow_mut() = Some(s);
            dioxus::core::spawn_forever(async move {
                let _ = consume_context::<super::super::state::Studio>();
                let mut shared = consume_context::<DanceStudio>();
                shared.status.set("Root task context verified".into());
            });
        });
        rsx! {super::super::workspace::Workbench{studio}if (studio.parts_open)(){super::super::parts::PartsPanel{studio}}}
    }
    #[tokio::test]
    async fn shared_workspace_adds_chart_and_renders_editing_controls() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let slot: Slot = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(slot.clone());
        let initial = dom.rebuild_to_vec();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while slot.borrow().unwrap().status.read().as_str() != "Root task context verified" {
                dom.wait_for_work().await;
                dom.render_immediate_to_vec();
            }
        })
        .await
        .expect("App services remain available to persistent tasks");
        // Dioxus associates the click listener with the button parent. Match
        // the accessibility title rather than relying on generated node IDs.
        let button = initial
            .edits
            .iter()
            .find_map(|m| match m {
                Mutation::SetAttribute {
                    name: "title",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Add a chart for pad" => Some(*id),
                _ => None,
            })
            .expect("Add chart button");
        dom.runtime().handle_event(
            "click",
            dioxus::prelude::Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::<
                    dioxus::html::SerializedMouseData,
                >::default(
                ))) as Rc<dyn Any>,
                true,
            ),
            button,
        );
        let mutations = dom.render_immediate_to_vec();
        let s = slot.borrow().unwrap();
        assert_eq!(
            s.project
                .read()
                .as_ref()
                .unwrap()
                .content
                .document
                .charts
                .len(),
            1
        );
        assert!(!mutations.edits.is_empty());
        assert_eq!((s.selected)(), "added-1");
        // Exercise the commands used by the shared toolbar against the dance
        // document, including sustain preservation and undo after paste.
        dom.in_scope(ScopeId::ROOT, || {
            let studio = consume_context::<super::super::state::Studio>();
            let mut s = s;
            let id = (s.selected)();
            s.apply(Action::AddNote {
                chart_id: id.clone(),
                row: 0,
                lane: 0,
                kind: NoteKind::Hold,
                end_row: Some(48),
            });
            let note = s.project.read().as_ref().unwrap().content.document.charts[0].notes[0]
                .id
                .clone();
            s.selection.set([note].into());
            super::super::session::copy(studio, false);
            s.cursor.set(96);
            super::super::session::paste(studio);
            let notes = s.project.read().as_ref().unwrap().content.document.charts[0]
                .notes
                .clone();
            assert_eq!(notes.len(), 2);
            assert_eq!((notes[1].row, notes[1].end_row), (96, Some(144)));
            super::super::session::undo(studio, false);
            assert_eq!(
                s.project.read().as_ref().unwrap().content.document.charts[0]
                    .notes
                    .len(),
                1
            );
            super::super::session::undo(studio, true);
            assert_eq!(
                s.project.read().as_ref().unwrap().content.document.charts[0]
                    .notes
                    .len(),
                2
            );
            super::super::session::rate(studio, 0.75);
            super::super::session::seek(studio, 2.);
            super::super::session::set_loop(
                studio,
                Some(crate::editor::TimeRange { start: 1., end: 3. }),
            );
            let transport = super::super::session::transport(studio).unwrap();
            assert_eq!(transport.playback_rate, 0.75);
            assert_eq!(
                transport.loop_range,
                Some(crate::editor::TimeRange { start: 1., end: 3. })
            );
            assert_eq!(super::super::session::position(studio), 2.);
            assert!(
                studio.snapshot().is_none(),
                "Dance remains its own chart data, without synthetic guitar tracks"
            );
        });
        let before = s.project.read().as_ref().unwrap().encode().unwrap();
        let replacement = Project::blank(
            "Other recording".into(),
            "YACE".into(),
            String::new(),
            120.,
            0.,
            Profile::Pad,
        )
        .unwrap();
        assert!(
            s.load(replacement, Files::new(), None, None)
                .unwrap_err()
                .to_string()
                .contains("current chart")
        );
        assert_eq!(s.project.read().as_ref().unwrap().encode().unwrap(), before);
    }
    type RecentSlot = Rc<RefCell<Option<super::super::state::Studio>>>;
    #[tokio::test]
    async fn sustain_length_change_commits_to_the_document_backup_and_undo() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let slot: Slot = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(slot.clone());
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::ROOT, || {
            let mut s = slot.borrow().unwrap();
            s.apply(Action::AddChart {
                difficulty: Difficulty::Edit,
                meter: 1,
                description: "Manual".into(),
            });
            s.apply(Action::AddNote {
                chart_id: (s.selected)(),
                row: 48,
                lane: 0,
                kind: NoteKind::Hold,
                end_row: Some(96),
            });
            let id = s.project.read().as_ref().unwrap().content.document.charts[0].notes[0]
                .id
                .clone();
            s.selection.set([id].into());
        });
        let changes = dom.render_immediate_to_vec();
        let input = changes
            .edits
            .iter()
            .find_map(|m| match m {
                Mutation::SetAttribute {
                    name: "aria-label",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Hold length in beats" => Some(*id),
                _ => None,
            })
            .expect("Sustain property in the common Selection pane");
        let offset = changes
            .edits
            .iter()
            .find_map(|m| match m {
                Mutation::SetAttribute {
                    name: "aria-label",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Song offset seconds" => Some(*id),
                _ => None,
            })
            .expect("Advanced timing offset field");
        dom.runtime().handle_event(
            "change",
            dioxus::prelude::Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::new(
                    dioxus::html::SerializedFormData {
                        value: "2".into(),
                        values: Vec::new(),
                        valid: true,
                    },
                ))) as Rc<dyn Any>,
                true,
            ),
            input,
        );
        dom.render_immediate_to_vec();
        let s = slot.borrow().unwrap();
        let project = s.project.read().as_ref().unwrap().clone();
        assert_eq!(
            project.content.document.charts[0].notes[0].end_row,
            Some(144)
        );
        let files = bundle::backup(&project, &Files::new()).unwrap();
        let restored = Project::decode(&files[MANIFEST]).unwrap();
        assert_eq!(
            restored.content.document.charts[0].notes[0].end_row,
            Some(144)
        );
        dom.in_scope(ScopeId::ROOT, || {
            history(false);
        });
        assert_eq!(
            s.project.read().as_ref().unwrap().content.document.charts[0].notes[0].end_row,
            Some(96)
        );
        dom.render_immediate_to_vec();
        dom.runtime().handle_event(
            "input",
            dioxus::prelude::Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::new(
                    dioxus::html::SerializedFormData {
                        value: "0.125".into(),
                        values: Vec::new(),
                        valid: true,
                    },
                ))) as Rc<dyn Any>,
                true,
            ),
            offset,
        );
        dom.render_immediate_to_vec();
        assert_eq!(
            s.project
                .read()
                .as_ref()
                .unwrap()
                .content
                .document
                .timing
                .offset,
            0.125
        );
        dom.in_scope(ScopeId::ROOT, || history(false));
        assert_eq!(
            s.project
                .read()
                .as_ref()
                .unwrap()
                .content
                .document
                .timing
                .offset,
            0.
        );
    }
    fn recent_app() -> Element {
        provide();
        let studio = super::super::state::use_studio();
        use_hook(move || dioxus::core::provide_root_context(studio));
        let (path, slot) = use_context::<(PathBuf, RecentSlot)>();
        use_hook(move || {
            *slot.borrow_mut() = Some(studio);
            dioxus::core::spawn_forever(async move {
                studio.open_recent(path).await;
            });
        });
        rsx! {super::super::workspace::Workbench{studio}}
    }
    #[tokio::test]
    async fn recent_opens_a_dance_project_without_creating_guitar_tracks() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Recent dance");
        let mut project = Project::blank(
            "Recent dance".into(),
            "YACE".into(),
            String::new(),
            120.,
            0.,
            Profile::Pad,
        )
        .unwrap();
        project
            .apply(Request {
                expected_revision: 0,
                actor: "fixture".into(),
                action: Action::AddChart {
                    difficulty: Difficulty::Edit,
                    meter: 4,
                    description: "Manual".into(),
                },
            })
            .unwrap();
        crate::stepmania_files::write_new(&path, &bundle::backup(&project, &Files::new()).unwrap())
            .unwrap();
        let slot = RecentSlot::default();
        let mut dom = VirtualDom::new(recent_app);
        dom.provide_root_context((path.clone(), slot.clone()));
        dom.rebuild_in_place();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while slot
                .borrow()
                .as_ref()
                .is_none_or(|studio| studio.recent_projects.read().entries.is_empty())
            {
                dom.wait_for_work().await;
                dom.render_immediate_to_vec();
            }
        })
        .await
        .expect("Recent dance project opens through the shared workflow");
        let studio = slot.borrow().unwrap();
        assert!(studio.snapshot().is_none());
        assert!((studio.error)().is_none());
        assert_eq!(studio.recent_projects.read().entries[0].path, path);
        assert_eq!(
            studio.recent_projects.read().entries[0].title,
            "YACE — Recent dance"
        );
    }
    #[tokio::test]
    async fn switching_peer_instruments_keeps_the_source_transport_and_history() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let folder = tempfile::tempdir().unwrap();
        let store = fixture::fixture(folder.path());
        let slot: Slot = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(slot.clone());
        dom.rebuild_in_place();
        let studio = dom.in_scope(ScopeId::ROOT, || {
            let mut s = slot.borrow().unwrap();
            let mut studio = consume_context::<super::super::state::Studio>();
            s.apply(Action::AddChart {
                difficulty: Difficulty::Hard,
                meter: 7,
                description: "Pad".into(),
            });
            let pad = s.project.read().as_ref().unwrap().content.document.charts[0]
                .id
                .clone();
            s.apply(Action::SetProfile {
                profile: Profile::Keyboard,
            });
            s.apply(Action::AddChart {
                difficulty: Difficulty::Hard,
                meter: 7,
                description: "Keys".into(),
            });
            let keyboard = s.project.read().as_ref().unwrap().content.document.charts[1]
                .id
                .clone();
            let mut doc = store.load().unwrap();
            doc.transport.playing = true;
            doc.transport.position = 2.5;
            doc.transport.playback_rate = 0.75;
            doc.transport.loop_range = Some(crate::editor::TimeRange { start: 1., end: 3. });
            let audio_hash = doc.audio_sha256.clone();
            studio.doc.set(Some(std::sync::Arc::new(doc)));
            studio.store.set(Some(store.clone()));
            studio.playhead.set(2.5);
            studio.audio_ready.set(true);
            s.parent.set(Some(store.root.clone()));
            {
                let mut p = s.project.write();
                let p = p.as_mut().unwrap();
                p.content
                    .document
                    .metadata
                    .insert("MUSIC".into(), "song.ogg".into());
                p.assets_sha256.insert("song.ogg".into(), audio_hash);
            }
            s.open.set(false);
            let band_before = serde_json::to_value(studio.snapshot().unwrap().as_ref()).unwrap();
            let dance_before = s.project.read().as_ref().unwrap().encode().unwrap();
            for id in [pad, keyboard] {
                select_track(s, id);
                assert!(uses_recording(studio));
                let t = super::super::session::transport(studio).unwrap();
                assert!(t.playing && t.ready);
                assert_eq!(t.playback_rate, 0.75);
                assert_eq!(
                    t.loop_range,
                    Some(crate::editor::TimeRange { start: 1., end: 3. })
                );
                assert_eq!(super::super::session::position(studio), 2.5);
                deactivate();
            }
            assert_eq!(
                serde_json::to_value(studio.snapshot().unwrap().as_ref()).unwrap(),
                band_before
            );
            assert_eq!(
                s.project.read().as_ref().unwrap().encode().unwrap(),
                dance_before,
                "Selecting a different profile must not create an edit or undo entry"
            );
            select_track(s, (s.selected)());
            studio.parts_open.set(true);
            studio
        });
        let changes = dom.render_immediate_to_vec();
        let add = changes
            .edits
            .iter()
            .find_map(|m| match m {
                Mutation::SetAttribute {
                    name: "aria-label",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Add blank Bass" => Some(*id),
                _ => None,
            })
            .expect("The same Add Part dialog lists band parts while Dance is selected");
        dom.runtime().handle_event(
            "click",
            dioxus::prelude::Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::<
                    dioxus::html::SerializedMouseData,
                >::default(
                ))) as Rc<dyn Any>,
                true,
            ),
            add,
        );
        dom.render_immediate_to_vec();
        assert!(!*slot.borrow().unwrap().open.peek());
        assert_eq!(
            studio.snapshot().unwrap().selected_track,
            "ExpertDoubleBass"
        );
    }
}

pub fn active() -> bool {
    try_consume_context::<DanceStudio>().is_some_and(|s| (s.open)() && s.project.read().is_some())
}
/// Observe the current durable workspace, not a background save's captured
/// root. A linked chart belongs to its band workspace even while visible.
#[cfg(target_arch = "wasm32")]
pub fn browser_workspace_target(
    studio: super::state::Studio,
) -> Option<crate::workspace_recovery::Target> {
    use crate::workspace_recovery::{Family, Target};
    let s = try_consume_context::<DanceStudio>()?;
    if (s.open)() && s.parent.read().is_none() {
        return (s.saved)()
            .and_then(|_| (s.root)())
            .filter(|root| !crate::project_folder::linked(root))
            .map(|root| Target::new(Family::Stepmania, root));
    }
    (studio.saved_revision)()?;
    studio
        .store
        .read()
        .as_ref()
        .filter(|store| !crate::project_folder::linked(&store.root))
        .map(|store| Target::new(Family::Band, store.root.clone()))
}
#[cfg(target_arch = "wasm32")]
pub fn standalone_folder() -> Option<PathBuf> {
    let s = try_consume_context::<DanceStudio>()?;
    if !(s.open)() || s.parent.read().is_some() {
        return None;
    }
    (s.root)()
}
pub fn view_epoch() -> u64 {
    try_consume_context::<DanceStudio>().map_or(0, |s| (s.epoch)())
}
pub(super) fn overview_source() -> Option<(PathBuf, u64)> {
    let s = try_consume_context::<DanceStudio>()?;
    Some(((s.root)()?, (s.epoch)()))
}
pub(super) fn overview_playback_stamp() -> Option<u64> {
    try_consume_context::<DanceStudio>().map(|s| (s.playback_intent)())
}
pub(super) fn overview_window(studio: super::state::Studio) -> (f64, f64) {
    workspace::overview_window(studio)
}
pub(super) fn overview_reveal(studio: super::state::Studio, seconds: f64) {
    workspace::overview_reveal(studio, seconds)
}
pub(super) fn overview_pan(studio: super::state::Studio, amount: f64) {
    workspace::overview_pan(studio, amount)
}
pub fn save_status() -> Option<&'static str> {
    let s = try_consume_context::<DanceStudio>()?;
    if !(s.open)() {
        return None;
    }
    let revision = s.project.read().as_ref()?.revision;
    #[cfg(target_arch = "wasm32")]
    let linked = s
        .parent
        .read()
        .clone()
        .or_else(|| (s.root)())
        .is_some_and(|root| crate::project_folder::linked(&root));
    Some(if (s.blocked)() {
        if cfg!(target_arch = "wasm32") {
            "Not saved · download project backup"
        } else {
            "Not saved · save project backup"
        }
    } else if (s.saved)() == Some(revision) {
        #[cfg(target_arch = "wasm32")]
        if linked {
            return Some("Saved to project folder");
        }
        if cfg!(target_arch = "wasm32") {
            "Saved in this browser"
        } else {
            "Saved locally"
        }
    } else if cfg!(target_arch = "wasm32") {
        #[cfg(target_arch = "wasm32")]
        if linked {
            return Some("Saving to project folder…");
        }
        "Saving in this browser…"
    } else {
        "Saving locally…"
    })
}
/// Hidden linked parts may still be saving when the band editor is visible.
#[cfg(target_arch = "wasm32")]
pub fn has_unsaved_changes() -> bool {
    try_consume_context::<DanceStudio>().is_some_and(|s| {
        s.project.read().as_ref().is_some_and(|project| {
            (s.saving)() || (s.blocked)() || (s.saved)() != Some(project.revision)
        })
    })
}
pub fn identity() -> Option<(String, String)> {
    let s = try_consume_context::<DanceStudio>()?;
    if !(s.open)() {
        return None;
    }
    let p = s.project.read();
    let p = p.as_ref()?;
    Some((
        p.content
            .document
            .metadata
            .get("TITLE")
            .cloned()
            .unwrap_or_default(),
        p.content
            .document
            .metadata
            .get("ARTIST")
            .cloned()
            .unwrap_or_default(),
    ))
}
pub fn new_recording(studio: super::state::Studio, generate: bool) {
    if studio.file_activity.peek().is_some() || *studio.library_busy.peek() {
        return;
    }
    if let Some(mut s) = try_consume_context::<DanceStudio>() {
        s.after_new.set(generate);
    }
    studio.new_chart();
}
pub fn generate(mut studio: super::state::Studio) {
    if active() {
        consume_context::<DanceStudio>().generation_open.set(true);
    } else if studio.snapshot().is_some() {
        studio.generation_open.set(true);
    } else {
        new_recording(studio, true);
    }
}
pub fn export(studio: super::state::Studio) {
    if active() {
        consume_context::<DanceStudio>().export_open.set(true);
    } else {
        studio.export_dialog();
    }
}
pub fn review(mut studio: super::state::Studio) {
    if active() {
        consume_context::<DanceStudio>().review_open.set(true);
    } else {
        studio.review_open.set(true);
    }
}
pub fn save_with_options(studio: super::state::Studio, options: crate::compact_backup::Options) {
    let linked = try_consume_context::<DanceStudio>().is_some_and(|s| {
        s.parent.read().is_some()
            && s.parent.read().as_ref() == studio.store.read().as_ref().map(|s| &s.root)
    });
    if active() || linked {
        let Some(guard) = super::activity::FileGuard::start(
            studio.file_activity,
            studio.status,
            super::activity::FileAction::Backup,
        ) else {
            return;
        };
        let Some(busy) = super::activity::BusyGuard::start(studio.export_busy) else {
            return;
        };
        let s = consume_context::<DanceStudio>();
        dioxus::core::spawn_forever(async move {
            let (_guard, _busy) = (guard, busy);
            s.save().await;
            s.prepare_with_options(None, options).await;
        });
    } else {
        studio.save_project_with_options(options);
    }
}
pub fn deactivate() {
    if let Some(mut s) = try_consume_context::<DanceStudio>() {
        if (s.open)() {
            pause_preview();
        }
        s.open.set(false);
    }
}
fn pause_preview() {
    if let Some(mut s) = try_consume_context::<DanceStudio>() {
        s.playing.set(false);
    }
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(audio) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("dance-audio"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlAudioElement>().ok())
        {
            let _ = audio.pause();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if try_consume_context::<DanceStudio>().is_some_and(|s| (s.open)()) {
            let evaluation =
                dioxus_document::eval("document.getElementById('dance-audio')?.pause();");
            dioxus::core::spawn_forever(async move {
                let _ = evaluation.await;
            });
        }
    }
}
pub async fn flush() -> Result<()> {
    let Some(mut s) = try_consume_context::<DanceStudio>() else {
        return Ok(());
    };
    if (s.busy)() {
        s.cancel.set(true);
    }
    while (s.saving)() || (s.busy)() {
        #[cfg(target_arch = "wasm32")]
        gloo_timers::future::TimeoutFuture::new(20).await;
        #[cfg(not(target_arch = "wasm32"))]
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    s.save().await;
    ensure!(
        s.project
            .read()
            .as_ref()
            .is_none_or(|p| (s.saved)() == Some(p.revision)),
        "Save a project backup before switching: the current chart could not be saved"
    );
    Ok(())
}
pub fn remember_recent(mut studio: super::state::Studio) {
    let Some(s) = try_consume_context::<DanceStudio>() else {
        return;
    };
    if s.parent.read().is_some() {
        return;
    }
    let Some(root) = (s.root)() else {
        return;
    };
    if !crate::storage::is_file(root.join(MANIFEST)) {
        return;
    }
    let project = s.project.read();
    let Some(project) = project.as_ref() else {
        return;
    };
    let metadata = &project.content.document.metadata;
    let fields = std::collections::BTreeMap::from([
        (
            "name".into(),
            metadata.get("TITLE").cloned().unwrap_or_default(),
        ),
        (
            "artist".into(),
            metadata.get("ARTIST").cloned().unwrap_or_default(),
        ),
    ]);
    match crate::recent_projects::History::record_open(root.clone(), crate::recent_projects::title(&fields, &root)) {
        Ok(history) => studio.recent_projects.set(history),
        Err(error) => studio
            .error
            .set(Some(format!("Project opened, but recent history could not be updated: {error:#}. Existing history has been kept."))),
    }
}
pub fn after_new() -> bool {
    try_consume_context::<DanceStudio>().is_some_and(|s| (s.after_new)())
}
fn timing_from_band(timing: &crate::timing::TempoMap) -> Timing {
    Timing {
        offset: -timing.offset,
        events: timing
            .tempos
            .iter()
            .map(|t| {
                Event::value(
                    TimingKind::Bpms,
                    t.tick as f64 / timing.resolution as f64,
                    t.bpm,
                )
            })
            .collect(),
    }
}
pub struct NewDestination {
    pub profile: Profile,
    pub location: super::new_location::Choice,
}
pub async fn finish_new(
    mut studio: super::state::Studio,
    source: PathBuf,
    title: String,
    artist: String,
    timing: crate::timing::TempoMap,
    info: crate::song_import::SongInfo,
    destination: NewDestination,
) -> Result<()> {
    let NewDestination { profile, location } = destination;
    flush().await?;
    let mut s = consume_context::<DanceStudio>();
    let generate = (s.after_new)();
    studio
        .finish_new_chart(source, title, artist, timing, info, location)
        .await?;
    if let Some(error) = (studio.error)() {
        anyhow::bail!(error);
    }
    studio.generation_open.set(false);
    from_recording(studio, profile, generate);
    s.after_new.set(false);
    ensure!(
        active(),
        "The song was created, but its dance part could not be added"
    );
    Ok(())
}
/// Attach dance charts to the current recording without routing them through
/// guitar lanes. The sidecar is included in normal project backups.
pub fn from_recording(mut studio: super::state::Studio, profile: Profile, generate: bool) {
    let was_playing = studio.snapshot().is_some_and(|p| p.transport.playing);
    if active() && (!band_visible(studio) || studio.snapshot().is_none()) {
        let mut s = consume_context::<DanceStudio>();
        s.apply(Action::SetProfile { profile });
        s.apply(Action::AddChart {
            difficulty: Difficulty::Edit,
            meter: 1,
            description: if profile == Profile::Pad {
                "Dance".into()
            } else {
                "4-key".into()
            },
        });
        if let Some(c) = s
            .project
            .read()
            .as_ref()
            .and_then(|p| p.content.document.charts.last())
        {
            s.selected.set(c.id.clone());
        }
        s.selection.set(Default::default());
        s.generation_open.set(generate);
        studio.parts_open.set(false);
        return;
    }
    let result: Result<()> = (|| {
        let store = studio.store.read().clone().context("Open a song first")?;
        let source = studio
            .source_audio
            .read()
            .clone()
            .context("Attach a recording before adding this part")?;
        let mut s = consume_context::<DanceStudio>();
        let root = store.root.join("dance");
        let reused = s.root.read().as_ref() == Some(&root) && s.project.read().is_some();
        if !reused && !try_open_path(&root)? {
            let name = source
                .file_name()
                .context("No recording filename")?
                .to_string_lossy()
                .into_owned();
            let mut p = Project::blank(
                studio.metadata_value("name"),
                studio.metadata_value("artist"),
                name.clone(),
                120.,
                0.,
                profile,
            )?;
            if let Some(timing) = studio.timing.read().as_ref() {
                p.content.document.timing = timing_from_band(timing);
            }
            let assets = Files::from([(name, crate::storage::read(source)?)]);
            s.load(p, assets, Some(root), None)?;
        }
        #[cfg(target_arch = "wasm32")]
        if !reused {
            crate::persistence::remember_linked(&store.root, (s.saved)());
        }
        s.parent.set(Some(store.root));
        s.apply(Action::SetProfile { profile });
        s.apply(Action::AddChart {
            difficulty: Difficulty::Edit,
            meter: 1,
            description: if profile == Profile::Pad {
                "Dance".into()
            } else {
                "4-key".into()
            },
        });
        if let Some(c) = s
            .project
            .read()
            .as_ref()
            .and_then(|p| p.content.document.charts.last())
        {
            s.selected.set(c.id.clone());
        }
        s.open.set(true);
        s.generation_open.set(generate);
        s.selection.set(Default::default());
        studio.parts_open.set(false);
        studio.dispatch(crate::editor::Action::Playback {
            playing: was_playing,
        });
        Ok(())
    })();
    if let Err(error) = result {
        studio.error.set(Some(format!("{error:#}")));
    }
}
pub fn is_source(path: &std::path::Path) -> bool {
    if crate::storage::is_file(path.join(MANIFEST)) {
        return true;
    }
    if ["project.json", "notes.chart", "notes.mid"]
        .iter()
        .any(|name| crate::storage::is_file(path.join(name)))
    {
        return false;
    }
    crate::storage::read_dir(path).ok().is_some_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            e.path()
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("sm") || e.eq_ignore_ascii_case("ssc"))
        })
    })
}
pub fn try_open_path(path: &std::path::Path) -> Result<bool> {
    if !is_source(path) {
        return Ok(false);
    }
    let Some(mut s) = try_consume_context::<DanceStudio>() else {
        return Ok(false);
    };
    #[cfg(not(target_arch = "wasm32"))]
    let files = crate::stepmania_files::read(path)?;
    #[cfg(target_arch = "wasm32")]
    let files = crate::storage::list(path)?
        .into_iter()
        .map(|(p, b)| {
            Ok((
                p.strip_prefix(path)?.to_string_lossy().into_owned(),
                b.as_ref().clone(),
            ))
        })
        .collect::<Result<Files>>()?;
    let saved = files.contains_key(MANIFEST);
    let (p, assets) = if saved {
        bundle::restore(files)?
    } else {
        let choices = bundle::candidates(&files);
        let selected = choices
            .iter()
            .find(|p| p.to_ascii_lowercase().ends_with(".ssc"))
            .or(choices.first())
            .context("No dance chart")?
            .clone();
        bundle::import(files, &selected, Profile::Pad)?
    };
    let revision = p.revision;
    let destination = if saved { Some(path.to_owned()) } else { None };
    s.load(
        p,
        assets,
        destination,
        if saved { Some(revision) } else { None },
    )?;
    s.parent.set(None);
    Ok(true)
}
/// Hydrate the linked family whenever the ordinary band project is opened.
pub fn attach_existing(studio: super::state::Studio) {
    let Some(root) = studio.store.read().as_ref().map(|s| s.root.clone()) else {
        return;
    };
    match try_open_path(&root.join("dance")) {
        Ok(true) => {
            let mut s = consume_context::<DanceStudio>();
            #[cfg(target_arch = "wasm32")]
            crate::persistence::remember_linked(
                &root,
                if (studio.saved_revision)().is_some() {
                    (s.saved)()
                } else {
                    None
                },
            );
            s.parent.set(Some(root));
            s.open.set(false);
            super::track_views::restore_linked_selection(studio);
        }
        Ok(false) => {}
        Err(error) => {
            let mut studio = studio;
            studio.error.set(Some(format!(
                "Unable to restore Dance/4-key parts: {error:#}"
            )));
        }
    }
}
pub fn band_visible(studio: super::state::Studio) -> bool {
    try_consume_context::<DanceStudio>().is_none_or(|s| {
        !(s.open)()
            || s.parent.read().as_ref().is_some_and(|root| {
                studio
                    .store
                    .read()
                    .as_ref()
                    .is_some_and(|store| &store.root == root)
            })
    })
}
pub fn try_open_files(files: Files) -> Result<bool> {
    let Some(selected) = bundle::entry(&files) else {
        return Ok(false);
    };
    let Some(mut s) = try_consume_context::<DanceStudio>() else {
        return Ok(false);
    };
    if std::path::Path::new(&selected)
        .file_name()
        .is_some_and(|n| n == MANIFEST)
    {
        let (p, assets) = bundle::restore(files)?;
        s.load(p, assets, None, None)?;
        s.parent.set(None);
        return Ok(true);
    }
    let (p, assets) = bundle::import(files, &selected, Profile::Pad)?;
    s.load(p, assets, None, None)?;
    s.parent.set(None);
    Ok(true)
}
pub fn tracks(studio: super::state::Studio) -> Vec<(String, String)> {
    let Some(s) = try_consume_context::<DanceStudio>() else {
        return Vec::new();
    };
    if !(s.open)() && s.parent.read().as_ref() != studio.store.read().as_ref().map(|s| &s.root) {
        return Vec::new();
    }
    s.project
        .read()
        .as_ref()
        .map(|p| {
            p.content
                .document
                .charts
                .iter()
                .enumerate()
                .map(|(index, c)| {
                    let duplicate = p
                        .content
                        .document
                        .charts
                        .iter()
                        .filter(|other| {
                            other.profile == c.profile
                                && other.difficulty == c.difficulty
                                && other.meter == c.meter
                        })
                        .count()
                        > 1;
                    let suffix = if duplicate {
                        format!(
                            " · {} ({})",
                            if c.description.is_empty() {
                                "Chart"
                            } else {
                                &c.description
                            },
                            index + 1
                        )
                    } else {
                        String::new()
                    };
                    (
                        format!("dance:{}", c.id),
                        format!(
                            "{} · {} · {}{}",
                            if c.profile.unwrap_or(p.content.profile) == Profile::Pad {
                                "Dance"
                            } else {
                                "4-key"
                            },
                            c.difficulty,
                            c.meter,
                            suffix
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
pub fn selected_track() -> Option<String> {
    let s = try_consume_context::<DanceStudio>()?;
    (s.open)().then(|| format!("dance:{}", (s.selected)()))
}
pub fn select_track(mut s: DanceStudio, id: String) {
    if !s
        .project
        .read()
        .as_ref()
        .is_some_and(|p| p.content.document.charts.iter().any(|c| c.id == id))
    {
        return;
    }
    if let Some(studio) = try_consume_context::<super::state::Studio>() {
        if !uses_recording(studio) && studio.snapshot().is_some_and(|p| p.transport.playing) {
            studio.dispatch(crate::editor::Action::Playback { playing: false });
        }
        set_snap((studio.snap)());
    }
    s.selected.set(id);
    s.open.set(true);
    s.selection.set(Default::default());
}
pub fn uses_recording(studio: super::state::Studio) -> bool {
    let Some(s) = try_consume_context::<DanceStudio>() else {
        return false;
    };
    let Some(doc) = studio.snapshot() else {
        return false;
    };
    s.parent.read().as_ref().is_some_and(|root| {
        studio
            .store
            .read()
            .as_ref()
            .is_some_and(|store| &store.root == root)
    }) && s.project.read().as_ref().is_some_and(|p| {
        p.content
            .document
            .metadata
            .get("MUSIC")
            .and_then(|name| p.assets_sha256.get(name))
            == Some(&doc.audio_sha256)
    })
}

pub fn linked_root(studio: super::state::Studio) -> Option<PathBuf> {
    let state = try_consume_context::<DanceStudio>()?;
    let root = state.parent.read().clone()?;
    studio
        .store
        .read()
        .as_ref()
        .filter(|store| store.root == root)
        .map(|_| root)
}
impl DanceStudio {
    fn edit_selection(self, edit: sm::project::SelectionEdit) {
        let note_ids = self.selection.read().iter().cloned().collect();
        self.apply(Action::EditNotes {
            chart_id: (self.selected)(),
            note_ids,
            edit,
        });
    }
    fn copy_selection(mut self) {
        let notes = self
            .project
            .read()
            .as_ref()
            .and_then(|p| {
                p.content
                    .document
                    .charts
                    .iter()
                    .find(|c| c.id == (self.selected)())
                    .map(|c| {
                        c.notes
                            .iter()
                            .filter(|n| self.selection.read().contains(&n.id))
                            .cloned()
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        self.clipboard.set(notes);
    }
    fn paste(self) {
        self.apply(Action::PasteNotes {
            chart_id: (self.selected)(),
            notes: (self.clipboard)(),
            row: (self.cursor)(),
        });
    }
}

pub fn source_identity() -> Option<PathBuf> {
    if active() && !try_consume_context::<super::state::Studio>().is_some_and(uses_recording) {
        (consume_context::<DanceStudio>().root)()
    } else {
        None
    }
}
pub fn history(redo: bool) {
    consume_context::<DanceStudio>().apply(if redo { Action::Redo } else { Action::Undo });
}
pub fn history_available() -> (bool, bool) {
    let s = consume_context::<DanceStudio>();
    s.project.read().as_ref().map_or((false, false), |p| {
        (
            !(s.busy)() && !p.undo.is_empty(),
            !(s.busy)() && !p.redo.is_empty(),
        )
    })
}
pub fn has_selection() -> bool {
    !consume_context::<DanceStudio>().selection.read().is_empty()
}
pub fn copy(cut: bool) {
    let s = consume_context::<DanceStudio>();
    s.copy_selection();
    if cut {
        s.edit_selection(sm::project::SelectionEdit::Delete);
    }
}
pub fn paste() {
    consume_context::<DanceStudio>().paste();
}
pub fn delete() {
    consume_context::<DanceStudio>().edit_selection(sm::project::SelectionEdit::Delete);
}
pub fn clear() {
    consume_context::<DanceStudio>()
        .selection
        .set(Default::default());
}
pub fn select_all() {
    let mut s = consume_context::<DanceStudio>();
    let ids = s
        .project
        .read()
        .as_ref()
        .and_then(|p| {
            p.content
                .document
                .charts
                .iter()
                .find(|c| c.id == (s.selected)())
                .map(|c| c.notes.iter().map(|n| n.id.clone()).collect())
        })
        .unwrap_or_default();
    s.selection.set(ids);
}
pub fn set_snap(value: u32) {
    if let Some(mut s) = try_consume_context::<DanceStudio>() {
        s.snap_rows.set(if value == 0 {
            1
        } else {
            (48 / value.max(1)) as i64
        });
    }
}
pub fn handle_navigation(event: &KeyboardEvent) -> bool {
    let mut s = consume_context::<DanceStudio>();
    match event.key() {
        Key::ArrowUp => s.cursor.set(((s.cursor)() - (s.snap_rows)()).max(0)),
        Key::ArrowDown => s
            .cursor
            .set(((s.cursor)() + (s.snap_rows)()).min(sm::MAX_ROW)),
        Key::PageUp => s.view_row.set(((s.view_row)() - 192).max(0)),
        Key::PageDown => s.view_row.set(((s.view_row)() + 192).min(sm::MAX_ROW)),
        Key::Home => {
            s.cursor.set(0);
            s.view_row.set(0);
        }
        _ => return false,
    }
    event.prevent_default();
    event.stop_propagation();
    true
}
