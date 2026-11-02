//! Shared project actions, local audio and desktop jobs. Saved edits use Store.
use super::dialogs as rfd;
use super::presentation_clock::PresentationClock;
#[cfg(feature = "browser")]
#[path = "browser_audio.rs"]
mod browser_audio;
use crate::{
    editor::{Action, Actor, ActorKind, Project, Request, Store},
    models,
    stem_review::{LoadedStems, PreparedMix, StemId, StemManifest, StemMix},
    timing::TempoMap,
    waveform::Waveform,
};
use anyhow::{Context, Result};
#[cfg(feature = "browser")]
use browser_audio::Audio;
use dioxus::prelude::*;
#[cfg(not(feature = "browser"))]
use std::cell::Cell;
#[cfg(not(feature = "browser"))]
use std::time::Duration;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, RwLock},
    time::Instant,
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
        Action::EditSync { .. }
            | Action::SetStemMix { .. }
            | Action::SetKitAudition { part: None }
            | Action::AttachStems { .. }
            | Action::AttachKeyboardStems { .. }
            | Action::Playback { playing: false }
            | Action::Pause { .. }
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
#[cfg(not(feature = "browser"))]
#[derive(Clone)]
struct ReviewSource {
    loaded: Arc<LoadedStems>,
    kit_audio: Option<Arc<Vec<f32>>>,
    mix: Arc<RwLock<PreparedMix>>,
    sample: usize,
    frame: [f32; 2],
}
#[cfg(not(feature = "browser"))]
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
#[cfg(not(feature = "browser"))]
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
#[cfg(not(feature = "browser"))]
pub struct Audio {
    _device: Option<rodio::MixerDeviceSink>,
    player: super::timeline_player::Player,
    buffer: Option<ReviewSource>,
    rate: Cell<f64>,
    stream: RefCell<Option<crate::streaming_audio::Controller>>,
    output_observation: Cell<(u64, Instant)>,
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
        if self.sample_rate == 0 {
            return "Chart timeline · no recording".into();
        }
        if cfg!(feature = "browser") {
            return "Browser audio · shared source clock".into();
        }
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
#[cfg(not(feature = "browser"))]
impl Audio {
    fn timeline(duration: f64, rate: f64, assets_key: String) -> Result<Self> {
        Ok(Self {
            _device: None,
            player: super::timeline_player::Player::timeline(duration, rate)?,
            buffer: None,
            rate: Cell::new(rate),
            stream: RefCell::new(None),
            output_observation: Cell::new((0, Instant::now())),
            assets_key,
            source_hash: String::new(),
            kit_audition: None,
            output_config: AudioOutputConfig {
                requested_buffer_frames: 0,
                configured_buffer_frames: None,
                sample_rate: 0,
                nominal_buffer_ms: None,
                fallback_reason: None,
                hardware_output_latency_known: false,
            },
            clock: RefCell::new(PresentationClock::default()),
            epoch: Instant::now(),
        })
    }
    async fn new(
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
        let (stream, source) = buffer.stream(rate)?;
        player.append(source);
        Ok(Self {
            _device: Some(device),
            player: super::timeline_player::Player::recorded(player),
            buffer: Some(buffer),
            rate: Cell::new(rate),
            stream: RefCell::new(Some(stream)),
            output_observation: Cell::new((0, Instant::now())),
            assets_key,
            source_hash,
            kit_audition,
            output_config,
            clock: RefCell::new(PresentationClock::default()),
            epoch: Instant::now(),
        })
    }
    fn set_mix(&self, mix: &StemMix) -> Result<()> {
        self.set_routed_mix(mix, None)
    }
    fn set_routed_mix(
        &self,
        mix: &StemMix,
        route: Option<crate::reanalysis::AuditionRoute>,
    ) -> Result<()> {
        let Some(buffer) = &self.buffer else {
            mix.prepare([])?;
            return Ok(());
        };
        let original = StemMix::default();
        let mix = if buffer.loaded.stems.is_empty() {
            &original
        } else {
            mix
        };
        let mut mix = mix.clone();
        mix.channels
            .retain(|id, _| buffer.loaded.stems.contains_key(id));
        let mut prepared = mix.prepare(buffer.loaded.stems.keys().copied())?;
        // Cached buffers and every reanalysis route use original source time.
        prepared.route = route.map(|mut route| {
            route.playback_rate = 1.;
            route
        });
        *buffer
            .mix
            .write()
            .map_err(|_| anyhow::anyhow!("Audio mix lock poisoned"))? = prepared;
        Ok(())
    }
    fn seek(&self, seconds: f64) -> Result<()> {
        if let Some(buffer) = &self.buffer {
            let restart = self
                .stream
                .borrow()
                .as_ref()
                .is_none_or(|s| s.worker_exited());
            if self.player.empty() || restart {
                let loop_range = self
                    .stream
                    .borrow()
                    .as_ref()
                    .and_then(crate::streaming_audio::Controller::loop_range);
                let (stream, source) = buffer.stream(self.rate.get())?;
                stream.set_loop(loop_range)?;
                stream.seek(seconds.clamp(0., stream.duration()))?;
                self.output_observation.set((0, Instant::now()));
                self.stream.replace(Some(stream));
                self.player.append(source);
            } else if let Some(stream) = self.stream.borrow().as_ref() {
                stream.seek(seconds.clamp(0., stream.duration()))?;
            }
        } else {
            self.player
                .try_seek(Duration::from_secs_f64(seconds.max(0.0) / self.rate.get()))?;
        }
        self.clock
            .borrow_mut()
            .reset(self.epoch.elapsed().as_secs_f64(), seconds.max(0.0));
        if let Some(stream) = self.stream.borrow().as_ref() {
            self.output_observation
                .set((stream.output_sequence(), Instant::now()));
        }
        Ok(())
    }
    fn position(&self) -> f64 {
        let raw = self.raw_position();
        if self.player.is_paused() {
            return raw;
        }
        // The streaming ledger is observed every 2 ms. Its consumed source
        // position is already fine-grained, including variable rates and loops;
        // multiplying or extrapolating using a future DSP rate would lead audio.
        if self.stream.borrow().is_some() {
            return raw;
        }
        self.clock
            .borrow_mut()
            .sample(self.epoch.elapsed().as_secs_f64(), raw, self.rate.get())
    }
    fn raw_position(&self) -> f64 {
        self.stream.borrow().as_ref().map_or_else(
            || self.player.get_pos().as_secs_f64() * self.rate.get(),
            crate::streaming_audio::Controller::position,
        )
    }
    fn playback_rate(&self) -> f64 {
        self.rate.get()
    }
    fn is_stalled(&self) -> bool {
        if let Some(stream) = self.stream.borrow().as_ref() {
            let sequence = stream.output_sequence();
            let (previous, observed) = self.output_observation.get();
            if sequence != previous {
                self.output_observation.set((sequence, Instant::now()));
                false
            } else {
                observed.elapsed() >= Duration::from_millis(250)
            }
        } else {
            self.clock.borrow().is_stale()
        }
    }
    fn set_rate(&self, rate: f64) -> Result<()> {
        if self.rate.get() == rate {
            return Ok(());
        }
        let position = self.raw_position();
        if let Some(stream) = self.stream.borrow().as_ref() {
            if self.player.is_paused() {
                stream.set_rate_while_paused(rate)?;
            } else {
                stream.set_rate(rate)?;
            }
        } else {
            self.player.set_timeline_rate(rate)?;
        }
        self.rate.set(rate);
        self.clock
            .borrow_mut()
            .reset(self.epoch.elapsed().as_secs_f64(), position);
        Ok(())
    }
    fn set_loop(&self, range: Option<crate::editor::TimeRange>, playing: bool) -> Result<()> {
        if let Some(stream) = self.stream.borrow().as_ref() {
            // A paused seek can be anywhere in the recording. Activate the loop
            // only while playing, retaining ordinary Stop/rewind semantics.
            stream.set_loop(
                range
                    .filter(|_| playing)
                    .map(|r| (r.start.min(stream.duration()), r.end.min(stream.duration()))),
            )?;
        }
        Ok(())
    }
    fn ready_for_playback(&self) -> bool {
        self.stream
            .borrow()
            .as_ref()
            .is_none_or(crate::streaming_audio::Controller::ready)
    }
    fn mark_playback_started(&self) {
        if self.player.is_paused() {
            if let Some(stream) = self.stream.borrow().as_ref() {
                self.output_observation
                    .set((stream.output_sequence(), Instant::now()));
            }
            self.clock
                .borrow_mut()
                .reset(self.epoch.elapsed().as_secs_f64(), self.raw_position());
        }
    }
    fn processing_error(&self) -> Option<String> {
        self.stream
            .borrow()
            .as_ref()
            .and_then(crate::streaming_audio::Controller::error)
    }
    fn processing_diagnostics(&self) -> serde_json::Value {
        self.stream.borrow().as_ref().map_or(serde_json::Value::Null, |stream| serde_json::json!({
            "source_seconds":stream.position(), "requested_rate":stream.requested_rate(),
            "achieved_rate":stream.achieved_rate(), "priming_frames":stream.priming_frames(),
            "underflow_frames":stream.underflow_frames(), "observed_loop_wraps":stream.loop_wraps(),
            "ready":stream.ready(), "worker_exited":stream.worker_exited(),
            "buffers":stream.diagnostics(), "error":stream.error(),
        }))
    }
}
#[cfg(not(feature = "browser"))]
impl ReviewSource {
    fn stream(
        &self,
        rate: f64,
    ) -> Result<(
        crate::streaming_audio::Controller,
        crate::streaming_audio::AudioSource,
    )> {
        let loaded = self.loaded.clone();
        let kit = self.kit_audio.clone();
        let mix = self.mix.clone();
        crate::streaming_audio::start(
            loaded.sample_rate,
            loaded.frame_count() as u64,
            rate,
            Box::new(move |start, output| {
                let frame = usize::try_from(start).context("Audition source offset overflow")?;
                let offset = frame
                    .checked_mul(2)
                    .context("Audition source offset overflow")?;
                let end = offset
                    .checked_add(output.len())
                    .context("Audition source range overflow")?;
                if let Some(kit) = &kit {
                    output.copy_from_slice(
                        kit.get(offset..end)
                            .context("Kit source range outside recording")?,
                    );
                } else if let Ok(mix) = mix.read() {
                    anyhow::ensure!(
                        end <= loaded.original.len(),
                        "Audition source outside recording"
                    );
                    for (index, destination) in output.chunks_exact_mut(2).enumerate() {
                        destination.copy_from_slice(&loaded.frame_with_mix(frame + index, &mix));
                    }
                } else {
                    output.copy_from_slice(
                        loaded
                            .original
                            .get(offset..end)
                            .context("Original source range outside recording")?,
                    );
                }
                Ok(())
            }),
        )
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
    pub analysis_preview: Signal<Option<Arc<Project>>>,
    pub follow_analysis_sources: Signal<bool>,
    pub recent_projects: Signal<crate::recent_projects::History>,
    pub doc: Signal<Option<Arc<Project>>>,
    pub store: Signal<Option<Store>>,
    pub library: Signal<Option<crate::chart_library::Library>>,
    pub library_busy: Signal<bool>,
    pub library_selected: Signal<Option<usize>>,
    pub source_choice: Signal<Option<super::source_choice::Prompt>>,
    pub timing: Signal<Option<TempoMap>>,
    pub playhead: Signal<f64>,
    pub audio_ready: Signal<bool>,
    pub audio_stalled: Signal<bool>,
    pub kit_audition_busy: Signal<Option<String>>,
    pub status: Signal<String>,
    pub error: Signal<Option<String>>,
    pub clean_open: Signal<bool>,
    pub generation_open: Signal<bool>,
    pub new_song: Signal<Option<(PathBuf, String)>>,
    pub blank_chart: Signal<bool>,
    pub clean_ranges: Signal<Vec<(f64, f64)>>,
    pub review_open: Signal<bool>,
    pub export_open: Signal<bool>,
    pub export_busy: Signal<bool>,
    pub export_state: Signal<crate::export_state::State>,
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
    pub vocal_note: Signal<Option<usize>>,
    pub group: Signal<crate::note_groups::Selection>,
    pub selection_dragging: Signal<bool>,
    pub editor_focus: Signal<Option<Rc<MountedData>>>,
    #[cfg(not(feature = "browser"))]
    pub clipboard: Signal<Option<Rc<std::cell::RefCell<arboard::Clipboard>>>>,
    pub metadata: Signal<BTreeMap<String, String>>,
    pub chart_hash: Signal<String>,
    pub stem_status: Signal<String>,
    pub stem_waveforms: Signal<Option<Arc<StemWaveformCache>>>,
    pub stem_waveform_path: Signal<Option<PathBuf>>,
    audio: Signal<Option<Rc<Audio>>>,
    practice_rate_preview: Signal<Option<(PathBuf, f64)>>,
    lock: Signal<Option<Arc<WindowLock>>>,
    audio_epoch: Signal<u64>,
    audio_target: Signal<Option<(String, f64, String)>>,
    kit_audition_epoch: Signal<u64>,
    kit_autoplay: Signal<Option<(u64, crate::part_analysis::KitAudition)>>,
    file_stamp: Signal<Option<(std::time::SystemTime, u64)>>,
}
impl Studio {
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

