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
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, GainNode, HtmlAudioElement, MediaElementAudioSourceNode};
pub struct Channel {
    pub element: HtmlAudioElement,
    pub gain: GainNode,
    _node: MediaElementAudioSourceNode,
    _url: ObjectUrl,
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
pub struct Media {
    identity: MediaIdentity,
    context: AudioContext,
    pub original: Channel,
    stems: BTreeMap<(bool, StemId), Channel>,
    waveforms: [super::state::ReviewWaveforms; 2],
    play_epoch: Rc<Cell<u64>>,
    error: Rc<RefCell<Option<String>>>,
    kit: RefCell<Option<Channel>>,
    master: GainNode,
}
/// Content identity survives portable asset relocation. Track selection and
/// mix settings reuse the same graph; undoing an attachment must rebuild it.
#[derive(PartialEq, Eq)]
struct MediaIdentity {
    audio: String,
    stems: [Option<ManifestIdentity>; 2],
}
#[derive(PartialEq, Eq)]
struct ManifestIdentity {
    rate: u32,
    channels: u16,
    frames: u64,
    assets: Vec<(StemId, String)>,
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
                    rate: manifest.sample_rate,
                    channels: manifest.channels,
                    frames: manifest.frames,
                    assets: manifest
                        .stems
                        .iter()
                        .map(|asset| (asset.id, asset.sha256.to_ascii_lowercase()))
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
        _node: node,
        _url: url,
    })
}
impl Media {
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
                let pcm = decode(&context, crate::storage::shared(&asset.path)?.as_slice()).await?;
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
        let promises = std::iter::once(self.context.resume().map_err(err)?);
        let mut promises: Vec<_> = promises.collect();
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            promises.push(channel.element.play().map_err(err)?);
        }
        for promise in promises {
            let generation = self.play_epoch.clone();
            let error = self.error.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if JsFuture::from(promise).await.is_err() && generation.get() == epoch {
                    *error.borrow_mut()=Some("Browser playback could not start. Press Play again and check audio output permissions; the chart and recording are retained.".into());
                }
            });
        }
        Ok(())
    }
    pub fn pause(&self) {
        self.play_epoch.set(self.play_epoch.get().wrapping_add(1));
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            let _ = channel.element.pause();
        }
    }
    pub fn seek(&self, position: f64) {
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
            channel.element.set_current_time(position.max(0.));
        }
    }
    pub fn set_rate(&self, rate: f64) {
        for channel in std::iter::once(&self.original)
            .chain(self.stems.values())
            .chain(self.kit.borrow().iter())
        {
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
        self.master.gain().set_value(volume);
        let ids: Vec<_> = self
            .stems
            .keys()
            .filter_map(|(k, id)| (*k == keyboard).then_some(*id))
            .collect();
        let prepared = mix.prepare(ids.iter().copied()).ok();
        self.original.gain.gain().set_value(if kit {
            0.
        } else if mix.mode == MixMode::Original || ids.is_empty() {
            1.
        } else {
            0.
        });
        for ((channel_keyboard, id), channel) in &self.stems {
            channel.gain.gain().set_value(
                if *channel_keyboard != keyboard || kit || mix.mode == MixMode::Original {
                    0.
                } else {
                    prepared
                        .as_ref()
                        .and_then(|mix| mix.gains.get(id))
                        .copied()
                        .unwrap_or(0.)
                },
            );
        }
        if let Some(channel) = self.kit.borrow().as_ref() {
            channel.gain.gain().set_value(if kit { 1. } else { 0. });
        }
    }
    pub fn synchronize(&self) {
        let position = self.position();
        for channel in self.stems.values().chain(self.kit.borrow().iter()) {
            if channel.element.ready_state() >= 1
                && (channel.element.current_time() - position).abs() > 0.03
            {
                channel.element.set_current_time(position);
            }
        }
    }
}
impl Drop for Channel {
    fn drop(&mut self) {
        let _ = self.element.pause();
        self.element.remove();
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        self.pause();
        let _ = self.context.close();
    }
}
