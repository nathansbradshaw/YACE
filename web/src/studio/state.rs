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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DownloadKind {
    Song,
    Project,
    File,
}
#[derive(Clone, Debug)]
pub struct ReviewWaveforms {
    pub original: Arc<Waveform>,
    pub stems: BTreeMap<StemId, Arc<Waveform>>,
}
#[derive(Clone, Copy, PartialEq)]
pub struct Studio {
    pub analysis_preview: Signal<Option<Arc<Project>>>,
    pub follow_analysis_sources: Signal<bool>,
    pub recent_projects: Signal<crate::recent_projects::History>,
    pub library: Signal<Option<crate::chart_library::Library>>,
    pub library_busy: Signal<bool>,
    pub library_selected: Signal<Option<usize>>,
    pub source_choice: Signal<Option<super::source_choice::Prompt>>,
    library_files: Signal<Option<Rc<crate::files::FolderSelection>>>,
    pub saved_projects: Signal<Option<Vec<crate::chart_library::Entry>>>,
    pub export_ready: Signal<Option<(PathBuf, String, DownloadKind)>>,
    pub export_busy: Signal<bool>,
    pub export_open: Signal<bool>,
    pub export_state: Signal<crate::export_state::State>,
    pub song_download_receipt: Signal<Option<(PathBuf, crate::export_state::Receipt)>>,
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
    pub synth_reference: Signal<bool>,
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
    pub generation_after_import: Signal<bool>,
    pub new_song: Signal<Option<(PathBuf, String)>>,
    pub blank_chart: Signal<bool>,
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
        analysis_preview: use_signal(|| None),
        follow_analysis_sources: use_signal(|| true),
        recent_projects: use_signal(|| crate::recent_projects::History::load().unwrap_or_default()),
        library: use_signal(|| None),
        library_busy: use_signal(|| false),
        library_selected: use_signal(|| None),
        source_choice: use_signal(|| None),
        library_files: use_signal(|| None),
        saved_projects: use_signal(|| None),
        export_ready: use_signal(|| None),
        export_state: use_signal(Default::default),
        song_download_receipt: use_signal(|| None),
        export_busy: use_signal(|| false),
        export_open: use_signal(|| false),
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
        synth_reference: use_signal(|| false),
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
        generation_after_import: use_signal(|| false),
        new_song: use_signal(|| None),
        blank_chart: use_signal(|| false),
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
/// Failed or superseded opens release only their inactive working set. Durable
/// IndexedDB snapshots and the currently editable project are never removed.
struct Candidate {
    studio: Studio,
    root: PathBuf,
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if self
            .studio
            .store
            .read()
            .as_ref()
            .is_none_or(|store| store.root != self.root)
        {
            crate::storage::clear(&self.root);
        }
    }
}
impl Studio {
    pub fn remember_recent(mut self, path: PathBuf) {
        let mut history = crate::recent_projects::History::load().unwrap_or_default();
        history.remember(
            path.clone(),
            crate::recent_projects::title(&self.metadata.read(), &path),
        );
        if let Err(error) = history.save() {
            self.error.set(Some(format!(
                "Project saved, but recent history could not be saved: {error:#}"
            )));
        }
        self.recent_projects.set(history);
    }

    pub fn forget_recent(mut self, path: Option<PathBuf>) {
        let mut history = (self.recent_projects)();
        if let Some(path) = path {
            history.forget(&path);
        } else {
            history.entries.clear();
        }
        match history.save() {
            Ok(()) => self.recent_projects.set(history),
            Err(error) => self
                .error
                .set(Some(format!("Could not update recent projects: {error:#}"))),
        }
    }

    pub async fn open_recent(self, path: PathBuf) {
        self.open_saved_project(path).await;
    }

