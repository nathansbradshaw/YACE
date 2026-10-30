//! Import and job controls; expensive work runs in the same executable's worker.
use super::dialogs as rfd;
use super::state::Studio;
use crate::{
    generation::{self, Request, Source, State, Status},
    pipeline, song_import, tempo_analysis,
};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    fs,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::Duration,
};

const CSS: &str = include_str!("../../assets/generation.css");

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::Mutation;
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    type Slot = Rc<RefCell<Option<Studio>>>;
    fn loaded() -> Element {
        let mut studio = super::super::state::use_studio();
        let (store, slot) = use_context::<(crate::editor::Store, Slot)>();
        use_hook(move || {
            studio
                .doc
                .set(Some(std::sync::Arc::new(store.load().unwrap())));
            studio.store.set(Some(store));
            *slot.borrow_mut() = Some(studio);
        });
        rsx! { GenerationPanel { studio } }
    }
    #[tokio::test]
    async fn requesting_a_part_after_mount_opens_its_recording_and_single_part_form() {
        for (track, label) in [
            ("ExpertSingle", "Guitar · Expert"),
            ("ExpertDoubleBass", "Bass · Expert"),
            ("ExpertDrums", "Drums · Expert"),
            ("ExpertKeyboard", "Piano · Expert"),
            ("ExpertProKeys", "Pro Piano · Expert"),
            ("Vocals", "Vocals"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = fixture::fixture(root.path());
            let slot = Rc::new(RefCell::new(None::<Studio>));
            let mut dom = VirtualDom::new(loaded);
            dom.provide_root_context((store, slot.clone()));
            dom.rebuild_to_vec();
            dom.render_immediate_to_vec();
            let mut studio = slot.borrow().unwrap();
            studio.generation_target.set(Some(track.into()));
            studio.generation_open.set(true);
            let mut texts = Vec::new();
            for _ in 0..6 {
                for mutation in dom.render_immediate_to_vec().edits {
                    match mutation {
                        Mutation::CreateTextNode { value, .. }
                        | Mutation::SetText { value, .. } => texts.push(value),
                        _ => {}
                    }
                }
                tokio::task::yield_now().await;
            }
            assert!(
                texts.iter().any(|text| text == label),
                "Missing selected part {label}: {texts:?}"
            );
            assert!(
                texts.iter().any(|text| text == "song.ogg"),
                "Part generation lost its recording: {texts:?}"
            );
            assert!(studio.generation_target.peek().is_none());
        }
    }
}

struct Worker {
    child: Child,
    cancelled: bool,
}
impl Worker {
    fn cancel(&mut self) {
        self.cancelled = true;
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.cancel();
        }
        let _ = self.child.wait();
    }
}

