//! Import and job controls; expensive work runs in the same executable's worker.
use super::dialogs as rfd;
use super::state::Studio;
use crate::{
    generation::{self, Request, Source, State, Status},
    pipeline, song_import,
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

fn stage_title(stage: &str) -> &str {
    match stage {
        "queued" => "Ready to start",
        "import" | "decode" => "Preparing recording",
        "models" => "Preparing transcription models",
        "separation" => "Separating instrument recordings",
        "transcription" => "Detecting notes",
        "chart" => "Arranging chart difficulties",
        "vocals" => "Detecting vocal pitch and lyrics",
        "editor" => "Preparing editing project",
        "game_export" => "Preparing game files",
        "complete" => "Draft ready",
        "stopped" => "Generation stopped",
        other => other,
    }
}

fn worker_log_tail(root: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = fs::File::open(root.join("worker.log")) else {
        return String::new();
    };
    let Ok(size) = file.metadata().map(|metadata| metadata.len()) else {
        return String::new();
    };
    let offset = size.saturating_sub(8192);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(8192).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let text = if offset > 0 {
        text.split_once('\n').map_or("", |(_, tail)| tail)
    } else {
        &text
    };
    text.lines()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::Mutation;
    #[test]
    fn activity_preview_is_bounded_and_keeps_the_latest_worker_output() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(worker_log_tail(root.path()), "");
        let log = (0..1000)
            .map(|line| format!("worker activity line {line}\n"))
            .collect::<String>();
        fs::write(root.path().join("worker.log"), log).unwrap();
        let tail = worker_log_tail(root.path());
        assert_eq!(tail.lines().count(), 12);
        assert!(tail.ends_with("worker activity line 999"));
        assert!(!tail.contains("line 987"));
    }
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
            assert!(
                texts.iter().any(|text| text == "50"),
                "Part analysis must expose sensitivity: {texts:?}"
            );
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
    detection: crate::detection_settings::Settings,
    loop_only: bool,
    job_settings: Option<String>,
    previous_draft: Option<PathBuf>,
    target: Option<crate::parts::Target>,
    applied: bool,
    audio: Option<PathBuf>,
    title: String,
    artist: String,
    bpm: String,
    use_detected_timing: bool,
    guitar: bool,
    guitar_source: crate::mapping::GuitarSource,
    bass: bool,
    drums: bool,
    vocals: bool,
    vocal_language: crate::lyric_language::Language,
    vocal_analysis_mode: crate::vocal_generation::AnalysisMode,
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
    progress: Option<crate::generation_progress::Progress>,
    started_ms: Option<u64>,
    log_tail: String,
    error: Option<String>,
    notice: String,
    worker: Option<Rc<RefCell<Worker>>>,
    info: Option<song_import::SongInfo>,
    song_details_open: bool,
    cover_preview: Option<String>,
    duration: f64,
}
impl Default for View {
    fn default() -> Self {
        Self {
            detection: Default::default(),
            loop_only: false,
            job_settings: None,
            previous_draft: None,
            target: None,
            applied: false,
            audio: None,
            title: String::new(),
            artist: String::new(),
            bpm: String::new(),
            use_detected_timing: true,
            guitar: true,
            guitar_source: Default::default(),
            bass: true,
            drums: true,
            vocals: false,
            vocal_language: crate::lyric_language::Language::Auto,
            vocal_analysis_mode: Default::default(),
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
            progress: None,
            started_ms: None,
            log_tail: String::new(),
            error: None,
            notice: String::new(),
            worker: None,
            info: None,
            song_details_open: false,
            cover_preview: None,
            duration: 0.0,
        }
    }
}
impl View {
    fn signature(&self) -> String {
        serde_json::to_string(&(
            self.detection,
            self.loop_only,
            self.guitar_source,
            self.keyboard_sound,
            self.piano_quality,
            self.vocal_language,
            self.vocal_analysis_mode,
            self.full_mix,
            self.standard,
        ))
        .unwrap_or_default()
    }
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
            guitar_source: crate::reanalysis::source_at(
                &project.content,
                track,
                (studio.playhead)(),
                store.guitar_source(&project)?,
            ),
            bass: instrument == "bass",
            drums: instrument == "drums",
            piano: instrument == "piano",
            pro_piano: instrument == "pro_piano",
            vocals: instrument == "vocals",
            parent: Some(crate::song_layout::project_data(&store.root).join("jobs/parts")),
            name: format!(
                "{}-{instrument}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ),
            target: Some(target),
            lyrics: false,
            duration: project.duration,
            ..Self::default()
        })
    }
    fn request(&self) -> Result<Request> {
        let request = Request {
            detection: self.detection,
            source: Source::Audio {
                path: self.audio.clone().context("Choose an audio file")?,
            },
            bpm: self.bpm.parse().context("Enter the song tempo in BPM")?,
            tempo_map: if self.target.is_none() && self.use_detected_timing {
                self.info
                    .as_ref()
                    .filter(|info| !info.tempo.segments.is_empty())
                    .map(|info| info.tempo.tempo_map())
                    .transpose()?
            } else {
                None
            },
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
            vocal_analysis_mode: self.vocal_analysis_mode,
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
    if data.peek().busy {
        return;
    }
    data.with_mut(|view| {
        view.busy = true;
        view.started_ms = Some(crate::generation_progress::now_ms());
        view.notice = "Reading audio information and estimating tempo…".into();
    });
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
        match tokio::task::spawn_blocking(move || -> Result<_> { song_import::local(&input) }).await
        {
            Ok(Ok(recording)) => data.with_mut(|v| {
                let meta = recording.metadata;
                let tempo = &recording.info.tempo;
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
                v.song_details_open = true;
                v.bpm = tempo.bpm.unwrap_or(120.0).to_string();
                v.use_detected_timing = !tempo.segments.is_empty();
                v.cover_preview = recording.info.artwork.as_deref().and_then(cover_data);
                v.info = Some(recording.info);
                v.duration = meta.duration;
                v.name = format!("{} chart", v.title.replace(['/', ':'], "-"));
                v.notice = format!(
                    "Recording loaded · {:.1} seconds. Metadata and estimated tempo are editable.",
                    meta.duration
                );
                v.audio = Some(path);
            }),
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

fn lookup_song(mut data: Signal<View>) {
    if data.peek().busy {
        return;
    }
    let view = data.read().clone();
    let Some(audio) = view.audio else {
        return;
    };
    data.with_mut(|v| {
        v.busy = true;
        v.started_ms = Some(crate::generation_progress::now_ms());
        v.notice = "Looking up song information and cover art…".into();
        v.error = None;
    });
    spawn(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut info = view.info.unwrap_or_default();
            let mut metadata = pipeline::Metadata {
                name: view.title,
                artist: view.artist,
                album: info.album.clone(),
                year: info.year.clone(),
                genre: info.genre.clone(),
                duration: view.duration,
            };
            song_import::lookup(
                &mut metadata,
                &mut info,
                &song_import::artwork_cache(&audio)?,
                view.lyrics,
            )?;
            let preview = info.artwork.as_deref().and_then(cover_data);
            Ok((metadata, info, preview))
        })
        .await;
        match result {
            Ok(Ok((metadata, info, preview))) => data.with_mut(|v| { v.artist = metadata.artist; v.info = Some(info); v.cover_preview = preview; v.busy = false; v.notice = "Lookup complete. Review song details and sources; edit the title or artist and retry if needed.".into(); }),
            Ok(Err(error)) => fail(data, format!("{error:#}")),
            Err(error) => fail(data, error),
        }
    });
}

