//! Shared native/browser BS-RoFormer spectral and overlap math.
use crate::stem_review::{SeparationIdentity, StemId};
use anyhow::{Context, Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const SAMPLE_RATE: u32 = 44_100;
pub const FFT_SIZE: usize = 2048;
pub const FFT_HOP: usize = 512;
pub const FREQUENCIES: usize = FFT_SIZE / 2 + 1;
pub const CHUNK_SAMPLES: usize = 588800;
pub const STEP_SAMPLES: usize = CHUNK_SAMPLES / 2;
pub const TIME_FRAMES: usize = CHUNK_SAMPLES / FFT_HOP + 1;
pub const RECEIPT_FILENAME: &str = "keyboard-separation-receipt.json";
pub const STEM_ORDER: [StemId; 6] = [
    StemId::Bass,
    StemId::Drums,
    StemId::Other,
    StemId::Vocals,
    StemId::Guitar,
    StemId::Piano,
];
pub(crate) const MASK_ELEMENTS: usize = 6 * FREQUENCIES * 2 * TIME_FRAMES * 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub source_audio_sha256: String,
    pub source_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub generation: SeparationIdentity,
    pub stem_sha256: BTreeMap<String, String>,
    pub chunk_starts: Vec<u64>,
    pub chunk_seconds: Vec<f64>,
    pub skipped_padding_chunks: u64,
}

pub(crate) fn reflect(mut i: i128, len: usize) -> usize {
    debug_assert!(len > 1);
    let len = len as i128;
    while i < 0 || i >= len {
        if i < 0 {
            i = -i;
        }
        if i >= len {
            i = 2 * len - 2 - i;
        }
    }
    i as usize
}
#[derive(Clone, Debug)]
pub struct ChunkPlan {
    pub source_frames: usize,
    pub border: usize,
    pub padded: usize,
    pub starts: Vec<usize>,
    pub skipped_padding_chunks: usize,
}
impl ChunkPlan {
    pub fn new(source_frames: usize) -> Result<Self> {
        ensure!(
            source_frames > 1,
            "HQ separation requires at least two source frames"
        );
        let border = if source_frames > CHUNK_SAMPLES {
            STEP_SAMPLES
        } else {
            0
        };
        let padded = source_frames
            .checked_add(border.checked_mul(2).context("PCM border overflow")?)
            .context("PCM frame count overflow")?;
        let end = source_frames
            .checked_add(border)
            .context("PCM crop overflow")?;
        let starts: Vec<_> = (0..padded)
            .step_by(STEP_SAMPLES)
            .filter(|start| *start < end && start.saturating_add(CHUNK_SAMPLES) > border)
            .collect();
        let skipped_padding_chunks = padded.div_ceil(STEP_SAMPLES) - starts.len();
        Ok(Self {
            source_frames,
            border,
            padded,
            starts,
            skipped_padding_chunks,
        })
    }
    pub fn weight(&self, start: usize, i: usize) -> f64 {
        let fade = CHUNK_SAMPLES / 10;
        if start == 0 && i < fade
            || start.saturating_add(STEP_SAMPLES) >= self.padded && i >= CHUNK_SAMPLES - fade
        {
            1.
        } else if i < fade {
            i as f64 / (fade - 1) as f64
        } else if i >= CHUNK_SAMPLES - fade {
            (CHUNK_SAMPLES - 1 - i) as f64 / (fade - 1) as f64
        } else {
            1.
        }
    }
    pub(crate) fn source_index(&self, start: usize, i: usize) -> Option<usize> {
        let available = (self.padded - start).min(CHUNK_SAMPLES);
        let j = if i < available {
            i
        } else if available > STEP_SAMPLES {
            reflect(i as i128, available)
        } else {
            return None;
        };
        Some(reflect(
            (start + j) as i128 - self.border as i128,
            self.source_frames,
        ))
    }
}