    pub fn snapshot(self) -> Option<Arc<Project>> {
        self.analysis_preview
            .read()
            .clone()
            .or_else(|| self.doc.read().clone())
    }
    pub fn show_analysis_preview(mut self, proposed: Option<Project>) {
        self.analysis_preview.set(proposed.map(Arc::new));
        if let (Some(store), Some(doc)) = ((self.store)(), self.snapshot()) {
            let media = self.media.peek().clone();
            if media.as_ref().is_none_or(|media| !media.matches(&doc)) {
                self.refresh_audio(store, doc);
            }
        }
    }
    pub fn metadata_value(self, key: &str) -> String {
        self.metadata.read().get(key).cloned().unwrap_or_default()
    }
    pub fn is_vocal_track(self) -> bool {
        self.snapshot()
            .is_some_and(|doc| doc.selected_track == "Vocals")
    }
    pub fn reload(mut self, transport: bool) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let snapshot = crate::storage::cached_project(&store.root)
            .map(Ok)
            .unwrap_or_else(|| store.load().map(Arc::new));
        match snapshot {
            Ok(doc) => {
                let preview = self.analysis_preview.peek().clone();
                if let Some(preview) = preview {
                    if preview.audio_sha256 != doc.audio_sha256 {
                        self.analysis_preview.set(None);
                    } else {
                        let mut preview = (*preview).clone();
                        preview.transport = doc.transport.clone();
                        preview.revision = doc.revision;
                        self.analysis_preview.set(Some(Arc::new(preview)));
                    }
                }
                let reference_changed = self.doc.peek().as_ref().is_none_or(|old| {
                    old.selected_track != doc.selected_track
                        || old.audio_sha256 != doc.audio_sha256
                        || old.content.keyboard_stems != doc.content.keyboard_stems
                        || old.content.stems != doc.content.stems
                        || serde_json::to_value(&old.content.track_origins).ok()
                            != serde_json::to_value(&doc.content.track_origins).ok()
                });
                if reference_changed {
                    self.synth_reference.set(store.synth_reference(&doc));
                }
                let source_audio = match doc
                    .has_recording()
                    .then(|| store.audio_path(&doc))
                    .transpose()
                {
                    Ok(path) => path,
                    Err(error) => {
                        self.error.set(Some(format!("{error:#}")));
                        return;
                    }
                };
                if *self.source_audio.peek() != source_audio {
                    self.source_audio.set(source_audio);
                }
                self.timing.set(doc.timing(&store).ok());
                self.chart_hash
                    .set(doc.chart_hash(&store).unwrap_or_default());
                if !transport {
                    self.export_state
                        .set(crate::export_state::State::load(&store, &doc));
                }
                self.doc.set(Some(doc));
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    pub fn dispatch(mut self, action: Action) {
        if self.analysis_preview.peek().is_some() && !crate::reanalysis::preview_transport(&action)
        {
            self.error.set(Some(
                "Switch to Current before editing; the proposed draft is read-only".into(),
            ));
            return;
        }
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let Some(doc) = self.doc.read().clone() else {
            return;
        };
        let seek = matches!(
            action,
            Action::Seek { .. } | Action::SelectRegion { .. } | Action::SetLoop { .. }
        );
        let selected = matches!(
            action,
            Action::SelectTrack { .. } | Action::SelectNote { .. } | Action::Undo | Action::Redo
        );
        let session = matches!(
            action,
            Action::Seek { .. }
                | Action::Playback { .. }
                | Action::Pause { .. }
                | Action::SetPlaybackRate { .. }
                | Action::SetLoop { .. }
                | Action::SelectRegion { .. }
                | Action::SelectTrack { .. }
                | Action::SelectNote { .. }
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
                self.reload(session);
                if selected {
                    self.group.set(Default::default());
                    self.vocal_note.set(None);
                }
                if let Some(doc) = self.snapshot()
                    && let Some(media) = self.media.read().clone()
                {
                    if !media.matches(&doc) {
                        if (self.audio_ready)() {
                            self.refresh_audio(store, doc);
                        }
                        return;
                    }
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
    fn refresh_audio(mut self, store: Store, doc: Arc<Project>) {
        self.epoch.set((self.epoch)().wrapping_add(1));
        let epoch = (self.epoch)();
        if let Some(media) = self.media.read().as_ref() {
            media.pause();
        }
        self.audio_ready.set(false);
        self.status.set("Restoring project audio…".into());
        spawn(async move {
            let result = Media::load(&store, &doc).await;
            if (self.epoch)() != epoch
                || self
                    .store
                    .read()
                    .as_ref()
                    .is_none_or(|current| current.root != store.root)
            {
                return;
            }
            match result {
                Ok((media, _, notice)) => {
                    let Some(current) = self.snapshot() else {
                        return;
                    };
                    if !media.matches(&current) {
                        self.refresh_audio(store, current);
                        return;
                    }
                    if current.kit_audition.is_some()
                        && let Err(error) = media.replace_kit(&store, &current)
                    {
                        self.error
                            .set(Some(format!("Could not restore kit audio: {error:#}")));
                        return;
                    }
                    media.set_rate(current.transport.playback_rate);
                    media.seek(current.transport.position);
                    media.mix(
                        &current.content.stem_mix,
                        (self.volume)(),
                        current.kit_audition.is_some(),
                        super::media::keyboard_stems(&current),
                    );
                    self.waveforms.set(Some(Arc::new(
                        media.waveforms(super::media::keyboard_stems(&current)),
                    )));
                    if current.transport.playing
                        && let Err(error) = media.play()
                    {
                        self.error.set(Some(format!("{error:#}")));
                    }
                    self.media.set(Some(Rc::new(media)));
                    self.audio_ready.set(true);
                    self.stem_status.set(notice);
                    self.status.set("Project audio restored".into());
                }
                Err(error) => self
                    .error
                    .set(Some(format!("Could not restore project audio: {error:#}"))),
            }
        });
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
                let selected = match crate::part_analysis::select_kit_audition(&store, &doc, &part)
                {
                    Ok(selected) => selected,
                    Err(_) => {
                        crate::worker::run(
                            crate::worker::Task::SelectKit {
                                root: store.root.clone(),
                                project: Box::new(doc.as_ref().clone()),
                                audio: store.audio_path(&doc)?,
                                part: part.clone(),
                            },
                            std::slice::from_ref(&store.root),
                        )
                        .await?
                    }
                };
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
                let media = self
                    .media
                    .read()
                    .clone()
                    .context("Prepare original audio first")?;
                if (self.epoch)() != epoch {
                    return Ok(());
                }
                media.replace_kit(&store, &current)?;
                media.mix(
                    &current.content.stem_mix,
                    (self.volume)(),
                    true,
                    super::media::keyboard_stems(&current),
                );
                self.audio_ready.set(true);
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
        if !(self.audio_ready)() {
            return;
        }
        let Some(doc) = self.snapshot() else { return };
        if let Some(media) = self.media.peek().as_ref() {
            media.analysis_route(
                &doc,
                (self.guitar_source)(),
                (self.follow_analysis_sources)(),
                (self.volume)(),
            );
        }
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
        self.audio_stalled.set(media.stalled());
        if let Some(range) = doc.transport.loop_range {
            if position >= range.end {
                position = range.start + (position - range.end).rem_euclid(range.end - range.start);
                media.seek(position);
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
                    self.generation_after_import.set(false);
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
                self.generation_after_import.set(false);
                self.error.set(Some(format!("{error:#}")));
            }
        });
    }
    pub fn new_blank_chart(mut self) {
        self.blank_chart.set(true);
    }
    pub fn attach_recording_dialog(mut self) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let Some(doc) = self.snapshot() else {
            return;
        };
        if doc.has_recording() {
            return;
        }
        dioxus::core::spawn_forever(async move {
            let result=async{
                let Some(mut files)=crate::files::pick(false,".ogg,.opus,.mp3,.flac,.wav,.m4a,.aiff,.aac").await? else{return Ok(());};
                let (name,bytes)=files.pop().context("Choose a recording")?;
                let input_root=crate::files::unique_root();let _candidate=Candidate{studio:self,root:input_root.clone()};
                crate::storage::create_dir_all(&input_root)?;let input=input_root.join(&name);crate::storage::write_shared(&input,bytes)?;
                self.status.set("Retaining recording locally…".into());
                let prepared:crate::project_recording::PreparedRecording=crate::worker::run(crate::worker::Task::PrepareRecording{source:input,root:store.root.clone()},std::slice::from_ref(&store.root)).await?;
                ensure!(self.store.read().as_ref().is_some_and(|current|current.root==store.root)&&self.snapshot().is_some_and(|current|current.revision==doc.revision),"The chart changed while choosing audio; attach it again to the current revision");
                self.dispatch(Action::AttachRecording{prepared});
                if self.error.peek().is_some() {
                    self.status.set("Recording not attached · chart preserved".into());
                }
                Ok::<_,anyhow::Error>(())
            }.await;
            if let Err(error) = result {
                self.error.set(Some(error.to_string()));
                self.status
                    .set("Recording not attached · chart preserved".into());
            }
        });
    }
    pub async fn finish_blank_chart(
        mut self,
        title: String,
        artist: String,
        bpm: f64,
        duration: f64,
    ) -> Result<()> {
        let (ini, chart) = crate::project_recording::blank_song(&title, &artist, bpm, duration)?;
        self.persist_current().await?;
        let root = crate::files::unique_root();
        let _candidate = Candidate {
            studio: self,
            root: root.clone(),
        };
        let source = root.join("source");
        crate::storage::create_dir_all(&source)?;
        crate::storage::write(source.join("song.ini"), ini)?;
        crate::storage::write(source.join("notes.chart"), chart)?;
        Store::new(&root).initialize(&source, None, duration)?;
        self.open_project(root).await?;
        self.detach_library();
        self.blank_chart.set(false);
        Ok(())
    }
    pub fn cancel_new_chart(mut self, root: &std::path::Path) {
        crate::storage::clear(root);
        self.new_song.set(None);
        self.generation_after_import.set(false);
    }
    pub async fn prepare_new_chart(
        self,
        root: PathBuf,
    ) -> Result<crate::song_import::RecordingInfo> {
        let audio = crate::pipeline::audio_file(&root)?;
        crate::worker::run(
            crate::worker::Task::InspectRecording {
                audio,
                output: root.join(".yace/audio/import"),
            },
            std::slice::from_ref(&root),
        )
        .await
    }
    pub async fn lookup_new_chart(
        self,
        root: PathBuf,
        metadata: crate::pipeline::Metadata,
        info: crate::song_import::SongInfo,
        lyrics: bool,
    ) -> Result<crate::song_import::RecordingInfo> {
        crate::song_import::lookup(metadata, info, &root.join(".yace/audio/import"), lyrics).await
    }
    pub fn stage_new_cover(
        self,
        root: &std::path::Path,
        cover: &std::path::Path,
    ) -> Result<PathBuf> {
        let bytes = crate::storage::shared(cover)?;
        let extension = crate::song_metadata::cover_extension(&bytes)?;
        let path = root.join(format!(
            ".yace/audio/import/selected-{}.{extension}",
            crate::storage::unique_id()
        ));
        crate::storage::copy(cover, &path)?;
        Ok(path)
    }
    pub async fn finish_new_chart(
        mut self,
        root: PathBuf,
        title: String,
        artist: String,
        timing: TempoMap,
        info: crate::song_import::SongInfo,
    ) -> Result<()> {
        let (ini, chart) = crate::song_metadata::blank_song_with_timing(&title, &artist, &timing)?;
        crate::storage::write(root.join("song.ini"), ini)?;
        crate::storage::write(root.join("notes.chart"), chart)?;
        crate::song_metadata::write_song_info(&root, &info)?;
        self.status.set("Preparing new chart…".into());
        self.open_project(root.clone()).await?;
        crate::storage::clear(&root);
        self.new_song.set(None);
        self.detach_library();
        if self.generation_after_import.replace(false) {
            self.generation_open.set(true);
        }
        Ok(())
    }
    pub fn import(mut self, folder: bool) {
        if (self.library_busy)() {
            return;
        }
        self.library_busy.set(true);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let (selection, files) = if folder {
                    let Some(selected) = crate::files::pick_library().await? else {
                        return Ok(());
                    };
                    (Some(Rc::new(selected)), Vec::new())
                } else {
                    let Some(files) = crate::files::pick(false, ".zip").await? else {
                        return Ok(());
                    };
                    let files = crate::files::unpack(files)?;
                    super::dance::flush().await?;
                    if super::dance::try_open_files(
                        files
                            .iter()
                            .map(|(name, bytes)| (name.clone(), bytes.as_ref().clone()))
                            .collect(),
                    )? {
                        return Ok(());
                    }
                    (None, files)
                };
                self.status.set("Reading local song…".into());
                let staging = crate::files::unique_root();
                let result: Result<()> = async {
                    if let Some(selection) = &selection {
                        selection.catalogue(&staging).await?;
                    } else {
                        crate::files::mount(&staging, files)?;
                    }
                    let library = crate::chart_library::discover(&staging)?;
                    let opened = self
                        .open_selected_entry(&library, 0, selection.as_deref())
                        .await?;
                    if let Some(previous) = self.library.replace(Some(library)) {
                        crate::storage::clear(&previous.root);
                    }
                    self.library_files.set(selection);
                    self.library_selected.set(opened.then_some(0));
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
            let selection = self.library_files.read().clone();
            if self
                .open_selected_entry(&library, index, selection.as_deref())
                .await?
            {
                self.library_selected.set(Some(index));
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.library_busy.set(false);
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    async fn open_selected_entry(
        self,
        library: &crate::chart_library::Library,
        index: usize,
        selection: Option<&crate::files::FolderSelection>,
    ) -> Result<bool> {
        let selected = &library
            .entries
            .get(index)
            .context("Chart is not in this folder")?
            .path;
        let result = async {
            if let Some(selection) = selection {
                selection.hydrate(&library.root, selected).await?;
            }
            self.open_library_entry(library, index).await
        }
        .await;
        if let Some(selection) = selection {
            selection.release_audio(&library.root, selected);
        }
        result
    }
    async fn open_library_entry(
        self,
        library: &crate::chart_library::Library,
        index: usize,
    ) -> Result<bool> {
        super::dance::flush().await?;
        use sha2::{Digest, Sha256};
        let item = library
            .entries
            .get(index)
            .context("Chart is not in this folder")?;
        if super::dance::try_open_path(&item.path)? {
            super::dance::remember_recent(self);
            return Ok(true);
        }
        let project_source = if item.kind == crate::chart_library::Kind::Project {
            Some(item.path.clone())
        } else if crate::storage::is_file(item.path.join(".yace/project/project.json")) {
            Some(item.path.join(".yace/project"))
        } else {
            None
        };
        let source = if let Some(project) = &project_source {
            project.join("source")
        } else {
            item.path.clone()
        };
        let choice = if project_source.is_some() {
            None
        } else {
            let Some(choice) = self.choose_chart_source(&source).await? else {
                return Ok(false);
            };
            Some(choice)
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
        let chart = crate::song_files::source_choice::selected_file(&source, choice)?;
        if let Some(audio) = crate::song_metadata::recording_file(&source)? {
            identity.update(crate::models::hash_file(&audio)?.as_bytes());
        } else {
            identity.update(b"chart-only-source-v1");
        }
        identity.update(crate::models::hash_file(&chart)?.as_bytes());
        if let Some(provenance) =
            crate::song_files::source_choice::recovery_identity(&source, choice)?
        {
            identity.update(provenance.as_bytes());
        }
        for path in [
            source.join("song.ini"),
            project_source
                .as_ref()
                .unwrap_or(&item.path)
                .join("project.json"),
        ] {
            if crate::storage::is_file(&path) {
                identity.update(crate::models::hash_file(&path)?.as_bytes());
            }
        }
        let legacy = PathBuf::from(format!(
            "/projects/library-{:x}",
            identity.clone().finalize()
        ));
        if project_source.is_none()
            && let Some(channels) = crate::stem_review::conventional::audio_identity(&source)?
        {
            identity.update(channels.as_bytes());
        }
        let root = PathBuf::from(format!("/projects/library-{:x}", identity.finalize()));
        let _candidate = Candidate {
            studio: self,
            root: root.clone(),
        };
        self.persist_current().await?;
        if let Some((root, revision)) = crate::persistence::load(&root).await? {
            self.open_project_with_revision(root, Some(revision))
                .await?;
            return Ok(true);
        }
        // Keep earlier editing copies discoverable. Their audio snapshot and
        // undo history remain authoritative; importing a ZIP creates a fresh
        // project when the user wants the newly discovered channel mix.
        if root != legacy
            && let Some((root, revision)) = crate::persistence::load(&legacy).await?
        {
            self.open_project_with_revision(root, Some(revision))
                .await?;
            let mut studio = self;
            studio.status.set("Opened your saved editing copy. Import the song ZIP to start a new project with its supplied audio channels.".into());
            return Ok(true);
        }
        if let Some(project) = project_source {
            crate::pipeline::copy_folder(&project, &root)?;
            self.open_project(root).await?;
        } else {
            crate::song_files::prepare_editable_source_with_choice(
                &source,
                &root.join("source"),
                choice,
            )?;
            let _: bool = crate::worker::run(
                crate::worker::Task::PrepareGameChannels {
                    root: root.join("source"),
                },
                &[root.join("source")],
            )
            .await?;
            let duration = super::media::source_duration(&root.join("source")).await?;
            Store::new(&root).initialize(&root.join("source"), None, duration)?;
            let report = crate::song_layout::resolve(
                &root.join("source"),
                "chart-engine-vocal-generation.json",
            );
            if crate::storage::is_file(&report) {
                let report = serde_json::from_slice(&crate::storage::read(report)?)?;
                let store = Store::new(&root);
                crate::vocal_review::attach_generation(&store, store.load()?, &report)?;
            }
            self.open_project(root).await?;
        }
        Ok(true)
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
            self.detach_library();
            self.saved_projects.set(None);
            Ok::<_, anyhow::Error>(())
        }
        .await;
        self.library_busy.set(false);
        if let Err(error) = result {
            self.error.set(Some(format!("{error:#}")));
        }
    }
    pub(super) async fn persist_current(mut self) -> Result<()> {
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
        let expected = (self.saved_revision)();
        let result = crate::persistence::save(&store.root, doc.revision, expected).await;
        self.persistence_busy.set(false);
        result?;
        self.saved_revision.set(Some(doc.revision));
        if expected.is_none() {
            self.remember_recent(store.root.clone());
        }
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
    pub(super) fn detach_library(mut self) {
        self.library_selected.set(None);
        self.library_files.set(None);
        if let Some(previous) = self.library.replace(None)
            && self
                .store
                .read()
                .as_ref()
                .is_none_or(|store| !store.root.starts_with(&previous.root))
        {
            crate::storage::clear(&previous.root);
        }
    }
    pub async fn open_project_with_revision(
        mut self,
        source: PathBuf,
        saved_revision: Option<u64>,
    ) -> Result<()> {
        self.analysis_preview.set(None);
        super::dance::flush().await?;
        if super::dance::try_open_path(&source)? {
            super::dance::remember_recent(self);
            return Ok(());
        }
        let project = crate::storage::is_file(source.join("project.json"));
        let root = if project {
            source.clone()
        } else {
            crate::files::unique_root()
        };
        let _candidate = Candidate {
            studio: self,
            root: root.clone(),
        };
        if !project {
            let Some(choice) = self.choose_chart_source(&source).await? else {
                return Ok(());
            };
            crate::song_files::prepare_editable_source_with_choice(
                &source,
                &root.join("source"),
                Some(choice),
            )?;
            let _: bool = crate::worker::run(
                crate::worker::Task::PrepareGameChannels {
                    root: root.join("source"),
                },
                &[root.join("source")],
            )
            .await?;
            let duration = super::media::source_duration(&root.join("source")).await?;
            Store::new(&root).initialize(&root.join("source"), None, duration)?;
            let report = crate::song_layout::resolve(
                &root.join("source"),
                "chart-engine-vocal-generation.json",
            );
            if crate::storage::is_file(&report) {
                let report = serde_json::from_slice(&crate::storage::read(report)?)?;
                let store = Store::new(&root);
                crate::vocal_review::attach_generation(&store, store.load()?, &report)?;
            }
        }
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
        // Resolve every fallible view input before replacing the active store.
        // A chart-only project intentionally has no audio path; a failed open
        // must not pair the previous document with the new project's store.
        let timing = doc.timing(&store)?;
        let sections = store.source_sections(&doc)?;
        let sync = store.source_sync(&doc).ok();
        let star_power = store.source_star_power(&doc)?;
        let source_audio = doc
            .has_recording()
            .then(|| store.audio_path(&doc))
            .transpose()?;
        let guitar_source = store.guitar_source(&doc)?;
        let synth_reference = store.synth_reference(&doc);
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
        self.song_download_receipt.set(None);
        self.export_state
            .set(crate::export_state::State::load(&store, &doc));
        self.saved_revision.set(saved_revision);
        self.metadata.set(metadata);
        self.timing.set(Some(timing));
        self.source_sections.set(sections);
        self.source_sync.set(sync);
        self.source_star_power.set(star_power);
        self.star_power_open.set(false);
        self.star_power_target.set(None);
        self.timing_open.set(false);
        self.source_audio.set(source_audio);
        self.guitar_source.set(guitar_source);
        self.synth_reference.set(synth_reference);
        self.group.set(Default::default());
        self.vocal_note.set(None);
        self.sections_open.set(false);
        self.playhead.set(doc.transport.position);
        self.view_start
            .set((doc.transport.position - 0.5).clamp(0., (doc.duration - (self.span)()).max(0.)));
        self.doc.set(Some(Arc::new(doc.clone())));
        super::dance::deactivate();
        super::dance::attach_existing(self);
        self.dirty.set(true);
        self.waveforms.set(Some(Arc::new(waves)));
        self.media.set(Some(Rc::new(media)));
        self.audio_ready.set(true);
        self.stem_status.set(kit_notice.unwrap_or(notice));
        self.status.set("Ready · files stay in this browser".into());
        if saved_revision.is_some() {
            self.remember_recent(root.clone());
        }
        if let Some(previous) = previous_root.filter(|previous| previous != &root) {
            crate::storage::clear(&previous);
        }
        Ok(())
    }
    pub fn export_dialog(mut self) {
        self.export_open.set(true);
    }
    pub fn export_selected(
        mut self,
        options: crate::export_options::Options,
        root: PathBuf,
        revision: u64,
    ) {
        if (self.export_busy)() {
            return;
        }
        self.export_busy.set(true);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let store = self.store.read().clone().context("Open a project")?;
                let doc = self.snapshot().context("Open a project")?;
                ensure!(
                    store.root == root && doc.revision == revision,
                    "The project changed; reopen Export"
                );
                options.selected(&store, &doc)?;
                let name = format!("{} reviewed.zip", self.metadata_value("name"));
                let receipt = crate::export_state::Receipt::capture(
                    &store,
                    &doc,
                    &options,
                    name.clone(),
                    crate::export_state::Delivery::PreparedZip,
                )?;
                let output = PathBuf::from(format!("/exports/{}", js_sys::Date::now() as u64));
                self.status
                    .set("Preparing playable song ZIP locally…".into());
                let actor = Actor {
                    kind: ActorKind::Human,
                    name: (self.reviewer)(),
                };
                let task = if doc.has_recording() && options.include_audio {
                    crate::worker::Task::Export {
                        root: store.root.clone(),
                        project: Box::new(doc.as_ref().clone()),
                        audio: store.audio_path(&doc)?,
                        output: output.clone(),
                        actor,
                        options,
                    }
                } else {
                    crate::worker::Task::ExportChartOnly {
                        root: store.root.clone(),
                        project: Box::new(doc.as_ref().clone()),
                        output: output.clone(),
                        actor,
                        options,
                    }
                };
                let _: () = crate::worker::run_with_progress(
                    task,
                    std::slice::from_ref(&store.root),
                    Rc::new(move |label| {
                        let mut studio = self;
                        studio.status.set(label);
                    }),
                )
                .await?;
                let archive = output.with_extension("zip");
                if self.store.peek().as_ref().is_some_and(|s| s.root == root) {
                    if let Err(error) = receipt.save(&store) {
                        self.error.set(Some(format!(
                            "Song ZIP prepared, but its history could not be saved: {error:#}"
                        )));
                    }
                    let current = self.snapshot().context("Project closed")?;
                    self.export_state
                        .set(crate::export_state::State::load(&store, &current));
                    self.dirty.set(true);
                }
                self.song_download_receipt
                    .set(Some((root.clone(), receipt)));
                self.export_open.set(false);
                if let Some((previous, _, _)) =
                    self.export_ready
                        .replace(Some((archive, name, DownloadKind::Song)))
                {
                    crate::storage::clear(&previous);
                }
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
        self.song_download_receipt.set(None);
        if let Some((path, _, _)) = self.export_ready.take() {
            crate::storage::clear(&path);
        }
    }
    pub fn save_project(mut self) {
        if (self.export_busy)() {
            return;
        }
        self.export_busy.set(true);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let store = self.store.read().clone().context("Open a project")?;
                let name = format!("{} project.zip", self.metadata_value("name"));
                let output = PathBuf::from(format!(
                    "/exports/project-{}.zip",
                    crate::storage::unique_id()
                ));
                self.status.set("Preparing project backup locally…".into());
                let _: () = crate::worker::run(
                    crate::worker::Task::Backup {
                        root: store.root.clone(),
                        output: output.clone(),
                    },
                    std::slice::from_ref(&store.root),
                )
                .await?;
                if let Some((previous, _, _)) =
                    self.export_ready
                        .replace(Some((output, name, DownloadKind::Project)))
                {
                    crate::storage::clear(&previous);
                }
                self.song_download_receipt.set(None);
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            } else {
                self.status.set("Project snapshot ready to download".into());
            }
            self.export_busy.set(false);
        });
    }
    pub fn song_download_requested(mut self) {
        let Some((root, mut receipt)) = (self.song_download_receipt)() else {
            return;
        };
        let Some(store) = self.store.peek().clone().filter(|s| s.root == root) else {
            return;
        };
        let result = receipt
            .request_download()
            .and_then(|()| receipt.save(&store));
        if let Err(error) = result {
            self.error.set(Some(format!(
                "Download requested, but its history could not be saved: {error:#}"
            )));
        }
        if let Some(doc) = self.snapshot() {
            self.export_state
                .set(crate::export_state::State::load(&store, &doc));
        }
        self.song_download_receipt.set(Some((root, receipt)));
        self.dirty.set(true);
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
                            if crate::models::hash_shared(bytes) == asset.sha256 {
                                let filename = PathBuf::from(name)
                                    .file_name()
                                    .context("Invalid stem filename")?
                                    .to_owned();
                                crate::storage::write_shared(
                                    store.root.join("source/stems").join(filename),
                                    bytes.clone(),
                                )?;
                                crate::storage::write_shared(&asset.path, bytes.clone())?;
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
