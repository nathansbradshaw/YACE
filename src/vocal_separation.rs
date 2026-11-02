//! Research-only MDX karaoke adapter. Standard STFT operations implemented in
//! Rust against UVR's pinned metadata; no upstream runtime or weights bundled.
use anyhow::{Result, ensure};
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex32};
use std::{f32::consts::TAU, path::Path};
const FFT: usize = 5120;
const HOP: usize = 1024;
const BINS: usize = 2048;
const FRAMES: usize = 256;
const CHUNK: usize = HOP * (FRAMES - 1);
pub const MODEL_SHA256: &str = "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4";

fn reflect(index: isize, length: usize) -> usize {
    let period = 2 * (length - 1) as isize;
    let at = index.rem_euclid(period) as usize;
    if at < length {
        at
    } else {
        period as usize - at
    }
}
fn spectrogram(pcm: &[f32], frames: usize, bins: usize) -> Vec<f32> {
    let mut output = vec![0.; 4 * bins * frames];
    let fft = FftPlanner::new().plan_fft_forward(FFT);
    for channel in 0..2 {
        for time in 0..frames {
            let mut values: Vec<_> = (0..FFT)
                .map(|i| {
                    let at = reflect(
                        (time * HOP + i) as isize - (FFT / 2) as isize,
                        pcm.len() / 2,
                    );
                    Complex32::new(
                        pcm[at * 2 + channel] * (0.5 - 0.5 * (TAU * i as f32 / FFT as f32).cos()),
                        0.,
                    )
                })
                .collect();
            fft.process(&mut values);
            for (bin, value) in values.iter().take(bins).enumerate() {
                output[((channel * 2) * bins + bin) * frames + time] = value.re;
                output[((channel * 2 + 1) * bins + bin) * frames + time] = value.im;
            }
        }
    }
    output
}
fn inverse(spectrum: &[f32], frames: usize, bins: usize) -> Vec<f32> {
    let length = (frames - 1) * HOP;
    let mut output = vec![0f32; length * 2];
    let mut weights = vec![0f32; length];
    let fft = FftPlanner::new().plan_fft_inverse(FFT);
    for time in 0..frames {
        for channel in 0..2 {
            let mut values = vec![Complex32::new(0., 0.); FFT];
            for bin in 0..bins {
                values[bin] = Complex32::new(
                    spectrum[((channel * 2) * bins + bin) * frames + time],
                    spectrum[((channel * 2 + 1) * bins + bin) * frames + time],
                );
                if bin > 0 && bin < FFT / 2 {
                    values[FFT - bin] = values[bin].conj();
                }
            }
            fft.process(&mut values);
            for (i, value) in values.iter().enumerate() {
                let at = (time * HOP + i) as isize - (FFT / 2) as isize;
                if at < 0 || at as usize >= length {
                    continue;
                }
                let at = at as usize;
                let window = 0.5 - 0.5 * (TAU * i as f32 / FFT as f32).cos();
                output[at * 2 + channel] += value.re / FFT as f32 * window;
                if channel == 0 {
                    weights[at] += window * window;
                }
            }
        }
    }
    for (sample, &weight) in output.chunks_exact_mut(2).zip(&weights) {
        if weight > 1e-8 {
            sample[0] /= weight;
            sample[1] /= weight;
        }
    }
    output
}

pub fn separate(pcm: &[f32], model: &Path) -> Result<(Vec<f32>, Vec<f32>)> {
    ensure!(
        chart_engine::models::hash_file(model)? == MODEL_SHA256,
        "Pinned karaoke model checksum differs"
    );
    ensure!(
        !pcm.is_empty()
            && pcm.len().is_multiple_of(2)
            && pcm.len() <= 44100 * 2 * 600
            && pcm.iter().all(|v| v.is_finite()),
        "Karaoke research requires finite stereo 44.1kHz PCM, at most ten minutes"
    );
    let peak = pcm.iter().map(|v| v.abs()).fold(0f32, f32::max);
    if peak == 0. {
        return Ok((pcm.to_vec(), pcm.to_vec()));
    }
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model)?;
    let source_frames = pcm.len() / 2;
    let trim = FFT / 2;
    let generated = CHUNK - FFT;
    let mut backing = vec![0f32; pcm.len()];
    for start in (0..source_frames).step_by(generated) {
        let mut chunk = vec![0f32; CHUNK * 2];
        for (i, frame) in chunk.chunks_exact_mut(2).enumerate() {
            let at = (start + i) as isize - trim as isize;
            if at >= 0 && (at as usize) < source_frames {
                frame[0] = pcm[at as usize * 2] / peak;
                frame[1] = pcm[at as usize * 2 + 1] / peak;
            }
        }
        let mut input = spectrogram(&chunk, FRAMES, BINS);
        for channel in 0..4 {
            input[channel * BINS * FRAMES..channel * BINS * FRAMES + 3 * FRAMES].fill(0.);
        }
        let tensor = Tensor::from_array(([1, 4, BINS, FRAMES], input.into_boxed_slice()))?;
        let outputs = session.run(ort::inputs![tensor])?;
        let (shape, values) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [1, 4, BINS as i64, FRAMES as i64]
                && values.iter().all(|v| v.is_finite()),
            "Unexpected/nonfinite karaoke output: {shape:?}"
        );
        let estimate = inverse(values, FRAMES, BINS);
        let length = generated.min(source_frames - start);
        for i in 0..length {
            for channel in 0..2 {
                backing[(start + i) * 2 + channel] =
                    estimate[(trim + i) * 2 + channel] * peak * 1.065;
            }
        }
        eprintln!(
            "Karaoke source frames {}/{source_frames}",
            (start + length).min(source_frames)
        );
    }
    let lead: Vec<f32> = pcm
        .iter()
        .zip(&backing)
        .map(|(source, backing)| source - backing)
        .collect();
    ensure!(
        lead.iter().chain(&backing).all(|v| v.is_finite()),
        "Nonfinite reconstructed karaoke audio"
    );
    ensure!(
        chart_engine::models::hash_file(model)? == MODEL_SHA256,
        "Karaoke model changed during inference"
    );
    Ok((lead, backing))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_stft_inverse_preserves_phase_amplitude_and_source_origin() {
        let count = HOP * 7;
        let mut pcm = vec![0.; count * 2];
        for (i, frame) in pcm.chunks_exact_mut(2).enumerate() {
            let tone = (TAU * 440. * i as f32 / 44100.).sin() * 0.2;
            frame[0] = tone;
            frame[1] = -tone * 0.7;
        }
        let spec = spectrogram(&pcm, 8, FFT / 2 + 1);
        let output = inverse(&spec, 8, FFT / 2 + 1);
        assert_eq!(output.len(), pcm.len());
        assert!(output.iter().zip(&pcm).all(|(a, b)| (a - b).abs() < 2e-6));
    }
}
