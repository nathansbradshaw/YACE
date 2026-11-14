//! Bounded Rust synthesis scheduled against the recording's existing AudioContext.
use crate::audition_clicks::Mix;
use anyhow::{Result, ensure};
use std::sync::Arc;
use wasm_bindgen::JsCast;
use web_sys::{AudioBufferSourceNode, AudioContext, AudioScheduledSourceNode, GainNode};
const FRAMES: usize = 1024;
const LEAD: f64 = 0.006;
const HORIZON: f64 = 0.09;
// Bound drift in listening time. A fixed source-time tolerance becomes four
// times as audible at 25% practice speed.
const MAX_CLOCK_DRIFT: f64 = 0.012;
pub struct ClickAudio {
    context: AudioContext,
    gain: GainNode,
    owned: bool,
    mix: Option<Arc<Mix>>,
    nodes: Vec<(f64, AudioBufferSourceNode)>,
    anchor: Option<(f64, f64, f64)>,
    next: f64,
    volume: f32,
}
fn error(e: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow::anyhow!("Click audition: {e:?}")
}
impl ClickAudio {
    pub fn new(context: AudioContext, owned: bool) -> Result<Self> {
        let gain = context.create_gain().map_err(error)?;
        gain.connect_with_audio_node(&context.destination())
            .map_err(error)?;
        Ok(Self {
            context,
            gain,
            owned,
            mix: None,
            nodes: Vec::new(),
            anchor: None,
            next: 0.,
            volume: 0.7,
        })
    }
    pub fn active(&self) -> bool {
        self.mix.as_ref().is_some_and(|m| m.active())
    }
    pub fn set_mix(&mut self, mix: Option<Arc<Mix>>) {
        if self
            .mix
            .as_ref()
            .zip(mix.as_ref())
            .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
            || self.mix.is_none() && mix.is_none()
        {
            return;
        }
        self.cancel();
        self.mix = mix;
    }
    pub fn volume(&mut self, volume: f32) {
        if self.volume == volume.clamp(0., 1.) {
            return;
        }
        self.volume = volume.clamp(0., 1.);
        let now = self.context.current_time();
        let p = self.gain.gain();
        let _ = p.cancel_scheduled_values(now);
        let _ = p.set_value_at_time(p.value(), now);
        let _ = p.linear_ramp_to_value_at_time(self.volume, now + 0.006);
    }
    pub fn resume(&self) {
        if self.active() {
            let _ = self.context.resume();
        }
    }
    pub fn cancel(&mut self) {
        let now = self.context.current_time();
        let gain = self.gain.gain();
        let _ = gain.cancel_scheduled_values(now);
        let _ = gain.set_value_at_time(gain.value(), now);
        let _ = gain.linear_ramp_to_value_at_time(0., now + LEAD);
        for (end, node) in &mut self.nodes {
            let _ = node
                .unchecked_ref::<AudioScheduledSourceNode>()
                .stop_with_when(now + LEAD);
            *end = (*end).min(now + LEAD);
        }
        self.anchor = None;
    }
    pub fn synchronize(&mut self, position: f64, rate: f64, running: bool) -> Result<()> {
        ensure!(
            position.is_finite() && rate.is_finite() && (0.25..=2.).contains(&rate),
            "Invalid click audition clock"
        );
        if !running || !self.active() {
            if self.anchor.is_some() {
                self.cancel();
            }
            return Ok(());
        }
        let now = self.context.current_time();
        self.nodes.retain(|(end, node)| {
            if *end <= now {
                let _ = node.disconnect();
                false
            } else {
                true
            }
        });
        let reset = self.anchor.is_none_or(|(at, source, old_rate)| {
            old_rate != rate
                || (position - (source + (now - at) * rate)).abs() > MAX_CLOCK_DRIFT * rate
        }) || self.next < now;
        if reset {
            self.cancel();
            self.anchor = Some((now + LEAD, position, rate));
            self.next = now + LEAD;
            let gain = self.gain.gain();
            let _ = gain.set_value_at_time(0., now + LEAD);
            let _ = gain.linear_ramp_to_value_at_time(self.volume, now + LEAD + 0.006);
        }
        let (at, source, rate) = self.anchor.unwrap();
        let sample_rate = self.context.sample_rate() as u32;
        let step = FRAMES as f64 / sample_rate as f64;
        let mix = self.mix.as_ref().unwrap();
        while self.next < now + HORIZON {
            ensure!(
                self.nodes.len() < 32,
                "Click audition scheduling fell behind"
            );
            let start = source + (self.next - at) * rate;
            let mut samples = vec![0.; FRAMES];
            mix.render_output(start, rate, sample_rate, &mut samples)?;
            let buffer = self
                .context
                .create_buffer(1, FRAMES as u32, sample_rate as f32)
                .map_err(error)?;
            buffer.copy_to_channel(&samples, 0).map_err(error)?;
            let node = self.context.create_buffer_source().map_err(error)?;
            node.set_buffer(Some(&buffer));
            node.connect_with_audio_node(&self.gain).map_err(error)?;
            node.start_with_when(self.next).map_err(error)?;
            self.next += step;
            self.nodes.push((self.next, node));
        }
        Ok(())
    }
}
impl Drop for ClickAudio {
    fn drop(&mut self) {
        for (_, node) in self.nodes.drain(..) {
            let _ = node.unchecked_ref::<AudioScheduledSourceNode>().stop();
            let _ = node.disconnect();
        }
        let _ = self.gain.disconnect();
        if self.owned {
            let _ = self.context.close();
        }
    }
}
