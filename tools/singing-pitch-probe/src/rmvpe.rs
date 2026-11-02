//! RVC RMVPE frontend, translated to Rust from MIT upstream infer/rmvpe.py
//! revision 81eed5e8f68b6bed1789f682fe78cdd324495afc. Weights are separate.
use anyhow::{Result, ensure};
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex32};
use std::f32::consts::TAU;
const FFT: usize = 1024;
const HOP: usize = 160;
const MELS: usize = 128;

fn reflected(index: isize, len: usize) -> usize {
    let period = 2 * (len - 1) as isize;
    let at = index.rem_euclid(period) as usize;
    if at < len { at } else { period as usize - at }
}

/// HTK mel scale with Slaney area normalization, matching librosa's defaults
/// when htk=true; magnitude (not power), periodic Hann and natural logarithm.
fn filters() -> Vec<Vec<(usize, f32)>> {
    let hz_to_mel = |hz: f64| 2595. * (1. + hz / 700.).log10();
    let mel_to_hz = |mel: f64| 700. * (10f64.powf(mel / 2595.) - 1.);
    let lo = hz_to_mel(30.);
    let hi = hz_to_mel(8000.);
    let points: Vec<_> = (0..MELS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64))
        .collect();
    (0..MELS)
        .map(|m| {
            (0..=FFT / 2)
                .filter_map(|bin| {
                    let hz = bin as f64 * 16000. / FFT as f64;
                    let weight = ((hz - points[m]) / (points[m + 1] - points[m]))
                        .min((points[m + 2] - hz) / (points[m + 2] - points[m + 1]))
                        .max(0.)
                        * 2.
                        / (points[m + 2] - points[m]);
                    (weight > 0.).then_some((bin, weight as f32))
                })
                .collect()
        })
        .collect()
}

pub fn infer(session: &mut Session, pcm: &[f32], start: usize, end: usize) -> Result<Vec<f32>> {
    ensure!(
        pcm.len() > FFT / 2,
        "RMVPE reflection padding needs more than 512 samples"
    );
    let count = pcm.len() / HOP + 1;
    let left = start.saturating_sub(100);
    let right = (end + 100).min(count);
    let padded = (right - left).div_ceil(32) * 32;
    let mut mel = vec![0f32; MELS * padded];
    let fft = FftPlanner::new().plan_fft_forward(FFT);
    let filters = filters();
    for frame in left..right {
        let mut values: Vec<_> = (0..FFT)
            .map(|i| {
                let sample =
                    pcm[reflected((frame * HOP + i) as isize - (FFT / 2) as isize, pcm.len())];
                Complex32::new(
                    sample * (0.5 - 0.5 * (TAU * i as f32 / FFT as f32).cos()),
                    0.,
                )
            })
            .collect();
        fft.process(&mut values);
        let magnitude: Vec<_> = values[..=FFT / 2].iter().map(|v| v.norm()).collect();
        for (m, filter) in filters.iter().enumerate() {
            let value: f32 = filter
                .iter()
                .map(|&(bin, weight)| magnitude[bin] * weight)
                .sum();
            mel[m * padded + frame - left] = value.max(1e-5).ln();
        }
    }
    let input = Tensor::from_array(([1, MELS, padded], mel.into_boxed_slice()))?;
    let output = session.run(ort::inputs![input])?;
    let (shape, values) = output[0].try_extract_tensor::<f32>()?;
    ensure!(
        shape.len() == 3 && shape[0] == 1 && shape[1] == padded as i64 && shape[2] == 360,
        "Unexpected pinned RMVPE tensor contract: {shape:?}"
    );
    Ok(values[(start - left) * 360..(end - left) * 360].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflection_and_filter_bank_follow_source_conventions() {
        assert_eq!(
            (-4..8).map(|i| reflected(i, 4)).collect::<Vec<_>>(),
            vec![2, 3, 2, 1, 0, 1, 2, 3, 2, 1, 0, 1]
        );
        let bank = filters();
        assert_eq!(bank.len(), 128);
        assert!(bank.iter().all(|row| {
            !row.is_empty()
                && row
                    .iter()
                    .all(|&(b, w)| b <= 512 && w > 0. && w.is_finite())
        }));
    }
}
