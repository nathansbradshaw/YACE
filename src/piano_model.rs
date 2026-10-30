//! Native Kong/ByteDance high-resolution piano model, including regression
//! peak decoding and the reference 50%-overlap/quarter-window deframing.
//!
//! Model: LanOss/mobimml-piano-transcription ONNX conversion of Kong et al.'s
//! checkpoint (CC BY4.0). Regression algorithm: bytedance/piano_transcription
//! (declares Apache-2.0), also published in qiuqiangkong's inference toolkit
//! revision0226e74cbc805660e34bbd6a8fed2083890ebb88.
//! No Python runtime or conversion is used. Pedal events stay separate from
//! key releases; they must not become artificially long controller holds.
//! Intentional decoder delta: sustained notes have no fixed six-second cap;
//! frame/offset evidence, retriggers and the recording end determine releases.
use crate::mapping::PitchNote;
use anyhow::{Result, ensure};
#[cfg(not(target_arch = "wasm32"))]
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use {crate::models, anyhow::Context};

pub const MODEL_REVISION: &str = "7dff58faf160d4c0bf13be48e30e614faecdba72";
pub const MODEL_SHA256: &str = "6ec4f07640837df2fcd2cede9c540865acff341def93fbd6432548dee169195a";
pub const MODEL_URL: &str = "https://huggingface.co/LanOss/mobimml-piano-transcription/resolve/7dff58faf160d4c0bf13be48e30e614faecdba72/piano_transcription.onnx";
pub const METHOD: &str = "kong-high-resolution-piano-onnx-rust-v2;16kHz;100fps;10s/5s overlap;regression decoder with six-second note cap removed;frame/offset/retrigger/audio-end releases;separate key/pedal releases";
#[cfg(not(target_arch = "wasm32"))]
const RATE: usize = 16_000;
#[cfg(not(target_arch = "wasm32"))]
const WINDOW: usize = 160_000;
#[cfg(not(target_arch = "wasm32"))]
const HOP: usize = 80_000;
#[cfg(not(target_arch = "wasm32"))]
const FRAMES: usize = 1_001;
const KEYS: usize = 88;
#[cfg(not(target_arch = "wasm32"))]
const HEADS: [(&str, usize); 6] = [
    ("reg_onset_output", KEYS),
    ("reg_offset_output", KEYS),
    ("frame_output", KEYS),
    ("velocity_output", KEYS),
    ("reg_pedal_offset_output", 1),
    ("pedal_frame_output", 1),
];

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Options {
    pub onset_threshold: f32,
    pub offset_threshold: f32,
    pub frame_threshold: f32,
    pub pedal_offset_threshold: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            onset_threshold: 0.3,
            offset_threshold: 0.3,
            frame_threshold: 0.1,
            pedal_offset_threshold: 0.2,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<()> {
        ensure!(
            [
                self.onset_threshold,
                self.offset_threshold,
                self.frame_threshold,
                self.pedal_offset_threshold
            ]
            .iter()
            .all(|v| v.is_finite() && (0.0..1.0).contains(v)),
            "Invalid piano decoder threshold"
        );
        Ok(())
    }
}