fn hann() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT_SIZE as f64).cos()) as f32)
        .collect()
}
/// Model feature layout: [time,frequency,stereo,real/imag].
pub fn stft(audio: &[Vec<f32>; 2]) -> Result<(Vec<f32>, Vec<Complex32>)> {
    ensure!(
        audio
            .iter()
            .all(|c| c.len() == CHUNK_SAMPLES && c.iter().all(|x| x.is_finite())),
        "Invalid HQ STFT PCM"
    );
    let window = hann();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut original = vec![Complex32::new(0., 0.); 2 * FREQUENCIES * TIME_FRAMES];
    let mut input = vec![0.; TIME_FRAMES * FREQUENCIES * 4];
    let mut frame = vec![Complex32::new(0., 0.); FFT_SIZE];
    for ch in 0..2 {
        for t in 0..TIME_FRAMES {
            for j in 0..FFT_SIZE {
                frame[j] = Complex32::new(
                    audio[ch][reflect(
                        (t * FFT_HOP + j) as i128 - (FFT_SIZE / 2) as i128,
                        CHUNK_SAMPLES,
                    )] * window[j],
                    0.,
                );
            }
            fft.process(&mut frame);
            for f in 0..FREQUENCIES {
                let z = frame[f];
                original[(ch * FREQUENCIES + f) * TIME_FRAMES + t] = z;
                let index = t * FREQUENCIES * 4 + (f * 2 + ch) * 2;
                input[index] = z.re;
                input[index + 1] = z.im;
            }
        }
    }
    ensure!(input.iter().all(|x| x.is_finite()), "HQ STFT overflow");
    Ok((input, original))
}
/// A single source/channel plane; passing None reconstructs the original.
pub fn reconstruct(
    original: &[Complex32],
    mask: Option<(&[f32], usize)>,
    ch: usize,
) -> Result<Vec<f32>> {
    ensure!(
        ch < 2
            && original.len() == 2 * FREQUENCIES * TIME_FRAMES
            && original
                .iter()
                .all(|z| z.re.is_finite() && z.im.is_finite()),
        "Invalid original HQ spectrum"
    );
    if let Some((m, s)) = mask {
        ensure!(s < 6 && m.len() == MASK_ELEMENTS, "Invalid HQ mask plane");
    }
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_inverse(FFT_SIZE);
    let window = hann();
    let len = (TIME_FRAMES - 1) * FFT_HOP + FFT_SIZE;
    let mut out = vec![0f64; len];
    let mut norm = vec![0f64; len];
    let mut frame = vec![Complex32::new(0., 0.); FFT_SIZE];
    for t in 0..TIME_FRAMES {
        for f in 0..FREQUENCIES {
            let mut z = original[(ch * FREQUENCIES + f) * TIME_FRAMES + t];
            if let Some((m, s)) = mask {
                let i = ((s * FREQUENCIES * 2 + f * 2 + ch) * TIME_FRAMES + t) * 2;
                ensure!(
                    m[i].is_finite() && m[i + 1].is_finite(),
                    "Nonfinite HQ mask"
                );
                z *= Complex32::new(m[i], m[i + 1]);
            }
            frame[f] = z;
        }
        for f in 1..FFT_SIZE / 2 {
            frame[FFT_SIZE - f] = frame[f].conj();
        }
        fft.process(&mut frame);
        for j in 0..FFT_SIZE {
            let w = window[j] as f64;
            out[t * FFT_HOP + j] += frame[j].re as f64 / FFT_SIZE as f64 * w;
            norm[t * FFT_HOP + j] += w * w;
        }
    }
    (FFT_SIZE / 2..FFT_SIZE / 2 + CHUNK_SAMPLES)
        .map(|i| {
            ensure!(norm[i] > 1e-12, "Uncovered inverse-STFT frame");
            let value = (out[i] / norm[i]) as f32;
            ensure!(value.is_finite(), "Nonfinite HQ reconstruction");
            Ok(value)
        })
        .collect()
}

pub(crate) struct Overlap {
    pub(crate) planes: Vec<Vec<f32>>,
    pub(crate) counter: Vec<f64>,
}
impl Overlap {
    pub(crate) fn new() -> Self {
        Self {
            planes: (0..12).map(|_| vec![0.; CHUNK_SAMPLES]).collect(),
            counter: vec![0.; CHUNK_SAMPLES],
        }
    }
    pub(crate) fn add(
        &mut self,
        plane: usize,
        data: &[f32],
        weights: &[f64],
        available: usize,
    ) -> Result<()> {
        for i in 0..available {
            self.planes[plane][i] += (data[i] as f64 * weights[i]) as f32;
            ensure!(self.planes[plane][i].is_finite(), "HQ overlap overflow");
        }
        Ok(())
    }
    pub(crate) fn flush(
        &mut self,
        plan: &ChunkPlan,
        start: usize,
        until: usize,
        mut write: impl FnMut(usize, f32, f32) -> Result<()>,
    ) -> Result<()> {
        let count = until - start;
        ensure!(count <= CHUNK_SAMPLES, "Invalid HQ overlap flush");
        for i in 0..count {
            let position = start + i;
            if position >= plan.border && position < plan.border + plan.source_frames {
                ensure!(self.counter[i] > 0., "Uncovered HQ source sample");
                for s in 0..6 {
                    write(
                        s,
                        (self.planes[s * 2][i] as f64 / self.counter[i]) as f32,
                        (self.planes[s * 2 + 1][i] as f64 / self.counter[i]) as f32,
                    )?;
                }
            }
        }
        for plane in &mut self.planes {
            plane.copy_within(count.., 0);
            plane[CHUNK_SAMPLES - count..].fill(0.);
        }
        self.counter.copy_within(count.., 0);
        self.counter[CHUNK_SAMPLES - count..].fill(0.);
        Ok(())
    }
}
