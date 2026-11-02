//! Browser audio uses one Web Audio clock; no audio plays on the Rust host.
use super::*;
use crate::browser::BrowserContext;
use std::{
    cell::Cell,
    io::{BufWriter, Write},
    path::Path,
};

#[derive(Default, serde::Deserialize)]
struct Position {
    #[serde(default)]
    position: f64,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    ended: bool,
    #[serde(default)]
    context: String,
    #[serde(default)]
    channels: Vec<ChannelPosition>,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Default, serde::Deserialize)]
struct ChannelPosition {
    position: f64,
    ready: bool,
}
#[derive(Clone)]
pub(super) struct Player {
    state: Rc<RefCell<Position>>,
    id: String,
}
impl Player {
    fn command(&self, method: &str, value: Option<f64>) {
        let argument = value.map(|v| v.to_string()).unwrap_or_default();
        let id = serde_json::to_string(&self.id).unwrap();
        let _ = dioxus_document::eval(&format!(
            "if(window.__yaceAudio?.id==={id})window.__yaceAudio.{method}({argument});"
        ));
    }
    pub(super) fn pause(&self) {
        self.command("pause", None);
        self.state.borrow_mut().paused = true;
    }
    pub(super) fn play(&self) {
        self.command("play", None);
        self.state.borrow_mut().paused = false;
    }
    pub(super) fn set_volume(&self, volume: f32) {
        self.command("volume", Some(volume as f64));
    }
    pub(super) fn empty(&self) -> bool {
        self.state.borrow().ended
    }
    pub(super) fn is_paused(&self) -> bool {
        self.state.borrow().paused
    }
}
pub(super) struct Audio {
    pub(super) player: Player,
    pub(super) buffer: BrowserBuffer,
    rate: Cell<f64>,
    pub(super) assets_key: String,
    pub(super) source_hash: String,
    pub(super) kit_audition: Option<crate::part_analysis::KitAudition>,
    pub(super) output_config: AudioOutputConfig,
    pub(super) clock: RefCell<PresentationClock>,
    epoch: Instant,
    _files: tempfile::TempDir,
    keys: Vec<Option<StemId>>,
    last_mix: RefCell<Vec<f32>>,
    disposed: Rc<Cell<bool>>,
    registry: Arc<RwLock<BTreeMap<String, std::path::PathBuf>>>,
    asset_ids: Vec<String>,
    followers: RefCell<Vec<crate::media_clock::Follower>>,
}
pub(super) struct BrowserBuffer {
    pub(super) loaded: AudioAssets,
    pub(super) mix: Arc<RwLock<PreparedMix>>,
}
pub(super) struct AudioAssets {
    pub(super) sample_rate: u32,
    pub(super) stems: BTreeMap<StemId, ()>,
}
fn write_pcm(path: &Path, pcm: &[f32], sample_rate: u32) -> Result<()> {
    let bytes = u32::try_from(pcm.len().checked_mul(4).context("Audio size overflow")?)?;
    anyhow::ensure!(bytes <= u32::MAX - 36, "Audio exceeds WAV limit");
    let mut file = BufWriter::with_capacity(256 * 1024, std::fs::File::create(path)?);
    file.write_all(b"RIFF")?;
    file.write_all(&(bytes + 36).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&3u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 8).to_le_bytes())?;
    file.write_all(&8u16.to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&bytes.to_le_bytes())?;
    for chunk in pcm.chunks(16384) {
        let bytes: Vec<_> = chunk
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        file.write_all(&bytes)?;
    }
    file.flush()?;
    Ok(())
}
impl Audio {
    pub(super) fn ready_for_playback(&self) -> bool {
        true
    }
    pub(super) fn is_stalled(&self) -> bool {
        self.clock.borrow().is_stale()
    }
    pub(super) fn playback_rate(&self) -> f64 {
        self.rate.get()
    }
    pub(super) fn set_rate(&self, rate: f64) -> Result<()> {
        anyhow::ensure!(
            rate.is_finite() && (0.25..=2.).contains(&rate),
            "Practice rate must be between 0.25 and 2.0"
        );
        if self.rate.replace(rate) != rate {
            for follower in self.followers.borrow_mut().iter_mut() {
                follower.reset();
            }
            self.player.command("rate", Some(rate));
            self.clock
                .borrow_mut()
                .reset(self.epoch.elapsed().as_secs_f64(), self.raw_position());
        }
        Ok(())
    }
    pub(super) fn raw_position(&self) -> f64 {
        self.player.state.borrow().position
    }
    pub(super) fn synchronize(&self) {
        let state = self.player.state.borrow();
        let now = self.epoch.elapsed().as_secs_f64();
        let running = !state.paused && state.context == "running";
        let rate = self.rate.get();
        let mut followers = self.followers.borrow_mut();
        let mut commands = Vec::new();
        for (index, channel) in state.channels.iter().enumerate().skip(1) {
            if let Some(follower) = followers.get_mut(index) {
                match follower.update(
                    now,
                    state.position,
                    channel.position,
                    rate,
                    running && channel.ready,
                ) {
                    crate::media_clock::Correction::None => {}
                    crate::media_clock::Correction::Rate(value) => {
                        commands.push(serde_json::json!({"index":index,"rate":value}))
                    }
                    crate::media_clock::Correction::Seek(seconds) => {
                        commands.push(serde_json::json!({"index":index,"rate":rate,"seek":seconds}))
                    }
                }
            }
        }
        if !commands.is_empty() {
            let id = serde_json::to_string(&self.player.id).unwrap();
            let _ = dioxus_document::eval(&format!(
                "if(window.__yaceAudio?.id==={id})window.__yaceAudio.synchronize({});",
                serde_json::to_string(&commands).unwrap()
            ));
        }
    }
    pub(super) fn take_error(&self) -> Option<String> {
        self.player.state.borrow_mut().error.take()
    }
    pub(super) fn set_routed_mix(
        &self,
        mix: &StemMix,
        route: Option<crate::reanalysis::AuditionRoute>,
    ) -> Result<()> {
        let mut mix = if self.buffer.loaded.stems.is_empty() {
            StemMix::default()
        } else {
            mix.clone()
        };
        mix.channels
            .retain(|id, _| self.buffer.loaded.stems.contains_key(id));
        let mut prepared = mix.prepare(self.buffer.loaded.stems.keys().copied())?;
        if let Some(route) = &route {
            prepared.original = false;
            for gain in prepared.gains.values_mut() {
                *gain = 0.;
            }
            for (id, gain) in route.weights(self.position()) {
                if self.buffer.loaded.stems.contains_key(&id) {
                    *prepared.gains.entry(id).or_default() += gain;
                } else {
                    prepared.original = true;
                }
            }
        }
        prepared.route = route.map(|mut route| {
            route.playback_rate = 1.;
            route
        });
        let values = weights(
            &prepared,
            &self.keys,
            self.kit_audition.is_some(),
            self.position(),
        );
        if *self.last_mix.borrow() != values {
            let _ = dioxus_document::eval(&format!(
                "window.__yaceAudio?.weights({});",
                serde_json::to_string(&values)?
            ));
            *self.last_mix.borrow_mut() = values;
        }
        *self
            .buffer
            .mix
            .write()
            .map_err(|_| anyhow::anyhow!("Audio mix lock poisoned"))? = prepared;
        Ok(())
    }
    pub(super) async fn new(
        loaded: LoadedStems,
        kit_audio: Option<Arc<Vec<f32>>>,
        kit_audition: Option<crate::part_analysis::KitAudition>,
        rate: f64,
        assets_key: String,
        source_hash: String,
        mix: &StemMix,
    ) -> Result<Self> {
        let context = consume_context::<BrowserContext>();
        let registry = context.assets.clone();
        let prepared = mix.prepare(loaded.stems.keys().copied())?;
        let mut sources = vec![(
            None,
            kit_audio.clone().unwrap_or_else(|| loaded.original.clone()),
        )];
        if kit_audio.is_none() {
            sources.extend(
                loaded
                    .stems
                    .iter()
                    .map(|(id, pcm)| (Some(*id), pcm.clone())),
            );
        }
        let keys: Vec<_> = sources.iter().map(|(id, _)| *id).collect();
        let weights = weights(&prepared, &keys, kit_audio.is_some(), 0.);
        let metadata = AudioAssets {
            sample_rate: loaded.sample_rate,
            stems: loaded.stems.keys().map(|id| (*id, ())).collect(),
        };
        let sample_rate = loaded.sample_rate;
        drop(loaded);
        drop(kit_audio);
        // PCM is written once off the UI thread; HTTP streams it to the client.
        let (_files, urls) = tokio::task::spawn_blocking(move || -> Result<_> {
            let files = tempfile::tempdir()?;
            let mut urls = Vec::new();
            for (index, (_, pcm)) in sources.iter().enumerate() {
                let path = files.path().join(format!("{index}.wav"));
                write_pcm(&path, pcm, sample_rate)?;
                urls.push(context.register(&path)?);
            }
            Ok((files, urls))
        })
        .await??;
        let mut probe = dioxus_document::eval(include_str!("../../assets/browser-audio.js"));
        let id = urls[0].clone();
        let asset_ids = urls
            .iter()
            .filter_map(|url| url.strip_prefix("/asset/").map(str::to_owned))
            .collect();
        probe.send(serde_json::json!({"id":id,"urls":urls,"weights":weights,"rate":rate}))?;
        let mut ready = serde_json::Value::Null;
        while ready.get("ready").is_none() {
            ready = probe.recv::<serde_json::Value>().await?;
        }
        anyhow::ensure!(ready["ready"] == true, "Browser audio: {}", ready["error"]);
        let player = Player {
            state: Rc::new(RefCell::new(Position {
                paused: true,
                ..Default::default()
            })),
            id,
        };
        let player_state = player.state.clone();
        let disposed = Rc::new(Cell::new(false));
        let stop = disposed.clone();
        spawn(async move {
            while !stop.get() {
                let Ok(next) = probe.recv::<Position>().await else {
                    break;
                };
                *player_state.borrow_mut() = next;
            }
        });
        let output_config = AudioOutputConfig {
            requested_buffer_frames: 0,
            configured_buffer_frames: None,
            sample_rate: ready["sample_rate"].as_u64().unwrap_or(0) as u32,
            nominal_buffer_ms: None,
            fallback_reason: None,
            hardware_output_latency_known: false,
        };
        Ok(Self {
            player,
            buffer: BrowserBuffer {
                loaded: metadata,
                mix: Arc::new(RwLock::new(prepared)),
            },
            rate: Cell::new(rate),
            assets_key,
            source_hash,
            kit_audition,
            output_config,
            clock: RefCell::new(PresentationClock::default()),
            epoch: Instant::now(),
            _files,
            followers: RefCell::new(keys.iter().map(|_| Default::default()).collect()),
            keys,
            last_mix: RefCell::new(weights),
            disposed,
            registry,
            asset_ids,
        })
    }
    pub(super) fn set_mix(&self, mix: &StemMix) -> Result<()> {
        self.set_routed_mix(mix, None)
    }
    pub(super) fn seek(&self, seconds: f64) -> Result<()> {
        self.player.command("seek", Some(seconds.max(0.0)));
        self.player.state.borrow_mut().position = seconds.max(0.0);
        for follower in self.followers.borrow_mut().iter_mut() {
            follower.reset();
        }
        self.clock
            .borrow_mut()
            .reset(self.epoch.elapsed().as_secs_f64(), seconds.max(0.0));
        Ok(())
    }
    pub(super) fn position(&self) -> f64 {
        let state = self.player.state.borrow();
        let raw = state.position;
        if state.paused || state.context != "running" {
            return raw;
        }
        self.clock
            .borrow_mut()
            .sample(self.epoch.elapsed().as_secs_f64(), raw, self.rate.get())
    }
}
fn weights(mix: &PreparedMix, keys: &[Option<StemId>], kit: bool, seconds: f64) -> Vec<f32> {
    if !kit && let Some(route) = &mix.route {
        let mut values = vec![0.; keys.len()];
        for (id, gain) in route.weights(seconds) {
            if let Some(index) = keys
                .iter()
                .position(|key| *key == Some(id))
                .or_else(|| keys.iter().position(Option::is_none))
            {
                values[index] += gain;
            }
        }
        return values;
    }
    keys.iter()
        .map(|id| match id {
            None => f32::from(mix.original || kit),
            Some(id) => {
                if mix.original {
                    0.
                } else {
                    mix.gains.get(id).copied().unwrap_or(0.)
                }
            }
        })
        .collect()
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.disposed.set(true);
        self.player.command("dispose", None);
        if let Ok(mut assets) = self.registry.write() {
            for id in &self.asset_ids {
                assets.remove(id);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routed_missing_stem_crossfade_weights_the_original_without_doubling_it() {
        let mut mix = StemMix::default().prepare([StemId::Guitar]).unwrap();
        mix.route = Some(crate::reanalysis::AuditionRoute {
            intervals: vec![crate::reanalysis::SourceInterval {
                range: crate::editor::TimeRange {
                    start: 0.75,
                    end: 1.25,
                },
                source: crate::mapping::GuitarSource::Other,
            }],
            fallback: crate::mapping::GuitarSource::Guitar,
            playback_rate: 0.25,
        });
        let keys = [None, Some(StemId::Guitar)];
        let crossfade = weights(&mix, &keys, false, 0.755);
        assert!((crossfade[0] - 0.5).abs() < 1e-5);
        assert!((crossfade[1] - 0.5).abs() < 1e-5);
        assert_eq!(weights(&mix, &keys, false, 1.), [1., 0.]);
        assert_eq!(weights(&mix, &keys, true, 1.), [1., 0.]);
    }
    #[test]
    fn float_wav_preserves_channels_and_samples() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        write_pcm(&path, &[0.25, -0.5, 1., -1.], 44100).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[20..22], &3u16.to_le_bytes());
        assert_eq!(bytes.len(), 60);
        assert_eq!(&bytes[44..48], &0.25f32.to_le_bytes());
        assert_eq!(&bytes[48..52], &(-0.5f32).to_le_bytes());
    }
}