/// Row-major100Hz emissions in original recording time. Kept independently
/// from decoder thresholds so a benchmark can compare decoders without another
/// neural forward pass. Scores are uncalibrated model outputs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Activations {
    pub duration_seconds: f64,
    pub reg_onset: Vec<f32>,
    pub reg_offset: Vec<f32>,
    pub frame: Vec<f32>,
    pub velocity: Vec<f32>,
    pub reg_pedal_offset: Vec<f32>,
    pub pedal_frame: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pedal {
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub method: String,
    pub notes: Vec<PitchNote>,
    /// Aligned with notes, including after onset sorting.
    pub velocities: Vec<u8>,
    pub pedals: Vec<Pedal>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Transcriber {
    session: Session,
}
#[cfg(not(target_arch = "wasm32"))]
impl Transcriber {
    pub fn load(model: &Path) -> Result<Self> {
        ensure!(
            models::hash_file(model)? == MODEL_SHA256,
            "Kong piano model checksum mismatch"
        );
        let session = Session::builder()?
            .with_intra_threads(4)?
            .with_optimization_level(GraphOptimizationLevel::Disable)?
            .commit_from_file(model)?;
        ensure!(
            session.inputs.len() == 1 && session.inputs[0].name == "waveform",
            "Unexpected Kong ONNX input contract"
        );
        for (name, _) in HEADS {
            ensure!(
                session.outputs.iter().any(|o| o.name == name),
                "Kong ONNX output missing {name}"
            );
        }
        Ok(Self { session })
    }
    pub fn infer_samples(&mut self, audio: &[f32]) -> Result<Activations> {
        ensure!(
            !audio.is_empty() && audio.iter().all(|x| x.is_finite()),
            "Piano inference needs finite nonempty16kHz mono audio"
        );
        let padded = audio
            .len()
            .checked_add(WINDOW - 1)
            .context("Audio too long")?
            / WINDOW
            * WINDOW;
        let windows = (padded - WINDOW) / HOP + 1;
        let mut values: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::new());
        for index in 0..windows {
            let start = index * HOP;
            let mut waveform = vec![0.; WINDOW];
            if start < audio.len() {
                let n = WINDOW.min(audio.len() - start);
                waveform[..n].copy_from_slice(&audio[start..start + n]);
            }
            let tensor = Tensor::from_array(([1, WINDOW], waveform.into_boxed_slice()))?;
            let outputs = self.session.run(ort::inputs!["waveform"=>tensor])?;
            let (from, to) = retained_frames(index, windows);
            for (head, (name, classes)) in HEADS.iter().enumerate() {
                let (shape, data) = outputs
                    .get(*name)
                    .with_context(|| format!("Kong output missing {name}"))?
                    .try_extract_tensor::<f32>()?;
                ensure!(
                    shape.as_ref() == [1, FRAMES as i64, *classes as i64],
                    "Unexpected Kong {name} shape: {shape:?}"
                );
                let chunk = &data[from * classes..to * classes];
                ensure!(
                    chunk.iter().all(|v| v.is_finite()),
                    "Nonfinite Kong {name} output"
                );
                values[head].extend_from_slice(chunk);
            }
            eprintln!(
                "Piano Kong: {:.1}/{:.1} seconds",
                ((start + WINDOW).min(audio.len())) as f64 / RATE as f64,
                audio.len() as f64 / RATE as f64
            );
        }
        let frame_count = (audio.len() / 160 + 1).min(values[0].len() / KEYS);
        for (head, (_, classes)) in HEADS.iter().enumerate() {
            values[head].truncate(frame_count * classes);
        }
        let [
            reg_onset,
            reg_offset,
            frame,
            velocity,
            reg_pedal_offset,
            pedal_frame,
        ] = values;
        Ok(Activations {
            duration_seconds: audio.len() as f64 / RATE as f64,
            reg_onset,
            reg_offset,
            frame,
            velocity,
            reg_pedal_offset,
            pedal_frame,
        })
    }
    pub fn transcribe(&mut self, audio: &Path, options: Options) -> Result<Output> {
        decode(
            &self.infer_samples(&models::decode(audio, RATE as u32, 1)?)?,
            options,
        )
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn pitch(audio: &Path, model: &Path) -> Result<Vec<PitchNote>> {
    Ok(Transcriber::load(model)?
        .transcribe(audio, Options::default())?
        .notes)
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_frames(index: usize, windows: usize) -> (usize, usize) {
    if windows == 1 {
        (0, FRAMES)
    } else if index == 0 {
        (0, 750)
    } else if index + 1 == windows {
        (250, 1000)
    } else {
        (250, 750)
    }
}

/// Regression maxima and sub-frame shifts from Kong's monotonic-neighbour
/// detector. A saturated flat peak uses zero shift instead of producing NaN.
fn peaks(x: &[f32], threshold: f32, neighbour: usize) -> Vec<Option<f32>> {
    let mut output = vec![None; x.len()];
    if x.len() <= neighbour * 2 {
        return output;
    }
    for n in neighbour..x.len() - neighbour {
        if x[n] > threshold
            && (0..neighbour).all(|i| x[n - i] >= x[n - i - 1] && x[n + i] >= x[n + i + 1])
        {
            let denominator = if x[n - 1] > x[n + 1] {
                x[n] - x[n + 1]
            } else {
                x[n] - x[n - 1]
            };
            output[n] = Some(if denominator > 0. {
                (x[n + 1] - x[n - 1]) / denominator / 2.
            } else {
                0.
            });
        }
    }
    output
}

pub fn decode(a: &Activations, options: Options) -> Result<Output> {
    options.validate()?;
    ensure!(
        a.duration_seconds.is_finite()
            && a.duration_seconds > 0.
            && a.reg_onset.len().is_multiple_of(KEYS),
        "Invalid piano activation duration/shape"
    );
    let count = a.reg_onset.len() / KEYS;
    ensure!(
        [&a.reg_offset, &a.frame, &a.velocity]
            .iter()
            .all(|v| v.len() == count * KEYS)
            && a.pedal_frame.len() == count
            && a.reg_pedal_offset.len() == count,
        "Inconsistent piano activation shapes"
    );
    ensure!(
        [
            &a.reg_onset,
            &a.reg_offset,
            &a.frame,
            &a.velocity,
            &a.reg_pedal_offset,
            &a.pedal_frame
        ]
        .iter()
        .flat_map(|v| v.iter())
        .all(|v| v.is_finite()),
        "Nonfinite piano activations"
    );
    let mut notes = Vec::new();
    for key in 0..KEYS {
        let onset: Vec<_> = (0..count).map(|i| a.reg_onset[i * KEYS + key]).collect();
        let offset: Vec<_> = (0..count).map(|i| a.reg_offset[i * KEYS + key]).collect();
        let onsets = peaks(&onset, options.onset_threshold, 2);
        let offsets = peaks(&offset, options.offset_threshold, 4);
        let mut begin = None;
        let mut disappear = None;
        let mut offset_occur = None;
        let mut finish = |start: usize, end: usize, shift: f32| {
            let onset_shift = onsets[start].unwrap_or(0.);
            let start_seconds = ((start as f64 + onset_shift as f64) / 100.).max(0.);
            let end_seconds = ((end as f64 + shift as f64) / 100.).min(a.duration_seconds);
            if start_seconds < end_seconds && start_seconds < a.duration_seconds {
                notes.push((
                    PitchNote {
                        start: start_seconds,
                        end: end_seconds,
                        pitch: 21 + key as u8,
                        confidence: onset[start].clamp(0., 1.),
                    },
                    (a.velocity[start * KEYS + key] * 128.).clamp(1., 127.) as u8,
                ));
            }
        };
        for i in 0..count {
            if onsets[i].is_some() {
                if let Some(start) = begin {
                    finish(start, i.saturating_sub(1), 0.);
                    disappear = None;
                    offset_occur = None;
                }
                begin = Some(i);
            }
            if let Some(start) = begin
                && i > start
            {
                if a.frame[i * KEYS + key] <= options.frame_threshold && disappear.is_none() {
                    disappear = Some(i);
                }
                if offsets[i].is_some() && offset_occur.is_none() {
                    offset_occur = Some(i);
                }
                if let Some(frame_end) = disappear {
                    let end = offset_occur
                        .filter(|&off| off - start > frame_end - off)
                        .unwrap_or(frame_end);
                    finish(start, end, offsets[end].unwrap_or(0.));
                    begin = None;
                    disappear = None;
                    offset_occur = None;
                }
                if begin.is_some() && i + 1 == count {
                    finish(start, i, offsets[i].unwrap_or(0.));
                    begin = None;
                    disappear = None;
                    offset_occur = None;
                }
            }
        }
    }
    notes.sort_by(|a, b| {
        a.0.start
            .total_cmp(&b.0.start)
            .then(a.0.pitch.cmp(&b.0.pitch))
    });
    let (notes, velocities) = notes.into_iter().unzip();
    let pedal_offsets = peaks(&a.reg_pedal_offset, options.pedal_offset_threshold, 4);
    let mut pedals = Vec::new();
    let mut begin = None;
    let mut disappear = None;
    for i in 1..count {
        if a.pedal_frame[i] >= 0.5 && a.pedal_frame[i] > a.pedal_frame[i - 1] && begin.is_none() {
            begin = Some(i);
        }
        if let Some(start) = begin
            && i > start
        {
            if a.pedal_frame[i] <= 0.5 && disappear.is_none() {
                disappear = Some(i);
            }
            let end = if pedal_offsets[i].is_some() {
                Some(i)
            } else {
                disappear.filter(|d| i - d >= 10)
            };
            if let Some(end) = end {
                let finish = ((end as f64 + pedal_offsets[end].unwrap_or(0.) as f64) / 100.)
                    .min(a.duration_seconds);
                if finish > start as f64 / 100. {
                    pedals.push(Pedal {
                        start: start as f64 / 100.,
                        end: finish,
                    });
                }
                begin = None;
                disappear = None;
            }
        }
    }
    Ok(Output {
        method: METHOD.into(),
        notes,
        velocities,
        pedals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deframing_preserves_absolute_clock_and_context_at_every_seam() {
        for windows in [1, 3, 5] {
            let mut absolute = Vec::new();
            for segment in 0..windows {
                let (from, to) = retained_frames(segment, windows);
                absolute.extend((from..to).map(|frame| segment * 500 + frame));
            }
            assert!(absolute.windows(2).all(|p| p[1] == p[0] + 1));
            assert_eq!(absolute[0], 0);
            assert_eq!(
                absolute.len(),
                if windows == 1 {
                    1001
                } else {
                    windows.div_ceil(2) * 1000
                }
            );
        }
    }
    #[test]
    fn regression_interpolation_and_flat_peaks_remain_finite() {
        let x = [0., 0., 0.1, 0.2, 0.8, 0.6, 0.1, 0., 0.];
        assert!((peaks(&x, 0.3, 2)[4].unwrap() - 1. / 3.).abs() < 1e-6);
        assert!(
            peaks(&[1.; 10], 0.3, 2)
                .into_iter()
                .flatten()
                .all(f32::is_finite)
        );
        assert!(peaks(&[0.; 20], 0.3, 2).iter().all(Option::is_none));
    }
}
