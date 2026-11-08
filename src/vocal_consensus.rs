//! Experimental recovery from two independent, already source-bound contours.
//! Neither lyrics nor authored chart notes are inputs. Existing voiced frames
//! are retained; callers run the shared note-centre decoder after recovery.
use crate::{
    vocal_generation::{PitchFrame, PitchSupport},
    vocal_pitch::{Evidence, Frame},
};
use anyhow::{Result, ensure};
const RATE: usize = 22050;
pub const ALGORITHM: &str = "two-model-recovery-v1; RMVPE>=0.1; CREPE>=0.5; agreement<=0.75st; local-energy; periodicity>=0.55; stable4of5; retain-existing-voice";
fn near(time: f64, frames: &[Frame]) -> Option<&Frame> {
    let at = frames.partition_point(|f| f.time_seconds < time);
    [at.saturating_sub(1), at.min(frames.len().saturating_sub(1))]
        .into_iter()
        .filter_map(|i| frames.get(i))
        .min_by(|a, b| {
            (a.time_seconds - time)
                .abs()
                .total_cmp(&(b.time_seconds - time).abs())
        })
        .filter(|f| (f.time_seconds - time).abs() <= 0.02)
}
fn midi(frame: &Frame, confidence: f32) -> Option<f64> {
    (frame.confidence >= confidence)
        .then_some(frame.frequency_hz)
        .flatten()
        .filter(|hz| hz.is_finite() && *hz > 0.)
        .map(|hz| 69. + 12. * (hz / 440.).log2())
        .filter(|p| (35.5..84.5).contains(p))
}
fn periodicity(signal: &[f32], time: f64, pitch: f64) -> f64 {
    let center = (time * RATE as f64).round() as usize;
    let lag = (RATE as f64 / (440. * 2f64.powf((pitch - 69.) / 12.))).round() as usize;
    if center < 1024 || center + 1024 > signal.len() || lag == 0 || lag >= 1024 {
        return 0.;
    }
    let values = &signal[center - 1024..center + 1024];
    let mean = values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64;
    let (mut cross, mut a, mut b) = (0., 0., 0.);
    for i in 0..values.len() - lag {
        let x = values[i] as f64 - mean;
        let y = values[i + lag] as f64 - mean;
        cross += x * y;
        a += x * x;
        b += y * y;
    }
    (cross / (a * b).sqrt().max(f64::EPSILON)).clamp(-1., 1.)
}
/// Model identity, file/PCM hashes and source clocks must be checked by the
/// caller before using this low-level stage. Returns recovered frame indices
/// so their uncertainty can survive subsequent note-centre decoding.
pub fn recover(
    frames: &mut [PitchFrame],
    signal: &[f32],
    primary: &Evidence,
    independent: &[Frame],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<usize>> {
    ensure!(!cancelled(), "Vocal recovery cancelled");
    ensure!(
        frames.len() <= 400_000
            && signal.len() <= RATE * 3600
            && !signal.is_empty()
            && signal.iter().all(|s| s.is_finite())
            && !primary.frames.is_empty()
            && !independent.is_empty(),
        "Invalid recovery input budget"
    );
    ensure!(
        frames.iter().all(|f| f.time_seconds.is_finite()
            && f.time_seconds >= 0.
            && f.time_seconds <= signal.len() as f64 / RATE as f64
            && f.midi
                .is_none_or(|p| p.is_finite() && (35.5..84.5).contains(&p)))
            && frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds),
        "Invalid recovery frame clock"
    );
    for contour in [&primary.frames[..], independent] {
        ensure!(
            contour.len() <= 360_001
                && contour.iter().all(|f| f.time_seconds.is_finite()
                    && f.time_seconds >= 0.
                    && f.confidence.is_finite()
                    && (0. ..=1.).contains(&f.confidence)
                    && f.frequency_hz.is_none_or(|p| p.is_finite() && p > 0.))
                && contour
                    .windows(2)
                    .all(|w| w[0].time_seconds < w[1].time_seconds),
            "Invalid independent pitch contour"
        );
    }
    let candidates: Vec<_> = frames
        .iter()
        .map(|f| {
            if f.midi.is_some() || f.local_energy_active != Some(true) {
                return None;
            }
            let p = midi(near(f.time_seconds, &primary.frames)?, 0.1)?;
            let other = midi(near(f.time_seconds, independent)?, 0.5)?;
            ((p - other).abs() <= 0.75).then_some(p)
        })
        .collect();
    let mut pending = Vec::new();
    for (index, pitch) in candidates.iter().enumerate() {
        if index % 1024 == 0 {
            ensure!(!cancelled(), "Vocal recovery cancelled");
        }
        let Some(pitch) = *pitch else { continue };
        let stable = (index.saturating_sub(2)..(index + 3).min(frames.len()))
            .filter(|&i| {
                let f = &frames[i];
                (f.time_seconds - frames[index].time_seconds).abs() <= 0.021
                    && f.local_energy_active == Some(true)
                    && near(f.time_seconds, &primary.frames)
                        .and_then(|f| midi(f, 0.1))
                        .is_some_and(|p| (p - pitch).abs() <= 0.5)
                    && near(f.time_seconds, independent)
                        .and_then(|f| midi(f, 0.5))
                        .is_some_and(|p| (p - pitch).abs() <= 0.75)
            })
            .count();
        if stable >= 4 && periodicity(signal, frames[index].time_seconds, pitch) >= 0.55 {
            pending.push((index, pitch));
        }
    }
    ensure!(!cancelled(), "Vocal recovery cancelled");
    for &(index, pitch) in &pending {
        frames[index].midi = Some(pitch);
        frames[index].support = PitchSupport::AcousticAlternative;
        frames[index].clarity = 0.5;
    }
    Ok(pending.into_iter().map(|(index, _)| index).collect())
}
