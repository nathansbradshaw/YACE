//! Client-owned chart state. Commands and validation are the shared Rust store.
use super::media::Media;
use crate::{
    editor::{Action, Actor, ActorKind, Project, Request, Store},
    stem_review::{StemId, StemManifest},
    timing::TempoMap,
    waveform::Waveform,
};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::{collections::BTreeMap, path::PathBuf, rc::Rc, sync::Arc};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditTool {
    Select,
    Place,
    Erase,
}
#[derive(Clone, Debug)]
pub struct ReviewWaveforms {
    pub original: Arc<Waveform>,
    pub stems: BTreeMap<StemId, Arc<Waveform>>,
}
#[derive(Clone, Copy, PartialEq)]
pub struct Studio {
    pub library: Signal<Option<crate::chart_library::Library>>,
    pub library_busy: Signal<bool>,
    pub library_selected: Signal<Option<usize>>,
    pub saved_projects: Signal<Option<Vec<crate::chart_library::Entry>>>,
    pub export_ready: Signal<Option<(PathBuf, String)>>,
    pub export_busy: Signal<bool>,
    pub doc: Signal<Option<Arc<Project>>>,
    pub store: Signal<Option<Store>>,
    pub timing: Signal<Option<TempoMap>>,
    pub playhead: Signal<f64>,
    pub audio_ready: Signal<bool>,
    pub audio_stalled: Signal<bool>,
    pub kit_audition_busy: Signal<Option<String>>,
    pub status: Signal<String>,
    pub error: Signal<Option<String>>,
    pub clean_open: Signal<bool>,
    pub generation_open: Signal<bool>,
    pub clean_ranges: Signal<Vec<(f64, f64)>>,
    pub review_open: Signal<bool>,
    pub assistant_open: Signal<bool>,
    pub view_start: Signal<f64>,
    pub span: Signal<f64>,
    pub tool: Signal<EditTool>,
    pub snap: Signal<u32>,
    pub follow: Signal<bool>,
    pub volume: Signal<f32>,
    pub reviewer: Signal<String>,
    pub waveforms: Signal<Option<Arc<ReviewWaveforms>>>,
    pub source_audio: Signal<Option<PathBuf>>,
    pub guitar_source: Signal<crate::mapping::GuitarSource>,
    pub album_art: Signal<Option<String>>,
    pub collaboration_open: Signal<bool>,
    pub parts_open: Signal<bool>,
    pub sections_open: Signal<bool>,
    pub source_sections: Signal<Vec<crate::sections::Marker>>,
    pub timing_open: Signal<bool>,
    pub timing_target: Signal<Option<(bool, u64)>>,
    pub source_sync: Signal<Option<crate::sync_track::SyncTrack>>,
    pub source_star_power: Signal<crate::star_power::Source>,
    pub star_power_open: Signal<bool>,
    pub star_power_target: Signal<Option<crate::star_power::Phrase>>,
    pub generation_target: Signal<Option<String>>,
    pub new_song: Signal<Option<(PathBuf, String)>>,
    pub vocal_note: Signal<Option<usize>>,
    pub group: Signal<crate::note_groups::Selection>,
    pub selection_dragging: Signal<bool>,
    pub editor_focus: Signal<Option<Rc<MountedData>>>,
    pub metadata: Signal<BTreeMap<String, String>>,
    pub chart_hash: Signal<String>,
    pub stem_status: Signal<String>,
    media: Signal<Option<Rc<Media>>>,
    epoch: Signal<u64>,
    pub dirty: Signal<bool>,
    pub saved_revision: Signal<Option<u64>>,
    pub persistence_busy: Signal<bool>,
}
pub fn use_studio() -> Studio {
    Studio {
        library: use_signal(|| None),
        library_busy: use_signal(|| false),
        library_selected: use_signal(|| None),
        saved_projects: use_signal(|| None),
        export_ready: use_signal(|| None),
        export_busy: use_signal(|| false),
        doc: use_signal(|| None),
        store: use_signal(|| None),
        timing: use_signal(|| None),
        playhead: use_signal(|| 0.),
        audio_ready: use_signal(|| false),
        audio_stalled: use_signal(|| false),
        kit_audition_busy: use_signal(|| None),
        status: use_signal(|| "Open a song folder, song ZIP or saved project".into()),
        error: use_signal(|| None),
        clean_open: use_signal(|| false),
        generation_open: use_signal(|| false),
        clean_ranges: use_signal(Vec::new),
        review_open: use_signal(|| false),
        assistant_open: use_signal(|| false),
        view_start: use_signal(|| 0.),
        span: use_signal(|| 12.),
        tool: use_signal(|| EditTool::Select),
        snap: use_signal(|| 4),
        follow: use_signal(|| true),
        volume: use_signal(|| 0.7),
        reviewer: use_signal(|| "You".into()),
        waveforms: use_signal(|| None),
        source_audio: use_signal(|| None),
        guitar_source: use_signal(Default::default),
        album_art: use_signal(|| None),
        collaboration_open: use_signal(|| false),
        parts_open: use_signal(|| false),
        sections_open: use_signal(|| false),
        source_sections: use_signal(Vec::new),
        timing_open: use_signal(|| false),
        timing_target: use_signal(|| None),
        source_sync: use_signal(|| None),
        source_star_power: use_signal(Default::default),
        star_power_open: use_signal(|| false),
        star_power_target: use_signal(|| None),
        generation_target: use_signal(|| None),
        new_song: use_signal(|| None),
        vocal_note: use_signal(|| None),
        group: use_signal(Default::default),
        selection_dragging: use_signal(|| false),
        editor_focus: use_signal(|| None),
        metadata: use_signal(BTreeMap::new),
        chart_hash: use_signal(String::new),
        stem_status: use_signal(String::new),
        media: use_signal(|| None),
        epoch: use_signal(|| 0),
        dirty: use_signal(|| false),
        saved_revision: use_signal(|| None),
        persistence_busy: use_signal(|| false),
    }
}
impl Studio {
    pub fn snapshot(self) -> Option<Arc<Project>> {
        self.doc.read().clone()
    }
    pub fn metadata_value(self, key: &str) -> String {
        self.metadata.read().get(key).cloned().unwrap_or_default()
    }
    pub fn is_vocal_track(self) -> bool {
        self.snapshot()
            .is_some_and(|doc| doc.selected_track == "Vocals")
    }
    pub fn reload(mut self, _transport: bool) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let snapshot = crate::storage::cached_project(&store.root)
            .map(Ok)
            .unwrap_or_else(|| store.load().map(Arc::new));
        match snapshot {
            Ok(doc) => {
                self.timing.set(doc.timing(&store).ok());
                self.chart_hash
                    .set(doc.chart_hash(&store).unwrap_or_default());
                self.doc.set(Some(doc));
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    pub fn dispatch(mut self, action: Action) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let Some(doc) = self.snapshot() else { return };
        let seek = matches!(
            action,
            Action::Seek { .. } | Action::SelectRegion { .. } | Action::SetLoop { .. }
        );
        let selected = matches!(
            action,
            Action::SelectTrack { .. } | Action::SelectNote { .. } | Action::Undo | Action::Redo
        );
        match store.apply(Request {
            expected_revision: doc.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: (self.reviewer)(),
            },
            action,
        }) {
            Ok(_) => {
                self.error.set(None);
                self.dirty.set(true);
                self.reload(false);
                if selected {
                    self.group.set(Default::default());
                    self.vocal_note.set(None);
                }
                if let Some(doc) = self.snapshot()
                    && let Some(media) = self.media.read().as_ref()
                {
                    media.set_rate(doc.transport.playback_rate);
                    media.mix(
                        &doc.content.stem_mix,
                        (self.volume)(),
                        doc.kit_audition.is_some(),
                        super::media::keyboard_stems(&doc),
                    );
                    if selected {
                        self.waveforms.set(Some(Arc::new(
                            media.waveforms(super::media::keyboard_stems(&doc)),
                        )));
                    }
                    if seek {
                        media.seek(doc.transport.position);
                        self.view_start.set(
                            (doc.transport.position - 0.5)
                                .clamp(0., (doc.duration - (self.span)()).max(0.)),
                        );
                    }
                    if doc.transport.playing {
                        if media.paused()
                            && let Err(error) = media.play()
                        {
                            self.error.set(Some(format!("{error:#}")));
                        }
                    } else {
                        media.pause();
                    }
                    self.playhead.set(media.position());
                }
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    pub fn seek(self, seconds: f64) {
        if let Some(doc) = self.snapshot() {
            self.dispatch(Action::Seek {
                seconds: seconds.clamp(0., doc.duration),
            });
        }
    }
    pub fn toggle_playback(self) {
        if let Some(doc) = self.snapshot() {
            if doc.transport.playing {
                self.seek(self.current_audio_position());
            }
            self.dispatch(Action::Playback {
                playing: !doc.transport.playing,
            });
        }
    }
    pub fn current_audio_position(self) -> f64 {
        self.media
            .read()
            .as_ref()
            .map_or((self.playhead)(), |media| media.position())
    }
    pub fn set_playback_rate(self, rate: f64) {
        self.seek(self.current_audio_position());
        self.dispatch(Action::SetPlaybackRate { rate });
    }
    pub fn update_volume(mut self, value: f32) {
        self.volume.set(value.clamp(0., 1.));
        if let (Some(doc), Some(media)) = (self.snapshot(), self.media.read().as_ref()) {
            media.mix(
                &doc.content.stem_mix,
                (self.volume)(),
                doc.kit_audition.is_some(),
                super::media::keyboard_stems(&doc),
            );
        }
    }
    pub fn audition_kit_part(mut self, part: String, start: Option<f64>) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        if self.kit_audition_busy.read().is_some() {
            return;
        }
        let position = start.unwrap_or_else(|| self.current_audio_position());
        self.dispatch(Action::Playback { playing: false });
        self.seek(position);
        if self
            .snapshot()
            .is_some_and(|doc| doc.content.tracks.contains_key("ExpertDrums"))
        {
            self.dispatch(Action::SelectTrack {
                track: "ExpertDrums".into(),
            });
        }
        let Some(doc) = self.snapshot() else { return };
        self.epoch.set((self.epoch)().wrapping_add(1));
        let epoch = (self.epoch)();
        self.kit_audition_busy.set(Some(part.clone()));
        self.error.set(None);
        spawn(async move {
            let result = async {
                let selected: crate::part_analysis::KitAudition = crate::worker::run(
                    crate::worker::Task::SelectKit {
                        root: store.root.clone(),
                        project: Box::new(doc.as_ref().clone()),
                        audio: store.audio_path(&doc)?,
                        part: part.clone(),
                    },
                    std::slice::from_ref(&store.root),
                )
                .await?;
                ensure!(
                    (self.epoch)() == epoch
                        && self
                            .store
                            .read()
                            .as_ref()
                            .is_some_and(|s| s.root == store.root),
                    "Project changed while preparing kit audio"
                );
                crate::part_analysis::remember_selection(&store, &doc, selected)?;
                store.apply(Request {
                    expected_revision: doc.revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: (self.reviewer)(),
                    },
                    action: Action::SetKitAudition { part: Some(part) },
                })?;
                self.reload(false);
                self.dirty.set(true);
                let current = self.snapshot().context("Project closed")?;
                let (media, waves, notice) = Media::load(&store, &current).await?;
                if (self.epoch)() != epoch {
                    return Ok(());
                }
                if let Some(old) = self.media.read().as_ref() {
                    old.pause();
                }
                self.media.set(Some(Rc::new(media)));
                self.waveforms.set(Some(Arc::new(waves)));
                self.audio_ready.set(true);
                self.stem_status.set(notice);
                self.seek(position);
                self.dispatch(Action::Playback { playing: true });
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if (self.epoch)() == epoch {
                self.kit_audition_busy.set(None);
                if let Err(error) = result {
                    self.dispatch(Action::SetKitAudition { part: None });
                    self.error.set(Some(format!(
                        "Kit playback failed; original audio remains available: {error:#}"
                    )));
                }
            }
        });
    }
    pub fn tick(mut self) {
        let Some(doc) = self.snapshot() else { return };
        if !doc.transport.playing {
            return;
        }
        let Some(media) = self.media.read().clone() else {
            return;
        };
        if let Some(error) = media.take_error() {
            self.dispatch(Action::Playback { playing: false });
            self.error.set(Some(error));
            return;
        }
        let mut position = media.position();
        self.audio_stalled
            .set(!media.ended() && media.original.element.ready_state() < 3);
        if let Some(range) = doc.transport.loop_range {
            if position >= range.end {
                media.seek(range.start);
                position = range.start;
            }
        } else if media.ended() {
            self.seek(doc.duration);
            self.dispatch(Action::Playback { playing: false });
            return;
        }
        media.synchronize();
        self.playhead.set(position);
        if (self.follow)()
            && (position < (self.view_start)()
                || position > (self.view_start)() + (self.span)() * 0.8)
        {
            self.view_start
                .set((position - 0.5).clamp(0., (doc.duration - (self.span)()).max(0.)));
        }
    }
    pub fn open_dialog(self) {
        self.import(false);
    }
    pub fn create_dialog(self) {
        self.import(true);
    }
    pub fn import_song(self) {
        self.import(false);
    }
    pub fn open_zip(self) {
        self.open_dialog();
    }
    pub fn new_chart(mut self) {
        spawn(async move {
            let result = async {
                let Some(mut files) =
                    crate::files::pick(false, ".ogg,.opus,.mp3,.flac,.wav,.m4a").await?
                else {
                    return Ok(());
                };
                let (name, bytes) = files.pop().context("Choose a recording")?;
                let path = std::path::Path::new(&name);
                let extension = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                ensure!(
                    ["ogg", "opus", "mp3", "flac", "wav", "m4a"].contains(&extension.as_str()),
                    "Choose an OGG, Opus, MP3, FLAC, WAV or M4A recording"
                );
                let title: String = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect();
                let root = crate::files::unique_root();
                crate::files::mount(&root, vec![(format!("song.{extension}"), bytes)])?;
                self.error.set(None);
                self.new_song.set(Some((root, title)));
                Ok(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
        });
    }
    pub fn cancel_new_chart(mut self, root: &std::path::Path) {
        crate::storage::clear(root);
        self.new_song.set(None);
    }
    pub async fn analyze_new_chart(self, root: PathBuf) -> Result<crate::tempo_analysis::Estimate> {
        let audio = crate::pipeline::audio_file(&root)?;
        crate::worker::run(
            crate::worker::Task::Tempo { audio },
            std::slice::from_ref(&root),
        )
        .await
    }
    pub async fn finish_new_chart(
        mut self,
        root: PathBuf,
        title: String,
        artist: String,
        timing: TempoMap,
    ) -> Result<()> {
        let (ini, chart) = crate::song_metadata::blank_song_with_timing(&title, &artist, &timing)?;
        crate::storage::write(root.join("song.ini"), ini)?;
        crate::storage::write(root.join("notes.chart"), chart)?;
        self.status.set("Preparing new chart…".into());
        self.open_project(root.clone()).await?;
        crate::storage::clear(&root);
        self.new_song.set(None);
        self.library_selected.set(None);
        Ok(())
    }
    pub fn import(mut self, folder: bool) {
        if (self.library_busy)() {
            return;
        }
        self.library_busy.set(true);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let Some(files) =
                    crate::files::pick(folder, if folder { "" } else { ".zip" }).await?
                else {
                    return Ok(());
                };
                self.status.set("Reading local song…".into());
                let files = crate::files::unpack(files)?;
                let staging = crate::files::unique_root();
                let result: Result<()> = async {
                    crate::files::mount(&staging, files)?;
                    let library = crate::chart_library::discover(&staging)?;
                    self.open_library_entry(&library, 0).await?;
                    if let Some(previous) = self.library.replace(Some(library)) {
                        crate::storage::clear(&previous.root);
                    }
                    self.library_selected.set(Some(0));
                    Ok(())
                }
                .await;
                if result.is_err() {
                    crate::storage::clear(&staging);
                }
                result
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
            self.library_busy.set(false);
        });
    }
    pub async fn open_library_chart(mut self, index: usize) {
        if (self.library_busy)() {
            return;
        }
        self.library_busy.set(true);
        let library = (self.library)();
        let result = async {
            let library = library.context("Open a charting folder")?;
            self.open_library_entry(&library, index).await?;
            self.library_selected.set(Some(index));
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.library_busy.set(false);
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    async fn open_library_entry(
        self,
        library: &crate::chart_library::Library,
        index: usize,
    ) -> Result<()> {
        use sha2::{Digest, Sha256};
        let item = library
            .entries
            .get(index)
            .context("Chart is not in this folder")?;
        let source = if item.kind == crate::chart_library::Kind::Project {
            item.path.join("source")
        } else {
            item.path.clone()
        };
        let mut identity = Sha256::new();
        identity.update(
            item.path
                .strip_prefix(&library.root)?
                .to_string_lossy()
                .as_bytes(),
        );
        // Re-selecting a collection finds its editing copies, while replaced
        // source audio/charts receive a separate history.
        let chart = ["notes.chart", "notes.mid"]
            .into_iter()
            .map(|name| source.join(name))
            .find(|path| crate::storage::is_file(path))
            .context("Song has no chart")?;
        for path in [crate::pipeline::audio_file(&source)?, chart] {
            identity.update(crate::models::hash_file(&path)?.as_bytes());
        }
        for path in [source.join("song.ini"), item.path.join("project.json")] {
            if crate::storage::is_file(&path) {
                identity.update(crate::models::hash_file(&path)?.as_bytes());
            }
        }
        let root = PathBuf::from(format!("/projects/library-{:x}", identity.finalize()));
        self.persist_current().await?;
        if let Some((root, revision)) = crate::persistence::load(&root).await? {
            return self.open_project_with_revision(root, Some(revision)).await;
        }
        if item.kind == crate::chart_library::Kind::Project {
            crate::pipeline::copy_folder(&item.path, &root)?;
            self.open_project(root).await
        } else {
            crate::pipeline::copy_folder(&source, &root.join("source"))?;
            crate::stem_review::game_audio::restore_recording(&root.join("source"))?;
            let audio = crate::pipeline::audio_file(&root.join("source"))?;
            let duration =
                super::media::duration(crate::storage::shared(&audio)?.as_slice()).await?;
            Store::new(&root).initialize(&root.join("source"), None, duration)?;
            self.open_project(root).await
        }
    }
    pub fn open_saved_projects(mut self) {
        dioxus::core::spawn_forever(async move {
            match crate::persistence::projects().await {
                Ok(entries) => self.saved_projects.set(Some(entries)),
                Err(error) => self.error.set(Some(format!("{error:#}"))),
            }
        });
    }
    pub async fn open_saved_project(mut self, root: PathBuf) {
        if (self.library_busy)() {
            return;
        }
        self.library_busy.set(true);
        let result = async {
            self.persist_current().await?;
            let (root, revision) = crate::persistence::load(&root)
                .await?
                .context("Saved project is missing")?;
            self.open_project_with_revision(root, Some(revision))
                .await?;
            self.library_selected.set(None);
            self.saved_projects.set(None);
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.library_busy.set(false);
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    async fn persist_current(mut self) -> Result<()> {
        while (self.persistence_busy)() {
            gloo_timers::future::TimeoutFuture::new(20).await;
        }
        let Some(store) = self.store.read().clone() else {
            return Ok(());
        };
        let Some(doc) = self.snapshot() else {
            return Ok(());
        };
        if (self.saved_revision)() == Some(doc.revision) && !(self.dirty)() {
            return Ok(());
        }
        self.persistence_busy.set(true);
        let result =
            crate::persistence::save(&store.root, doc.revision, (self.saved_revision)()).await;
        self.persistence_busy.set(false);
        result?;
        self.saved_revision.set(Some(doc.revision));
        ensure!(
            self.snapshot()
                .is_some_and(|current| current.revision == doc.revision),
            "The chart changed while switching. Your edits are retained; choose the next chart again."
        );
        self.dirty.set(false);
        Ok(())
    }
    pub async fn open_project(self, source: PathBuf) -> Result<()> {
        self.open_project_with_revision(source, None).await
    }
    pub async fn open_project_with_revision(
        mut self,
        source: PathBuf,
        saved_revision: Option<u64>,
    ) -> Result<()> {
        let root = if crate::storage::is_file(source.join("project.json")) {
            source.clone()
        } else {
            let audio = crate::pipeline::audio_file(&source)?;
            let duration =
                super::media::duration(crate::storage::shared(&audio)?.as_slice()).await?;
            let root = crate::files::unique_root();
            crate::pipeline::copy_folder(&source, &root.join("source"))?;
            crate::stem_review::game_audio::restore_recording(&root.join("source"))?;
            Store::new(&root).initialize(&root.join("source"), None, duration)?;
            root
        };
        let store = Store::new(&root);
        if saved_revision.is_none() {
            self.status
                .set("Preparing portable project history…".into());
            let _: Vec<PathBuf> = crate::worker::run(
                crate::worker::Task::Relocate { root: root.clone() },
                std::slice::from_ref(&root),
            )
            .await?;
        }
        let mut doc = store.load()?;
        doc.transport.playing = false;
        let mut kit_notice = None;
        if let Some(selected) = doc.kit_audition.clone() {
            let prepared = async {
                let prepared: crate::part_analysis::KitAudition = crate::worker::run(
                    crate::worker::Task::SelectKit {
                        root: root.clone(),
                        project: Box::new(doc.clone()),
                        audio: store.audio_path(&doc)?,
                        part: selected.part,
                    },
                    std::slice::from_ref(&root),
                )
                .await?;
                crate::part_analysis::remember_selection(&store, &doc, prepared.clone())?;
                Ok::<_, anyhow::Error>(prepared)
            }
            .await;
            match prepared {
                Ok(selected) => doc.kit_audition = Some(selected),
                Err(error) => {
                    doc.kit_audition = None;
                    kit_notice = Some(format!(
                        "Saved kit unavailable; using original audio: {error:#}"
                    ));
                }
            }
        }
        crate::storage::write(root.join("project.json"), serde_json::to_vec(&doc)?)?;
        // Resolve archived/imported stems by content, never by a server filesystem path.
        restore_stem_aliases(&store, &doc)?;
        if review_manifest(&doc, doc.selected_track == "Vocals").is_none() {
            doc.content.stem_mix.mode = crate::stem_review::MixMode::Original;
            crate::storage::write(root.join("project.json"), serde_json::to_vec(&doc)?)?;
        }
        let metadata =
            crate::pipeline::ini_fields(&crate::pipeline::song_ini(&root.join("source"))?);
        self.epoch.set((self.epoch)().wrapping_add(1));
        let epoch = (self.epoch)();
        self.status.set("Preparing browser audio…".into());
        let (media, waves, notice) = Media::load(&store, &doc).await?;
        if epoch != (self.epoch)() {
            return Ok(());
        }
        self.persist_current().await?;
        let previous_root = self.store.read().as_ref().map(|store| store.root.clone());
        self.audio_ready.set(false);
        if let Some(media) = self.media.read().as_ref() {
            media.pause();
        }
        self.media.set(None);
        self.store.set(Some(store.clone()));
        self.saved_revision.set(saved_revision);
        self.metadata.set(metadata);
        self.timing.set(Some(doc.timing(&store)?));
        self.source_sections.set(store.source_sections(&doc)?);
        self.source_sync.set(store.source_sync(&doc).ok());
        self.source_star_power.set(store.source_star_power(&doc)?);
        self.star_power_open.set(false);
        self.star_power_target.set(None);
        self.timing_open.set(false);
        self.source_audio.set(Some(store.audio_path(&doc)?));
        self.guitar_source.set(store.guitar_source(&doc)?);
        self.group.set(Default::default());
        self.vocal_note.set(None);
        self.sections_open.set(false);
        self.playhead.set(doc.transport.position);
        self.view_start
            .set((doc.transport.position - 0.5).clamp(0., (doc.duration - (self.span)()).max(0.)));
        self.doc.set(Some(Arc::new(doc.clone())));
        self.dirty.set(true);
        self.waveforms.set(Some(Arc::new(waves)));
        self.media.set(Some(Rc::new(media)));
        self.audio_ready.set(true);
        self.stem_status.set(kit_notice.unwrap_or(notice));
        self.status.set("Ready · files stay in this browser".into());
        if let Some(previous) = previous_root.filter(|previous| previous != &root) {
            crate::storage::clear(&previous);
        }
        Ok(())
    }
    pub fn export_dialog(mut self) {
        if (self.export_busy)() {return;}
        self.export_busy.set(true);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let store = self.store.read().clone().context("Open a project")?;
                let doc = self.snapshot().context("Open a project")?;
                let output = PathBuf::from(format!("/exports/{}", js_sys::Date::now() as u64));
                self.status
                    .set("Preparing playable song ZIP locally…".into());
                let _: () = crate::worker::run(
                    crate::worker::Task::Export {
                        root: store.root.clone(),
                        project: Box::new(doc.as_ref().clone()),
                        audio: store.audio_path(&doc)?,
                        output: output.clone(),
                        actor: Actor {
                            kind: ActorKind::Human,
                            name: (self.reviewer)(),
                        },
                    },
                    std::slice::from_ref(&store.root),
                )
                .await?;
                let archive = output.with_extension("zip");
                let name = format!("{} reviewed.zip", self.metadata_value("name"));
                if let Some((previous,_)) = self.export_ready.replace(Some((archive,name))) {crate::storage::clear(&previous);}
                Ok::<_, anyhow::Error>(())
            }
            .await;
            match result {
                Ok(()) => self.status.set("Song ZIP ready to download".into()),
                Err(error) => self.error.set(Some(format!("{error:#}"))),
            }
            self.export_busy.set(false);
        });
    }
    pub fn close_export(mut self) {
        if let Some((path,_))=self.export_ready.take() {crate::storage::clear(&path);}
    }
    pub fn save_project(mut self) {
        let result = (|| -> Result<_> {
            let store = self.store.read().clone().context("Open a project")?;
            let bytes = crate::files::archive(&store.root)?;
            crate::files::download(
                &format!("{} project.zip", self.metadata_value("name")),
                &bytes,
            )
        })();
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    pub fn attach_stems_dialog(mut self) {
        let expected_root = self.store.read().as_ref().map(|store| store.root.clone());
        let keyboard = !self.is_vocal_track()
            && self.snapshot().is_some_and(|doc| {
                crate::keys::is_keys(&doc.selected_track) && doc.content.keyboard_stems.is_some()
            });
        spawn(async move {
            let result = async {
                let Some(files) = crate::files::pick(true, "").await? else {
                    return Ok(());
                };
                let store = self.store.read().clone().context("Open a project")?;
                ensure!(
                    Some(&store.root) == expected_root.as_ref(),
                    "The active project changed; choose stems again"
                );
                let doc = self.snapshot().context("Open a project")?;
                let output = store
                    .root
                    .join("assets/stems")
                    .join(crate::storage::unique_id());
                let attachment = async {
                    let Some(manifest) = crate::stem_import::manifest(&files, &doc, &output)?
                    else {
                        return Ok(false);
                    };
                    self.stem_status
                        .set("Validating supplied stems locally…".into());
                    let manifest: StemManifest = crate::worker::run(
                        crate::worker::Task::ValidateStems {
                            audio: store.audio_path(&doc)?,
                            manifest,
                        },
                        std::slice::from_ref(&output),
                    )
                    .await?;
                    crate::models::remember_stems(&manifest)?;
                    let path = output.join("manifest.json");
                    crate::storage::write(&path, serde_json::to_vec_pretty(&manifest)?)?;
                    let mut candidate = doc.as_ref().clone();
                    if keyboard {
                        candidate.content.keyboard_stems = Some(manifest);
                    } else {
                        candidate.content.stems = Some(manifest);
                    }
                    candidate.content.stem_mix = Default::default();
                    candidate.kit_audition = None;
                    candidate.transport.playing = false;
                    let (media, waves, notice) = Media::load(&store, &candidate).await?;
                    ensure!(
                        self.store
                            .read()
                            .as_ref()
                            .is_some_and(|s| s.root == store.root),
                        "The active project changed during stem validation"
                    );
                    store.apply(Request {
                        expected_revision: doc.revision,
                        actor: Actor {
                            kind: ActorKind::Human,
                            name: (self.reviewer)(),
                        },
                        action: if keyboard {
                            Action::AttachKeyboardStems { manifest: path }
                        } else {
                            Action::AttachStems { manifest: path }
                        },
                    })?;
                    if let Some(old) = self.media.read().as_ref() {
                        old.pause();
                    }
                    self.media.set(Some(Rc::new(media)));
                    self.waveforms.set(Some(Arc::new(waves)));
                    self.reload(false);
                    self.dirty.set(true);
                    self.audio_ready.set(true);
                    self.stem_status.set(notice);
                    self.error.set(None);
                    Ok::<_, anyhow::Error>(true)
                }
                .await;
                match attachment {
                    Ok(true) => return Ok(()),
                    Ok(false) => crate::storage::clear(&output),
                    Err(error) => {
                        crate::storage::clear(&output);
                        return Err(error);
                    }
                }
                let mut matched = 0;
                for manifest in doc
                    .content
                    .stems
                    .iter()
                    .chain(doc.content.keyboard_stems.iter())
                {
                    for asset in &manifest.stems {
                        for (name, bytes) in &files {
                            use sha2::{Digest, Sha256};
                            if format!("{:x}", Sha256::digest(bytes)) == asset.sha256 {
                                let filename = PathBuf::from(name)
                                    .file_name()
                                    .context("Invalid stem filename")?
                                    .to_owned();
                                crate::storage::write(
                                    store.root.join("source/stems").join(filename),
                                    bytes,
                                )?;
                                crate::storage::write(&asset.path, bytes)?;
                                matched += 1;
                                break;
                            }
                        }
                    }
                }
                ensure!(
                    matched > 0,
                    "No selected files match this project's bound stems"
                );
                let position = self.current_audio_position();
                self.open_project_with_revision(store.root, (self.saved_revision)())
                    .await?;
                self.seek(position);
                self.stem_status
                    .set(format!("Attached {matched} matching stem files"));
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
        });
    }
    pub fn copy_selection(mut self, cut: bool) {
        let root = self.store.read().as_ref().map(|s| s.root.clone());
        let track = self.snapshot().map(|d| d.selected_track.clone());
        let result = (|| -> Result<_> {
            let doc = self.snapshot().context("Open a project")?;
            let timing = (self.timing)().context("Missing timing")?;
            let text = crate::note_groups::copy(&doc, &timing, &self.selection())?.encode()?;
            let selected = self.selection_snapshot().context("Select notes")?;
            Ok((text, selected))
        })();
        match result {
            Ok((text, selected)) => {
                let clipboard = web_sys::window().unwrap().navigator().clipboard();
                let promise = clipboard.write_text(&text);
                spawn(async move {
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(_) => {
                            if root != self.store.read().as_ref().map(|s| s.root.clone())
                                || track != self.snapshot().map(|d| d.selected_track.clone())
                            {
                                self.status
                                    .set("Notes copied; the active chart changed".into());
                                return;
                            }
                            if cut {
                                self.edit_group(crate::note_groups::Edit::Delete {
                                    selection: selected,
                                });
                            }
                            self.status.set("Notes copied".into());
                        }
                        Err(error) => self.error.set(Some(format!("Clipboard: {error:?}"))),
                    }
                });
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    pub fn paste_selection(mut self) {
        let root = self.store.read().as_ref().map(|s| s.root.clone());
        let track = self.snapshot().map(|d| d.selected_track.clone());
        let tick = super::note_canvas::snap(self, (self.playhead)());
        let promise = web_sys::window()
            .unwrap()
            .navigator()
            .clipboard()
            .read_text();
        spawn(async move {
            let result = async {
                let value = wasm_bindgen_futures::JsFuture::from(promise)
                    .await
                    .map_err(|error| anyhow::anyhow!("Clipboard: {error:?}"))?;
                ensure!(
                    self.store.read().as_ref().map(|s| s.root.clone()) == root
                        && self.snapshot().map(|d| d.selected_track.clone()) == track,
                    "The project changed; paste again"
                );
                let clipboard = crate::note_groups::Clipboard::decode(
                    &value.as_string().context("Clipboard text unavailable")?,
                )?;
                self.edit_group(crate::note_groups::Edit::Paste {
                    track: track.context("Select a track")?,
                    tick: tick.context("Invalid paste point")?,
                    clipboard,
                });
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
        });
    }
}
fn restore_stem_aliases(store: &Store, doc: &Project) -> Result<()> {
    let source = crate::storage::list(&store.root)?;
    if source.is_empty() {
        return Ok(());
    }
    for manifest in doc
        .content
        .stems
        .iter()
        .chain(doc.content.keyboard_stems.iter())
    {
        for asset in &manifest.stems {
            for (path, _) in &source {
                if crate::models::hash_file(path)?.eq_ignore_ascii_case(&asset.sha256) {
                    crate::storage::copy(path, &asset.path)?;
                    break;
                }
            }
        }
    }
    Ok(())
}
pub fn clock(seconds: f64) -> String {
    let tenths = (seconds.max(0.) * 10.).round() as u64;
    format!(
        "{}:{:02}.{}",
        tenths / 600,
        (tenths % 600) / 10,
        tenths % 10
    )
}
pub fn instrument(track: &str) -> &'static str {
    if track == "Vocals" {
        "Vocals"
    } else if crate::keys::is_pro(track) {
        "Pro Piano"
    } else if track.ends_with("Keyboard") {
        "Piano"
    } else if track.ends_with("Drums") {
        "Drums"
    } else if track.ends_with("DoubleBass") {
        "Bass"
    } else {
        "Guitar"
    }
}
pub fn review_manifest(doc: &Project, vocals: bool) -> Option<&StemManifest> {
    let manifest = if vocals || doc.selected_track == "Vocals" {
        doc.content.stems.as_ref()
    } else {
        doc.content.audition_stems(&doc.selected_track)
    }?;
    manifest
        .stems
        .iter()
        .all(|asset| crate::storage::is_file(&asset.path))
        .then_some(manifest)
}