fn create_blank(mut studio: Studio, mut data: Signal<View>) {
    if data.peek().busy {
        return;
    }
    let view = data.read().clone();
    let result = (|| -> Result<_> {
        let audio = view.audio.clone().context("Choose an audio file")?;
        let output = view.output()?;
        let recording = song_import::RecordingInfo {
            metadata: pipeline::Metadata {
                name: view.title.clone(),
                artist: view.artist.clone(),
                duration: view.duration,
                ..Default::default()
            },
            info: view.info.clone().unwrap_or_default(),
        };
        let bpm = if view.use_detected_timing {
            None
        } else {
            Some(view.bpm.parse().context("Enter BPM")?)
        };
        Ok((audio, recording, bpm, output))
    })();
    let (audio, recording, bpm, output) = match result {
        Ok(values) => values,
        Err(error) => {
            fail(data, error);
            return;
        }
    };
    data.with_mut(|v| {
        v.busy = true;
        v.notice = "Creating an editable song project…".into();
        v.error = None;
    });
    spawn(async move {
        match tokio::task::spawn_blocking(move || {
            song_import::create_project(&audio, &recording, bpm, &output)
        })
        .await
        {
            Ok(Ok(project)) => {
                data.write().busy = false;
                studio.open_project(project).await;
                studio.generation_open.set(false);
            }
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
    if data.peek().busy {
        return;
    }
    data.with_mut(|view| {
        view.busy = true;
        view.notice = "Choosing cover artwork…".into();
    });
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Choose cover artwork")
            .add_filter("Cover image", &["jpg", "jpeg", "png"])
            .pick_file()
            .await
        else {
            data.write().busy = false;
            return;
        };
        let path = file.path().to_owned();
        let input = path.clone();
        let preview = tokio::task::spawn_blocking(move || cover_data(&input))
            .await
            .ok()
            .flatten();
        data.with_mut(|view| {
            view.busy = false;
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
        v.status = None;
        v.worker = Some(Rc::new(RefCell::new(Worker {
            child,
            cancelled: false,
        })));
        v.busy = true;
        v.error = None;
        v.progress = None;
        v.started_ms = Some(crate::generation_progress::now_ms());
        v.log_tail.clear();
    });
    Ok(())
}

fn generate(mut studio: Studio, mut data: Signal<View>) {
    if data.peek().busy {
        return;
    }
    studio.analysis_preview.set(None);
    let mut view = data.read().clone();
    if let Some(target) = &view.target {
        let result = (|| -> Result<_> {
            let store = (studio.store)().context("Open the target chart")?;
            let project = (studio.doc)().context("Open the target chart")?;
            let range = if view.loop_only {
                Some(project.transport.loop_range.context("Set a loop first")?)
            } else {
                None
            };
            crate::parts::Target::capture(&store, &project, &target.track)?
                .with_range(&project, range)
        })();
        match result {
            Ok(target) => view.target = Some(target),
            Err(error) => {
                fail(data, error);
                return;
            }
        }
        view.name = format!("part-{}", crate::generation_progress::now_ms());
    }
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
        if v.status
            .as_ref()
            .is_some_and(|status| status.state == State::Complete)
            && v.error.is_none()
            && v.root
                .as_ref()
                .is_some_and(|root| root.join("reanalysis-review.json").is_file())
        {
            v.previous_draft = v.root.clone();
        }
        v.target = view.target.clone();
        v.job_settings = Some(view.signature());
        v.busy = true;
        v.error = None;
        v.status = None;
        v.progress = None;
        v.started_ms = Some(crate::generation_progress::now_ms());
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
    if data.peek().busy {
        return;
    }
    studio.analysis_preview.set(None);
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
                    view.root = None;
                    view.status = None;
                    view.error = None;
                    view.job_settings = None;
                    view.previous_draft = None;
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
    if data.peek().busy {
        return;
    }
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
        let selected = folder.path().to_owned();
        let loaded = tokio::task::spawn_blocking(move || -> Result<_> {
            let read_root = generation::resolve_job(&selected)?;
            Ok((
                generation::read_request(&read_root)?,
                generation::status(&read_root)?,
                crate::parts::target(&read_root)?,
                crate::generation_progress::read(&read_root).ok(),
                worker_log_tail(&read_root),
                read_root,
            ))
        })
        .await;
        match loaded {
            Ok(Ok((request, status, target, progress, log_tail, root))) => data.with_mut(|v| {
                v.detection = request.detection;
                v.loop_only = target.as_ref().is_some_and(|t| t.requested_range.is_some());
                let Source::Audio { path } = request.source;
                v.target = target;
                v.applied = false;
                v.audio = Some(path);
                v.title = request.title;
                v.artist = request.artist;
                v.bpm = request.bpm.to_string();
                v.use_detected_timing = request.tempo_map.is_some();
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
                v.vocal_analysis_mode = request.vocal_analysis_mode;
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
                v.job_settings = Some(v.signature());
                v.started_ms = progress.as_ref().map(|progress| progress.started_ms);
                v.progress = progress;
                v.log_tail = log_tail;
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
fn review_saved_part(studio: Studio, mut data: Signal<View>, root: PathBuf) {
    if data.peek().busy {
        return;
    }
    let loaded = (|| -> Result<View> {
        let request = generation::read_request(&root)?;
        let target = crate::parts::target(&root)?.context("Saved job has no part target")?;
        let mut view = View::for_part(studio, &target.track)?;
        view.detection = request.detection;
        view.loop_only = target.requested_range.is_some();
        view.guitar_source = request.guitar_source;
        view.keyboard_sound = request.keyboard_sound;
        view.piano_quality = request.piano_quality;
        view.vocal_language = request.vocal_language;
        view.vocal_analysis_mode = request.vocal_analysis_mode;
        view.full_mix = request.full_mix_drums;
        view.standard = request.standard_drums;
        view.target = Some(target);
        view.status = Some(generation::status(&root)?);
        view.root = Some(root);
        view.job_settings = Some(view.signature());
        Ok(view)
    })();
    studio.show_analysis_preview(None);
    match loaded {
        Ok(view) => data.set(view),
        Err(error) => fail(data, error),
    }
}

#[component]
pub fn GenerationPanel(mut studio: Studio) -> Element {
    let mut data = use_signal(View::default);
    let mut now_ms = use_signal(crate::generation_progress::now_ms);
    use_effect(move || {
        if (studio.generation_open)() && data.peek().applied {
            let track = data
                .peek()
                .target
                .as_ref()
                .map(|target| target.track.clone());
            if let Some(track) = track
                && let Ok(view) = View::for_part(studio, &track)
            {
                data.set(view);
            }
        }
    });
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
            if data.peek().busy {
                now_ms.set(crate::generation_progress::now_ms());
            }
            let (root, worker) = {
                let v = data.read();
                (v.root.clone(), v.worker.clone())
            };
            let (Some(root), Some(worker)) = (root, worker) else {
                continue;
            };
            let exit = worker.borrow_mut().child.try_wait();
            let read_root = root.clone();
            if let Ok(Ok((status, progress, log_tail))) =
                tokio::task::spawn_blocking(move || -> Result<_> {
                    Ok((
                        generation::status(&read_root)?,
                        crate::generation_progress::read(&read_root).ok(),
                        worker_log_tail(&read_root),
                    ))
                })
                .await
            {
                data.with_mut(|view| {
                    view.status = Some(status);
                    view.progress = progress;
                    view.log_tail = log_tail;
                });
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
                let review_root = root.clone();
                if let Ok(Err(error)) =
                    tokio::task::spawn_blocking(move || crate::parts::save_review(&review_root))
                        .await
                {
                    data.write().error = Some(format!(
                        "Draft retained; comparison needs attention: {error:#}"
                    ));
                }
                studio
                    .status
                    .set("Part draft ready · review it in Generate".into());
                studio.generation_open.set(true);
            } else if complete {
                studio
                    .status
                    .set("Generated chart draft · opening review project".into());
                studio.open_project(generation::project_path(&root)).await;
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
    use_effect(move || {
        let busy = data.read().busy;
        if *studio.generation_busy.peek() != busy {
            studio.generation_busy.set(busy);
        }
    });
    let view = data.read().clone();
    let elapsed = view
        .started_ms
        .map(|start| {
            let end = if view.busy {
                now_ms()
            } else {
                view.progress
                    .as_ref()
                    .map_or(now_ms(), |progress| progress.updated_ms)
            };
            let seconds = end.saturating_sub(start) / 1000;
            format!("{}:{:02}", seconds / 60, seconds % 60)
        })
        .unwrap_or_else(|| "0:00".into());
    let progress = view.progress.as_ref().filter(|progress| {
        view.status
            .as_ref()
            .is_some_and(|status| status.stage == progress.stage)
    });
    let detail = progress
        .map(|progress| progress.detail.clone())
        .or_else(|| view.status.as_ref().map(|status| status.detail.clone()))
        .unwrap_or_else(|| view.notice.clone());
    let notice_detail = if view.busy {
        format!("{elapsed} · {detail}")
    } else {
        view.error.clone().unwrap_or_else(|| detail.clone())
    };
    if !(studio.generation_open)() {
        return if view.busy || view.root.is_some() || view.error.is_some() {
            rsx! {
                style { "{CSS}" }
                super::activity::JobNotice {busy:view.busy,state:view.status.as_ref().map(|status|status.state.clone()),failed:view.error.is_some(),detail:notice_detail,onopen:move |_|studio.generation_open.set(true)}
            }
        } else {
            rsx! {}
        };
    }
    let busy = view.busy;
    let saved = view.root.is_some();
    let part_mode = view.target.is_some();
    let stale = view
        .job_settings
        .as_ref()
        .is_some_and(|settings| settings != &view.signature());
    let sheet_title: &'static str = if part_mode {
        "Reanalyze a part"
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
    if let Some(target) = &view.target {
        use super::reanalysis_panel::{Controls, Options, Panel};
        let track = target.track.clone();
        let recording = view
            .audio
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let instrument = crate::parts::instrument(&track)
            .unwrap_or("guitar")
            .to_owned();
        let options = Options {
            sensitivity: view.detection.sensitivity,
            loop_only: view.loop_only,
            guitar_source: view.guitar_source,
            keyboard_sound: view.keyboard_sound,
            piano_quality: view.piano_quality,
            language: view.vocal_language,
            vocal_mode: view.vocal_analysis_mode,
            standard_drums: view.standard,
            full_mix_drums: view.full_mix,
        };
        let loop_label = studio
            .doc
            .read()
            .as_ref()
            .and_then(|doc| doc.transport.loop_range)
            .map(|range| {
                format!(
                    "{}–{}",
                    super::state::clock(range.start),
                    super::state::clock(range.end)
                )
            });
        let phase = crate::reanalysis::Phase::for_job(
            busy,
            view.status.as_ref().map(|status| &status.state),
            view.error.is_some(),
        )
        .with_problem(view.error.as_deref());
        let activity = view
            .progress
            .as_ref()
            .map(|progress| progress.detail.clone())
            .or_else(|| {
                view.status
                    .as_ref()
                    .map(|status| stage_title(&status.stage).to_owned())
            })
            .unwrap_or_else(|| "Preparing recording…".into());
        let elapsed = view
            .started_ms
            .map(|start| format!("Elapsed {}s", now_ms().saturating_sub(start) / 1000))
            .unwrap_or_default();
        let quiet_for = view
            .progress
            .as_ref()
            .map(|progress| progress.updated_ms)
            .or(view.started_ms)
            .map_or(0, |updated| now_ms().saturating_sub(updated) / 1000);
        let controls = rsx! { Controls {options,instrument,loop_label,onchange:move|next:Options|data.with_mut(|v|{v.detection.sensitivity=next.sensitivity;v.loop_only=next.loop_only;v.guitar_source=next.guitar_source;v.keyboard_sound=next.keyboard_sound;v.piano_quality=next.piano_quality;v.vocal_language=next.language;v.vocal_analysis_mode=next.vocal_mode;v.standard=next.standard_drums;v.full_mix=next.full_mix_drums;}),advanced:rsx!{
            label{class:"ra-check",input{r#type:"checkbox",checked:view.fetch,onchange:move|event|data.write().fetch=event.checked()},"Download missing models"}
            details{summary{"Model licenses"}for asset in crate::models::assets(){p{class:"ra-hint","{asset.id}: {asset.license}"}}}
        }} };
        let review = if let Some(root) = view.root.clone() {
            rsx! {super::reanalysis_review::ReviewDraft{key:"{root:?}",studio,job:root,stale:stale||busy||view.applied,onapply:move |_|apply_part(studio,data),ondiscard:move |_|{studio.show_analysis_preview(None);studio.status.set("Draft discarded · chart unchanged".into());data.with_mut(|v|{v.root=None;v.status=None;v.error=None;v.job_settings=None;v.applied=false;});}}}
        } else {
            rsx! {}
        };
        let diagnostics = rsx! {div{class:"ra-saved-list",
            button{class:"action-secondary",disabled:busy,onclick:move |_|recover(data),"Open saved analysis…"}
            if let Some(root)=view.root.clone(){
                if !busy && view.status.as_ref().is_some_and(|status|status.state!=State::Complete){button{class:"action-secondary",disabled:stale,onclick:move |_|{if !data.peek().busy && let Err(error)=launch_worker(data,root.clone()){fail(data,error);}},"Resume saved analysis"}}
                if complete && view.error.is_some(){button{class:"action-secondary",onclick:move |_|data.write().error=None,"Review saved draft"}}
            }
            if let Some(previous)=view.previous_draft.clone(){button{class:"action-secondary",disabled:busy,onclick:move |_|review_saved_part(studio,data,previous.clone()),"Previous successful draft"}}
            if let Some(root)=&view.root{p{class:"ra-hint","Saved: {root.display()}"}}
            if !view.log_tail.is_empty(){pre{class:"ra-error-detail","{view.log_tail}"}}
        }};
        return rsx! { Panel {studio,track,recording,phase,activity,elapsed,quiet_for,problem:view.error.clone(),controls,review,diagnostics,can_start:!busy&&can_generate,blocked_reason:if validation.is_empty(){None}else{Some(validation)},onstart:move |_|generate(studio,data),oncancel:move |_|{if let Some(worker)=data.peek().worker.clone(){worker.borrow_mut().cancel();}}} };
    }
    rsx! {
        style{"{CSS}"}
        super::sheet::ToolSheet {
        id: "generation-dialog", title: sheet_title, description: sheet_description,
        class: "generation-modal", close_label: "Close generation", onclose: move |_| { studio.analysis_preview.set(None); studio.generation_open.set(false); },
        actions: rsx! {
                    div{class:"generation-actions",
                        if let Some(previous) = view.previous_draft.clone() { button { disabled: busy, onclick: move |_| review_saved_part(studio,data,previous.clone()), "Review previous successful draft" } }
                        button{class:"action-secondary",disabled:busy,"data-browser-pick":"folder",onclick:move|_|recover(data),"Resume saved job…"}
                        if (saved || part_mode) && !busy { button { class: "action-secondary", onclick: move |_| data.set(View::default()), if part_mode { "New song…" } else { "New generation" } } }
                    }
                    div{class:"generation-actions",
                        if view.worker.is_some(){button{class:"action-secondary",onclick:move|_|{if let Some(worker)=data.read().worker.clone(){worker.borrow_mut().cancel();}},"Cancel"}}
                        if saved&&!busy&&!complete{button{class:"action-primary",onclick:move|_|{if data.peek().busy{return;}let root=data.read().root.clone();if let Some(root)=root && let Err(error)=launch_worker(data,root){fail(data,format!("{error:#}"));}},"Resume generation"}}
                        if complete && !part_mode{button{class:"action-primary",onclick:move|_|{spawn(async move{let root=data.read().root.clone();if let Some(root)=root{studio.open_project(generation::project_path(&root)).await;studio.generation_open.set(false);}});},"Open generated chart"}}
                        if !saved && !part_mode { button { class:"action-secondary", disabled:busy || view.audio.is_none() || view.title.trim().is_empty() || view.artist.trim().is_empty() || view.output().is_err(), onclick:move |_|create_blank(studio, data), "Create blank chart" } }
                        if !saved || part_mode {button{class:"action-primary",disabled:busy||!can_generate,title:"{validation}",aria_describedby: if !busy && !can_generate { Some("generation-validation") } else { None },aria_busy:busy,onclick:move|_|generate(studio,data),super::activity::BusyLabel{busy,label:if part_mode{"Analyze"}else{"Generate charts"},active:"Analyzing recording…"}}}
                    }
                 },


                    fieldset{disabled:busy||(saved&&!part_mode),

                        h3 { class: "generation-step", if part_mode { "{part_title}" } else { "1. Choose audio" } }
                        if !part_mode { div{class:"generation-actions",button{class:"action-secondary","data-browser-pick":"file","data-browser-accept":"audio/*,.ogg,.mp3,.wav,.flac,.m4a,.aac,.opus,.webm,.mp4",onclick:move|_|local_file(data),"Choose audio file…"}} }
                        if let Some(path)=&view.audio{p{class:"generation-path",title:"{path.display()}","{path.file_name().unwrap_or_default().to_string_lossy()}"}}
                        if !part_mode { details { open: view.song_details_open, summary { onclick: move |event| { event.prevent_default(); data.with_mut(|view| view.song_details_open = !view.song_details_open); }, "Song details" }
                        div{class:"generation-grid",
                            label{class:"generation-field","Song title",input{class:"field-control",value:"{view.title}",oninput:move|e|data.write().title=e.value()}}
                            label{class:"generation-field","Artist",input{class:"field-control",value:"{view.artist}",oninput:move|e|data.write().artist=e.value()}}
                            label{class:"generation-field","Starting tempo (BPM)",input{class:"field-control",r#type:"number",min:"1",max:"1000",step:"any",placeholder:"Enter BPM",value:"{view.bpm}",oninput:move|e|data.with_mut(|v|{v.bpm=e.value();v.use_detected_timing=false;})}}
                            label{class:"generation-field","New song folder name",input{class:"field-control",value:"{view.name}",oninput:move|e|data.write().name=e.value()}}
                        }
                        p{class:"field-hint","Filled in from the recording. Editing BPM switches to a constant tempo."}
                        div { class:"generation-actions",
                            button { class:"action-secondary", disabled:view.audio.is_none() || view.title.trim().is_empty(), onclick:move |_|lookup_song(data), "Look up song info & cover art" }
                            button { class:"action-secondary", disabled:view.audio.is_none(), onclick:move |_|choose_artwork(data), "Choose cover art…" }
                            if view.info.as_ref().is_some_and(|info| info.artwork.is_some()) { button { class:"action-secondary", onclick:move |_|data.with_mut(|v| { v.cover_preview = None; if let Some(info) = &mut v.info { info.artwork = None; info.artwork_source.clear(); } }), "Clear cover art" } }
                        }
                        p { class:"field-hint", "Lookup shares title, artist, album and duration with music catalogues. Existing details and chosen cover art are kept. Clear a field to find a replacement." }
                        if let Some(info)=&view.info{
                            if !info.tempo.segments.is_empty() {
                                label {class:"form-check",input {r#type:"checkbox",checked:view.use_detected_timing,onchange:move|e|data.with_mut(|v|{v.use_detected_timing=e.checked();if v.use_detected_timing && let Some(bpm)=v.info.as_ref().and_then(|i|i.tempo.bpm){v.bpm=bpm.to_string();}})},"Use detected tempo map"}
                                p {class:"field-hint","{info.tempo.note}"}
                                details {summary {"Detected tempos"}
                                    for segment in &info.tempo.segments {p {class:"field-hint","{segment.start_seconds:.1}s · {segment.bpm:.3} BPM"}}
                                }
                            }
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
                                    select{class:"field-control",value:match view.guitar_source {crate::mapping::GuitarSource::Guitar=>"guitar",crate::mapping::GuitarSource::Other=>"other",crate::mapping::GuitarSource::Piano=>"piano"},onchange:move|e|data.write().guitar_source=match e.value().as_str(){"other"=>crate::mapping::GuitarSource::Other,"piano"=>crate::mapping::GuitarSource::Piano,_=>crate::mapping::GuitarSource::Guitar},
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
                                label{class:"generation-field","Vocal analysis",
                                    select{class:"field-control",value:if view.vocal_analysis_mode==crate::vocal_generation::AnalysisMode::Standard{"standard"}else{"high"},onchange:move|e|data.write().vocal_analysis_mode=if e.value()=="high"{crate::vocal_generation::AnalysisMode::HighAccuracy}else{crate::vocal_generation::AnalysisMode::Standard},option{value:"standard","Standard"}option{value:"high","High accuracy (experimental)"}}
                                    if view.vocal_analysis_mode==crate::vocal_generation::AnalysisMode::HighAccuracy{p{class:"field-hint","Experimental: extra pitches can be wrong. Review amber notes before export."}}
                                }
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

                    if let Some(status)=&view.status{
                        div{class:"generation-progress",role:"status",aria_live:"polite",
                            strong{"{stage_title(&status.stage)}"}
                            span{"{detail}"}
                            if busy {
                                span { "Elapsed {elapsed} · completed work is saved for retry" }
                                if let Some(progress) = progress && let (Some(done), Some(total)) = (progress.completed, progress.total) {
                                    progress { aria_label:"Current generation step", max:"{total}", value:"{done}" }
                                } else { progress { aria_label:"Current generation step" } }
                            }
                        }
                    } else if busy {
                        div { class:"generation-progress", role:"status", span { "{detail}" } span { "Elapsed {elapsed}" } progress { aria_label:"Preparing generation" } }
                    }
                    if !view.log_tail.is_empty() { details { summary { "Recent worker activity" } pre { class:"generation-log", "{view.log_tail}" } } }
                    if let Some(root)=&view.root{
                        if let Some(song)=generation::song_folder(root){p{class:"generation-path","Song folder: {song.display()}"}}
                        else{p{class:"generation-path","Saved job: {root.display()}"}}
                        p{class:"generation-path","Detailed log: {log_path}"}
                    }
                    if let Some(error)=&view.error{div{class:"generation-error",role:"alert","{error}"}}
                    p{class:"generation-footnote","Listen through the draft before exporting."}
                    if !saved && !busy && !can_generate {
                        p { id: "generation-validation", class: "generation-validation", role: "status", "{validation}" }
                    }

    }
    }
}
