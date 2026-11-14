//! Browser codecs and pitch-preserving media playback, driven from Rust.
use crate::{
    stem_review::{MixMode, StemId, StemMix},
    waveform::Waveform,
};
use anyhow::{Result, ensure};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::Path,
    rc::Rc,
    sync::Arc,
};
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, GainNode, HtmlAudioElement, MediaElementAudioSourceNode};
pub struct Channel {
    pub element: HtmlAudioElement,
    pub gain: GainNode,
    node: MediaElementAudioSourceNode,
    _url: ObjectUrl,
    clock: RefCell<crate::media_clock::Follower>,
    activity: RefCell<crate::playback_activity::Gain>,
    play_intent: Rc<Cell<u64>>,
}
struct ObjectUrl(String);
impl Drop for ObjectUrl {
    fn drop(&mut self) {
        let _ = web_sys::Url::revoke_object_url(&self.0);
    }
}
struct PendingContext(Option<AudioContext>);
impl Drop for PendingContext {
    fn drop(&mut self) {
        if let Some(context) = &self.0 {
            let _ = context.close();
        }
    }
}
fn ramp_gain(node: &GainNode, value: f32, now: f64) {
    let param = node.gain();
    let _ = param.cancel_scheduled_values(now);
    let _ = param.set_value_at_time(param.value(), now);
    let _ = param.linear_ramp_to_value_at_time(value, now + crate::playback_activity::FADE_SECONDS);
}
impl Channel {
    fn set_gain(&self, value: f32, now: f64) {
        if self.activity.borrow_mut().update(value, now) {
            ramp_gain(&self.gain, value, now);
        }
    }
    fn pause(&self) {
        self.play_intent.set(self.play_intent.get().wrapping_add(1));
        let _ = self.element.pause();
    }
}
struct RecordingMedia {
    identity: MediaIdentity,
    context: AudioContext,
    pub original: Channel,
    stems: BTreeMap<(bool, StemId), Channel>,
    waveforms: [super::state::ReviewWaveforms; 2],
    play_epoch: Rc<Cell<u64>>,
    error: Rc<RefCell<Option<String>>>,
    kit: RefCell<Option<Channel>>,
    master: GainNode,
    rate: Cell<f64>,
    audition: Cell<bool>,
    volume: Cell<f32>,
}
pub struct Media(
    Playback,
    Rc<RefCell<Option<super::click_audio::ClickAudio>>>,
    Rc<RefCell<Option<String>>>,
    Vec<ClickListener>,
);
struct ClickListener {
    element: HtmlAudioElement,
    event: &'static str,
    callback: Closure<dyn FnMut(web_sys::Event)>,
}
impl Drop for ClickListener {
    fn drop(&mut self) {
        let _ = self.element.remove_event_listener_with_callback(
            self.event,
            self.callback.as_ref().unchecked_ref(),
        );
    }
}
type ClickState = Rc<RefCell<Option<super::click_audio::ClickAudio>>>;
fn click_listeners(
    element: &HtmlAudioElement,
    clicks: &ClickState,
    warning: &Rc<RefCell<Option<String>>>,
) -> Result<Vec<ClickListener>> {
    let mut listeners = Vec::new();
    for event in ["playing", "seeked", "ratechange"] {
        let source = element.clone();
        let weak = Rc::downgrade(clicks);
        let warning = Rc::downgrade(warning);
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let mut state = state.borrow_mut();
            let Some(clicks) = state.as_mut() else {
                return;
            };
            let running = !source.paused() && !source.seeking() && source.ready_state() >= 3;
            if let Err(error) =
                clicks.synchronize(source.current_time(), source.playback_rate(), running)
            {
                clicks.set_mix(None);
                if let Some(warning) = warning.upgrade() {
                    warning.replace(Some(format!(
                        "Click audition stopped; recording playback continues: {error:#}"
                    )));
                }
            }
        });
        element
            .add_event_listener_with_callback(event, callback.as_ref().unchecked_ref())
            .map_err(err)?;
        listeners.push(ClickListener {
            element: element.clone(),
            event,
            callback,
        });
    }
    Ok(listeners)
}
impl Drop for Media {
    fn drop(&mut self) {
        self.3.clear();
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.cancel();
        }
    }
}
enum Playback {
    Recording(Box<RecordingMedia>),
    Timeline(RefCell<crate::timeline_clock::TimelineClock>),
}
fn monotonic_time() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0., |performance| performance.now() / 1000.)
}
fn empty_waveforms() -> super::state::ReviewWaveforms {
    super::state::ReviewWaveforms {
        original: Arc::new(Waveform::from_stereo(&[], 44100)),
        stems: BTreeMap::new(),
    }
}
impl Media {
    pub async fn load(
        store: &crate::editor::Store,
        doc: &crate::editor::Project,
    ) -> Result<(Self, super::state::ReviewWaveforms, String)> {
        if doc.has_recording() {
            let (media, waves, notice) = RecordingMedia::load(store, doc).await?;
            let clicks = super::click_audio::ClickAudio::new(media.context.clone(), false)?;
            let clicks = Rc::new(RefCell::new(Some(clicks)));
            let warning = Rc::new(RefCell::new(None));
            let listeners = click_listeners(&media.original.element, &clicks, &warning)?;
            Ok((
                Self(
                    Playback::Recording(Box::new(media)),
                    clicks,
                    warning,
                    listeners,
                ),
                waves,
                notice,
            ))
        } else {
            ensure!(
                web_sys::window()
                    .and_then(|window| window.performance())
                    .is_some(),
                "Browser monotonic clock is unavailable"
            );
            let now = monotonic_time();
            let mut clock = crate::timeline_clock::TimelineClock::new(doc.duration, now)?;
            clock.set_rate(doc.transport.playback_rate, now)?;
            clock.seek(doc.transport.position, now)?;
            Ok((
                Self(
                    Playback::Timeline(RefCell::new(clock)),
                    Rc::new(RefCell::new(None)),
                    Rc::new(RefCell::new(None)),
                    Vec::new(),
                ),
                empty_waveforms(),
                "Chart timeline · no recording".into(),
            ))
        }
    }
    pub fn position(&self) -> f64 {
        match &self.0 {
            Playback::Recording(media) => media.position(),
            Playback::Timeline(clock) => clock.borrow().raw_position(monotonic_time()),
        }
    }
    pub fn matches(&self, doc: &crate::editor::Project) -> bool {
        match &self.0 {
            Playback::Recording(media) => doc.has_recording() && media.matches(doc),
            Playback::Timeline(clock) => {
                !doc.has_recording() && clock.borrow().duration() == doc.duration
            }
        }
    }
    pub fn replace_kit(
        &self,
        store: &crate::editor::Store,
        doc: &crate::editor::Project,
    ) -> Result<()> {
        match &self.0 {
            Playback::Recording(media) => media.replace_kit(store, doc),
            Playback::Timeline(_) => anyhow::bail!("Attach a recording before using kit audition"),
        }
    }
    pub fn paused(&self) -> bool {
        match &self.0 {
            Playback::Recording(media) => media.paused(),
            Playback::Timeline(clock) => clock.borrow().paused(),
        }
    }
    pub fn ended(&self) -> bool {
        match &self.0 {
            Playback::Recording(media) => media.ended(),
            Playback::Timeline(clock) => clock.borrow().ended(monotonic_time()),
        }
    }
    pub fn stalled(&self) -> bool {
        match &self.0 {
            Playback::Recording(media) => {
                !media.ended() && media.original.element.ready_state() < 3
            }
            Playback::Timeline(_) => false,
        }
    }
    pub fn play(&self) -> Result<()> {
        if let Some(clicks) = self.1.borrow().as_ref() {
            clicks.resume();
        }
        let result = match &self.0 {
            Playback::Recording(media) => media.play(),
            Playback::Timeline(clock) => {
                clock.borrow_mut().set_playing(true, monotonic_time())?;
                Ok(())
            }
        };
        if result.is_ok() {
            self.synchronize();
        }
        result
    }
    pub fn pause(&self) {
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.cancel();
        }
        match &self.0 {
            Playback::Recording(media) => media.pause(),
            Playback::Timeline(clock) => {
                let _ = clock.borrow_mut().set_playing(false, monotonic_time());
            }
        }
    }
    pub fn seek(&self, position: f64) {
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.cancel();
        }
        match &self.0 {
            Playback::Recording(media) => media.seek(position),
            Playback::Timeline(clock) => {
                let mut clock = clock.borrow_mut();
                let duration = clock.duration();
                let _ = clock.seek(position.clamp(0., duration), monotonic_time());
            }
        }
        self.synchronize();
    }
    pub fn set_rate(&self, rate: f64) {
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.cancel();
        }
        match &self.0 {
            Playback::Recording(media) => media.set_rate(rate),
            Playback::Timeline(clock) => {
                let _ = clock.borrow_mut().set_rate(rate, monotonic_time());
            }
        }
    }
    pub fn waveforms(&self, keyboard: bool) -> super::state::ReviewWaveforms {
        match &self.0 {
            Playback::Recording(media) => media.waveforms(keyboard),
            Playback::Timeline(_) => empty_waveforms(),
        }
    }
    pub fn take_error(&self) -> Option<String> {
        match &self.0 {
            Playback::Recording(media) => media.take_error(),
            Playback::Timeline(_) => None,
        }
    }
    pub fn mix(&self, mix: &StemMix, volume: f32, kit: bool, keyboard: bool) {
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.volume(volume);
        }
        if let Playback::Recording(media) = &self.0 {
            media.mix(mix, volume, kit, keyboard);
        }
    }
    pub fn synchronize(&self) {
        if let Playback::Recording(media) = &self.0 {
            let active = self.1.borrow().as_ref().is_some_and(|c| c.active());
            if media.audition.get() != active {
                media.audition.set(active);
                media.set_volume(media.volume.get());
            }
            media.synchronize();
        }
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            let (rate, running) = match &self.0 {
                Playback::Recording(media) => (
                    media.rate.get(),
                    !media.original.element.paused()
                        && !media.original.element.seeking()
                        && media.original.element.ready_state() >= 3,
                ),
                Playback::Timeline(clock) => {
                    let c = clock.borrow();
                    (c.rate(), !c.paused() && !c.ended(monotonic_time()))
                }
            };
            if let Err(error) = clicks.synchronize(self.position(), rate, running) {
                clicks.set_mix(None);
                self.2.replace(Some(format!(
                    "Click audition stopped; recording playback continues: {error:#}"
                )));
                if let Playback::Recording(media) = &self.0 {
                    media.audition.set(false);
                    media.set_volume(media.volume.get());
                }
            }
        }
    }
    pub fn take_audition_error(&self) -> Option<String> {
        self.2.borrow_mut().take()
    }
    pub fn set_audition(
        &self,
        mix: Option<Arc<crate::audition_clicks::Mix>>,
        volume: f32,
    ) -> Result<()> {
        let active = mix.as_ref().is_some_and(|m| m.active());
        if active && self.1.borrow().is_none() {
            let context = AudioContext::new().map_err(err)?;
            self.1
                .replace(Some(super::click_audio::ClickAudio::new(context, true)?));
        }
        if let Some(clicks) = self.1.borrow_mut().as_mut() {
            clicks.set_mix(mix);
            clicks.volume(volume);
        }
        if let Playback::Recording(media) = &self.0 {
            media.audition.set(active);
            media.set_volume(volume);
        }
        Ok(())
    }
    pub fn analysis_route(
        &self,
        project: &crate::editor::Project,
        fallback: crate::mapping::GuitarSource,
        enabled: bool,
        volume: f32,
    ) {
        if let Playback::Recording(media) = &self.0 {
            media.analysis_route(project, fallback, enabled, volume);
        }
    }
}
/// Content identity survives portable asset relocation. Track selection and
/// mix settings reuse the same graph; attachment or availability changes rebuild it.
#[derive(PartialEq, Eq)]
struct MediaIdentity {
    audio: String,
    stems: [Option<ManifestIdentity>; 2],
}
#[derive(PartialEq, Eq)]
struct ManifestIdentity {
    conventional: Option<crate::stem_review::conventional::Clock>,
    rate: u32,
    channels: u16,
    frames: u64,
    assets: Vec<(StemId, String, bool)>,
}
impl MediaIdentity {
    fn new(doc: &crate::editor::Project) -> Self {
        Self {
            audio: doc.audio_sha256.to_ascii_lowercase(),
            stems: [
                doc.content.stems.as_ref(),
                doc.content.keyboard_stems.as_ref(),
            ]
            .map(|manifest| {
                manifest.map(|manifest| ManifestIdentity {
                    conventional: manifest.conventional.clone(),
                    rate: manifest.sample_rate,
                    channels: manifest.channels,
                    frames: manifest.frames,
                    assets: manifest
                        .stems
                        .iter()
                        .map(|asset| {
                            (
                                asset.id,
                                asset.sha256.to_ascii_lowercase(),
                                crate::storage::is_file(&asset.path),
                            )
                        })
                        .collect(),
                })
            }),
        }
    }
}
pub fn keyboard_stems(doc: &crate::editor::Project) -> bool {
    crate::keys::is_keys(&doc.selected_track) && doc.content.keyboard_stems.is_some()
}
fn err(value: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow::anyhow!("Audio: {value:?}")
}
pub fn context() -> Result<AudioContext> {
    let options = web_sys::AudioContextOptions::new();
    options.set_sample_rate(44100.);
    AudioContext::new_with_context_options(&options).map_err(err)
}
pub async fn duration(bytes: &[u8]) -> Result<f64> {
    let context = context()?;
    let result = decode(&context, bytes).await.map(|pcm| pcm.duration());
    let _ = context.close();
    result
}
pub async fn source_duration(source: &Path) -> Result<f64> {
    if let Some(audio) = crate::song_metadata::recording_file(source)? {
        duration(crate::storage::shared(&audio)?.as_slice()).await
    } else {
        crate::project_recording::inferred_duration(
            &crate::chart::Chart::parse(crate::storage::read_to_string(
                source.join("notes.chart"),
            )?)?,
            &crate::pipeline::song_ini(source)?,
        )
    }
}
/// Use Rust codecs first. Browser-only codecs such as Opus stay local too.
pub async fn decode(context: &AudioContext, bytes: &[u8]) -> Result<crate::audio_decode::Pcm> {
    match crate::audio_decode::decode(bytes).await {
        Ok(pcm) => Ok(pcm),
        Err(rust_error) => {
            let input = js_sys::Uint8Array::from(bytes).buffer();
            let decode = JsFuture::from(context.decode_audio_data(&input).map_err(err)?);
            let timeout = gloo_timers::future::TimeoutFuture::new(15_000);
            futures_util::pin_mut!(decode, timeout);
            let value = match futures_util::future::select(decode, timeout).await {
                futures_util::future::Either::Left((value, _)) => value.map_err(|error| {
                    anyhow::anyhow!(
                        "Unsupported or damaged recording: {rust_error:#}; browser: {error:?}"
                    )
                })?,
                futures_util::future::Either::Right(_) => anyhow::bail!(
                    "The browser could not decode this recording in 15 seconds: {rust_error:#}"
                ),
            };
            use wasm_bindgen::JsCast;
            let buffer: web_sys::AudioBuffer = value.dyn_into().map_err(err)?;
            ensure!(buffer.number_of_channels() > 0, "Recording has no channels");
            let left = buffer.get_channel_data(0).map_err(err)?;
            let right = if buffer.number_of_channels() > 1 {
                buffer.get_channel_data(1).map_err(err)?
            } else {
                left.clone()
            };
            let stereo: Vec<_> = left
                .into_iter()
                .zip(right)
                .flat_map(|(left, right)| [left, right])
                .collect();
            ensure!(
                !stereo.is_empty() && stereo.iter().all(|v| v.is_finite()),
                "The browser decoded invalid audio"
            );
            Ok(crate::audio_decode::Pcm {
                stereo,
                rate: buffer.sample_rate() as u32,
                channels: buffer.number_of_channels() as usize,
            })
        }
    }
}
fn channel(
    context: &AudioContext,
    master: &GainNode,
    path: &Path,
    bytes: &[u8],
) -> Result<Channel> {
    let chunks = js_sys::Array::new();
    chunks.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("audio/wav");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&chunks, &options).map_err(err)?;
    let url = ObjectUrl(web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?);
    let element = HtmlAudioElement::new_with_src(&url.0).map_err(err)?;
    element.set_preload("auto");
    js_sys::Reflect::set(&element, &"preservesPitch".into(), &true.into()).map_err(err)?;
    element
        .set_attribute("data-source", &path.to_string_lossy())
        .map_err(err)?;
    element.set_hidden(true);
    let node = context.create_media_element_source(&element).map_err(err)?;
    let gain = context.create_gain().map_err(err)?;
    gain.gain().set_value(0.);
    node.connect_with_audio_node(&gain).map_err(err)?;
    gain.connect_with_audio_node(master).map_err(err)?;
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap()
        .append_child(&element)
        .map_err(err)?;
    element.load();
    Ok(Channel {
        element,
        gain,
        node,
        _url: url,
        clock: RefCell::new(Default::default()),
        activity: RefCell::new(Default::default()),
        play_intent: Rc::new(Cell::new(0)),
    })
}
impl RecordingMedia {
    fn analysis_route(
        &self,
        project: &crate::editor::Project,
        fallback: crate::mapping::GuitarSource,
        enabled: bool,
        volume: f32,
    ) {
        let route = crate::reanalysis::audition_route(project, fallback, enabled);
        if route.is_none() {
            let original = StemMix::default();
            self.mix(
                if enabled {
                    &project.content.stem_mix
                } else {
                    &original
                },
                volume,
                project.kit_audition.is_some(),
                keyboard_stems(project),
            );
            return;
        }
        let route = route.unwrap();
        let weights = route.weights(self.position());
        let now = self.context.current_time();
        let mut original = 0.;
        for (id, gain) in weights {
            if !self.stems.contains_key(&(false, id)) {
                original += gain;
            }
        }
        self.set_volume(volume);
        self.original.set_gain(original, now);
        for ((keyboard, id), channel) in &self.stems {
            let gain = if *keyboard {
                0.
            } else {
                weights
                    .iter()
                    .filter(|(selected, _)| selected == id)
                    .map(|(_, gain)| *gain)
                    .sum()
            };
            channel.set_gain(gain, now);
        }
        if let Some(channel) = self.kit.borrow().as_ref() {
            channel.set_gain(0., now);
        }
        self.update_activity();
    }
    pub async fn load(
        store: &crate::editor::Store,
        doc: &crate::editor::Project,
    ) -> Result<(Self, super::state::ReviewWaveforms, String)> {
        let context = context()?;
        let mut pending_context = PendingContext(Some(context.clone()));
        let master = context.create_gain().map_err(err)?;
        master
            .connect_with_audio_node(&context.destination())
            .map_err(err)?;
        let source = store.audio_path(doc)?;
        ensure!(
            crate::models::hash_file(&source)?.eq_ignore_ascii_case(&doc.audio_sha256),
            "Source recording checksum differs from the project"
        );
        let pcm = decode(&context, crate::storage::shared(&source)?.as_slice()).await?;
        ensure!(
            (pcm.duration() - doc.duration).abs() < 0.2,
            "The recording duration differs from the project"
        );
        let original_waveform = Arc::new(Waveform::from_stereo(&pcm.stereo, pcm.rate));
        let original = channel(&context, &master, &source, &pcm.float_wave()?)?;
        drop(pcm);
        let mut stems = BTreeMap::new();
        let mut waveforms = [BTreeMap::new(), BTreeMap::new()];
        let mut missing = 0;
        for (keyboard, manifest) in [
            (false, doc.content.stems.as_ref()),
            (true, doc.content.keyboard_stems.as_ref()),
        ] {
            let Some(manifest) = manifest else {
                continue;
            };
            for asset in &manifest.stems {
                if !crate::storage::is_file(&asset.path) {
                    missing += 1;
                    continue;
                }
                ensure!(
                    crate::models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
                    "Stem checksum differs: {}",
                    asset.id.as_str()
                );
                let mut pcm =
                    decode(&context, crate::storage::shared(&asset.path)?.as_slice()).await?;
                if manifest.conventional.is_some() {
                    pcm = crate::audio_decode::Pcm {
                        stereo: manifest
                            .prepare_channel(pcm.samples(crate::stem_review::SAMPLE_RATE, 2)?)?,
                        rate: crate::stem_review::SAMPLE_RATE,
                        channels: 2,
                    };
                }
                ensure!(
                    (pcm.duration() - doc.duration).abs() < 0.2,
                    "Stem duration differs: {}",
                    asset.id.as_str()
                );
                waveforms[keyboard as usize].insert(
                    asset.id,
                    Arc::new(Waveform::from_stereo(&pcm.stereo, pcm.rate)),
                );
                stems.insert(
                    (keyboard, asset.id),
                    channel(&context, &master, &asset.path, &pcm.float_wave()?)?,
                );
                drop(pcm);
            }
        }
        let kit = if let Some(selected) = &doc.kit_audition {
            let path = crate::part_analysis::prepared_kit_path(store, doc, selected)?;
            let pcm = decode(&context, crate::storage::shared(&path)?.as_slice()).await?;
            ensure!(
                pcm.rate == 44100
                    && pcm.channels == 2
                    && pcm.stereo.len() as u64 == selected.frames * 2,
                "Saved kit audio has another sample clock"
            );
            Some(channel(&context, &master, &path, &pcm.float_wave()?)?)
        } else {
            None
        };
        for _ in 0..3000 {
            if original.element.ready_state() >= 1 {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(10).await;
        }
        ensure!(
            original.element.ready_state() >= 1,
            "The browser could not load the recording"
        );
        let media = Self {
            identity: MediaIdentity::new(doc),
            context,
            original,
            stems,
            waveforms: waveforms.map(|stems| super::state::ReviewWaveforms {
                original: original_waveform.clone(),
                stems,
            }),
            play_epoch: Rc::new(Cell::new(0)),
            error: Rc::new(RefCell::new(None)),
            kit: RefCell::new(kit),
            master,
            // New media elements begin at 1×; set_rate applies a saved custom
            // rate exactly once before an unchanged request can be skipped.
            rate: Cell::new(1.),
            audition: Cell::new(false),
            volume: Cell::new(0.7),
        };
        pending_context.0.take();
        media.set_rate(doc.transport.playback_rate);
        media.seek(doc.transport.position);
        for _ in 0..500 {
            if !media.original.element.seeking() {
                break;
            }
            gloo_timers::future::TimeoutFuture::new(10).await;
        }
        ensure!(
            !media.original.element.seeking(),
            "The browser could not finish seeking the recording"
        );
        media.mix(
            &doc.content.stem_mix,
            0.7,
            doc.kit_audition.is_some(),
            keyboard_stems(doc),
        );
        let notice = if missing > 0 {
            format!(
                "{missing} external stems are missing. Attach matching stem files to audition them."
            )
        } else {
            "Browser audio ready".into()
        };
        let waves = media.waveforms(keyboard_stems(doc));
        Ok((media, waves, notice))
    }
    pub fn position(&self) -> f64 {
        self.original.element.current_time()
    }
    pub fn matches(&self, doc: &crate::editor::Project) -> bool {
        self.identity == MediaIdentity::new(doc)
    }
    /// The saved set has already been decoded and validated on a worker. Reuse
    /// this graph and its clock instead of rebuilding every recording/stem.
    pub fn replace_kit(
        &self,
        store: &crate::editor::Store,
        doc: &crate::editor::Project,
    ) -> Result<()> {
        let selected = doc
            .kit_audition
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Select a kit component"))?;
        let path = crate::part_analysis::prepared_kit_path(store, doc, selected)?;
        let candidate = channel(
            &self.context,
            &self.master,
            &path,
            crate::storage::shared(&path)?.as_slice(),
        )?;
        candidate
            .element
            .set_playback_rate(doc.transport.playback_rate);
        candidate.element.set_current_time(doc.transport.position);
        self.kit.replace(Some(candidate));
        Ok(())
    }
    pub fn paused(&self) -> bool {
        self.original.element.paused()
    }
    pub fn ended(&self) -> bool {
        self.original.element.ended()
    }
    pub fn play(&self) -> Result<()> {
        let epoch = self.play_epoch.get().wrapping_add(1);
        self.play_epoch.set(epoch);
        self.watch_play(self.context.resume().map_err(err)?, None);
        self.play_channel(&self.original)?;
        self.update_activity();
        Ok(())
    }
    fn play_channel(&self, channel: &Channel) -> Result<()> {
        channel
            .play_intent
            .set(channel.play_intent.get().wrapping_add(1));
        self.watch_play(channel.element.play().map_err(err)?, Some(channel));
        Ok(())
    }
    fn watch_play(&self, promise: js_sys::Promise, channel: Option<&Channel>) {
        let epoch = self.play_epoch.get();
        let generation = self.play_epoch.clone();
        let error = self.error.clone();
        let intent =
            channel.map(|channel| (channel.play_intent.clone(), channel.play_intent.get()));
        wasm_bindgen_futures::spawn_local(async move {
            if JsFuture::from(promise).await.is_err()
                && generation.get() == epoch
                && intent.is_none_or(|(intent, ticket)| intent.get() == ticket)
            {
                *error.borrow_mut() = Some("Browser playback could not start. Press Play again and check audio output permissions; the chart and recording are retained.".into());
            }
        });
    }
    fn update_activity(&self) {
        use crate::playback_activity::Activity;
        // The original remains the transport clock even when its gain is zero.
        // Other decoders run only while they contribute to the audible mix.
        let playing = !self.original.element.paused() && !self.original.element.ended();
        let now = self.context.current_time();
        let position = self.position();
        for channel in self.stems.values().chain(self.kit.borrow().iter()) {
            let element = &channel.element;
            let activity = channel
                .activity
                .borrow()
                .activity(playing, element.paused(), now);
            match activity {
                Activity::Play => {
                    channel.clock.borrow_mut().reset();
                    element.set_playback_rate(self.rate.get());
                    if (element.current_time() - position).abs() > 0.001 {
                        element.set_current_time(position);
                    }
                    if let Err(error) = self.play_channel(channel) {
                        *self.error.borrow_mut() = Some(format!("{error:#}"));
                    }
                }
                Activity::Pause => {
                    channel.pause();
                    channel.clock.borrow_mut().reset();
                }
                Activity::Keep => {}
            }
        }
    }
    pub fn pause(&self) {
        self.play_epoch.set(self.play_epoch.get().wrapping_add(1));
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            channel.pause();
            channel.clock.borrow_mut().reset();
            channel.element.set_playback_rate(self.rate.get());
        }
    }
    pub fn seek(&self, position: f64) {
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            channel.clock.borrow_mut().reset();
            channel.element.set_playback_rate(self.rate.get());
            channel.element.set_current_time(position.max(0.));
        }
    }
    pub fn set_rate(&self, rate: f64) {
        if self.rate.replace(rate) == rate {
            return;
        }
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            channel.clock.borrow_mut().reset();
            channel.element.set_playback_rate(rate);
        }
    }
    pub fn waveforms(&self, keyboard: bool) -> super::state::ReviewWaveforms {
        self.waveforms[keyboard as usize].clone()
    }
    pub fn take_error(&self) -> Option<String> {
        self.error.borrow_mut().take()
    }
    pub fn mix(&self, mix: &StemMix, volume: f32, kit: bool, keyboard: bool) {
        self.set_volume(volume);
        let now = self.context.current_time();
        let ids: Vec<_> = self
            .stems
            .keys()
            .filter_map(|(k, id)| (*k == keyboard).then_some(*id))
            .collect();
        let prepared = mix.prepare_playback(ids.iter().copied()).ok();
        self.original.set_gain(
            if kit {
                0.
            } else if mix.mode == MixMode::Original || ids.is_empty() {
                1.
            } else {
                0.
            },
            now,
        );
        for ((channel_keyboard, id), channel) in &self.stems {
            channel.set_gain(
                if *channel_keyboard != keyboard || kit || mix.mode == MixMode::Original {
                    0.
                } else {
                    prepared
                        .as_ref()
                        .and_then(|mix| mix.gains.get(id))
                        .copied()
                        .unwrap_or(0.)
                },
                now,
            );
        }
        if let Some(channel) = self.kit.borrow().as_ref() {
            channel.set_gain(if kit { 1. } else { 0. }, now);
        }
        self.update_activity();
    }
    fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
        let target = volume
            * if self.audition.get() {
                1. - crate::audition_clicks::PEAK
            } else {
                1.
            };
        if self.master.gain().value() != target {
            ramp_gain(&self.master, target, self.context.current_time());
        }
    }
    pub fn synchronize(&self) {
        let position = self.position();
        let leader = &self.original.element;
        let running = !leader.paused() && !leader.seeking() && leader.ready_state() >= 3;
        let now = self.context.current_time();
        let rate = self.rate.get();
        for channel in self.stems.values().chain(self.kit.borrow().iter()) {
            let element = &channel.element;
            if element.paused() {
                continue;
            }
            let ready =
                running && !element.paused() && !element.seeking() && element.ready_state() >= 3;
            match channel.clock.borrow_mut().update(
                now,
                position,
                element.current_time(),
                rate,
                ready,
            ) {
                crate::media_clock::Correction::None => {}
                crate::media_clock::Correction::Rate(value) => element.set_playback_rate(value),
                crate::media_clock::Correction::Seek(target) => {
                    element.set_playback_rate(rate);
                    element.set_current_time(target);
                }
            }
        }
    }
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.pause();
        let _ = self.node.disconnect();
        let _ = self.gain.disconnect();
        let _ = self.element.remove_attribute("src");
        self.element.load();
        self.element.remove();
    }
}
impl Drop for RecordingMedia {
    fn drop(&mut self) {
        self.pause();
        let _ = self.context.close();
    }
}
