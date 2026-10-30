//! Browser codecs and pitch-preserving media playback, driven from Rust.
use crate::{
    stem_review::{MixMode, StemId, StemMix},
    waveform::Waveform,
};
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, path::Path};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AudioContext, GainNode, HtmlAudioElement, MediaElementAudioSourceNode};
pub struct Channel {
    pub element: HtmlAudioElement,
    pub gain: GainNode,
    _node: MediaElementAudioSourceNode,
    url: String,
}
pub struct Media {
    context: AudioContext,
    pub original: Channel,
    stems: BTreeMap<StemId, Channel>,
    master: GainNode,
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
    Ok(crate::audio_decode::decode(bytes).await?.duration())
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
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let element = HtmlAudioElement::new_with_src(&url).map_err(err)?;
    element.set_preload("auto");
    js_sys::Reflect::set(&element, &"preservesPitch".into(), &true.into()).map_err(err)?;
    element
        .set_attribute("data-source", &path.to_string_lossy())
        .map_err(err)?;
    element.set_hidden(true);
    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap()
        .append_child(&element)
        .map_err(err)?;
    let node = context.create_media_element_source(&element).map_err(err)?;
    let gain = context.create_gain().map_err(err)?;
    node.connect_with_audio_node(&gain).map_err(err)?;
    gain.connect_with_audio_node(master).map_err(err)?;
    element.load();
    Ok(Channel {
        element,
        gain,
        _node: node,
        url,
    })
}
impl Media {
    pub async fn load(
        store: &crate::editor::Store,
        doc: &crate::editor::Project,
    ) -> Result<(Self, super::state::ReviewWaveforms, String)> {
        let context = context()?;
        let master = context.create_gain().map_err(err)?;
        master
            .connect_with_audio_node(&context.destination())
            .map_err(err)?;
        let source = store.audio_path(doc)?;
        ensure!(
            crate::models::hash_file(&source)?.eq_ignore_ascii_case(&doc.audio_sha256),
            "Source recording checksum differs from the project"
        );
        let pcm = crate::audio_decode::decode(crate::storage::shared(&source)?.as_slice()).await?;
        ensure!(
            (pcm.duration() - doc.duration).abs() < 0.2,
            "The recording duration differs from the project"
        );
        let original_waveform = Waveform::from_stereo(&pcm.stereo, pcm.rate);
        let original = channel(&context, &master, &source, &pcm.wave()?)?;
        drop(pcm);
        let mut stems = BTreeMap::new();
        let mut waveforms = BTreeMap::new();
        let mut missing = 0;
        for manifest in doc
            .content
            .stems
            .iter()
            .chain(doc.content.keyboard_stems.iter())
        {
            for asset in &manifest.stems {
                if stems.contains_key(&asset.id) {
                    continue;
                }
                if !crate::storage::is_file(&asset.path) {
                    missing += 1;
                    continue;
                }
                ensure!(
                    crate::models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
                    "Stem checksum differs: {}",
                    asset.id.as_str()
                );
                let pcm =
                    crate::audio_decode::decode(crate::storage::shared(&asset.path)?.as_slice())
                        .await?;
                ensure!(
                    (pcm.duration() - doc.duration).abs() < 0.2,
                    "Stem duration differs: {}",
                    asset.id.as_str()
                );
                waveforms.insert(asset.id, Waveform::from_stereo(&pcm.stereo, pcm.rate));
                stems.insert(
                    asset.id,
                    channel(&context, &master, &asset.path, &pcm.wave()?)?,
                );
                drop(pcm);
            }
        }
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
            context,
            original,
            stems,
            master,
        };
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
        media.mix(&doc.content.stem_mix, 0.7);
        let notice = if missing > 0 {
            format!(
                "{missing} external stems are missing. Attach matching stem files to audition them."
            )
        } else {
            "Browser audio ready".into()
        };
        Ok((
            media,
            super::state::ReviewWaveforms {
                original: original_waveform,
                stems: waveforms,
            },
            notice,
        ))
    }
    pub fn position(&self) -> f64 {
        self.original.element.current_time()
    }
    pub fn paused(&self) -> bool {
        self.original.element.paused()
    }
    pub fn ended(&self) -> bool {
        self.original.element.ended()
    }
    pub fn play(&self) -> Result<()> {
        let _ = self.context.resume().map_err(err)?;
        for channel in std::iter::once(&self.original).chain(self.stems.values()) {
            let promise = channel.element.play().map_err(err)?;
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(promise).await;
            });
        }
        Ok(())
    }
    pub fn pause(&self) {
        for channel in std::iter::once(&self.original).chain(self.stems.values()) {
            let _ = channel.element.pause();
        }
    }
    pub fn seek(&self, position: f64) {
        for channel in std::iter::once(&self.original).chain(self.stems.values()) {
            channel.element.set_current_time(position.max(0.));
        }
    }
    pub fn set_rate(&self, rate: f64) {
        for channel in std::iter::once(&self.original).chain(self.stems.values()) {
            channel.element.set_playback_rate(rate);
        }
    }
    pub fn mix(&self, mix: &StemMix, volume: f32) {
        self.master.gain().set_value(volume);
        let prepared = mix.prepare(self.stems.keys().copied()).ok();
        self.original.gain.gain().set_value(
            if mix.mode == MixMode::Original || self.stems.is_empty() {
                1.
            } else {
                0.
            },
        );
        for (id, channel) in &self.stems {
            channel
                .gain
                .gain()
                .set_value(if mix.mode == MixMode::Original {
                    0.
                } else {
                    prepared
                        .as_ref()
                        .and_then(|mix| mix.gains.get(id))
                        .copied()
                        .unwrap_or(0.)
                });
        }
    }
    pub fn synchronize(&self) {
        let position = self.position();
        for channel in self.stems.values() {
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
        let _ = web_sys::Url::revoke_object_url(&self.url);
    }
}
impl Drop for Media {
    fn drop(&mut self) {
        self.pause();
        let _ = self.context.close();
    }
}