    pub async fn open_recent(mut self, path: PathBuf) {
        if (self.library_busy)() {
            return;
        }
        if !path.join("project.json").is_file()
            && !path.join(crate::stepmania::bundle::MANIFEST).is_file()
        {
            self.error.set(Some(format!("This recent project is no longer at {}. Locate it with Open saved project, or remove it from Open Recent.", path.display())));
            return;
        }
        self.library_busy.set(true);
        self.open_project(path).await;
        self.library_busy.set(false);
    }

    pub fn metadata_value(self, key: &str) -> String {
        self.metadata.read().get(key).cloned().unwrap_or_default()
    }

    pub fn audio_details(self) -> String {
        self.audio
            .read()
            .as_ref()
            .map_or_else(String::new, |audio| audio.output_config.status())
    }

    pub fn snapshot(self) -> Option<Arc<Project>> {
        self.analysis_preview
            .read()
            .clone()
            .or_else(|| self.doc.read().clone())
    }
    pub fn show_analysis_preview(mut self, proposed: Option<Project>) {
        let was = self.snapshot();
        self.playhead.set(self.current_audio_position());
        self.analysis_preview.set(proposed.map(Arc::new));
        let current = self.snapshot();
        if was
            .as_ref()
            .map(|doc| stem_assets_key(doc, doc.selected_track == "Vocals"))
            != current
                .as_ref()
                .map(|doc| stem_assets_key(doc, doc.selected_track == "Vocals"))
        {
            if let Some(audio) = self.audio.read().as_ref() {
                audio.player.pause();
            }
            spawn(async move {
                self.prepare_practice_audio().await;
            });
        }
    }
    pub fn dispatch(self, action: Action) {
        self.dispatch_with_transport(action, true);
    }
    fn dispatch_with_transport(mut self, action: Action, apply_audio_transport: bool) -> bool {
        if self.analysis_preview.peek().is_some() && !crate::reanalysis::preview_transport(&action)
        {
            self.error.set(Some(
                "Switch to Current before editing; the proposed draft is read-only".into(),
            ));
            return false;
        }
        // Stop sound before reading, validating or saving a project. Large charts
        // and a busy disk must never delay a musician's pause command.
        let action = if matches!(
            action,
            Action::Playback { playing: false } | Action::Pause { .. }
        ) {
            let seconds = if let Some(audio) = self.audio.read().as_ref() {
                audio.player.pause();
                audio.position()
            } else {
                (self.playhead)()
            };
            let seconds = if let Action::Pause { seconds } = action {
                seconds
            } else {
                self.snapshot()
                    .map_or(seconds, |doc| seconds.clamp(0.0, doc.duration))
            };
            self.playhead.set(seconds);
            Action::Pause { seconds }
        } else {
            action
        };
        let paused_at = if let Action::Pause { seconds } = &action {
            Some(*seconds)
        } else {
            None
        };
        if cancels_kit_autoplay(&action) {
            self.kit_audition_epoch
                .set((self.kit_audition_epoch)().wrapping_add(1));
            self.kit_autoplay.set(None);
            self.kit_audition_busy.set(None);
        }
        if matches!(
            &action,
            Action::SelectTrack { .. } | Action::SelectNote { .. } | Action::Undo | Action::Redo
        ) {
            self.vocal_note.set(None);
            self.group.set(Default::default());
        }
        let Some(store) = self.store.read().clone() else {
            return false;
        };
        let Some(doc) = self.doc.read().clone() else {
            return false;
        };
        let actor = Actor {
            kind: ActorKind::Human,
            name: self.reviewer.read().clone(),
        };
        let transport = matches!(
            action,
            Action::Seek { .. }
                | Action::Playback { .. }
                | Action::Pause { .. }
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
                    let receipt = crate::export_state::Receipt::capture(
                        &store,
                        &doc,
                        &Default::default(),
                        path.display().to_string(),
                        crate::export_state::Delivery::PublishedFolder,
                    )
                    .and_then(|receipt| receipt.save(&store));
                    if let Err(error) = receipt {
                        self.error.set(Some(format!(
                            "Song exported, but its history could not be saved: {error:#}"
                        )));
                    }
                    #[cfg(not(feature = "browser"))]
                    self.status.set(format!("Exported {}", path.display()));
                    #[cfg(feature = "browser")]
                    self.status.set("Song exported · downloading ZIP…".into());
                    spawn(async move {
                        rfd::download(&path).await;
                    });
                }
                self.reload(transport && apply_audio_transport);
                true
            }
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                self.reload(false);
                // A revision conflict can reject saving the pause. Keep the
                // local transport stopped while the user resolves that error.
                let current = self.doc.read().clone();
                if let Some(seconds) = paused_at
                    && let Some(doc) = current
                {
                    let mut doc = (*doc).clone();
                    doc.transport.playing = false;
                    doc.transport.position = seconds.clamp(0.0, doc.duration);
                    self.playhead.set(doc.transport.position);
                    self.doc.set(Some(Arc::new(doc)));
                }
                false
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
                let Ok(timing) = doc.timing(&store) else {
                    self.error.set(Some("Cannot read chart timing".into()));
                    return;
                };
                let timing_changed = self.timing.read().as_ref() != Some(&timing);
                if timing_changed {
                    self.timing.set(Some(timing));
                    self.selection_dragging.set(false);
                    self.group.set(Default::default());
                }
                let recording_changed = self.doc.read().as_ref().is_some_and(|old| {
                    old.audio_sha256 != doc.audio_sha256
                        || old.has_recording() != doc.has_recording()
                        || old.duration != doc.duration
                });
                let transport = transport || timing_changed || recording_changed;
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
                let mut group = (self.group)();
                if group.track != doc.selected_track {
                    group = Default::default();
                } else if group.track == "Vocals" {
                    if self
                        .doc
                        .read()
                        .as_ref()
                        .is_none_or(|old| old.content.vocals != doc.content.vocals)
                    {
                        group.vocals.clear();
                    }
                } else {
                    group.ids.retain(|id| {
                        doc.content
                            .tracks
                            .get(&group.track)
                            .is_some_and(|notes| notes.iter().any(|n| &n.id == id))
                    });
                }
                if group != (self.group)() {
                    self.group.set(group);
                }
                self.file_stamp.set(project_stamp(&store));
                if !transport || timing_changed || recording_changed {
                    self.export_state
                        .set(crate::export_state::State::load(&store, &doc));
                }
                match doc.chart_hash(&store) {
                    Ok(hash) => self.chart_hash.set(hash),
                    Err(error) => {
                        self.error.set(Some(error.to_string()));
                        return;
                    }
                }
                let assets_key = stem_assets_key(&doc, doc.selected_track == "Vocals");
                let source_audio = if doc.has_recording() {
                    match store.audio_path(&doc) {
                        Ok(path) => Some(path),
                        Err(error) => {
                            self.error.set(Some(error.to_string()));
                            return;
                        }
                    }
                } else {
                    None
                };
                if *self.source_audio.peek() != source_audio {
                    self.source_audio.set(source_audio);
                }
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
                        audio.assets_key != assets_key || audio.source_hash != doc.audio_sha256
                    });
                let target = (doc.audio_sha256.clone(), 1., assets_key);
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
                if !replace_audio && let Some(audio) = self.audio.read().as_ref() {
                    let preview = self.analysis_preview.peek();
                    let active = preview.as_deref().unwrap_or(&doc);
                    if let Err(error) = self.apply_audio_mix(audio, active) {
                        self.error.set(Some(format!(
                            "Stem mix unavailable; original audio retained: {error:#}"
                        )));
                    }
                }
                if !replace_audio {
                    self.audio_ready.set(self.audio.read().is_some());
                    if let Some(audio) = self.audio.read().as_ref()
                        && let Err(error) = self.apply_audio_transport(audio, &doc)
                    {
                        self.error
                            .set(Some(format!("Audio transport unavailable: {error:#}")));
                    }
                }
                if transport || cancelled_preparation {
                    let keep_source_clock = doc
                        .audit
                        .last()
                        .is_some_and(|a| matches!(a.action, Action::SetPlaybackRate { .. }));
                    let seconds = if keep_source_clock {
                        self.current_audio_position().min(doc.duration)
                    } else if transport {
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
                        if !keep_source_clock && let Err(error) = audio.seek(seconds) {
                            self.error.set(Some(error.to_string()));
                        }
                        let ready = audio.ready_for_playback();
                        self.audio_ready.set(ready);
                        if doc.transport.playing && ready {
                            #[cfg(not(feature = "browser"))]
                            audio.mark_playback_started();
                            audio.player.play();
                        } else {
                            audio.player.pause();
                        }
                    }
                }
                self.doc.set(Some(Arc::new(doc)));
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
    fn apply_audio_mix(self, audio: &Audio, doc: &Project) -> Result<()> {
        let original = StemMix::default();
        let follow = (self.follow_analysis_sources)();
        let mix = if follow {
            &doc.content.stem_mix
        } else {
            &original
        };
        let route = crate::reanalysis::audition_route(doc, (self.guitar_source)(), follow);
        if route.is_some() {
            audio.set_routed_mix(mix, route)
        } else {
            audio.set_mix(mix)
        }
    }
    pub fn set_playback_rate(mut self, rate: f64) {
        if !rate.is_finite() || !(0.25..=2.).contains(&rate) {
            self.error
                .set(Some("Practice rate must be between 0.25 and 2.0".into()));
            return;
        }
        self.preview_playback_rate(Some(rate));
        if !self.dispatch_with_transport(
            Action::Seek {
                seconds: self
                    .audio
                    .read()
                    .as_ref()
                    .map_or((self.playhead)(), |audio| audio.raw_position()),
            },
            false,
        ) {
            self.preview_playback_rate(None);
            return;
        }
        if self.dispatch_with_transport(Action::SetPlaybackRate { rate }, true) {
            self.status
                .set(format!("Playback speed {:.0}%", rate * 100.));
        }
        self.preview_playback_rate(None);
    }
    fn effective_playback_rate(self, saved: f64) -> f64 {
        self.practice_rate_preview
            .peek()
            .as_ref()
            .filter(|(root, _)| {
                self.store.peek().as_ref().is_some_and(|store| {
                    self.doc
                        .peek()
                        .as_ref()
                        .is_some_and(|doc| store.root.join(&doc.audio_sha256) == *root)
                })
            })
            .map_or(saved, |(_, rate)| *rate)
    }
    fn apply_audio_transport(self, audio: &Audio, doc: &Project) -> Result<()> {
        audio.set_rate(self.effective_playback_rate(doc.transport.playback_rate))?;
        #[cfg(not(feature = "browser"))]
        audio.set_loop(doc.transport.loop_range, doc.transport.playing)?;
        Ok(())
    }
    /// Preview a drag without writing project history for every pointer event.
    pub fn preview_playback_rate(mut self, rate: Option<f64>) {
        if rate.is_some_and(|rate| !rate.is_finite() || !(0.25..=2.).contains(&rate)) {
            return;
        }
        let root = self.store.peek().as_ref().and_then(|store| {
            self.doc
                .peek()
                .as_ref()
                .map(|doc| store.root.join(&doc.audio_sha256))
        });
        self.practice_rate_preview.set(root.zip(rate));
        if let Some(doc) = self.doc.peek().as_ref()
            && let Some(audio) = self.audio.peek().as_ref()
            && let Err(error) =
                audio.set_rate(self.effective_playback_rate(doc.transport.playback_rate))
        {
            self.error
                .set(Some(format!("Audio transport unavailable: {error:#}")));
        }
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
        if !doc.has_recording() {
            self.audio_epoch.set((self.audio_epoch)() + 1);
            self.audio_target.set(None);
            self.waveforms.set(None);
            self.stem_waveforms.set(None);
            self.stem_waveform_path.set(None);
            #[cfg(not(feature = "browser"))]
            match Audio::timeline(
                doc.duration,
                doc.transport.playback_rate,
                stem_assets_key(&doc, self.is_vocal_track()),
            ) {
                Ok(audio) => {
                    if let Err(error) = audio.seek((self.playhead)().min(doc.duration)) {
                        self.error.set(Some(error.to_string()));
                        return;
                    }
                    if doc.transport.playing {
                        audio.player.play();
                    }
                    self.audio.set(Some(Rc::new(audio)));
                    self.audio_ready.set(true);
                    self.audio_stalled.set(false);
                    self.stem_status.set("Chart timeline · no recording".into());
                    self.status
                        .set("Chart-only project · attach a recording when ready".into());
                }
                Err(error) => {
                    self.audio_ready.set(false);
                    self.error.set(Some(error.to_string()));
                }
            }
            #[cfg(feature = "browser")]
            {
                self.audio_ready.set(false);
                self.error.set(Some(
                    "Chart-only playback requires the desktop app or static browser client".into(),
                ));
            }
            return;
        }
        let Some(path) = self.source_audio.read().clone() else {
            return;
        };
        let rate = doc.transport.playback_rate;
        let preparation_rate = 1.;
        let key = stem_assets_key(&doc, self.is_vocal_track());
        let expected = doc.audio_sha256.clone();
        let manifest = review_manifest(&doc, self.is_vocal_track()).cloned();
        let epoch = (self.audio_epoch)() + 1;
        self.audio_epoch.set(epoch);
        self.audio_ready.set(false);
        self.audio_target
            .set(Some((expected.clone(), preparation_rate, key.clone())));
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
                preparation_rate,
                cache_root.as_deref(),
            )?;
            if let Some(selected) = &project.kit_audition {
                let pcm = crate::part_analysis::prepare_kit_audition(
                    store.as_ref().context("No project for kit audition")?,
                    &project,
                    selected,
                    preparation_rate,
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
            || current
                .as_ref()
                .is_none_or(|doc| stem_assets_key(doc, self.is_vocal_track()) != key)
        {
            return;
        }
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
                let audio = Audio::new(
                    loaded,
                    kit_audio,
                    current.kit_audition.clone(),
                    current.transport.playback_rate,
                    key,
                    current.audio_sha256.clone(),
                    &mix,
                )
                .await;
                if (self.audio_epoch)() != epoch {
                    return;
                }
                self.audio_target.set(None);
                let current = self.snapshot().unwrap_or(current);
                match audio {
                    Ok(audio) => {
                        if let Err(error) = self.apply_audio_transport(&audio, &current) {
                            self.error
                                .set(Some(format!("Audio transport unavailable: {error:#}")));
                            return;
                        }
                        if let Err(error) = self
                            .apply_audio_mix(&audio, &current)
                            .and_then(|_| audio.seek((self.playhead)()))
                        {
                            self.error
                                .set(Some(format!("Audio transport unavailable: {error:#}")));
                            return;
                        }
                        audio.player.set_volume((self.volume)());
                        self.audio.set(Some(Rc::new(audio)));
                        self.waveforms.set(Some(waveforms));
                        self.stem_waveforms.set(stem_waveforms);
                        self.stem_waveform_path.set(stem_waveform_path);
                        let ready = self
                            .audio
                            .read()
                            .as_ref()
                            .is_some_and(|audio| audio.ready_for_playback());
                        self.audio_ready.set(ready);
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
                        self.status.set(if current.transport.playback_rate == 1.0 {
                            "Ready to play".into()
                        } else {
                            format!(
                                "Ready at {:.0}% speed",
                                current.transport.playback_rate * 100.0
                            )
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
                self.audio_target.set(None);
                self.kit_autoplay.set(None);
                self.kit_audition_busy.set(None);
                self.stem_status.set("Audio unavailable".into());
                self.error.set(Some(format!("{error:#}")));
            }
            Err(error) => {
                self.audio_target.set(None);
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
        // Opening a chart removes the welcome screen. Keep its load task owned
        // by the app so audio preparation and library cleanup can finish.
        dioxus::core::spawn_forever(async move {
            if let Some(path) = rfd::AsyncFileDialog::new()
                .set_title("Open a YACE project or charting folder (searches 3 levels)")
                .pick_folder()
                .await
            {
                #[cfg(feature = "browser")]
                self.open_project(path.path().to_owned()).await;
                #[cfg(not(feature = "browser"))]
                self.open_library(path.path().to_owned()).await;
            }
        });
    }
    pub fn new_chart(mut self) {
        spawn(async move {
            if let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Start a manual chart from a recording")
                .add_filter("Recording", &["ogg", "opus", "mp3", "flac", "wav", "m4a"])
                .pick_file()
                .await
            {
                let path = file.path().to_owned();
                let title = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect();
                self.error.set(None);
                self.new_song.set(Some((path, title)));
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
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Attach a recording to this chart")
                .add_filter(
                    "Recording",
                    &["ogg", "opus", "mp3", "flac", "wav", "m4a", "aiff", "aac"],
                )
                .pick_file()
                .await
            else {
                return;
            };
            let result=async{
                self.status.set("Retaining recording…".into());
                let input=file.path().to_owned();let root=store.root.clone();
                let prepared=tokio::task::spawn_blocking(move||crate::project_recording::prepare_recording(&input,&root)).await??;
                anyhow::ensure!(self.store.read().as_ref().is_some_and(|current|current.root==store.root)&&self.snapshot().is_some_and(|current|current.revision==doc.revision),"The chart changed while choosing audio; attach it again to the current revision");
                self.dispatch(Action::AttachRecording{prepared});Ok::<_,anyhow::Error>(())
            }.await;
            if let Err(error) = result {
                self.error.set(Some(error.to_string()));
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
        let Some(output) = rfd::AsyncFileDialog::new()
            .set_title("Save blank editing project in a new folder")
            .set_file_name(format!("{title} Review Project"))
            .save_file()
            .await
        else {
            anyhow::bail!("Save cancelled; choose a new folder or cancel this setup");
        };
        let output = output.path().to_owned();
        let store = tokio::task::spawn_blocking(move || -> Result<Store> {
            let source = tempfile::tempdir()?;
            std::fs::write(source.path().join("song.ini"), ini)?;
            std::fs::write(source.path().join("notes.chart"), chart)?;
            Store::create(source.path(), None, &output)
        })
        .await??;
        self.open_project(store.root).await;
        self.blank_chart.set(false);
        Ok(())
    }
    pub fn cancel_new_chart(mut self, _source: &std::path::Path) {
        self.new_song.set(None);
    }
    pub fn open_zip(self) {
        self.import_zip(true);
    }
    pub fn import_song(self) {
        self.import_zip(false);
    }
    fn import_zip(mut self, project: bool) {
        dioxus::core::spawn_forever(async move {
            let Some(input) = rfd::AsyncFileDialog::new()
                .set_title(if project {
                    "Open project ZIP"
                } else {
                    "Import song ZIP"
                })
                .add_filter("ZIP", &["zip"])
                .pick_file()
                .await
            else {
                return;
            };
            let dance = async {
                super::dance::flush().await?;
                let files = crate::archive_codec::decode(&std::fs::read(input.path())?)?
                    .into_iter()
                    .collect();
                super::dance::try_open_files(files)
            }
            .await;
            match dance {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    self.error.set(Some(format!("{error:#}")));
                    return;
                }
            }
            let Some(output) = rfd::AsyncFileDialog::new()
                .set_title("Save the imported editing project in a new folder")
                .set_file_name("Imported Review Project")
                .save_file()
                .await
            else {
                return;
            };
            let input = input.path().to_owned();
            let output = output.path().to_owned();
            let result: Result<Option<Store>> = async {
                if project {
                    return Ok(Some(
                        tokio::task::spawn_blocking(move || {
                            crate::project_files::import(&input, &output)
                        })
                        .await??,
                    ));
                }
                let source =
                    tokio::task::spawn_blocking(move || crate::project_files::unpack_song(&input))
                        .await??;
                let Some(choice) = self.choose_chart_source(source.path()).await? else {
                    return Ok(None);
                };
                Ok(Some(
                    tokio::task::spawn_blocking(move || {
                        Store::create_with_source(source.path(), None, &output, Some(choice))
                    })
                    .await??,
                ))
            }
            .await;
            match result {
                Ok(Some(store)) => self.open_project(store.root).await,
                Ok(None) => (),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    pub fn save_project(mut self) {
        let Some(store) = self.store.read().clone() else {
            return;
        };
        let title = self.metadata_value("name");
        spawn(async move {
            let Some(output) = rfd::AsyncFileDialog::new()
                .set_title("Save a portable project backup")
                .set_file_name(format!("{title} project.zip"))
                .add_filter("ZIP", &["zip"])
                .save_file()
                .await
            else {
                return;
            };
            if self
                .store
                .read()
                .as_ref()
                .is_none_or(|s| s.root != store.root)
            {
                self.error.set(Some(
                    "The active project changed; choose the backup again".into(),
                ));
                return;
            }
            let output = output.path().to_owned();
            let download = output.clone();
            match tokio::task::spawn_blocking(move || crate::project_files::backup(&store, &output))
                .await
            {
                Ok(Ok(())) => {
                    self.status.set("Project ZIP backup saved".into());
                    rfd::download(&download).await;
                }
                Ok(Err(error)) => self.error.set(Some(format!("{error:#}"))),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    pub async fn prepare_new_chart(
        self,
        source: PathBuf,
    ) -> Result<crate::song_import::RecordingInfo> {
        tokio::task::spawn_blocking(move || crate::song_import::local(&source)).await?
    }
    pub async fn lookup_new_chart(
        self,
        source: PathBuf,
        mut metadata: crate::pipeline::Metadata,
        mut info: crate::song_import::SongInfo,
        lyrics: bool,
    ) -> Result<crate::song_import::RecordingInfo> {
        tokio::task::spawn_blocking(move || {
            let cache = crate::song_import::artwork_cache(&source)?;
            crate::song_import::lookup(&mut metadata, &mut info, &cache, lyrics)?;
            Ok(crate::song_import::RecordingInfo { metadata, info })
        })
        .await?
    }
    pub fn stage_new_cover(
        self,
        source: &std::path::Path,
        cover: &std::path::Path,
    ) -> Result<PathBuf> {
        let bytes = crate::storage::read(cover)?;
        let extension = crate::song_metadata::cover_extension(&bytes)?;
        let path = crate::song_import::artwork_cache(source)?.join(format!(
            "selected-{}.{extension}",
            crate::storage::unique_id()
        ));
        crate::storage::write(&path, bytes)?;
        Ok(path)
    }
    pub async fn finish_new_chart(
        mut self,
        source: PathBuf,
        title: String,
        artist: String,
        timing: TempoMap,
        info: crate::song_import::SongInfo,
    ) -> Result<()> {
        let (ini, chart) = crate::song_metadata::blank_song_with_timing(&title, &artist, &timing)?;
        let Some(output) = rfd::AsyncFileDialog::new()
            .set_title("Save manual editing project in a new folder")
            .set_file_name(format!("{title} Review Project"))
            .save_file()
            .await
        else {
            anyhow::bail!("Save cancelled; choose a new project folder or cancel this setup");
        };
        let output = output.path().to_owned();
        let store = tokio::task::spawn_blocking(move || -> Result<Store> {
            let song = tempfile::tempdir()?;
            let extension = source
                .extension()
                .context("Recording has no file extension")?;
            std::fs::copy(
                &source,
                song.path()
                    .join(format!("song.{}", extension.to_string_lossy())),
            )?;
            std::fs::write(song.path().join("song.ini"), ini)?;
            std::fs::write(song.path().join("notes.chart"), chart)?;
            crate::song_metadata::write_song_info(song.path(), &info)?;
            Store::create(song.path(), None, &output)
        })
        .await??;
        self.open_project(store.root).await;
        // The setup component owns this async task. Keep it mounted until
        // project loading and audio preparation finish, then close it.
        self.new_song.set(None);
        Ok(())
    }
    #[cfg(not(feature = "browser"))]
    pub async fn open_library(mut self, path: PathBuf) {
        if (self.library_busy)() {
            return;
        }
        self.library_busy.set(true);
        self.status
            .set("Finding charts (up to 3 folders deep)…".into());
        let found =
            tokio::task::spawn_blocking(move || crate::chart_library::discover(&path)).await;
        self.library_busy.set(false);
        match found {
            Ok(Ok(library)) => {
                let single = library.entries.len() == 1;
                self.status.set(format!(
                    "Found {} charts · choose a chart to edit",
                    library.entries.len()
                ));
                self.library.set(Some(library));
                self.library_selected.set(None);
                if single {
                    self.open_library_chart(0).await;
                }
            }
            Ok(Err(error)) => self.error.set(Some(format!("{error:#}"))),
            Err(error) => self.error.set(Some(error.to_string())),
        }
    }
    #[cfg(not(feature = "browser"))]
    pub async fn open_library_chart(mut self, index: usize) {
        if let Err(error) = super::dance::flush().await {
            self.error.set(Some(format!("{error:#}")));
            return;
        }
        if (self.library_busy)() {
            return;
        }
        let Some(library) = self.library.read().clone() else {
            return;
        };
        let Some(item) = library.entries.get(index).cloned() else {
            return;
        };
        match super::dance::try_open_path(&item.path) {
            Ok(true) => {
                super::dance::remember_recent(self);
                if self.snapshot().is_some() {
                    self.dispatch(Action::Playback { playing: false });
                }
                self.library_selected.set(Some(index));
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                return;
            }
        }
        if self
            .snapshot()
            .is_some_and(|project| project.transport.playing)
        {
            self.dispatch(Action::Playback { playing: false });
        }
        self.library_busy.set(true);
        self.status.set(format!("Opening {}…", item.title));
        let source = if library
            .project_path(&item)
            .is_ok_and(|p| p.join("project.json").is_file())
        {
            None
        } else {
            match self.choose_chart_source(&item.path).await {
                Ok(Some(source)) => Some(source),
                Ok(None) => {
                    self.library_busy.set(false);
                    return;
                }
                Err(error) => {
                    self.library_busy.set(false);
                    self.error.set(Some(format!("{error:#}")));
                    return;
                }
            }
        };
        let opened =
            tokio::task::spawn_blocking(move || library.open_with_source(&item, source)).await;
        match opened {
            Ok(Ok(path)) => {
                let current = self
                    .store
                    .read()
                    .as_ref()
                    .is_some_and(|store| store.root == path);
                if !current {
                    self.open_project(path.clone()).await;
                }
                if self
                    .store
                    .read()
                    .as_ref()
                    .is_some_and(|store| store.root == path)
                {
                    self.library_selected.set(Some(index));
                }
            }
            Ok(Err(error)) => self.error.set(Some(format!("{error:#}"))),
            Err(error) => self.error.set(Some(error.to_string())),
        }
        self.library_busy.set(false);
    }
    pub fn create_dialog(self) {
        #[cfg(not(feature = "browser"))]
        {
            self.open_dialog();
        }
        #[cfg(feature = "browser")]
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
            let choice = match self.choose_chart_source(&song).await {
                Ok(Some(choice)) => choice,
                Ok(None) => return,
                Err(error) => {
                    let mut studio = self;
                    studio.error.set(Some(format!("{error:#}")));
                    return;
                }
            };
            let result = tokio::task::spawn_blocking(move || {
                Store::create_with_source(&song, None, &output, Some(choice))
            })
            .await;
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
        spawn(async move {
            let result = async {
                let doc = self.snapshot().context("Open a project")?;
                let store = self.store.read().clone().context("Open a project")?;
                anyhow::ensure!(
                    store.root == root && doc.revision == revision,
                    "The project changed; reopen Export"
                );
                options.selected(&store, &doc)?;
                let name = self.metadata_value("name");
                let name = if name.trim().is_empty() {
                    &doc.title
                } else {
                    &name
                };
                if let Some(path) = rfd::AsyncFileDialog::new()
                    .set_title("Export song into a new folder")
                    .set_file_name(format!("{name} reviewed"))
                    .save_file()
                    .await
                {
                    let output = path.path().to_owned();
                    let current = self.snapshot().context("Open a project")?;
                    anyhow::ensure!(
                        self.store.read().as_ref().is_some_and(|s| s.root == root)
                            && current.revision == revision,
                        "The project changed; reopen Export"
                    );
                    let actor = Actor {
                        kind: ActorKind::Human,
                        name: (self.reviewer)(),
                    };
                    let download = output.clone();
                    self.status.set("Preparing song export…".into());
                    let receipt_error = tokio::task::spawn_blocking(move || {
                        store.export_with_status(&doc, &output, &actor, &options)
                    })
                    .await??;
                    if self.store.peek().as_ref().is_some_and(|s| s.root == root) {
                        self.reload(false);
                        self.status.set(format!(
                            "Song r{revision} exported to {}",
                            download.display()
                        ));
                        if let Some(error) = receipt_error {
                            self.error.set(Some(format!(
                                "Song exported, but its history could not be saved: {error}"
                            )));
                        }
                    }
                    rfd::download(&download).await;
                    self.export_open.set(false);
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
            self.export_busy.set(false);
        });
    }
    pub async fn open_project(mut self, path: PathBuf) {
        self.analysis_preview.set(None);
        if let Err(error) = super::dance::flush().await {
            self.error.set(Some(format!("{error:#}")));
            return;
        }
        match super::dance::try_open_path(&path) {
            Ok(true) => {
                super::dance::remember_recent(self);
                if self.snapshot().is_some() {
                    self.dispatch(Action::Playback { playing: false });
                }
                return;
            }
            Ok(false) => {}
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                return;
            }
        }
        self.library_selected.set(None);
        // Command-line opens and generation recovery may point to a containing
        // folder rather than the saved project itself.
        #[cfg(not(feature = "browser"))]
        let path = if path.join("project.json").is_file() {
            path
        } else {
            let selected = path.clone();
            match tokio::task::spawn_blocking(move || crate::chart_library::discover(&selected))
                .await
            {
                Ok(Ok(library)) => {
                    if library.entries.len() != 1 {
                        self.library.set(Some(library));
                        self.library_selected.set(None);
                        self.status
                            .set("Choose a chart from the charting folder".into());
                        return;
                    }
                    match tokio::task::spawn_blocking(move || library.open(&library.entries[0]))
                        .await
                    {
                        Ok(Ok(project)) => project,
                        Ok(Err(error)) => {
                            self.error.set(Some(format!("{error:#}")));
                            return;
                        }
                        Err(error) => {
                            self.error.set(Some(error.to_string()));
                            return;
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.error.set(Some(format!("{error:#}")));
                    return;
                }
                Err(error) => {
                    self.error.set(Some(error.to_string()));
                    return;
                }
            }
        };
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
            let synth_reference = store.synth_reference(&doc);
            let timing = doc.timing(&store)?;
            let sections = store.source_sections(&doc)?;
            let sync = store.source_sync(&doc).ok();
            let star_power = store.source_star_power(&doc)?;
            let audio = doc
                .has_recording()
                .then(|| store.audio_path(&doc))
                .transpose()?;
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
                synth_reference,
                Arc::new(WindowLock(lock)),
                sections,
                sync,
                star_power,
            ))
        })
        .await;
        let (
            store,
            doc,
            timing,
            audio_path,
            metadata,
            art,
            guitar_source,
            synth_reference,
            lock,
            sections,
            sync,
            star_power,
        ) = match loaded {
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
            self.dispatch(Action::Playback { playing: false });
        }
        if let Some(old) = self.store.read().as_ref() {
            let _ = std::fs::remove_file(old.root.join("live-state.json"));
        }
        self.audio_epoch.set((self.audio_epoch)() + 1);
        self.audio.set(None);
        self.lock.set(Some(lock));
        let mut history = crate::recent_projects::History::load().unwrap_or_default();
        history.remember(
            store.root.clone(),
            crate::recent_projects::title(&metadata, &store.root),
        );
        let history_error = history.save().err();
        self.recent_projects.set(history);
        self.store.set(Some(store));
        self.library_selected.set(None);
        self.timing.set(Some(timing));
        self.view_start
            .set((doc.transport.current_position(doc.duration) - 0.5).max(0.0));
        self.playhead
            .set(doc.transport.current_position(doc.duration));
        self.group.set(Default::default());
        self.sections_open.set(false);
        self.source_sections.set(sections);
        self.source_sync.set(sync);
        self.source_star_power.set(star_power);
        self.star_power_open.set(false);
        self.star_power_target.set(None);
        self.timing_open.set(false);
        self.doc.set(Some(Arc::new(doc.clone())));
        super::dance::deactivate();
        super::dance::attach_existing(self);
        if let Some(store) = self.store.read().as_ref() {
            self.file_stamp.set(project_stamp(store));
        }
        if let Some(store) = self.store.read().as_ref() {
            self.chart_hash
                .set(doc.chart_hash(store).unwrap_or_default());
            self.export_state
                .set(crate::export_state::State::load(store, &doc));
        }
        self.metadata.set(metadata);
        self.guitar_source.set(guitar_source);
        self.synth_reference.set(synth_reference);
        self.album_art.set(art);
        self.source_audio.set(audio_path);
        self.waveforms.set(None);
        self.stem_waveforms.set(None);
        self.stem_waveform_path.set(None);
        self.audio_ready.set(false);
        self.error.set(None);
        self.status.set("Loading recording…".into());
        self.prepare_practice_audio().await;
        if let Some(error) = history_error {
            self.error.set(Some(format!(
                "Project opened, but recent history could not be saved: {error:#}"
            )));
        }
    }
    pub fn tick(mut self, publish: bool) {
        #[cfg(not(feature = "browser"))]
        if let Some(audio) = self.audio.read().as_ref() {
            if let Some(error) = audio.processing_error() {
                audio.player.pause();
                self.audio_ready.set(false);
                self.error
                    .set(Some(format!("Audio processing stopped: {error}")));
                return;
            }
            if !audio.ready_for_playback() {
                self.audio_ready.set(false);
                audio.player.pause();
                return;
            }
            if !(self.audio_ready)() {
                self.audio_ready.set(true);
                if self.snapshot().is_some_and(|doc| doc.transport.playing) {
                    audio.mark_playback_started();
                    audio.player.play();
                }
            }
        }
        if let (Some(doc), Some(audio)) = (self.snapshot(), self.audio.peek().as_ref()) {
            let _ = self.apply_audio_mix(audio, &doc);
            #[cfg(feature = "browser")]
            {
                audio.synchronize();
                if let Some(error) = audio.take_error() {
                    self.error.set(Some(error));
                }
            }
        }
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
            let stalled = transport.playing && audio.is_stalled();
            if *self.audio_stalled.peek() != stalled {
                self.audio_stalled.set(stalled);
            }
            if transport.playing {
                if let Some(range) = transport.loop_range {
                    // A visual interpolation must never cut the recording's
                    // loop short. Only the actual audio counter wraps transport.
                    #[cfg(feature = "browser")]
                    let raw = audio.raw_position();
                    #[cfg(not(feature = "browser"))]
                    let raw = audio.raw_position();
                    #[cfg(feature = "browser")]
                    let streamed_loop = false;
                    #[cfg(not(feature = "browser"))]
                    let streamed_loop = audio.stream.borrow().is_some();
                    if !streamed_loop && raw >= range.end {
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
                        | Action::Pause { .. }
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
            #[cfg(not(feature="browser"))]
            let Some(buffer)=audio.buffer.as_ref() else {return Some(serde_json::json!({"mode":"timeline","active_gains":{},"source_audio_sha256":null,"playback_rate":audio.playback_rate(),"player_paused":audio.player.is_paused(),"stems_ready":false,"sample_rate":null,"shared_cursor":true,"output_config":audio.output_config}));};
            #[cfg(feature="browser")]
            let buffer=&audio.buffer;
            buffer.mix.read().ok().map(|mix| {
                let identity = serde_json::to_vec(&(
                    audio.source_hash.as_str(),
                    audio.assets_key.as_str(),
                    1.,
                ))
                .unwrap_or_default();
                #[cfg(not(feature="browser"))]
                let processing = audio.processing_diagnostics();
                #[cfg(feature="browser")]
                let processing = serde_json::Value::Null;
                serde_json::json!({
                    "mode":if audio.kit_audition.is_some(){"kit"}else if mix.original{"original"}else{"stems"},
                    "active_gains":if mix.original || audio.kit_audition.is_some(){BTreeMap::new()}else{mix.gains.clone()},
                    "kit_audition":audio.kit_audition,
                    "source_audio_sha256":audio.source_hash,
                    "assets_identity_sha256":format!("{:x}",Sha256::digest(identity)),
                    "playback_rate":audio.playback_rate(),
                    "processing":processing,
                    "player_paused":audio.player.is_paused(),
                    "stems_ready":ready && !buffer.loaded.stems.is_empty(),
                    "sample_rate":buffer.loaded.sample_rate,
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
        analysis_preview: use_signal(|| None),
        follow_analysis_sources: use_signal(|| true),
        recent_projects: use_signal(|| crate::recent_projects::History::load().unwrap_or_default()),
        doc: use_signal(|| None),
        store: use_signal(|| None),
        library: use_signal(|| None),
        library_busy: use_signal(|| false),
        library_selected: use_signal(|| None),
        source_choice: use_signal(|| None),
        timing: use_signal(|| None),
        playhead: use_signal(|| 0.0),
        audio_ready: use_signal(|| false),
        audio_stalled: use_signal(|| false),
        kit_audition_busy: use_signal(|| None),
        status: use_signal(|| "Ready".into()),
        error: use_signal(|| None),
        clean_open: use_signal(|| false),
        generation_open: use_signal(|| false),
        new_song: use_signal(|| None),
        blank_chart: use_signal(|| false),
        clean_ranges: use_signal(Vec::new),
        review_open: use_signal(|| false),
        export_open: use_signal(|| false),
        export_busy: use_signal(|| false),
        export_state: use_signal(Default::default),
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
        vocal_note: use_signal(|| None),
        group: use_signal(Default::default),
        selection_dragging: use_signal(|| false),
        editor_focus: use_signal(|| None),
        #[cfg(not(feature = "browser"))]
        clipboard: use_signal(|| None),
        metadata: use_signal(BTreeMap::new),
        chart_hash: use_signal(String::new),
        stem_status: use_signal(|| "Original recording".into()),
        stem_waveforms: use_signal(|| None),
        stem_waveform_path: use_signal(|| None),
        audio: use_signal(|| None),
        practice_rate_preview: use_signal(|| None),
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
    if !doc.has_recording() {
        return serde_json::to_string(&("timeline", doc.duration)).unwrap_or_default();
    }
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
    if rate == 1.0 || cfg!(not(feature = "browser")) {
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
    use crate::stem_review::StemId;
    #[cfg(not(feature = "browser"))]
    use crate::stem_review::{MixMode, StemChannel};
    #[cfg(not(feature = "browser"))]
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
    #[cfg(not(feature = "browser"))]
    fn streaming_rate_and_end_restart_keep_cached_pcm_device_and_play_state() {
        let loaded = Arc::new(
            LoadedStems::from_pcm(
                44100,
                Arc::new(vec![0.1; 44100 * 4]),
                BTreeMap::from([
                    (StemId::Guitar, Arc::new(vec![0.2; 44100 * 4])),
                    (StemId::Other, Arc::new(vec![-0.2; 44100 * 4])),
                ]),
            )
            .unwrap(),
        );
        let buffer = ReviewSource {
            loaded: loaded.clone(),
            kit_audio: None,
            mix: Arc::new(RwLock::new(StemMix::default().prepare([]).unwrap())),
            sample: 0,
            frame: [0.; 2],
        };
        let (stream, source) = buffer.stream(1.).unwrap();
        stream.wait_ready(Duration::from_secs(5)).unwrap();
        let (player, mut output) = rodio::Player::new();
        player.pause();
        player.append(source);
        let audio = Audio {
            _device: None,
            player: super::super::timeline_player::Player::recorded(player),
            buffer: Some(buffer),
            rate: Cell::new(1.),
            stream: RefCell::new(Some(stream)),
            output_observation: Cell::new((0, Instant::now())),
            assets_key: "retained-assets".into(),
            source_hash: "a".repeat(64),
            kit_audition: None,
            output_config: AudioOutputConfig {
                requested_buffer_frames: 512,
                configured_buffer_frames: None,
                sample_rate: 44100,
                nominal_buffer_ms: None,
                fallback_reason: None,
                hardware_output_latency_known: false,
            },
            clock: RefCell::new(PresentationClock::default()),
            epoch: Instant::now(),
        };
        for rate in [0.25, 0.5, 0.83, 1., 1.5, 2.] {
            audio
                .set_routed_mix(
                    &StemMix::default(),
                    Some(crate::reanalysis::AuditionRoute {
                        intervals: vec![crate::reanalysis::SourceInterval {
                            range: crate::editor::TimeRange {
                                start: 0.75,
                                end: 1.25,
                            },
                            source: crate::mapping::GuitarSource::Other,
                        }],
                        fallback: crate::mapping::GuitarSource::Guitar,
                        playback_rate: rate,
                    }),
                )
                .unwrap();
            let prepared = audio.buffer.as_ref().unwrap().mix.read().unwrap();
            assert_eq!(loaded.frame_with_mix(44100 / 2, &prepared), [0.2; 2]);
            assert_eq!(
                loaded.frame_with_mix(44100, &prepared),
                [-0.2; 2],
                "Source routing moved to a different recording interval at rate {rate}"
            );
            assert_eq!(loaded.frame_with_mix(44100 * 3 / 2, &prepared), [0.2; 2]);
        }
        audio.set_mix(&StemMix::default()).unwrap();
        audio.set_rate(0.25).unwrap();
        assert!(audio.player.is_paused());
        audio
            .stream
            .borrow()
            .as_ref()
            .unwrap()
            .wait_ready(Duration::from_secs(5))
            .unwrap();
        assert_eq!(audio.position(), 0.);
        audio.mark_playback_started();
        audio.player.play();
        audio.set_rate(0.83).unwrap();
        assert!(!audio.player.is_paused());
        assert!(
            audio.ready_for_playback(),
            "Playing rate change discarded buffered audio"
        );
        assert!(Arc::ptr_eq(&loaded, &audio.buffer.as_ref().unwrap().loaded));
        assert_eq!(audio.assets_key, "retained-assets");
        assert_eq!(audio.source_hash, "a".repeat(64));
        assert_eq!(
            audio.position(),
            0.,
            "Rendering changed consumed source position"
        );
        for _ in 0..1024 {
            output.next().unwrap();
        }
        audio.player.pause();
        audio
            .set_loop(
                Some(crate::editor::TimeRange {
                    start: 0.5,
                    end: 1.5,
                }),
                false,
            )
            .unwrap();
        audio.seek(0.).unwrap();
        audio
            .stream
            .borrow()
            .as_ref()
            .unwrap()
            .wait_ready(Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            audio.position(),
            0.,
            "Paused rewind was trapped inside the loop"
        );
        audio.seek(2.).unwrap();
        audio
            .stream
            .borrow()
            .as_ref()
            .unwrap()
            .wait_ready(Duration::from_secs(5))
            .unwrap();
        audio.player.play();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !audio.player.empty() {
            for _ in 0..512 {
                output.next();
            }
            std::thread::sleep(Duration::from_millis(1));
            assert!(Instant::now() < deadline, "End failed to drain");
        }
        assert_eq!(audio.position(), 2.);
        audio.seek(0.).unwrap();
        audio
            .stream
            .borrow()
            .as_ref()
            .unwrap()
            .wait_ready(Duration::from_secs(5))
            .unwrap();
        assert_eq!(audio.position(), 0.);
        assert!(Arc::ptr_eq(&loaded, &audio.buffer.as_ref().unwrap().loaded));
        drop(output);
    }
    #[test]
    #[cfg(not(feature = "browser"))]
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
    #[cfg(not(feature = "browser"))]
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
        assert!(cancels_kit_autoplay(&Action::Pause { seconds: 1.25 }));
        assert!(!cancels_kit_autoplay(&Action::SetPlaybackRate {
            rate: 0.5
        }));
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
