//! Shared project actions, local audio and desktop jobs. Saved edits use Store.
use super::presentation_clock::PresentationClock;
use crate::{
    editor::{Action, Actor, ActorKind, Project, Request, Store},
    models,
    stem_review::{LoadedStems, PreparedMix, StemId, StemManifest, StemMix},
    timing::TempoMap,
    waveform::Waveform,
};
use anyhow::{Context, Result};
use dioxus::prelude::*;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditTool {
    Select,
    Place,
    Erase,
}
fn cancels_kit_autoplay(action: &Action) -> bool {
    matches!(
        action,
        Action::SetStemMix { .. }
            | Action::SetKitAudition { part: None }
            | Action::AttachStems { .. }
            | Action::AttachKeyboardStems { .. }
            | Action::Playback { playing: false }
            | Action::SetPlaybackRate { .. }
    )
}

/// Peak amplitudes in original recording seconds, independent of audition gain.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StemWaveformCache {
    pub version: u32,
    pub source_audio_sha256: String,
    pub assets_identity_sha256: String,
    pub sample_rate: u32,
    pub source_frames: usize,
    pub frames_per_bin: usize,
    pub normalization: String,
    pub alignment_scope: String,
    pub peaks: BTreeMap<StemId, Vec<f32>>,
}
struct PreparedReviewAudio {
    loaded: LoadedStems,
    kit_audio: Option<Arc<Vec<f32>>>,
    waveforms: Arc<ReviewWaveforms>,
    warning: Option<String>,
    stem_waveforms: Option<Arc<StemWaveformCache>>,
    stem_waveform_path: Option<PathBuf>,
}
/// Detailed source envelopes stay independent of tempo-converted audition PCM.
#[derive(Debug)]
pub struct ReviewWaveforms {
    pub original: Waveform,
    pub stems: BTreeMap<StemId, Waveform>,
}
/// A single interleaved sample cursor addresses the original and every stem.
/// Mix weights change atomically at a stereo frame boundary; buffers stay shared.
#[derive(Clone)]
struct ReviewSource {
    loaded: Arc<LoadedStems>,
    kit_audio: Option<Arc<Vec<f32>>>,
    mix: Arc<RwLock<PreparedMix>>,
    sample: usize,
    frame: [f32; 2],
}
impl Iterator for ReviewSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.sample >= self.loaded.original.len() {
            return None;
        }
        if self.sample.is_multiple_of(2) {
            self.frame = if let Some(kit) = &self.kit_audio {
                [kit[self.sample], kit[self.sample + 1]]
            } else {
                match self.mix.read() {
                    Ok(mix) => self.loaded.frame_with_mix(self.sample / 2, &mix),
                    Err(_) => [
                        self.loaded.original[self.sample],
                        self.loaded.original[self.sample + 1],
                    ],
                }
            };
        }
        let sample = self.frame[self.sample % 2];
        self.sample += 1;
        Some(sample)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.loaded.original.len().saturating_sub(self.sample);
        (remaining, Some(remaining))
    }
}
impl rodio::Source for ReviewSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.loaded.original.len().saturating_sub(self.sample))
    }
    fn channels(&self) -> rodio::ChannelCount {
        2.try_into().unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.loaded.sample_rate.try_into().unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.loaded.frame_count() as f64 / self.loaded.sample_rate as f64,
        ))
    }
    fn try_seek(
        &mut self,
        position: Duration,
    ) -> std::result::Result<(), rodio::source::SeekError> {
        let channel = self.sample % 2;
        let frame = (position.as_secs_f64() * self.loaded.sample_rate as f64).floor() as usize;
        let frame = frame.min(self.loaded.frame_count());
        self.sample = frame * 2;
        if channel == 1 && frame < self.loaded.frame_count() {
            self.frame = if let Some(kit) = &self.kit_audio {
                [kit[self.sample], kit[self.sample + 1]]
            } else {
                match self.mix.read() {
                    Ok(mix) => self.loaded.frame_with_mix(frame, &mix),
                    Err(_) => [
                        self.loaded.original[self.sample],
                        self.loaded.original[self.sample + 1],
                    ],
                }
            };
            self.sample += 1;
        }
        Ok(())
    }
}
pub struct Audio {
    _device: rodio::MixerDeviceSink,
    player: rodio::Player,
    buffer: ReviewSource,
    rate: f64,
    assets_key: String,
    source_hash: String,
    kit_audition: Option<crate::part_analysis::KitAudition>,
    output_config: AudioOutputConfig,
    clock: RefCell<PresentationClock>,
    epoch: Instant,
}
#[derive(Clone, Debug, serde::Serialize)]
struct AudioOutputConfig {
    requested_buffer_frames: u32,
    configured_buffer_frames: Option<u32>,
    sample_rate: u32,
    nominal_buffer_ms: Option<f64>,
    fallback_reason: Option<String>,
    hardware_output_latency_known: bool,
}
impl AudioOutputConfig {
    fn status(&self) -> String {
        let timing = self.nominal_buffer_ms.map_or_else(
            || "device-default buffer".into(),
            |ms| format!("{ms:.1} ms nominal buffer"),
        );
        if self.fallback_reason.is_some() {
            format!("Audio fallback: {timing}; DAC latency unmeasured")
        } else {
            format!("{timing}; DAC latency unmeasured")
        }
    }
}
impl Audio {
    fn new(
        loaded: LoadedStems,
        kit_audio: Option<Arc<Vec<f32>>>,
        kit_audition: Option<crate::part_analysis::KitAudition>,
        rate: f64,
        assets_key: String,
        source_hash: String,
        mix: &StemMix,
    ) -> Result<Self> {
        anyhow::ensure!(
            kit_audio
                .as_ref()
                .is_none_or(|audio| audio.len() == loaded.original.len()),
            "Kit audio must share the complete transport clock"
        );
        anyhow::ensure!(
            kit_audio.is_some() == kit_audition.is_some(),
            "Kit PCM/selection mismatch"
        );
        let mut mix = mix.clone();
        mix.channels.retain(|id, _| loaded.stems.contains_key(id));
        let prepared = mix.prepare(loaded.stems.keys().copied())?;
        let preferred = rodio::DeviceSinkBuilder::from_default_device().and_then(|builder| {
            builder
                .with_buffer_size(rodio::cpal::BufferSize::Fixed(512))
                .open_stream()
        });
        let (mut device, fallback_reason) = match preferred {
            Ok(device) => (device, None),
            Err(error) => (
                rodio::DeviceSinkBuilder::open_default_sink()
                    .context("Cannot open audio output device")?,
                Some(error.to_string()),
            ),
        };
        let configured_buffer_frames = match device.config().buffer_size() {
            rodio::cpal::BufferSize::Fixed(frames) => Some(*frames),
            rodio::cpal::BufferSize::Default => None,
        };
        let output_rate = device.config().sample_rate().get();
        let output_config = AudioOutputConfig {
            requested_buffer_frames: 512,
            configured_buffer_frames,
            sample_rate: output_rate,
            nominal_buffer_ms: configured_buffer_frames
                .map(|frames| frames as f64 / output_rate as f64 * 1000.0),
            fallback_reason,
            hardware_output_latency_known: false,
        };
        device.log_on_drop(false);
        let player = rodio::Player::connect_new(device.mixer());
        player.pause();
        let buffer = ReviewSource {
            loaded: Arc::new(loaded),
            kit_audio,
            mix: Arc::new(RwLock::new(prepared)),
            sample: 0,
            frame: [0.0; 2],
        };
        player.append(buffer.clone());
        Ok(Self {
            _device: device,
            player,
            buffer,
            rate,
            assets_key,
            source_hash,
            kit_audition,
            output_config,
            clock: RefCell::new(PresentationClock::default()),
            epoch: Instant::now(),
        })
    }
    fn set_mix(&self, mix: &StemMix) -> Result<()> {
        let original = StemMix::default();
        let mix = if self.buffer.loaded.stems.is_empty()
            && mix.mode == crate::stem_review::MixMode::Original
        {
            &original
        } else {
            mix
        };
        let mut mix = mix.clone();
        mix.channels
            .retain(|id, _| self.buffer.loaded.stems.contains_key(id));
        let prepared = mix.prepare(self.buffer.loaded.stems.keys().copied())?;
        *self
            .buffer
            .mix
            .write()
            .map_err(|_| anyhow::anyhow!("Audio mix lock poisoned"))? = prepared;
        Ok(())
    }
    fn seek(&self, seconds: f64) -> Result<()> {
        if self.player.empty() {
            self.player.append(self.buffer.clone());
        }
        self.player
            .try_seek(Duration::from_secs_f64(seconds.max(0.0) / self.rate))?;
        self.clock
            .borrow_mut()
            .reset(self.epoch.elapsed().as_secs_f64(), seconds.max(0.0));
        Ok(())
    }
    fn position(&self) -> f64 {
        let raw = self.player.get_pos().as_secs_f64() * self.rate;
        if self.player.is_paused() {
            return raw;
        }
        self.clock
            .borrow_mut()
            .sample(self.epoch.elapsed().as_secs_f64(), raw, self.rate)
    }
}
struct WindowLock(std::fs::File);
impl Drop for WindowLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Studio {
    pub doc: Signal<Option<Project>>,
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
    pub generation_target: Signal<Option<String>>,
    pub vocal_note: Signal<Option<usize>>,
    pub metadata: Signal<BTreeMap<String, String>>,
    pub chart_hash: Signal<String>,
    pub stem_status: Signal<String>,
    pub stem_waveforms: Signal<Option<Arc<StemWaveformCache>>>,
    pub stem_waveform_path: Signal<Option<PathBuf>>,
    audio: Signal<Option<Rc<Audio>>>,
    lock: Signal<Option<Arc<WindowLock>>>,
    audio_epoch: Signal<u64>,
    audio_target: Signal<Option<(String, f64, String)>>,
    kit_audition_epoch: Signal<u64>,
    kit_autoplay: Signal<Option<(u64, crate::part_analysis::KitAudition)>>,
    file_stamp: Signal<Option<(std::time::SystemTime, u64)>>,
}
impl Studio {
    pub fn metadata_value(self, key: &str) -> String {
        self.metadata.read().get(key).cloned().unwrap_or_default()
    }

