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
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
    busy: Signal<bool>,
    cancel: Signal<bool>,
    pub generation_open: Signal<bool>,
    pub review_open: Signal<bool>,
    pub export_open: Signal<bool>,
    pub after_new: Signal<bool>,
    prepared: Signal<Option<PathBuf>>,
    parent: Signal<Option<PathBuf>>,
    selection: Signal<std::collections::BTreeSet<String>>,
    clipboard: Signal<Vec<sm::Note>>,
    view_row: Signal<i64>,
    cursor: Signal<i64>,
    snap_rows: Signal<i64>,
    note_kind: Signal<NoteKind>,
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
        busy: Signal::new(false),
        cancel: Signal::new(false),
        generation_open: Signal::new(false),
        review_open: Signal::new(false),
        export_open: Signal::new(false),
        after_new: Signal::new(false),
        prepared: Signal::new(None),
        parent: Signal::new(None),
        selection: Signal::new(Default::default()),
        clipboard: Signal::new(Vec::new()),
        view_row: Signal::new(0),
        cursor: Signal::new(0),
        snap_rows: Signal::new(12),
        note_kind: Signal::new(NoteKind::Tap),
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
            let ids: std::collections::BTreeSet<_> = self
                .project
                .read()
                .as_ref()
                .map(|p| {
                    p.content
                        .document
                        .charts
                        .iter()
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
        project.encode()?;
        if project.assets_sha256.is_empty() {
            bundle::bind(&mut project, &assets)?;
        }
        bundle::verify(&project, &assets)?;
        let audio = bundle::check_music(&project, &assets)?
            .map(|(name, bytes)| audio_url(name, bytes))
            .transpose()?
            .unwrap_or_default();
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
        self.audio.set(audio);
        self.clock.set(0.);
        self.selection.set(Default::default());
        self.view_row.set(0);
        self.cursor.set(0);
        self.root.set(Some(root.unwrap_or_else(unique_root)));
        self.saved.set(saved);
        self.blocked.set(false);
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
                Ok(()) => self.saved.set(Some(project.revision)),
                Err(error) => {
                    self.blocked.set(true);
                    self.error.set(Some(format!("Autosave failed: {error:#}. Save a project ZIP to preserve this working copy.")));
                }
            }
        }
    }
    async fn restore(self) {
        let result = async {
            #[cfg(target_arch = "wasm32")]
            let recovered = crate::persistence::restore_stepmania().await?;
            #[cfg(not(target_arch = "wasm32"))]
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
                #[cfg(target_arch = "wasm32")]
                let assets = crate::storage::list(&root.join("source"))?
                    .into_iter()
                    .map(|(p, b)| {
                        Ok((
                            p.strip_prefix(root.join("source"))?
                                .to_string_lossy()
                                .into_owned(),
                            b.as_ref().clone(),
                        ))
                    })
                    .collect::<Result<Files>>()?;
                #[cfg(not(target_arch = "wasm32"))]
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
    async fn prepare(mut self, format: Option<Format>) {
        let result = async {
            let project = self.project.read().clone().context("No project")?;
            let mut files = if let Some(format) = format {
                bundle::export(&project, &self.assets.read(), format)?
            } else {
                bundle::backup(&project, &self.assets.read())?
            };
            if format.is_none()
                && let Some(parent) = self.parent.read().clone()
            {
                #[cfg(not(target_arch = "wasm32"))]
                let mut combined = {
                    let temporary = tempfile::tempdir()?;
                    let path = temporary.path().join("backup.zip");
                    crate::project_files::backup(&crate::editor::Store::new(&parent), &path)?;
                    crate::archive_codec::decode(&std::fs::read(path)?)?
                        .into_iter()
                        .collect::<Files>()
                };
                #[cfg(target_arch = "wasm32")]
                let mut combined = crate::storage::list(&parent)?
                    .into_iter()
                    .map(|(path, bytes)| {
                        Ok((
                            path.strip_prefix(&parent)?.to_string_lossy().into_owned(),
                            bytes.as_ref().clone(),
                        ))
                    })
                    .collect::<Result<Files>>()?;
                for (name, bytes) in files {
                    combined.insert(format!("dance/{name}"), bytes);
                }
                files = combined;
            }
            let bytes = crate::archive_codec::encode(
                files.iter().map(|(n, b)| (n.as_str(), b.as_slice())),
            )?;
            #[cfg(target_arch = "wasm32")]
            {
                let path = unique_root().join(if format.is_some() {
                    "YACE-song.zip"
                } else {
                    "YACE-project.zip"
                });
                crate::storage::write(&path, &bytes)?;
                self.status.set(format!(
                    "Download ready: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                if format.is_none() {
                    let mut studio = consume_context::<super::state::Studio>();
                    studio.export_ready.set(Some((
                        path,
                        "YACE-project.zip".into(),
                        super::state::DownloadKind::Project,
                    )));
                } else {
                    self.download_ready(Some(path));
                }
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
                    std::fs::write(file.path(), bytes)?;
                    self.status.set(format!("Saved {}", file.path().display()));
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.report(result);
    }
    #[cfg(target_arch = "wasm32")]
    fn download_ready(self, path: Option<PathBuf>) {
        let mut s = self;
        s.prepared.set(path);
    }
    async fn generate(mut self, request: GenerationRequest) {
        let result=async{ensure!(!(self.busy)(),"Generation is running");let project=self.project.read().clone().context("No project")?;let name={let assets=self.assets.read();bundle::check_music(&project,&assets)?.context("Attach a recording first")?.0.to_owned()};let root=(self.root)().context("No project storage")?;
            self.save().await;ensure!(!(self.blocked)(),"Save the current project before generation");let audio=root.join("source").join(name);let revision=project.revision;let source_hash=project.source_sha256.clone();self.busy.set(true);self.cancel.set(false);self.status.set("Analyzing attacks and arranging draft charts…".into());
            #[cfg(not(target_arch="wasm32"))]
            let result={let cancelled=Arc::new(AtomicBool::new(false));let flag=cancelled.clone();let mut task=tokio::task::spawn_blocking(move ||{let samples=crate::models::decode(&audio,11025,1)?;let mut project=project;project.generate(revision,"human".into(),&request,&samples,11025,||flag.load(Ordering::Relaxed))?;Ok::<_,anyhow::Error>(project)});loop{tokio::select!{value=&mut task=>break value.context("Generation worker stopped")?,_=tokio::time::sleep(std::time::Duration::from_millis(40))=>{if (self.cancel)(){cancelled.store(true,Ordering::Relaxed);}}}}};
            #[cfg(target_arch="wasm32")]
            let result={let task=crate::worker::run::<Project>(crate::worker::Task::StepMania{audio,project:Box::new(project),request},std::slice::from_ref(&root));let cancellation=async{loop{gloo_timers::future::TimeoutFuture::new(40).await;if (self.cancel)(){break;}}};futures_util::pin_mut!(task,cancellation);match futures_util::future::select(task,cancellation).await{futures_util::future::Either::Left((result,_))=>result,futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Generation cancelled"))}};
            self.busy.set(false);ensure!(!(self.cancel)(),"Generation cancelled; existing charts kept");let candidate=result?;ensure!((self.root)().as_ref()==Some(&root)&&self.project.read().as_ref().is_some_and(|p|p.revision==revision&&p.source_sha256==source_hash),"Chart changed during generation; existing edits kept");self.selected.set(candidate.content.document.charts.last().map_or(String::new(),|c|c.id.clone()));self.project.set(Some(candidate));self.status.set("Generated drafts added. Review timing, patterns and estimated ratings.".into());Ok::<_,anyhow::Error>(())
        }.await;
        self.busy.set(false);
        self.report(result);
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn recovery_root() -> PathBuf {
    std::env::temp_dir().join("yace-stepmania-recovery")
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
        s.restore().await;
        if studio.snapshot().is_some() {
            s.open.set(false);
        }
        loop {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(500).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            s.save().await;
        }
    });
    rsx! {if (s.export_open)(){super::sheet::ToolSheet{id:"chart-export",title:"Export chart",description:"Choose the game format for this recording.",close_label:"Close export",class:"parts-sheet",onclose:move |_|s.export_open.set(false),actions:rsx!{button{onclick:move |_|s.export_open.set(false),"Close"}},
        button{onclick:move |_|async move{s.prepare(Some(Format::Ssc)).await;},"Dance simfiles (.ssc)"}
        button{onclick:move |_|async move{s.prepare(Some(Format::Sm)).await;},"Legacy dance simfiles (.sm)"}
        if band_visible(studio)&&studio.snapshot().is_some(){button{onclick:move |_|{s.export_open.set(false);studio.export_dialog();},"Band charts"}}
        {prepared_download(s,s.prepared)}
    }}}
}
#[component]
pub fn Workbench() -> Element {
    let mut s = use_context::<DanceStudio>();
    let download = s.prepared;
    let mut start = s.view_row;
    let mut snap = s.snap_rows;
    let mut row = use_signal(|| "0".to_owned());
    let mut lane = use_signal(|| 0u8);
    let mut kind = s.note_kind;
    let mut duration = use_signal(|| "1".to_owned());
    let mut selected_note = use_signal(String::new);
    let mut seed = use_signal(|| "1".to_owned());
    let mut density = use_signal(|| "1".to_owned());
    let mut difficulties = use_signal(|| Difficulty::GENERATED.to_vec());
    let mut review = use_signal(String::new);
    let mut cmod = use_signal(|| false);
    let footwork = use_memo(move || {
        let p = s.project.read();
        let p = p.as_ref()?;
        let id = (s.selected)();
        let c = p.content.document.charts.iter().find(|c| c.id == id)?;
        if c.profile.unwrap_or(p.content.profile) != Profile::Pad {
            return None;
        }
        Some(sm::project::footwork(
            c,
            p.content.document.effective_timing(&id).ok()?,
            &p.content.foot_overrides,
        ))
    });
    if !(s.open)() {
        return rsx! {};
    }
    let project = s.project.read();
    let chart = project.as_ref().and_then(|p| {
        p.content
            .document
            .charts
            .iter()
            .find(|c| c.id == (s.selected)())
    });
    let selected = chart
        .and_then(|c| c.notes.iter().find(|n| n.id == selected_note()))
        .cloned();
    let timing = project
        .as_ref()
        .and_then(|p| p.content.document.effective_timing(&(s.selected)()).ok())
        .cloned()
        .unwrap_or_default();
    let id = (s.selected)();
    let profile = project.as_ref().map_or(Profile::Pad, |p| p.content.profile);
    let beat = match timing.position((s.clock)()) {
        AudioPosition::Beat(b)
        | AudioPosition::Delay { beat: b }
        | AudioPosition::Stop { beat: b } => b,
        AudioPosition::Warp { to, .. } => to,
    };
    let warnings = footwork.read();
    rsx! {section{id:"dance-workbench",class:"chart-desk",aria_label:"Note editor",
        style{{include_str!("../../assets/dance.css")}}
        div{class:"dance-workbench",
            if let Some(error)=(s.error)(){p{class:"step-error",role:"alert","{error}"}button{onclick:move |_|s.error.set(None),"Dismiss"}}
            p{role:"status","{s.status}"}
            {prepared_download(s, download)}
            if let Some(project)=project.as_ref(){
                div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),
                    label{"Profile " select {value:profile.name(),disabled:(s.busy)(),onchange:move |e|s.apply(Action::SetProfile{profile:if e.value()=="keyboard"{Profile::Keyboard}else{Profile::Pad}}),option{value:"pad",selected:profile==Profile::Pad,"Dance pad"}option{value:"keyboard",selected:profile==Profile::Keyboard,"4-key keyboard"}}}
                    super::track_picker::TrackPicker{studio:consume_context::<super::state::Studio>()}
                    button{title:"Add a chart for {profile.name()}",onclick:move |_|{s.apply(Action::AddChart{difficulty:Difficulty::Edit,meter:1,description:"Manual chart".into()});if let Some(c)=s.project.read().as_ref().and_then(|p|p.content.document.charts.last()){s.selected.set(c.id.clone());}},"Add chart"}
                    button{disabled:project.undo.is_empty()||(s.busy)(),onclick:move |_|s.apply(Action::Undo),"Undo"}
                    button{disabled:project.redo.is_empty()||(s.busy)(),onclick:move |_|s.apply(Action::Redo),"Redo"}
                    span{if (s.saved)()==Some(project.revision){"Saved locally"}else{"Saving locally…"}}
                }
                {rsx!{SongProperties{key:"{project.source_sha256}",s}}}
                if !(s.audio)().is_empty(){audio{onkeydown:move|e|e.stop_propagation(),id:"dance-audio",controls:true,src:"{s.audio}",ontimeupdate:move |_|async move{
                    #[cfg(target_arch="wasm32")]{use wasm_bindgen::JsCast;if let Some(audio)=web_sys::window().and_then(|w|w.document()).and_then(|d|d.get_element_by_id("dance-audio")).and_then(|e|e.dyn_into::<web_sys::HtmlAudioElement>().ok()){s.clock.set(audio.current_time());}}
                    #[cfg(not(target_arch="wasm32"))]{if let Ok(value)=dioxus_document::eval("return document.getElementById('dance-audio')?.currentTime ?? 0;").await&&let Some(seconds)=value.as_f64(){s.clock.set(seconds);}}
                }}}
                details{onkeydown:move|e|e.stop_propagation(),open:(s.generation_open)(),summary{onclick:move |_|s.generation_open.set(!(s.generation_open)()),"Generate chart drafts"}p{"Choose difficulties, review the first beat, then generate. New drafts are added alongside existing charts."}
                    for d in Difficulty::GENERATED{label{input{r#type:"checkbox",checked:difficulties.read().contains(&d),onchange:move |e|{if e.checked(){difficulties.write().push(d);}else{difficulties.write().retain(|v|*v!=d);}}}"{d.name()} "}}
                    label{"Seed " input {value:"{seed}",oninput:move |e|seed.set(e.value())}}
                    label{"Density (0.25–2) " input {r#type:"number",min:"0.25",max:"2",step:"0.05",value:"{density}",oninput:move |e|density.set(e.value())}}
                    button{disabled:(s.busy)(),onclick:move |_|{let request=(||Ok::<_,anyhow::Error>(GenerationRequest{profile,difficulties:difficulties(),seed:seed().parse()?,density:density().parse()?}))();match request{Ok(request)=>{spawn(async move{s.generate(request).await;});},Err(error)=>s.report(Err(error))}},"Generate drafts"}
                    if (s.busy)(){button{onclick:move |_|s.cancel.set(true),"Cancel generation"}}
                }
                if let Some(chart)=chart{
                    if let Some(reason)=&chart.read_only_reason{p{"Preserved read-only: {reason}"}}
                    else{
                        {rsx!{ChartProperties{key:"{project.source_sha256}-{id}",s,chart:chart.clone()}}}
                        div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),
                            button{onclick:move |_|start.set((start()-192).max(0)),"Previous 4 beats"}
                            button{onclick:move |_|start.set(start()+192),"Next 4 beats"}
                            button{onclick:move |_|start.set(((beat*48.)as i64/192*192).max(0)),"Follow playhead"}
                            label{"Grid " select {value:"{snap}",onchange:move |e|if let Ok(value)=e.value().parse(){snap.set(value)},for(step,label)in [(48,"1/4"),(24,"1/8"),(16,"1/12"),(12,"1/16"),(8,"1/24"),(6,"1/32"),(4,"1/48"),(3,"1/64"),(2,"1/96"),(1,"1/192")]{option{value:"{step}",selected:step==snap(),"{label}"}}}}
                        }
                        div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),p{"Shift-click notes to select a passage."}button{onclick:move |_|s.copy_selection(),"Copy"}button{disabled:s.clipboard.read().is_empty(),onclick:move |_|s.paste(),"Paste at cursor"}button{onclick:move |_|s.edit_selection(sm::project::SelectionEdit::Delete),"Delete selection"}button{onclick:move |_|s.edit_selection(sm::project::SelectionEdit::Mirror),"Mirror"}button{onclick:move |_|s.edit_selection(sm::project::SelectionEdit::Quantize{rows:snap()}),"Quantize"}button{onclick:move |_|s.edit_selection(sm::project::SelectionEdit::Move{rows:-snap()}),"Earlier"}button{onclick:move |_|s.edit_selection(sm::project::SelectionEdit::Move{rows:snap()}),"Later"}span{"Cursor: {(s.cursor)() as f64/48.:.3}"}}
                        div{class:"step-editor-columns",
                            div{class:"step-grid",aria_label:"Stable beat grid",
                                div{class:"step-grid-row",span{"Beat"}for arrow in ["←","↓","↑","→"]{span{"{arrow}"}}}
                                for r in (start()..start()+192).step_by(snap()as usize){
                                    div{class:"step-grid-row",span{"{r as f64/48.:.3}"}for col in 0u8..4{
                                        button{class:if chart.notes.iter().any(|n|n.row==r&&n.lane==col){"step-cell occupied"}else{"step-cell"},"data-selected":chart.notes.iter().any(|n|n.row==r&&n.lane==col&&s.selection.read().contains(&n.id)).to_string(),aria_label:"Beat {r as f64/48.} lane {col}",onclick:{let id=id.clone();let existing=chart.notes.iter().find(|n|n.row==r&&n.lane==col).cloned();move |event:MouseEvent|{s.cursor.set(r);if let Some(n)=&existing{if !event.modifiers().shift(){s.selection.set(Default::default());}s.selection.write().insert(n.id.clone());selected_note.set(n.id.clone());row.set((n.row as f64/48.).to_string());lane.set(n.lane);kind.set(n.kind);duration.set((n.end_row.map_or(48,|end|end-n.row)as f64/48.).to_string());}else{let end=if kind().sustained(){Some(r+(duration().parse::<f64>().unwrap_or(1.)*48.).round()as i64)}else{None};s.apply(Action::AddNote{chart_id:id.clone(),row:r,lane:col,kind:kind(),end_row:end});}}},
                                            if let Some(n)=chart.notes.iter().find(|n|n.row==r&&n.lane==col){"{note_symbol(n.kind,col)}"}else if chart.notes.iter().any(|n|n.lane==col&&n.row<r&&n.end_row.is_some_and(|end|end>=r)){"│"}else{"·"}
                                        }
                                    }}
                                }
                            }
                            div{class:"step-preview",h3{"Scrolling preview"}p{"Audio {s.clock:.2}s · beat {beat:.3}"}
                                label{input{r#type:"checkbox",checked:cmod(),onchange:move |e|cmod.set(e.checked())}"Constant scroll speed"}
                                Preview{chart:chart.clone(),timing:timing.clone(),beat,seconds:(s.clock)(),cmod:cmod()}
                                p{"The editing grid stays visible through stops, warps and reverse scroll."}
                            }
                        }
                        div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),aria_label:"Note properties",
                            label{"Beat " input {r#type:"number",min:"0",step:"0.020833333",value:"{row}",oninput:move |e|row.set(e.value())}}
                            label{"Lane " select {value:"{lane}",onchange:move |e|if let Ok(v)=e.value().parse(){lane.set(v)},for(i,arrow)in ["←","↓","↑","→"].iter().enumerate(){option{value:"{i}",selected:i==lane()as usize,"{arrow}"}}}}
                            label{"Type " select {value:"{kind().symbol()}",onchange:move |e|if let Some(v)=NoteKind::ALL.into_iter().find(|k|k.symbol().to_string()==e.value()){kind.set(v)},for k in NoteKind::ALL{option{value:"{k.symbol()}",selected:k==kind(),"{k:?}"}}}}
                            if kind().sustained(){label{"Length (beats) " input {value:"{duration}",oninput:move |e|duration.set(e.value())}}}
                            button{onclick:{let id=id.clone();move |_|{let result=(||{let row=sm::row_from_beat(row().parse::<f64>()?)?;let end_row=if kind().sustained(){Some(row+sm::row_from_beat(duration().parse::<f64>()?)?)}else{None};Ok::<_,anyhow::Error>(Action::AddNote{chart_id:id.clone(),row,lane:lane(),kind:kind(),end_row})})();match result{Ok(action)=>s.apply(action),Err(error)=>s.report(Err(error))}}},"Add note"}
                            if let Some(note)=selected.clone(){
                                button{onclick:{let id=id.clone();let note=note.clone();move |_|{let result=(||{let mut note=note.clone();note.row=sm::row_from_beat(row().parse::<f64>()?)?;note.lane=lane();note.kind=kind();note.end_row=if kind().sustained(){Some(note.row+sm::row_from_beat(duration().parse::<f64>()?)?)}else{None};Ok::<_,anyhow::Error>(Action::UpdateNote{chart_id:id.clone(),note})})();match result{Ok(action)=>s.apply(action),Err(error)=>s.report(Err(error))}}},"Update selected"}
                                button{onclick:{let id=id.clone();let note_id=note.id.clone();move |_|{s.apply(Action::DeleteNote{chart_id:id.clone(),note_id:note_id.clone()});selected_note.set(String::new());}},"Delete selected"}
                                if profile==Profile::Pad{label{"Foot override " select {value:project.content.foot_overrides.get(&note.id).map_or("auto",|f|if *f==Foot::Left{"left"}else{"right"}),onchange:{let note_id=note.id.clone();move |e|s.apply(Action::SetFoot{note_id:note_id.clone(),foot:match e.value().as_str(){"left"=>Some(Foot::Left),"right"=>Some(Foot::Right),_=>None}})},option{value:"auto","Automatic"}option{value:"left","Left"}option{value:"right","Right"}}}}
                            }
                        }
                        {rsx!{TimingEditor{key:"{project.source_sha256}-{id}",s}}}
                        if let Some(analysis)=warnings.as_ref(){details{summary{"Footwork review · {analysis.warnings.len()} warnings"}p{"Foot assignments are estimates. Overrides can correct them; playtesting is required."}for w in analysis.warnings.iter().take(200){button{onclick:{let row=w.row;move |_|start.set(row/192*192)},"Beat {w.row as f64/48.:.3}: {w.message}"}}}}
                        details{open:(s.review_open)(),summary{onclick:move |_|s.review_open.set(!(s.review_open)()),"Review"}div{class:"step-toolbar",onkeydown:move |e|e.stop_propagation(),input{aria_label:"Review comment",placeholder:"Playtest notes",value:"{review}",oninput:move |e|review.set(e.value())}button{onclick:{let id=id.clone();move |_|s.apply(Action::Review{chart_id:id.clone(),comment:review()})},"Mark chart reviewed"}span{if project.reviewed(&id){"Review matches current chart"}else{"Pending review"}}}}
                    }
                }
            }else{p{"Import a song or start a chart from audio."}}
        }
    }}
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
fn Preview(chart: Chart, timing: Timing, beat: f64, seconds: f64, cmod: bool) -> Element {
    let notes: Vec<_> = chart
        .notes
        .iter()
        .filter_map(|n| {
            let distance = timing.display_at(n.row as f64 / 48., beat, seconds, cmod);
            let y = 80. + distance * 42.;
            ((-20.0..=500.).contains(&y)).then_some((n, y))
        })
        .take(2000)
        .collect();
    rsx! {svg{view_box:"0 0 320 500",role:"img","aria-label":"Dance scrolling arrow preview",style:"width:100%;height:400px;background:#141821",
        for(i,arrow)in ["←","↓","↑","→"].iter().enumerate(){text{x:"{40+i*80}",y:"80",fill:"#8190a5",text_anchor:"middle",font_size:"36","{arrow}"}}
        for(n,y)in notes{if let Some(end)=n.end_row{line{x1:"{40+n.lane as usize*80}",x2:"{40+n.lane as usize*80}",y1:"{y}",y2:"{(80.+timing.display_at(end as f64/48.,beat,seconds,cmod)*42.).clamp(-100.,600.)}",stroke:"#7c9cf4",stroke_width:"12"}}text{x:"{40+n.lane as usize*80}",y:"{y}",fill:if matches!(n.kind,NoteKind::Mine){"#ef6a72"}else if !timing.judgable(n.row as f64/48.)||n.kind==NoteKind::Fake{"#7f8797"}else{"#8bc5ff"},text_anchor:"middle",font_size:"32",{if n.kind==NoteKind::Mine {"✕"}else if n.kind==NoteKind::Lift{"◇"}else{["←","↓","↑","→"][n.lane as usize]}}}}
    }}
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
    rsx! {details{summary{"Advanced timing"}label{"Offset (seconds) " input {r#type:"number",step:"0.001",value:"{timing.offset}",onchange:{let id=id.clone();let timing=timing.clone();move |event:FormEvent|{let result=(||{let mut timing=timing.clone();timing.offset=event.value().parse::<f64>()?;timing.validate()?;Ok::<_,anyhow::Error>(Action::SetTiming{chart_id:local().then(||id.clone()),timing:Some(timing)})})();match result{Ok(action)=>s.apply(action),Err(error)=>s.report(Err(error))}}}}}label{"Scope " select {value:if local(){"chart"}else{"song"},onchange:move |e|{local.set(e.value()=="chart");selected.set(None);},option{value:"song","Song timing"}option{value:"chart","Selected chart timing"}}}
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
fn prepared_download(s: DanceStudio, download: Signal<Option<PathBuf>>) -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        rsx! {if let Some(path)=download(){button{onclick:move |_|{let name=path.file_name().unwrap().to_string_lossy();s.report(crate::files::download_path(&name,&path));},"Download prepared ZIP"}}}
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (s, download);
        rsx! {}
    }
}
#[cfg(all(test, not(feature = "browser"), not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::RefCell, rc::Rc};
    type Slot = Rc<RefCell<Option<DanceStudio>>>;
    fn app() -> Element {
        provide();
        let studio = super::super::state::use_studio();
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
        rsx! {Workbench{}}
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
    }
}

pub fn active() -> bool {
    try_consume_context::<DanceStudio>().is_some_and(|s| (s.open)() && s.project.read().is_some())
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
pub fn save(studio: super::state::Studio) {
    let linked = try_consume_context::<DanceStudio>().is_some_and(|s| {
        s.parent.read().is_some()
            && s.parent.read().as_ref() == studio.store.read().as_ref().map(|s| &s.root)
    });
    if active() || linked {
        let s = consume_context::<DanceStudio>();
        dioxus::core::spawn_forever(async move {
            s.save().await;
            s.prepare(None).await;
        });
    } else {
        studio.save_project();
    }
}
pub fn deactivate() {
    if let Some(mut s) = try_consume_context::<DanceStudio>() {
        s.open.set(false);
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
pub async fn finish_new(
    mut studio: super::state::Studio,
    source: PathBuf,
    title: String,
    artist: String,
    timing: crate::timing::TempoMap,
    info: crate::song_import::SongInfo,
    profile: Profile,
) -> Result<()> {
    #[cfg(target_arch = "wasm32")]
    let audio = crate::pipeline::audio_file(&source)?;
    #[cfg(not(target_arch = "wasm32"))]
    let audio = source;
    let name = audio
        .file_name()
        .context("Recording has no filename")?
        .to_string_lossy()
        .into_owned();
    let mut assets = Files::from([(name.clone(), crate::storage::read(&audio)?)]);
    let mut p = Project::blank(title, artist, name, 120., 0., profile)?;
    p.content.document.timing = timing_from_band(&timing);
    if let Some(path) = info.artwork {
        let bytes = crate::storage::read(path)?;
        let ext = crate::song_metadata::cover_extension(&bytes)?;
        let name = format!("jacket.{ext}");
        p.content
            .document
            .metadata
            .insert("JACKET".into(), name.clone());
        assets.insert(name, bytes);
    }
    p.apply(Request {
        expected_revision: p.revision,
        actor: "human".into(),
        action: Action::AddChart {
            difficulty: Difficulty::Edit,
            meter: 1,
            description: "Manual chart".into(),
        },
    })?;
    let mut s = consume_context::<DanceStudio>();
    let generate = (s.after_new)();
    #[cfg(not(target_arch = "wasm32"))]
    let root = {
        let output = super::dialogs::AsyncFileDialog::new()
            .set_title("Save editing project in a new folder")
            .set_file_name("Review Project")
            .save_file()
            .await
            .context("Save cancelled")?;
        ensure!(!output.path().exists(), "Choose a new project folder");
        Some(output.path().to_owned())
    };
    #[cfg(target_arch = "wasm32")]
    let root = None;
    s.load(p, assets, root, None)?;
    s.parent.set(None);
    s.generation_open.set(generate);
    studio.new_song.set(None);
    studio.generation_open.set(false);
    s.after_new.set(false);
    Ok(())
}
/// Attach dance charts to the current recording without routing them through
/// guitar lanes. The sidecar is included in normal project backups.
pub fn from_recording(mut studio: super::state::Studio, profile: Profile, generate: bool) {
    let result: Result<()> = (|| {
        let store = studio.store.read().clone().context("Open a song first")?;
        let source = studio.source_audio.read().clone().context("No recording")?;
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
        studio.parts_open.set(false);
        studio.dispatch(crate::editor::Action::Playback { playing: false });
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
    let profile = s.project.read().as_ref().and_then(|p| {
        p.content
            .document
            .charts
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.profile)
    });
    s.selected.set(id);
    s.open.set(true);
    s.selection.set(Default::default());
    let change = profile.filter(|profile| {
        s.project
            .read()
            .as_ref()
            .is_some_and(|p| p.content.profile != *profile)
    });
    if let Some(profile) = change {
        s.apply(Action::SetProfile { profile });
    }
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
pub fn handle(event: KeyboardEvent) {
    let Some(mut s) = try_consume_context::<DanceStudio>() else {
        return;
    };
    let mods = event.modifiers();
    if mods.alt() {
        return;
    }
    let key = event.key();
    let mut handled = true;
    if mods.meta() || mods.ctrl() {
        match key {
            Key::Character(k) => match k.to_ascii_lowercase().as_str() {
                "z" => s.apply(if mods.shift() {
                    Action::Redo
                } else {
                    Action::Undo
                }),
                "a" => {
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
                "c" => s.copy_selection(),
                "x" => {
                    s.copy_selection();
                    s.edit_selection(sm::project::SelectionEdit::Delete);
                }
                "v" => s.paste(),
                _ => handled = false,
            },
            _ => handled = false,
        }
    } else {
        match key {
            Key::Delete | Key::Backspace => s.edit_selection(sm::project::SelectionEdit::Delete),
            Key::Escape => s.selection.set(Default::default()),
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
            Key::Character(k) if k == " " => {
                spawn(async move {
                    #[cfg(target_arch = "wasm32")]
                    {
                        use wasm_bindgen::JsCast;
                        if let Some(audio) = web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|d| d.get_element_by_id("dance-audio"))
                            .and_then(|e| e.dyn_into::<web_sys::HtmlAudioElement>().ok())
                        {
                            if audio.paused() {
                                let _ = audio.play();
                            } else {
                                let _ = audio.pause();
                            }
                        }
                    }
                    #[cfg(not(target_arch="wasm32"))]
                    let _=dioxus_document::eval("const a=document.getElementById('dance-audio'); if(a){if(a.paused) await a.play();else a.pause();}").await;
                });
            }
            Key::Character(k) if ["1", "2", "3", "4"].contains(&k.as_str()) => {
                let lane = k.parse::<u8>().unwrap() - 1;
                let row = (s.cursor)();
                let existing = s.project.read().as_ref().and_then(|p| {
                    p.content
                        .document
                        .charts
                        .iter()
                        .find(|c| c.id == (s.selected)())
                        .and_then(|c| c.notes.iter().find(|n| n.row == row && n.lane == lane))
                        .map(|n| n.id.clone())
                });
                if let Some(note_id) = existing {
                    s.apply(Action::DeleteNote {
                        chart_id: (s.selected)(),
                        note_id,
                    });
                } else {
                    let kind = (s.note_kind)();
                    s.apply(Action::AddNote {
                        chart_id: (s.selected)(),
                        row,
                        lane,
                        kind,
                        end_row: kind.sustained().then_some(row + 48),
                    });
                }
            }
            _ => handled = false,
        }
    }
    if handled {
        event.prevent_default();
        event.stop_propagation();
    }
}
