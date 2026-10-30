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
#[derive(Debug)]
pub struct ReviewWaveforms {
    pub original: Waveform,
    pub stems: BTreeMap<StemId, Waveform>,
}
#[derive(Clone, Copy, PartialEq)]
pub struct Studio {
    pub doc: Signal<Option<Arc<Project>>>,
    pub store: Signal<Option<Store>>,
    pub timing: Signal<Option<TempoMap>>,
    pub playhead: Signal<f64>,
    pub audio_ready: Signal<bool>,
    pub audio_stalled: Signal<bool>,
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
    pub generation_target: Signal<Option<String>>,
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
        doc: use_signal(|| None),
        store: use_signal(|| None),
        timing: use_signal(|| None),
        playhead: use_signal(|| 0.),
        audio_ready: use_signal(|| false),
        audio_stalled: use_signal(|| false),
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
        generation_target: use_signal(|| None),
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
                    media.mix(&doc.content.stem_mix, (self.volume)());
                    if seek {
                        media.seek(doc.transport.position);
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
            media.mix(&doc.content.stem_mix, (self.volume)());
        }
    }
    pub fn tick(mut self) {
        let Some(doc) = self.snapshot() else { return };
        if !doc.transport.playing {
            return;
        }
        let Some(media) = self.media.read().clone() else {
            return;
        };
        let mut position = media.position();
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
    pub fn import(mut self, folder: bool) {
        spawn(async move {
            let result = async {
                let Some(files) =
                    crate::files::pick(folder, if folder { "" } else { ".zip" }).await?
                else {
                    return Ok(());
                };
                self.status.set("Reading local song…".into());
                let files = crate::files::unpack(files)?;
                let root = crate::files::unique_root();
                crate::files::mount(&root, files)?;
                let root = crate::files::imported_root(&root)?;
                self.open_project(root).await
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
        });
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
            Store::new(&root).initialize(&root.join("source"), None, duration)?;
            root
        };
        let store = Store::new(&root);
        let mut doc = store.load()?;
        doc.transport.playing = false;
        doc.kit_audition = None;
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
        self.stem_status.set(notice);
        self.status.set("Ready · files stay in this browser".into());
        Ok(())
    }
    pub fn export_dialog(mut self) {
        let result = (|| -> Result<_> {
            let store = self.store.read().clone().context("Open a project")?;
            let doc = self.snapshot().context("Open a project")?;
            let output = PathBuf::from(format!("/exports/{}", js_sys::Date::now() as u64));
            store.write_export(
                &doc,
                &output,
                &Actor {
                    kind: ActorKind::Human,
                    name: (self.reviewer)(),
                },
            )?;
            let bytes = crate::files::archive(&output)?;
            let name = format!("{} reviewed.zip", self.metadata_value("name"));
            crate::files::download(&name, &bytes)?;
            crate::storage::clear(&output);
            Ok(())
        })();
        match result {
            Ok(()) => self.status.set("Song ZIP downloaded".into()),
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
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
        spawn(async move {
            let result = async {
                let Some(files) = crate::files::pick(true, "").await? else {
                    return Ok(());
                };
                let store = self.store.read().clone().context("Open a project")?;
                let doc = self.snapshot().context("Open a project")?;
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
    let source = crate::storage::list(&store.root.join("source/stems"))?;
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