    pub fn audio_details(self) -> String {
        self.audio
            .read()
            .as_ref()
            .map_or_else(String::new, |audio| audio.output_config.status())
    }

    pub fn snapshot(self) -> Option<Project> {
        self.doc.read().clone()
    }
    pub fn dispatch(mut self, action: Action) {
        if cancels_kit_autoplay(&action) {
            self.kit_audition_epoch
                .set((self.kit_audition_epoch)().wrapping_add(1));
            self.kit_autoplay.set(None);
            self.kit_audition_busy.set(None);
        }
        if matches!(
            &action,
            Action::SelectTrack { .. } | Action::Undo | Action::Redo
        ) {
            self.vocal_note.set(None);
        }
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let Some(doc) = self.snapshot() else { return };
        let actor = Actor {
            kind: ActorKind::Human,
            name: self.reviewer.read().clone(),
        };
        let transport = matches!(
            action,
            Action::Seek { .. }
                | Action::Playback { .. }
                | Action::SetPlaybackRate { .. }
                | Action::SetLoop { .. }
                | Action::SelectRegion { .. }
        );
        match store.apply(Request {
            expected_revision: doc.revision,
            actor,
            action,
        }) {
            Ok(result) => {
                self.error.set(None);
                if let Some(path) = result.exported_to {
                    self.status.set(format!("Exported {}", path.display()));
                }
                self.reload(transport);
            }
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                self.reload(false);
            }
        }
    }
    pub fn is_vocal_track(self) -> bool {
        self.doc
            .read()
            .as_ref()
            .is_some_and(|doc| doc.selected_track == "Vocals")
    }
    pub fn reload(mut self, transport: bool) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        match store.load() {
            Ok(doc) => {
                if self
                    .doc
                    .read()
                    .as_ref()
                    .is_some_and(|old| old.revision != doc.revision)
                    && doc
                        .audit
                        .last()
                        .is_some_and(|entry| cancels_kit_autoplay(&entry.action))
                {
                    self.kit_audition_epoch
                        .set((self.kit_audition_epoch)().wrapping_add(1));
                    self.kit_autoplay.set(None);
                    self.kit_audition_busy.set(None);
                }
                if let Some(index) = (self.vocal_note)() {
                    let changed = self.doc.read().as_ref().is_none_or(|old| {
                        old.content
                            .vocals
                            .as_ref()
                            .and_then(|chart| chart.notes.get(index))
                            != doc
                                .content
                                .vocals
                                .as_ref()
                                .and_then(|chart| chart.notes.get(index))
                    });
                    if changed {
                        self.vocal_note.set(None);
                    }
                }
                self.file_stamp.set(project_stamp(&store));
                match doc.chart_hash(&store) {
                    Ok(hash) => self.chart_hash.set(hash),
                    Err(error) => {
                        self.error.set(Some(error.to_string()));
                        return;
                    }
                }
                let assets_key = stem_assets_key(&doc, doc.selected_track == "Vocals");
                let reattached = doc.audit.last().is_some_and(|entry| {
                    matches!(
                        entry.action,
                        Action::AttachStems { .. } | Action::AttachKeyboardStems { .. }
                    )
                }) && self
                    .doc
                    .read()
                    .as_ref()
                    .is_some_and(|old| old.revision != doc.revision);
                let replace_audio = reattached
                    || self.audio.read().as_ref().is_none_or(|audio| {
                        (audio.rate - doc.transport.playback_rate).abs() > 1e-9
                            || audio.assets_key != assets_key
                    });
                let target = (
                    doc.audio_sha256.clone(),
                    doc.transport.playback_rate,
                    assets_key,
                );
                let need_prepare =
                    replace_audio && self.audio_target.read().as_ref() != Some(&target);
                let cancelled_preparation = !replace_audio && self.audio_target.read().is_some();
                if replace_audio {
                    if !transport {
                        self.playhead.set(self.current_audio_position());
                    }
                    self.audio_ready.set(false);
                    if let Some(audio) = self.audio.read().as_ref() {
                        audio.player.pause();
                    }
                } else if self.audio_target.read().is_some() {
                    self.audio_epoch.set((self.audio_epoch)() + 1);
                    self.audio_target.set(None);
                    self.audio_ready.set(true);
                }
                if !replace_audio
                    && let Some(audio) = self.audio.read().as_ref()
                    && let Err(error) = audio.set_mix(&doc.content.stem_mix)
                {
                    self.error.set(Some(format!(
                        "Stem mix unavailable; original audio retained: {error:#}"
                    )));
                }
                if !replace_audio {
                    self.audio_ready.set(self.audio.read().is_some());
                }
                if transport || cancelled_preparation {
                    let seconds = if transport {
                        doc.transport.current_position(doc.duration)
                    } else {
                        (self.playhead)()
                    };
                    self.playhead.set(seconds);
                    self.view_start.set(
                        (seconds - 0.5)
                            .max(0.0)
                            .min((doc.duration - (self.span)()).max(0.0)),
                    );
                    if let Some(audio) = self.audio.read().as_ref().filter(|_| !replace_audio) {
                        if let Err(error) = audio.seek(seconds) {
                            self.error.set(Some(error.to_string()));
                        }
                        if doc.transport.playing {
                            audio.player.play();
                        } else {
                            audio.player.pause();
                        }
                    }
                }
                self.doc.set(Some(doc));
                if need_prepare {
                    self.audio_target.set(Some(target));
                    spawn(async move {
                        self.prepare_practice_audio().await;
                    });
                }
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    pub fn seek(self, seconds: f64) {
        if let Some(doc) = self.snapshot() {
            self.dispatch(Action::Seek {
                seconds: seconds.clamp(0.0, doc.duration),
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
        if !(self.audio_ready)() {
            return (self.playhead)();
        }
        self.audio
            .read()
            .as_ref()
            .map_or((self.playhead)(), |audio| audio.position())
    }
    pub fn set_playback_rate(self, rate: f64) {
        self.seek(self.current_audio_position());
        self.dispatch(Action::SetPlaybackRate { rate });
    }
    /// The same saved selection as editor/AI actions, with validation on a
    /// worker and autoplay only after the shared source has been installed.
    pub fn audition_kit_part(mut self, part: String, start: Option<f64>) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
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
        let Some(project) = self.snapshot() else {
            return;
        };
        let Ok(source_key) = crate::part_analysis::kit_source_key(&project) else {
            return;
        };
        let epoch = (self.kit_audition_epoch)().wrapping_add(1);
        self.kit_audition_epoch.set(epoch);
        self.kit_autoplay.set(None);
        self.kit_audition_busy.set(Some(part.clone()));
        self.error.set(None);
        let root = store.root.clone();
        let actor = Actor {
            kind: ActorKind::Human,
            name: (self.reviewer)(),
        };
        spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                store.apply(Request {
                    expected_revision: project.revision,
                    actor,
                    action: Action::SetKitAudition { part: Some(part) },
                })?;
                store.load()
            })
            .await;
            if (self.kit_audition_epoch)() != epoch
                || self
                    .store
                    .read()
                    .as_ref()
                    .is_none_or(|store| store.root != root)
                || self.snapshot().is_none_or(|doc| {
                    crate::part_analysis::kit_source_key(&doc).ok().as_ref() != Some(&source_key)
                })
            {
                return;
            }
            match result {
                Ok(Ok(project)) => {
                    let Some(selected) = project.kit_audition else {
                        self.kit_audition_busy.set(None);
                        self.reload(true);
                        return;
                    };
                    self.kit_autoplay.set(Some((epoch, selected.clone())));
                    self.reload(true);
                    if self
                        .snapshot()
                        .is_none_or(|doc| doc.kit_audition.as_ref() != Some(&selected))
                    {
                        self.kit_autoplay.set(None);
                        self.kit_audition_busy.set(None);
                    } else if (self.audio_ready)() {
                        self.kit_autoplay.set(None);
                        self.kit_audition_busy.set(None);
                        self.dispatch(Action::Playback { playing: true });
                    }
                }
                Ok(Err(error)) => {
                    self.kit_audition_busy.set(None);
                    self.error
                        .set(Some(format!("Kit audition unavailable: {error:#}")));
                    self.reload(true);
                }
                Err(error) => {
                    self.kit_audition_busy.set(None);
                    self.error
                        .set(Some(format!("Kit audition unavailable: {error}")));
                }
            }
        });
    }
    async fn prepare_practice_audio(mut self) {
        let Some(doc) = self.snapshot() else {
            return;
        };
        let Some(path) = self.source_audio.read().clone() else {
            return;
        };
        let rate = doc.transport.playback_rate;
        let key = stem_assets_key(&doc, self.is_vocal_track());
        let expected = doc.audio_sha256.clone();
        let manifest = review_manifest(&doc, self.is_vocal_track()).cloned();
        let epoch = (self.audio_epoch)() + 1;
        self.audio_epoch.set(epoch);
        self.audio_ready.set(false);
        self.audio_target
            .set(Some((expected.clone(), rate, key.clone())));
        if let Some(audio) = self.audio.read().as_ref() {
            audio.player.pause();
        }
        self.stem_status.set("Preparing aligned audio…".into());
        self.stem_waveforms.set(None);
        self.stem_waveform_path.set(None);
        self.status
            .set(format!("Preparing {rate:.2}× audition audio…"));
        let source = path.clone();
        let store = self.store.read().clone();
        let cache_root = store.as_ref().map(|store| store.root.clone());
        let project = doc.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut prepared = load_review_audio(
                &path,
                &expected,
                manifest.as_ref(),
                rate,
                cache_root.as_deref(),
            )?;
            if let Some(selected) = &project.kit_audition {
                let pcm = crate::part_analysis::prepare_kit_audition(
                    store.as_ref().context("No project for kit audition")?,
                    &project,
                    selected,
                    rate,
                    prepared.loaded.original.len(),
                )?;
                prepared.kit_audio = Some(Arc::new(pcm));
            }
            Ok::<_, anyhow::Error>(prepared)
        })
        .await;
        let current = self.snapshot();
        if (self.audio_epoch)() != epoch
            || self.source_audio.read().as_ref() != Some(&source)
            || current.as_ref().is_none_or(|doc| {
                doc.transport.playback_rate != rate
                    || stem_assets_key(doc, self.is_vocal_track()) != key
            })
        {
            return;
        }
        self.audio_target.set(None);
        match result {
            Ok(Ok(PreparedReviewAudio {
                loaded,
                kit_audio,
                waveforms,
                warning,
                stem_waveforms,
                stem_waveform_path,
            })) => {
                let current = current.unwrap();
                let has_stems = !loaded.stems.is_empty();
                let mix = if has_stems {
                    current.content.stem_mix.clone()
                } else {
                    StemMix::default()
                };
                let kit_label = current.kit_audition.as_ref().map(|part| part.label.clone());
                match Audio::new(
                    loaded,
                    kit_audio,
                    current.kit_audition.clone(),
                    rate,
                    key,
                    current.audio_sha256.clone(),
                    &mix,
                ) {
                    Ok(audio) => {
                        let _ = audio.seek((self.playhead)());
                        audio.player.set_volume((self.volume)());
                        self.audio.set(Some(Rc::new(audio)));
                        self.waveforms.set(Some(waveforms));
                        self.stem_waveforms.set(stem_waveforms);
                        self.stem_waveform_path.set(stem_waveform_path);
                        self.audio_ready.set(true);
                        self.stem_status.set(if let Some(label) = kit_label {
                            if let Some(warning) = warning {
                                self.error.set(Some(warning));
                            }
                            format!("{label} only")
                        } else if let Some(warning) = warning {
                            self.error.set(Some(warning));
                            "Stems unavailable · original recording active".into()
                        } else if has_stems {
                            "Stems ready · check alignment".into()
                        } else {
                            "Original recording".into()
                        });
                        self.status.set(if rate == 1.0 {
                            "Ready to play".into()
                        } else {
                            format!("Ready at {:.0}% speed", rate * 100.0)
                        });
                        let autoplay =
                            self.kit_autoplay
                                .read()
                                .as_ref()
                                .is_some_and(|(epoch, selected)| {
                                    *epoch == (self.kit_audition_epoch)()
                                        && current.kit_audition.as_ref() == Some(selected)
                                });
                        self.kit_autoplay.set(None);
                        self.kit_audition_busy.set(None);
                        if autoplay {
                            self.dispatch(Action::Playback { playing: true });
                        } else if current.transport.playing {
                            self.seek((self.playhead)());
                        }
                    }
                    Err(error) => {
                        self.kit_autoplay.set(None);
                        self.kit_audition_busy.set(None);
                        self.error.set(Some(format!("{error:#}")));
                    }
                }
            }
            Ok(Err(error)) => {
                self.kit_autoplay.set(None);
                self.kit_audition_busy.set(None);
                self.stem_status.set("Audio unavailable".into());
                self.error.set(Some(format!("{error:#}")));
            }
            Err(error) => {
                self.kit_autoplay.set(None);
                self.kit_audition_busy.set(None);
                self.error.set(Some(error.to_string()));
            }
        }
    }
    pub fn attach_stems_dialog(self) {
        let keyboard = !self.is_vocal_track()
            && self.snapshot().is_some_and(|doc| {
                crate::keys::is_keys(&doc.selected_track) && doc.content.keyboard_stems.is_some()
            });
        let expected_root = self.store.read().as_ref().map(|store| store.root.clone());
        spawn(async move {
            let Some(path) = rfd::AsyncFileDialog::new()
                .set_title("Attach aligned stem manifest")
                .add_filter("Stem manifest", &["json"])
                .pick_file()
                .await
            else {
                return;
            };
            if self.store.read().as_ref().map(|store| store.root.clone()) != expected_root {
                return;
            }
            self.attach_stems_manifest_with_target(path.path().to_owned(), keyboard)
                .await;
        });
    }
    pub async fn attach_stems_manifest(self, manifest: PathBuf) {
        self.attach_stems_manifest_with_target(manifest, false)
            .await;
    }
    async fn attach_stems_manifest_with_target(mut self, manifest: PathBuf, keyboard: bool) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let Some(revision) = self.doc.read().as_ref().map(|doc| doc.revision) else {
            return;
        };
        let actor = Actor {
            kind: ActorKind::Human,
            name: self.reviewer.read().clone(),
        };
        let expected_root = store.root.clone();
        self.stem_status.set("Validating supplied stems…".into());
        let result = tokio::task::spawn_blocking(move || {
            store.apply(Request {
                expected_revision: revision,
                actor,
                action: if keyboard {
                    Action::AttachKeyboardStems { manifest }
                } else {
                    Action::AttachStems { manifest }
                },
            })
        })
        .await;
        if self
            .store
            .read()
            .as_ref()
            .is_none_or(|store| store.root != expected_root)
        {
            return;
        }
        match result {
            Ok(Ok(_)) => {
                self.error.set(None);
                self.reload(false);
            }
            Ok(Err(error)) => {
                self.stem_status
                    .set("Attachment rejected · existing audio retained".into());
                self.error.set(Some(format!("{error:#}")));
            }
            Err(error) => self.error.set(Some(error.to_string())),
        }
    }
    pub fn update_volume(mut self, volume: f32) {
        self.volume.set(volume.clamp(0.0, 1.0));
        if let Some(audio) = self.audio.read().as_ref() {
            audio.player.set_volume((self.volume)());
        }
    }
    pub fn open_dialog(self) {
        spawn(async move {
            if let Some(path) = rfd::AsyncFileDialog::new()
                .set_title("Open saved YACE review project")
                .pick_folder()
                .await
            {
                self.open_project(path.path().to_owned()).await;
            }
        });
    }
    pub fn create_dialog(self) {
        spawn(async move {
            let Some(song) = rfd::AsyncFileDialog::new()
                .set_title("Open song folder (.chart or MIDI)")
                .pick_folder()
                .await
            else {
                return;
            };
            let Some(output) = rfd::AsyncFileDialog::new()
                .set_title("Save editing project in a new folder")
                .set_file_name(format!("{} Review Project", song.file_name()))
                .save_file()
                .await
            else {
                return;
            };
            let song = song.path().to_owned();
            let output = output.path().to_owned();
            let result =
                tokio::task::spawn_blocking(move || Store::create(&song, None, &output)).await;
            match result {
                Ok(Ok(store)) => self.open_project(store.root).await,
                Ok(Err(error)) => {
                    let mut this = self;
                    this.error.set(Some(format!("{error:#}")));
                }
                Err(error) => {
                    let mut this = self;
                    this.error.set(Some(error.to_string()));
                }
            }
        });
    }
    pub fn export_dialog(self) {
        spawn(async move {
            let Some(doc) = self.snapshot() else { return };
            if let Some(path) = rfd::AsyncFileDialog::new()
                .set_title("Export song into a new folder")
                .set_file_name(format!("{} reviewed", doc.title))
                .save_file()
                .await
            {
                self.dispatch(Action::Export {
                    output: path.path().to_owned(),
                });
            }
        });
    }
    pub async fn open_project(mut self, path: PathBuf) {
        self.kit_audition_epoch
            .set((self.kit_audition_epoch)().wrapping_add(1));
        self.kit_autoplay.set(None);
        self.kit_audition_busy.set(None);
        self.vocal_note.set(None);
        self.status.set("Opening project…".into());
        let loaded = tokio::task::spawn_blocking(move || -> Result<_> {
            let store = Store::new(&path.canonicalize()?);
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(store.root.join("window.lock"))?;
            lock.try_lock()
                .context("This project already has an editor window open")?;
            let doc = store.load()?;
            let guitar_source = store.guitar_source(&doc)?;
            let timing = doc.timing(&store)?;
            let audio = store.audio_path(&doc)?;
            let metadata = crate::pipeline::ini_fields(&crate::pipeline::song_ini(
                &store.root.join("source"),
            )?);
            let art = ["album.png", "album.jpg", "album.jpeg"]
                .iter()
                .map(|name| store.root.join("source").join(name))
                .find(|p| p.is_file());
            // data URL stays local and is portable inside the app's asset protocol.
            let art = art.and_then(|p| {
                use base64::Engine;
                let bytes = std::fs::read(&p).ok()?;
                let mime = if p.extension()?.to_str()? == "png" {
                    "image/png"
                } else {
                    "image/jpeg"
                };
                Some(format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ))
            });
            Ok((
                store,
                doc,
                timing,
                audio,
                metadata,
                art,
                guitar_source,
                Arc::new(WindowLock(lock)),
            ))
        })
        .await;
        let (store, doc, timing, audio_path, metadata, art, guitar_source, lock) = match loaded {
            Ok(Ok(value)) => value,
            Err(e) => {
                self.error.set(Some(e.to_string()));
                return;
            }
            Ok(Err(e)) => {
                self.error.set(Some(format!("{e:#}")));
                return;
            }
        };
        // Pause and close the former project before replacing its audio/lock.
        if let Some(previous) = self.snapshot()
            && previous.transport.playing
        {
            self.seek((self.playhead)());
            self.dispatch(Action::Playback { playing: false });
        }
        if let Some(old) = self.store.read().as_ref() {
            let _ = std::fs::remove_file(old.root.join("live-state.json"));
        }
        self.audio_epoch.set((self.audio_epoch)() + 1);
        self.audio.set(None);
        self.lock.set(Some(lock));
        self.store.set(Some(store));
        self.timing.set(Some(timing));
        self.view_start
            .set((doc.transport.current_position(doc.duration) - 0.5).max(0.0));
        self.playhead
            .set(doc.transport.current_position(doc.duration));
        self.doc.set(Some(doc.clone()));
        if let Some(store) = self.store.read().as_ref() {
            self.file_stamp.set(project_stamp(store));
        }
        if let Some(store) = self.store.read().as_ref() {
            self.chart_hash
                .set(doc.chart_hash(store).unwrap_or_default());
        }
        self.metadata.set(metadata);
        self.guitar_source.set(guitar_source);
        self.album_art.set(art);
        self.source_audio.set(Some(audio_path.clone()));
        self.waveforms.set(None);
        self.stem_waveforms.set(None);
        self.stem_waveform_path.set(None);
        self.audio_ready.set(false);
        self.error.set(None);
        self.status.set("Loading recording…".into());
        self.prepare_practice_audio().await;
    }
    pub fn tick(mut self, publish: bool) {
        if !(self.audio_ready)() {
            return;
        }
        let Some((transport, duration)) = self
            .doc
            .read()
            .as_ref()
            .map(|doc| (doc.transport.clone(), doc.duration))
        else {
            return;
        };
        let mut ended = false;
        if let Some(audio) = self.audio.read().as_ref() {
            let mut position = audio.position().min(duration);
            let stalled = transport.playing && audio.clock.borrow().is_stale();
            if *self.audio_stalled.peek() != stalled {
                self.audio_stalled.set(stalled);
            }
            if transport.playing {
                if let Some(range) = transport.loop_range {
                    // A visual interpolation must never cut the recording's
                    // loop short. Only the actual audio counter wraps transport.
                    let raw = audio.player.get_pos().as_secs_f64() * audio.rate;
                    if raw >= range.end {
                        position =
                            range.start + (raw - range.end).rem_euclid(range.end - range.start);
                        let _ = audio.seek(position);
                    }
                } else if audio.player.empty() {
                    audio.player.pause();
                    ended = true;
                }
                if publish {
                    self.playhead.set(position);
                }
                if publish
                    && (self.follow)()
                    && !((self.view_start)()..(self.view_start)() + (self.span)())
                        .contains(&position)
                {
                    self.view_start.set(
                        (position - 0.5)
                            .max(0.0)
                            .min((duration - (self.span)()).max(0.0)),
                    );
                }
            }
        }
        if ended {
            self.dispatch(Action::Playback { playing: false });
        }
    }
    pub fn poll(mut self) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let stamp = project_stamp(&store);
        if stamp != (self.file_stamp)()
            && let Ok(new) = store.load()
            && self
                .doc
                .read()
                .as_ref()
                .is_none_or(|doc| doc.revision != new.revision)
        {
            let transport = new.audit.last().is_some_and(|a| {
                matches!(
                    a.action,
                    Action::Seek { .. }
                        | Action::Playback { .. }
                        | Action::SetPlaybackRate { .. }
                        | Action::SetLoop { .. }
                        | Action::SelectRegion { .. }
                )
            });
            self.reload(transport);
        }
        self.file_stamp.set(stamp);
        let _ = store.report_live(
            self.doc.read().as_ref().map_or(0, |d| d.revision),
            self.current_audio_position(),
            self.doc
                .read()
                .as_ref()
                .is_some_and(|d| d.transport.playing)
                && (self.audio_ready)(),
            (self.audio_ready)(),
            self.audition_state(),
        );
    }
    pub fn audition_state(self) -> serde_json::Value {
        use sha2::{Digest, Sha256};
        let project = self.doc.read();
        let requested = project.as_ref().map(|doc| &doc.content.stem_mix);
        let audio = self.audio.read();
        let preparing = self.audio_target.read().is_some();
        let ready = (self.audio_ready)();
        let actual = audio.as_ref().and_then(|audio| {
            audio.buffer.mix.read().ok().map(|mix| {
                let identity = serde_json::to_vec(&(
                    audio.source_hash.as_str(),
                    audio.assets_key.as_str(),
                    audio.rate,
                ))
                .unwrap_or_default();
                serde_json::json!({
                    "mode":if audio.kit_audition.is_some(){"kit"}else if mix.original{"original"}else{"stems"},
                    "active_gains":if mix.original || audio.kit_audition.is_some(){BTreeMap::new()}else{mix.gains.clone()},
                    "kit_audition":audio.kit_audition,
                    "source_audio_sha256":audio.source_hash,
                    "assets_identity_sha256":format!("{:x}",Sha256::digest(identity)),
                    "playback_rate":audio.rate,
                    "player_paused":audio.player.is_paused(),
                    "stems_ready":ready && !audio.buffer.loaded.stems.is_empty(),
                    "sample_rate":audio.buffer.loaded.sample_rate,
                    "shared_cursor":true,
                    "output_config":audio.output_config,
                })
            })
        });
        let fallback = !preparing
            && requested.is_some_and(|mix| mix.mode == crate::stem_review::MixMode::Stems)
            && actual
                .as_ref()
                .is_some_and(|actual| actual["mode"] == "original");
        let waveform = self.stem_waveforms.read();
        let waveform=waveform.as_ref().map(|cache|serde_json::json!({
            "path":self.stem_waveform_path.read().as_ref(),
            "source_audio_sha256":cache.source_audio_sha256,
            "assets_identity_sha256":cache.assets_identity_sha256,
            "sample_rate":cache.sample_rate,
            "source_frames":cache.source_frames,
            "frames_per_bin":cache.frames_per_bin,
            "rendering":{
                "base_frames_per_bin":crate::waveform::BASE_FRAMES_PER_BIN,
                "detail":"Zoom-dependent signed min/max envelope; connected antialiased path",
                "coordinates":"Original recording seconds at every practice speed",
            },
            "available_stems":cache.peaks.keys().collect::<Vec<_>>(),
            "coordinates":"original recording seconds; bin i starts at i * frames_per_bin / sample_rate",
            "normalization":cache.normalization,
            "amplitude_is_confidence":false,
        }));
        serde_json::json!({
            "requested_mix":requested,
            "requested_kit_audition":project.as_ref().and_then(|doc|doc.kit_audition.as_ref()),
            "actual":actual,
            "preparing":preparing,
            "audio_ready":ready,
            "fallback_to_original":fallback,
            "status":self.stem_status.read().as_str(),
            "alignment_scope":"Supplied alignment provenance and engineering checks; instrument identity and musical accuracy are not certified",
            "stem_waveforms":waveform,
        })
    }
}
pub fn use_studio() -> Studio {
    Studio {
        doc: use_signal(|| None),
        store: use_signal(|| None),
        timing: use_signal(|| None),
        playhead: use_signal(|| 0.0),
        audio_ready: use_signal(|| false),
        audio_stalled: use_signal(|| false),
        kit_audition_busy: use_signal(|| None),
        status: use_signal(|| "Ready".into()),
        error: use_signal(|| None),
        clean_open: use_signal(|| false),
        generation_open: use_signal(|| false),
        clean_ranges: use_signal(Vec::new),
        review_open: use_signal(|| false),
        assistant_open: use_signal(|| false),
        view_start: use_signal(|| 0.0),
        span: use_signal(|| 12.0),
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
        generation_target: use_signal(|| None),
        vocal_note: use_signal(|| None),
        metadata: use_signal(BTreeMap::new),
        chart_hash: use_signal(String::new),
        stem_status: use_signal(|| "Original recording".into()),
        stem_waveforms: use_signal(|| None),
        stem_waveform_path: use_signal(|| None),
        audio: use_signal(|| None),
        lock: use_signal(|| None),
        audio_epoch: use_signal(|| 0),
        audio_target: use_signal(|| None),
        kit_audition_epoch: use_signal(|| 0),
        kit_autoplay: use_signal(|| None),
        file_stamp: use_signal(|| None),
    }
}
fn project_stamp(store: &Store) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::metadata(store.root.join("project.json")).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}
pub fn clock(seconds: f64) -> String {
    let tenths = if seconds.is_finite() {
        (seconds.max(0.0) * 10.0).round() as u64
    } else {
        0
    };
    format!(
        "{}:{:02}.{}",
        tenths / 600,
        (tenths % 600) / 10,
        tenths % 10
    )
}
pub fn instrument(track: &str) -> &'static str {
    if track == "Vocals" {
        return "Vocals";
    }
    if crate::keys::is_pro(track) {
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

pub(super) fn review_manifest(doc: &Project, vocals: bool) -> Option<&StemManifest> {
    if doc.selected_track == "Vocals" || vocals {
        doc.content.stems.as_ref()
    } else {
        doc.content.audition_stems(&doc.selected_track)
    }
}
fn stem_assets_key(doc: &Project, vocals: bool) -> String {
    serde_json::to_string(&(review_manifest(doc, vocals), &doc.kit_audition)).unwrap_or_default()
}

/// All file access, validation and tempo conversion happens off the UI thread.
/// A bad attachment never prevents auditioning an intact original recording.
fn load_review_audio(
    path: &std::path::Path,
    expected: &str,
    manifest: Option<&StemManifest>,
    rate: f64,
    cache_root: Option<&std::path::Path>,
) -> Result<PreparedReviewAudio> {
    anyhow::ensure!(
        models::hash_file(path)? == expected,
        "Source recording changed"
    );
    let (loaded, mut warning) = if let Some(manifest) = manifest {
        match manifest.load(path) {
            Ok(loaded) => (loaded, None),
            Err(error) => (
                LoadedStems::from_pcm(
                    44100,
                    Arc::new(models::decode(path, 44100, 2)?),
                    BTreeMap::new(),
                )?,
                Some(format!(
                    "Attached stems could not be loaded; original recording restored: {error:#}"
                )),
            ),
        }
    } else {
        (
            LoadedStems::from_pcm(
                44100,
                Arc::new(models::decode(path, 44100, 2)?),
                BTreeMap::new(),
            )?,
            None,
        )
    };
    anyhow::ensure!(
        models::hash_file(path)? == expected,
        "Source changed while loading audio"
    );
    let mut waveforms = ReviewWaveforms {
        original: Waveform::from_stereo(&loaded.original, loaded.sample_rate),
        stems: loaded
            .stems
            .iter()
            .map(|(id, pcm)| (*id, Waveform::from_stereo(pcm, loaded.sample_rate)))
            .collect(),
    };
    let mut stem_waveforms = (!loaded.stems.is_empty())
        .then(|| Arc::new(build_stem_waveforms(expected, manifest, &loaded)));
    let mut stem_waveform_path = stem_waveforms
        .as_ref()
        .zip(cache_root)
        .and_then(|(cache, root)| persist_stem_waveforms(root, cache).ok());
    if rate == 1.0 {
        return Ok(PreparedReviewAudio {
            loaded,
            kit_audio: None,
            waveforms: Arc::new(waveforms),
            warning,
            stem_waveforms,
            stem_waveform_path,
        });
    }
    let original = Arc::new(models::decode_at_tempo(path, 44100, 2, rate)?);
    let mut stems = BTreeMap::new();
    if !loaded.stems.is_empty() {
        let converted = (|| -> Result<_> {
            let mut converted = BTreeMap::new();
            for asset in &manifest.context("Stem provenance missing")?.stems {
                let mut samples = models::decode_at_tempo(&asset.path, 44100, 2, rate)?;
                anyhow::ensure!(
                    models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
                    "Attached stem changed during tempo conversion"
                );
                // Independent atempo filters can emit slightly different trailing
                // padding. Source PCM frame equality was checked before conversion.
                anyhow::ensure!(
                    samples.len().abs_diff(original.len())
                        <= ((44100.0 * 0.10 / rate) as usize) * 2,
                    "Tempo-converted stem duration differs by over 100 ms"
                );
                samples.resize(original.len(), 0.0);
                converted.insert(asset.id, Arc::new(samples));
            }
            Ok(converted)
        })();
        match converted {
            Ok(converted) => stems = converted,
            Err(error) => {
                stem_waveforms = None;
                stem_waveform_path = None;
                waveforms.stems.clear();
                warning = Some(format!(
                    "Practice stems unavailable; original recording restored: {error:#}"
                ))
            }
        }
    }
    anyhow::ensure!(
        models::hash_file(path)? == expected,
        "Source changed during tempo conversion"
    );
    Ok(PreparedReviewAudio {
        loaded: LoadedStems::from_pcm(44100, original, stems)?,
        kit_audio: None,
        waveforms: Arc::new(waveforms),
        warning,
        stem_waveforms,
        stem_waveform_path,
    })
}

const MAX_STEM_PEAK_BINS: usize = 32_768;
fn peak_bin_frames(frames: usize) -> usize {
    1024.max(frames.div_ceil(MAX_STEM_PEAK_BINS))
}
fn build_stem_waveforms(
    source_hash: &str,
    manifest: Option<&StemManifest>,
    loaded: &LoadedStems,
) -> StemWaveformCache {
    use sha2::{Digest, Sha256};
    let frames_per_bin = peak_bin_frames(loaded.frame_count());
    let binding = serde_json::to_vec(&(
        1u32,
        source_hash,
        manifest,
        loaded.frame_count(),
        frames_per_bin,
    ))
    .unwrap_or_default();
    StemWaveformCache{
        version:1,source_audio_sha256:source_hash.into(),assets_identity_sha256:format!("{:x}",Sha256::digest(binding)),
        sample_rate:loaded.sample_rate,source_frames:loaded.frame_count(),frames_per_bin,
        normalization:"Shared linear full scale; no per-stem normalization; amplitude is not confidence".into(),
        alignment_scope:"Estimated isolated audio; supplied alignment provenance, not certified instrument truth".into(),
        peaks:loaded.stems.iter().map(|(id,pcm)|(*id,pcm.chunks(frames_per_bin*2).map(|chunk|chunk.iter().copied().map(f32::abs).fold(0.0,f32::max)).collect())).collect(),
    }
}
fn persist_stem_waveforms(root: &std::path::Path, cache: &StemWaveformCache) -> Result<PathBuf> {
    let directory = root.join("waveforms");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{}.json", cache.assets_identity_sha256));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, cache)?;
    file.as_file().sync_all()?;
    file.persist(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stem_review::{MixMode, StemChannel, StemId};
    use rodio::Source;
    #[test]
    fn isolated_peak_cache_is_bounded_source_timed_and_not_individually_normalized() {
        let loaded = LoadedStems::from_pcm(
            44100,
            Arc::new(vec![0.9, -0.9, 0.8, -0.8]),
            BTreeMap::from([
                (StemId::Drums, Arc::new(vec![0.25, -0.2, 0.1, -0.1])),
                (StemId::Bass, Arc::new(vec![0.10, -0.05, 0.02, -0.01])),
            ]),
        )
        .unwrap();
        let cache = build_stem_waveforms(&"a".repeat(64), None, &loaded);
        assert_eq!(cache.peaks[&StemId::Drums], vec![0.25]);
        assert_eq!(cache.peaks[&StemId::Bass], vec![0.10]);
        assert_eq!(cache.sample_rate, 44100);
        assert_eq!(cache.source_frames, 2);
        assert_eq!(cache.frames_per_bin, 1024);
        let huge_song_frames = 44100usize * 60 * 90;
        assert!(huge_song_frames.div_ceil(peak_bin_frames(huge_song_frames)) <= MAX_STEM_PEAK_BINS);
        let root = tempfile::tempdir().unwrap();
        let path = persist_stem_waveforms(root.path(), &cache).unwrap();
        let restored: StemWaveformCache =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(cache, restored);
        assert_ne!(
            cache.assets_identity_sha256,
            build_stem_waveforms(&"b".repeat(64), None, &loaded).assets_identity_sha256
        );
    }
    #[test]
    fn clock_rounds_before_splitting_minutes() {
        assert_eq!(clock(59.94), "0:59.9");
        assert_eq!(clock(59.96), "1:00.0");
        assert_eq!(clock(239.96), "4:00.0");
        assert_eq!(clock(-1.0), "0:00.0");
        assert_eq!(clock(f64::NAN), "0:00.0");
        assert_eq!(clock(f64::INFINITY), "0:00.0");
    }
    #[test]
    fn one_cursor_switches_mix_only_between_stereo_frames_and_seeks_together() {
        let loaded = Arc::new(
            LoadedStems::from_pcm(
                44100,
                Arc::new(vec![0.2, 0.4, 0.6, 0.8]),
                BTreeMap::from([(StemId::Drums, Arc::new(vec![0.3, 0.5, 0.7, 0.9]))]),
            )
            .unwrap(),
        );
        let mix = StemMix {
            mode: MixMode::Stems,
            channels: BTreeMap::from([(StemId::Drums, StemChannel::default())]),
        };
        let weights = Arc::new(RwLock::new(
            mix.prepare(loaded.stems.keys().copied()).unwrap(),
        ));
        let mut source = ReviewSource {
            loaded: loaded.clone(),
            kit_audio: None,
            mix: weights.clone(),
            sample: 0,
            frame: [0.0; 2],
        };
        assert_eq!(source.next(), Some(0.3));
        *weights.write().unwrap() = StemMix::default()
            .prepare(loaded.stems.keys().copied())
            .unwrap();
        assert_eq!(source.next(), Some(0.5)); // right channel keeps the frame's earlier mix
        assert_eq!(source.next(), Some(0.6)); // next frame uses original
        source.try_seek(Duration::ZERO).unwrap();
        assert_eq!(source.next(), Some(0.4)); // preserve interleaved channel at the shared seek
        assert_eq!(source.next(), Some(0.6));
        assert_eq!(source.next(), Some(0.8));
        assert_eq!(source.next(), None);
    }
    #[test]
    fn kit_component_uses_the_main_stereo_cursor_and_never_the_recording_mix() {
        let loaded = Arc::new(
            LoadedStems::from_pcm(44100, Arc::new(vec![0.9; 8]), BTreeMap::new()).unwrap(),
        );
        let weights = Arc::new(RwLock::new(StemMix::default().prepare([]).unwrap()));
        let kit = Arc::new(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]);
        let mut source = ReviewSource {
            loaded,
            kit_audio: Some(kit.clone()),
            mix: weights.clone(),
            sample: 0,
            frame: [0.; 2],
        };
        assert_eq!(source.next(), Some(0.1));
        *weights.write().unwrap() = StemMix::default().prepare([]).unwrap();
        assert_eq!(source.next(), Some(0.2));
        // Seek inside frame 2. Nanosecond rounding can put the exact 2/44100
        // boundary just before that frame, where the player correctly floors to 1.
        source
            .try_seek(Duration::from_secs_f64(2.25 / 44100.))
            .unwrap();
        assert_eq!(source.next(), Some(0.5));
        assert_eq!(source.next(), Some(0.6));
        source.try_seek(Duration::ZERO).unwrap();
        assert_eq!(source.collect::<Vec<_>>(), *kit);
    }
    #[test]
    fn explicit_pause_or_normal_audio_choice_cancels_pending_kit_autoplay() {
        assert!(cancels_kit_autoplay(&Action::Playback { playing: false }));
        assert!(cancels_kit_autoplay(&Action::SetPlaybackRate { rate: 0.5 }));
        assert!(cancels_kit_autoplay(&Action::SetStemMix {
            mix: Default::default()
        }));
        assert!(cancels_kit_autoplay(&Action::SetKitAudition { part: None }));
        assert!(!cancels_kit_autoplay(&Action::Seek { seconds: 10. }));
        assert!(!cancels_kit_autoplay(&Action::SelectTrack {
            track: "ExpertDrums".into()
        }));
    }
}