#[derive(Clone)]
struct View {
    target: Option<crate::parts::Target>,
    applied: bool,
    audio: Option<PathBuf>,
    title: String,
    artist: String,
    bpm: String,
    guitar: bool,
    guitar_source: crate::mapping::GuitarSource,
    bass: bool,
    drums: bool,
    vocals: bool,
    vocal_language: crate::lyric_language::Language,
    piano: bool,
    pro_piano: bool,
    keyboard_sound: crate::keyboard_detection::KeyboardSound,
    piano_quality: crate::keyboard_detection::PianoQuality,
    full_mix: bool,
    standard: bool,
    lyrics: bool,
    fetch: bool,
    models: PathBuf,
    parent: Option<PathBuf>,
    name: String,
    busy: bool,
    root: Option<PathBuf>,
    status: Option<Status>,
    error: Option<String>,
    notice: String,
    worker: Option<Rc<RefCell<Worker>>>,
    info: Option<song_import::SongInfo>,
    song_details_open: bool,
    cover_preview: Option<String>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            target: None,
            applied: false,
            audio: None,
            title: String::new(),
            artist: String::new(),
            bpm: String::new(),
            guitar: true,
            guitar_source: Default::default(),
            bass: true,
            drums: true,
            vocals: false,
            vocal_language: crate::lyric_language::Language::Auto,
            piano: false,
            pro_piano: false,
            keyboard_sound: Default::default(),
            piano_quality: Default::default(),
            full_mix: false,
            standard: false,
            lyrics: true,
            fetch: true,
            models: generation::default_model_dir(),
            parent: rfd::output_parent(),
            name: "New song".into(),
            busy: false,
            root: None,
            status: None,
            error: None,
            notice: String::new(),
            worker: None,
            info: None,
            song_details_open: false,
            cover_preview: None,
        }
    }
}
impl View {
    fn for_part(studio: Studio, track: &str) -> Result<Self> {
        let store = studio.store.read().clone().context("Open a chart first")?;
        let project = studio.snapshot().context("Open a chart first")?;
        let target = crate::parts::Target::capture(&store, &project, track)?;
        let instrument = crate::parts::instrument(track)?;
        let title = studio.metadata_value("name");
        let artist = studio.metadata_value("artist");
        Ok(Self {
            audio: Some(store.audio_path(&project)?),
            title: if title.is_empty() {
                project.title.clone()
            } else {
                title
            },
            artist: if artist.is_empty() {
                "Unknown artist".into()
            } else {
                artist
            },
            bpm: target
                .timing
                .tempos
                .first()
                .context("Chart has no tempo")?
                .bpm
                .to_string(),
            guitar: instrument == "guitar",
            guitar_source: store.guitar_source(&project)?,
            bass: instrument == "bass",
            drums: instrument == "drums",
            piano: instrument == "piano",
            pro_piano: instrument == "pro_piano",
            vocals: instrument == "vocals",
            parent: Some(store.root.join("generation-jobs")),
            name: format!(
                "{}-{instrument}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ),
            target: Some(target),
            lyrics: false,
            ..Self::default()
        })
    }
    fn request(&self) -> Result<Request> {
        let request = Request {
            source: Source::Audio {
                path: self.audio.clone().context("Choose an audio file")?,
            },
            bpm: self.bpm.parse().context("Enter the song tempo in BPM")?,
            title: self.title.clone(),
            artist: self.artist.clone(),
            instruments: [
                (self.guitar, "guitar"),
                (self.bass, "bass"),
                (self.drums, "drums"),
                (self.vocals, "vocals"),
                (self.piano, "piano"),
                (self.pro_piano, "pro_piano"),
            ]
            .into_iter()
            .filter(|(selected, _)| *selected)
            .map(|(_, name)| name.to_owned())
            .collect(),
            model_dir: self.models.clone(),
            guitar_source: self.guitar_source,
            keyboard_sound: self.keyboard_sound,
            piano_quality: self.piano_quality,
            vocal_language: self.vocal_language,
            fetch_models: self.fetch,
            full_mix_drums: self.full_mix,
            standard_drums: self.standard,
            lyrics: self.lyrics,
            song_info: self.info.clone(),
        };
        request.validate()?;
        Ok(request)
    }
    fn output(&self) -> Result<PathBuf> {
        let name = Path::new(self.name.trim());
        ensure!(
            matches!(
                name.components().collect::<Vec<_>>().as_slice(),
                [Component::Normal(_)]
            ),
            "Enter a new folder name without slashes"
        );
        Ok(self
            .parent
            .as_ref()
            .context("Choose where to save your generated song")?
            .join(name))
    }
}

fn fail(mut data: Signal<View>, error: impl std::fmt::Display) {
    data.with_mut(|v| {
        v.busy = false;
        v.error = Some(error.to_string());
    });
}

fn local_file(mut data: Signal<View>) {
    data.write().busy = true;
    spawn(async move {
        let selected = rfd::AsyncFileDialog::new()
            .set_title("Import an audio recording")
            .add_filter(
                "Audio",
                &[
                    "mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "aiff", "aif", "wma",
                ],
            )
            .pick_file()
            .await;
        let Some(selected) = selected else {
            data.write().busy = false;
            return;
        };
        let path = selected.path().to_owned();
        let input = path.clone();
        match tokio::task::spawn_blocking(move || -> Result<_> {
            Ok((
                pipeline::metadata(&input)?,
                tempo_analysis::analyze(&input)?,
            ))
        })
        .await
        {
            Ok(Ok((meta, tempo))) => data.with_mut(|v| {
                v.busy = false;
                v.error = None;
                v.title = if meta.name.is_empty() {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    meta.name
                };
                v.artist = meta.artist;
                v.song_details_open = v.title.trim().is_empty() || v.artist.trim().is_empty();
                v.bpm = tempo.bpm.unwrap_or(120.0).to_string();
                v.info = Some(song_import::SongInfo {
                    album: meta.album,
                    year: meta.year,
                    genre: meta.genre,
                    tempo,
                    ..Default::default()
                });
                v.name = format!("{} chart", v.title.replace(['/', ':'], "-"));
                v.notice = format!(
                    "Recording loaded · {:.1} seconds. Metadata and estimated tempo are editable.",
                    meta.duration
                );
                v.audio = Some(path);
                v.cover_preview = None;
            }),
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

fn cover_data(path: &Path) -> Option<String> {
    use base64::Engine;
    let bytes = fs::read(path).ok()?;
    if bytes.len() > 5_000_000 {
        return None;
    }
    let mime = if path.extension().is_some_and(|s| s == "png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn choose_artwork(mut data: Signal<View>) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Choose cover artwork")
            .add_filter("Cover image", &["jpg", "jpeg", "png"])
            .pick_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let input = path.clone();
        let preview = tokio::task::spawn_blocking(move || cover_data(&input))
            .await
            .ok()
            .flatten();
        data.with_mut(|view| {
            view.cover_preview = preview;
            if let Some(info) = view.info.as_mut() {
                info.artwork = Some(path);
                info.artwork_source = "User-selected image".into();
            }
        });
    });
}

fn launch_worker(mut data: Signal<View>, root: PathBuf) -> Result<()> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("worker.log"))?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("generation")
        .arg("run")
        .arg(&root)
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .context("Could not start the generation worker")?;
    data.with_mut(|v| {
        v.root = Some(root);
        v.worker = Some(Rc::new(RefCell::new(Worker {
            child,
            cancelled: false,
        })));
        v.busy = true;
        v.error = None;
    });
    Ok(())
}

fn generate(mut data: Signal<View>) {
    let view = data.read().clone();
    let request = match view.request() {
        Ok(request) => request,
        Err(error) => {
            fail(data, format!("{error:#}"));
            return;
        }
    };
    let output = match view.output() {
        Ok(output) => output,
        Err(error) => {
            fail(data, format!("{error:#}"));
            return;
        }
    };
    data.with_mut(|v| {
        v.busy = true;
        v.error = None;
        v.status = None;
        v.notice = "Creating generation job…".into();
    });
    spawn(async move {
        match tokio::task::spawn_blocking(move || {
            if let Some(target) = view.target {
                fs::create_dir_all(output.parent().context("Missing job folder")?)?;
                crate::parts::create_job(request, &output, &target)
            } else {
                generation::create(request, &output)
            }
        })
        .await
        {
            Ok(Ok(root)) => {
                if let Err(error) = launch_worker(data, root.clone()) {
                    data.write().root = Some(root);
                    fail(data, format!("{error:#}"));
                }
            }
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

fn apply_part(mut studio: Studio, mut data: Signal<View>) {
    let view = data.read().clone();
    let Some(target) = view.target else {
        return;
    };
    let Some(job) = view.root else {
        return;
    };
    let Some(store) = studio.store.read().clone() else {
        return;
    };
    let Some(project) = studio.snapshot() else {
        return;
    };
    if let Err(error) = target.validate(&store, &project) {
        fail(data, error);
        return;
    }
    let actor = crate::editor::Actor {
        kind: crate::editor::ActorKind::Human,
        name: studio.reviewer.read().clone(),
    };
    let applied_project = store.root.clone();
    data.write().busy = true;
    spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            store.apply(crate::editor::Request {
                expected_revision: project.revision,
                actor,
                action: crate::editor::Action::ApplyGeneratedPart { job },
            })
        })
        .await;
        match result {
            Ok(Ok(_)) => {
                data.with_mut(|view| {
                    view.busy = false;
                    view.applied = true;
                });
                if studio
                    .store
                    .read()
                    .as_ref()
                    .is_some_and(|store| store.root == applied_project)
                {
                    studio.reload(false);
                    studio
                        .status
                        .set("Generated part added · listen and review before export".into());
                    studio.generation_open.set(false);
                } else {
                    data.write().notice =
                        "Part applied to its original chart. Your current chart stayed open."
                            .into();
                }
            }
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

fn recover(mut data: Signal<View>) {
    data.write().busy = true;
    spawn(async move {
        let Some(folder) = rfd::AsyncFileDialog::new()
            .set_title("Open a saved generation job folder")
            .pick_folder()
            .await
        else {
            data.write().busy = false;
            return;
        };
        let root = folder.path().to_owned();
        let read_root = root.clone();
        let loaded = tokio::task::spawn_blocking(
            move || -> Result<(Request, Status, Option<crate::parts::Target>)> {
                Ok((
                    serde_json::from_slice(&fs::read(read_root.join("request.json"))?)?,
                    generation::status(&read_root)?,
                    crate::parts::target(&read_root)?,
                ))
            },
        )
        .await;
        match loaded {
            Ok(Ok((request, status, target))) => data.with_mut(|v| {
                let Source::Audio { path } = request.source;
                v.target = target;
                v.applied = false;
                v.audio = Some(path);
                v.title = request.title;
                v.artist = request.artist;
                v.bpm = request.bpm.to_string();
                v.guitar = request.instruments.iter().any(|s| s == "guitar");
                v.bass = request.instruments.iter().any(|s| s == "bass");
                v.drums = request.instruments.iter().any(|s| s == "drums");
                v.piano = request.instruments.iter().any(|s| s == "piano");
                v.pro_piano = request.instruments.iter().any(|s| s == "pro_piano");
                v.keyboard_sound = request.keyboard_sound;
                v.piano_quality = request.piano_quality;
                v.vocals = request.instruments.iter().any(|s| s == "vocals");
                v.full_mix = request.full_mix_drums;
                v.standard = request.standard_drums;
                v.lyrics = request.lyrics;
                v.guitar_source = request.guitar_source;
                v.vocal_language = request.vocal_language;
                v.fetch = request.fetch_models;
                v.models = request.model_dir;
                v.info = request.song_info;
                v.parent = root.parent().map(Path::to_owned);
                v.name = root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                v.root = Some(root);
                v.error = status.error.clone();
                v.status = Some(status);
                v.busy = false;
                v.notice = "Saved job loaded. Resume uses its original generation settings.".into();
            }),
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

#[component]
pub fn GenerationPanel(mut studio: Studio) -> Element {
    let mut data = use_signal(View::default);
    use_effect(move || {
        let Some(track) = (studio.generation_target)() else {
            return;
        };
        studio.generation_target.set(None);
        if data.peek().busy || data.peek().worker.is_some() {
            studio.error.set(Some(
                "A generation job is already running. Let it finish or cancel it first.".into(),
            ));
            return;
        }
        match View::for_part(studio, &track) {
            Ok(view) => data.set(view),
            Err(error) => studio.error.set(Some(format!("{error:#}"))),
        }
    });
    // This component stays mounted when its dialog closes, so jobs and progress
    // remain active while the user reviews another song.
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let (root, worker) = {
                let v = data.read();
                (v.root.clone(), v.worker.clone())
            };
            let (Some(root), Some(worker)) = (root, worker) else {
                continue;
            };
            let exit = worker.borrow_mut().child.try_wait();
            let read_root = root.clone();
            if let Ok(Ok(status)) =
                tokio::task::spawn_blocking(move || generation::status(&read_root)).await
            {
                data.write().status = Some(status);
            }
            let stopped = match exit {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(error) => {
                    data.write().error = Some(error.to_string());
                    true
                }
            };
            if !stopped {
                continue;
            }
            let cancelled = worker.borrow().cancelled;
            data.with_mut(|v| {
                v.worker = None;
                v.busy = false;
            });
            let complete = data
                .read()
                .status
                .as_ref()
                .is_some_and(|s| s.state == State::Complete);
            if complete && data.read().target.is_some() {
                studio
                    .status
                    .set("Part draft ready · review it in Generate".into());
                studio.generation_open.set(true);
            } else if complete {
                studio
                    .status
                    .set("Generated chart draft · opening review project".into());
                studio.open_project(root.join("project")).await;
                studio.generation_open.set(false);
            } else {
                let stopped_root = root.clone();
                let _ = tokio::task::spawn_blocking(move || -> Result<()> {
                    if generation::status(&stopped_root)?.state != State::Failed {
                        generation::mark_interrupted(&stopped_root, cancelled)?;
                    }
                    Ok(())
                })
                .await;
                if let Ok(status) = generation::status(&root) {
                    data.with_mut(|v| {
                        v.error = status.error.clone();
                        v.status = Some(status);
                    });
                }
                studio.status.set(if cancelled {
                    "Generation cancelled; saved inputs retained".into()
                } else {
                    "Generation stopped; open Generate for details".into()
                });
            }
        }
    });
    use_drop(move || {
        if let Some(worker) = data.peek().worker.clone() {
            worker.borrow_mut().cancel();
        }
    });
    let view = data.read().clone();
    if !(studio.generation_open)() {
        return rsx! {};
    }
    let busy = view.busy;
    let saved = view.root.is_some();
    let part_mode = view.target.is_some();
    let sheet_title: &'static str = if part_mode {
        "Generate a part"
    } else {
        "Generate charts"
    };
    let sheet_description: &'static str = if part_mode {
        "Uses this recording and keeps the other parts."
    } else {
        "Choose audio and parts. Review the draft in the editor."
    };
    let part_title = view
        .target
        .as_ref()
        .map(|target| super::track_picker::title(&target.track))
        .unwrap_or_default();
    let replaces_part = view.target.as_ref().is_some_and(|target| {
        studio.snapshot().is_some_and(|doc| {
            if target.track == "Vocals" {
                doc.content
                    .vocals
                    .as_ref()
                    .is_some_and(|chart| !chart.notes.is_empty())
            } else {
                doc.content
                    .tracks
                    .get(&target.track)
                    .is_some_and(|notes| !notes.is_empty())
            }
        })
    });
    let complete = view
        .status
        .as_ref()
        .is_some_and(|s| s.state == State::Complete);
    let can_generate = view.request().is_ok() && view.output().is_ok();
    let validation = view
        .request()
        .err()
        .or_else(|| view.output().err())
        .map(|e| e.to_string())
        .unwrap_or_default();
    let log_path = view
        .root
        .as_ref()
        .map(|root| root.join("worker.log").display().to_string())
        .unwrap_or_default();
    rsx! {
        style{"{CSS}"}
        super::sheet::ToolSheet {
        id: "generation-dialog", title: sheet_title, description: sheet_description,
        class: "generation-modal", close_label: "Close generation", onclose: move |_| studio.generation_open.set(false),
        actions: rsx! {
                    div{class:"generation-actions",
                        button{class:"action-secondary",disabled:busy,"data-browser-pick":"folder",onclick:move|_|recover(data),"Resume saved job…"}
                        if (saved || part_mode) && !busy { button { class: "action-secondary", onclick: move |_| data.set(View::default()), if part_mode { "New song…" } else { "New generation" } } }
                    }
                    div{class:"generation-actions",
                        if view.worker.is_some(){button{class:"action-secondary",onclick:move|_|{if let Some(worker)=data.read().worker.clone(){worker.borrow_mut().cancel();}},"Cancel"}}
                        if saved&&!busy&&!complete{button{class:"action-primary",onclick:move|_|{let root=data.read().root.clone();if let Some(root)=root && let Err(error)=launch_worker(data,root){fail(data,format!("{error:#}"));}},"Resume generation"}}
                        if complete && part_mode { button { class: "action-primary", disabled: busy || view.applied, onclick: move |_| apply_part(studio, data), if view.applied { "Applied" } else if replaces_part { "Replace {part_title}" } else { "Add {part_title}" } } }
                        if complete && !part_mode{button{class:"action-primary",onclick:move|_|{spawn(async move{let root=data.read().root.clone();if let Some(root)=root{studio.open_project(root.join("project")).await;studio.generation_open.set(false);}});},"Open generated chart"}}
                        if !saved{button{class:"action-primary",disabled:busy||!can_generate,title:"{validation}",aria_describedby: if !busy && !can_generate { Some("generation-validation") } else { None },onclick:move|_|generate(data),if busy{"Working…"}else if part_mode{"Generate part"}else{"Generate charts"}}}
                    }
                 },

                    fieldset{disabled:busy||saved,
                        h3 { class: "generation-step", if part_mode { "{part_title}" } else { "1. Choose audio" } }
                        if !part_mode { div{class:"generation-actions",button{class:"action-secondary","data-browser-pick":"file","data-browser-accept":"audio/*,.ogg,.mp3,.wav,.flac,.m4a,.aac,.opus,.webm,.mp4",onclick:move|_|local_file(data),"Choose audio file…"}} }
                        if let Some(path)=&view.audio{p{class:"generation-path",title:"{path.display()}","{path.file_name().unwrap_or_default().to_string_lossy()}"}}
                        if !part_mode { details { open: view.song_details_open, summary { onclick: move |event| { event.prevent_default(); data.with_mut(|view| view.song_details_open = !view.song_details_open); }, "Song details" }
                        div{class:"generation-grid",
                            label{class:"generation-field","Song title",input{class:"field-control",value:"{view.title}",oninput:move|e|data.write().title=e.value()}}
                            label{class:"generation-field","Artist",input{class:"field-control",value:"{view.artist}",oninput:move|e|data.write().artist=e.value()}}
                            label{class:"generation-field","Tempo (BPM)",input{class:"field-control",r#type:"number",min:"1",max:"1000",step:"any",placeholder:"Enter BPM",value:"{view.bpm}",oninput:move|e|data.write().bpm=e.value()}}
                            label{class:"generation-field","New song folder name",input{class:"field-control",value:"{view.name}",oninput:move|e|data.write().name=e.value()}}
                        }
                        p{class:"field-hint","Filled in from the recording. Check the estimated BPM."}
                        if let Some(info)=&view.info{
                            if let Some(preview)=&view.cover_preview{img{class:"generation-artwork",src:"{preview}",alt:"Suggested artwork for this recording"}}
                            div{class:"generation-grid",
                                label{class:"generation-field","Album",input{class:"field-control",value:"{info.album}",placeholder:"Unavailable",oninput:move|e|{if let Some(info)=data.write().info.as_mut(){info.album=e.value();}}}}
                                label{class:"generation-field","Release year",input{class:"field-control",value:"{info.year}",placeholder:"Unavailable",oninput:move|e|{if let Some(info)=data.write().info.as_mut(){info.year=e.value();}}}}
                                label{class:"generation-field","Genre",input{class:"field-control",value:"{info.genre}",placeholder:"Unavailable",oninput:move|e|{if let Some(info)=data.write().info.as_mut(){info.genre=e.value();}}}}
                            }
                            details{summary{"Imported information and sources"}
                                p{class:"field-hint","{info.tempo.note}"}
                                for note in &info.notes{p{class:"field-hint","{note}"}}
                                if let Some(artwork)=&info.artwork{p{class:"generation-path","Artwork: {artwork.display()}"}p{class:"field-hint","Source: {info.artwork_source}"}}
                                div{class:"generation-actions",button{class:"action-secondary","data-browser-pick":"file",onclick:move|_|choose_artwork(data),"Change artwork…"}}
                                if let Some(lyrics)=&info.lyrics{
                                    label{class:"generation-field","Timed lyrics (editable LRC)",textarea{class:"field-control",rows:"6",value:lyrics["syncedLyrics"].as_str().unwrap_or_default(),oninput:move|e|{if let Some(lyrics)=data.write().info.as_mut().and_then(|info|info.lyrics.as_mut()){lyrics["syncedLyrics"]=serde_json::json!(e.value());}}}}
                                }else{p{class:"field-hint","No matching lyrics found."}}
                            }
                        }
                        }
                        }
                        if !part_mode { h3 { class: "generation-step", "2. Choose parts" } }
                        div{class:"generation-actions",
                            if !part_mode { label{class:"form-check",input{r#type:"checkbox",checked:view.guitar,disabled:view.full_mix,onchange:move|e|data.write().guitar=e.checked()},"Guitar"} }
                            if view.guitar {
                                label{class:"generation-field","Guitar chart follows",
                                    select{class:"field-control",disabled:part_mode,value:match view.guitar_source {crate::mapping::GuitarSource::Guitar=>"guitar",crate::mapping::GuitarSource::Other=>"other",crate::mapping::GuitarSource::Piano=>"piano"},onchange:move|e|data.write().guitar_source=match e.value().as_str(){"other"=>crate::mapping::GuitarSource::Other,"piano"=>crate::mapping::GuitarSource::Piano,_=>crate::mapping::GuitarSource::Guitar},
                                        option{value:"guitar","Guitar"}option{value:"other","Other melody / synth"}option{value:"piano","Piano"}
                                    }
                                }
                                if view.guitar_source != crate::mapping::GuitarSource::Guitar {p{class:"field-hint","Arranges this sound for five-fret Guitar. Check for missing notes."}}
                            }
                            if !part_mode { label{class:"form-check",input{r#type:"checkbox",checked:view.bass,disabled:view.full_mix,onchange:move|e|data.write().bass=e.checked()},"Bass"} }
                            if !part_mode { label{class:"form-check",input{r#type:"checkbox",checked:view.drums,onchange:move|e|data.write().drums=e.checked()},"Drums"} }
                            if !part_mode { label{class:"form-check",title:"Keyboard notes arranged into five lanes. Listen and review before export.",input{r#type:"checkbox",checked:view.piano,disabled:view.full_mix,onchange:move|e|data.write().piano=e.checked()},"Piano (Keys)"} }
                            if !part_mode { label{class:"form-check",title:"Piano notes fitted to the game keyboard. Review octave and release adjustments before export.",input{r#type:"checkbox",checked:view.pro_piano,disabled:view.full_mix,onchange:move|e|data.write().pro_piano=e.checked()},"Pro Piano (Pro Keys)"} }
                            if view.piano || view.pro_piano {
                                label{class:"form-check","Keyboard sound"
                                    select{value:if view.keyboard_sound == crate::keyboard_detection::KeyboardSound::Synth {"synth"} else {"piano"},
                                        onchange:move|e|data.write().keyboard_sound=if e.value()=="synth" {crate::keyboard_detection::KeyboardSound::Synth} else {crate::keyboard_detection::KeyboardSound::Piano},
                                        option{value:"piano","Piano"}
                                        option{value:"synth","Synth (experimental)"}
                                    }
                                }
                                if view.keyboard_sound == crate::keyboard_detection::KeyboardSound::Piano {
                                    label{class:"form-check","Piano separation"
                                        select{value:if view.piano_quality == crate::keyboard_detection::PianoQuality::HighQuality {"high_quality"} else {"standard"},
                                            onchange:move|e|data.write().piano_quality=if e.value()=="high_quality" {crate::keyboard_detection::PianoQuality::HighQuality} else {crate::keyboard_detection::PianoQuality::Standard},
                                            option{value:"standard","Standard"}
                                            option{value:"high_quality","Higher quality (slower)"}
                                        }
                                    }
                                    if view.piano_quality == crate::keyboard_detection::PianoQuality::HighQuality {
                                        p{class:"generation-note","Uses an extra model. Slower, with clearer piano audio."}
                                    }
                                }
                                p{class:"generation-note",if view.keyboard_sound == crate::keyboard_detection::KeyboardSound::Synth {"Experimental. Other instruments may appear in this synth part; review each phrase."} else {"Check missing notes, extra notes and key releases."}}
                            }
                            if !part_mode { label{class:"form-check",title:"Estimate sung pitch and recognize lyrics in the selected language. Review before export.",input{r#type:"checkbox",checked:view.vocals,disabled:view.full_mix,onchange:move|e|data.write().vocals=e.checked()},"Vocals"} }
                            if view.vocals {
                                label{class:"generation-field","Vocal language",
                                    select{class:"field-control",value:match view.vocal_language {crate::lyric_language::Language::Auto=>"auto",crate::lyric_language::Language::English=>"english",crate::lyric_language::Language::Spanish=>"spanish"},onchange:move|e|data.write().vocal_language=match e.value().as_str(){"english"=>crate::lyric_language::Language::English,"spanish"=>crate::lyric_language::Language::Spanish,_=>crate::lyric_language::Language::Auto},
                                        option{value:"auto","Detect automatically"} option{value:"spanish","Spanish"} option{value:"english","English"}
                                    }
                                }
                            }
                        }
                        if !part_mode && !cfg!(feature="browser") { h3 { class: "generation-step", "3. Choose where to save" }
                        div{class:"generation-actions",
                            button{class:"action-secondary",onclick:move|_|{spawn(async move{if let Some(folder)=rfd::AsyncFileDialog::new().set_title("Save generated songs here").pick_folder().await{data.write().parent=Some(folder.path().to_owned());}});},"Choose save location…"}
                            if let Some(parent)=&view.parent{span{class:"generation-path","{parent.display()}"}}
                        }
                        }
                        details{summary{"Generation options"}
                            label{class:"form-check",input{r#type:"checkbox",checked:view.fetch,onchange:move|e|data.write().fetch=e.checked()},"Download missing models"}
                            label{class:"form-check",input{r#type:"checkbox",checked:view.lyrics,onchange:move|e|data.write().lyrics=e.checked()},"Find lyrics online (shares song metadata)"}
                            label{class:"form-check",input{r#type:"checkbox",checked:view.standard,onchange:move|e|data.write().standard=e.checked()},"Standard drums (omit Pro cymbal markers)"}
                            if !part_mode || view.drums { label{class:"form-check",input{r#type:"checkbox",checked:view.full_mix,onchange:move|e|data.with_mut(|v|{v.full_mix=e.checked();if v.full_mix{v.guitar=false;v.bass=false;v.piano=false;v.pro_piano=false;v.vocals=false;v.drums=true;}})},"Drums only from the original mix (skip stem separation)"} }
                            div{class:"generation-actions",button{"data-browser-pick":"folder","data-browser-accept":"",class:"action-secondary",onclick:move|_|{spawn(async move{if let Some(folder)=rfd::AsyncFileDialog::new().set_title("Choose model folder").pick_folder().await{data.write().models=folder.path().to_owned();}});},"Model folder…"}span{class:"generation-path","{view.models.display()}"}}
                        }
                        p{class:"field-hint","Some models are noncommercial. Check licenses before downloading or sharing."}
                        details{summary{"Model licenses"}
                            for asset in crate::models::assets(){p{class:"field-hint",strong{"{asset.id}: "}"{asset.license}"}}
                            p{class:"field-hint","{crate::keyboard_separation::WEIGHT_LICENSE_NOTICE}"}
                        }
                    }
                    if !view.notice.is_empty(){p{class:"field-hint",role:"status","{view.notice}"}}
                    if complete && part_mode && !view.applied {
                        p { class: "field-hint", if replaces_part { "This replaces {part_title}. Undo restores the previous notes." } else { "The draft is ready to add to this chart." } }
                    }
                    if let Some(status)=&view.status{
                        div{class:"generation-progress",role:"status",strong{"{status.stage}"}span{"{status.detail}"}if busy{progress{aria_label:"Generating charts"}}}
                    }
                    if let Some(root)=&view.root{p{class:"generation-path","Saved job: {root.display()}"}p{class:"generation-path","Detailed log: {log_path}"}}
                    if let Some(error)=&view.error{div{class:"generation-error",role:"alert","{error}"}}
                    p{class:"generation-footnote","Listen through the draft before exporting."}
                    if !saved && !busy && !can_generate {
                        p { id: "generation-validation", class: "generation-validation", role: "status", "{validation}" }
                    }

    }
    }
}
