//! Model-free, local pulse analysis. Results are editable proposals, not musical truth.
use crate::timing::{Tempo, TempoMap};
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::path::Path;

const RATE: u32 = 11025;
const WINDOW: usize = 1024;
const HOP: usize = 256;
const FPS: f64 = RATE as f64 / HOP as f64;
const LOCAL_SECONDS: f64 = 12.0;
const STEP_SECONDS: f64 = 2.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_seconds: f64,
    pub bpm: f64,
    /// Normalized periodicity, not a probability of musical correctness.
    pub periodicity: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Estimate {
    pub bpm: Option<f64>,
    pub alternatives: Vec<f64>,
    pub periodicity: f64,
    pub note: String,
    /// Audio-time boundaries, retained in saved import information. Old imports
    /// containing only a single BPM remain readable.
    #[serde(default)]
    pub segments: Vec<Segment>,
}

impl Estimate {
    /// Convert only once, when creating the job, so retries use the same map.
    /// The first beat still needs human review; analysis does not infer downbeats.
    pub fn tempo_map(&self) -> Result<TempoMap> {
        ensure!(!self.segments.is_empty(), "No measured tempo map");
        let mut tick = 0.0_f64;
        let mut previous = &self.segments[0];
        ensure!(
            previous.start_seconds == 0.0,
            "Tempo map must start at zero"
        );
        let mut tempos = vec![Tempo {
            tick: 0,
            bpm: previous.bpm,
        }];
        for segment in &self.segments[1..] {
            ensure!(
                segment.start_seconds.is_finite() && segment.start_seconds > previous.start_seconds,
                "Tempo boundaries must increase"
            );
            tick += (segment.start_seconds - previous.start_seconds) * previous.bpm * 480.0 / 60.0;
            ensure!(
                tick.is_finite() && tick < u64::MAX as f64,
                "Tempo boundary is too large"
            );
            tempos.push(Tempo {
                tick: tick.round() as u64,
                bpm: segment.bpm,
            });
            previous = segment;
        }
        TempoMap::new(480, 0.0, tempos)
    }
}

pub fn analyze(path: &Path) -> Result<Estimate> {
    Ok(estimate(&crate::models::decode(path, RATE, 1)?))
}

/// Browser codecs supply interleaved PCM at the recording's sample rate.
/// Average source frames/channels onto the same analysis clock as desktop.
pub fn estimate_interleaved(samples: &[f32], rate: u32, channels: usize) -> Result<Estimate> {
    ensure!(
        rate > 0 && channels > 0 && samples.len().is_multiple_of(channels),
        "Invalid analysis PCM"
    );
    let frames = samples.len() / channels;
    let length = (frames as u64 * RATE as u64 / rate as u64) as usize;
    let mut mono = Vec::with_capacity(length);
    for i in 0..length {
        let begin = (i as u64 * rate as u64 / RATE as u64) as usize;
        let end = (((i + 1) as u64 * rate as u64 / RATE as u64) as usize)
            .max(begin + 1)
            .min(frames);
        let values = &samples[begin * channels..end * channels];
        mono.push(values.iter().filter(|v| v.is_finite()).sum::<f32>() / values.len() as f32);
    }
    Ok(estimate(&mono))
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    bpm: f64,
    score: f64,
    periodicity: f64,
}

fn candidates(onset: &[f64]) -> Vec<Candidate> {
    // Center the envelope: uncorrelated positive noise otherwise has a large
    // baseline correlation and can masquerade as a stable pulse.
    let mean = onset.iter().sum::<f64>() / onset.len() as f64;
    let centered = onset.iter().map(|v| v - mean).collect::<Vec<_>>();
    let onset = &centered;
    // Evaluate each integer lag once. Interpolating this small table avoids
    // rescanning audio for every quarter-BPM candidate in every local window.
    let max_lag = ((FPS * 60.0 / 60.0) * 3.0).ceil() as usize + 1;
    let mut correlations = vec![0.0; max_lag + 1];
    for (lag, correlation) in correlations.iter_mut().enumerate().skip(1) {
        if lag >= onset.len() {
            break;
        }
        let (mut dot, mut a, mut b) = (0.0, 0.0, 0.0);
        for i in lag..onset.len() {
            dot += onset[i] * onset[i - lag];
            a += onset[i] * onset[i];
            b += onset[i - lag] * onset[i - lag];
        }
        *correlation = dot / (a * b).sqrt().max(1e-12);
    }
    let correlation = |lag: f64| {
        let whole = lag.floor() as usize;
        let fraction = lag.fract();
        // Interpolating a pulse reduces its energy. Normalize that reduction,
        // or fractional lags unfairly lose to integer half-tempo harmonics.
        let normalization =
            (1.0 - 2.0 * fraction * (1.0 - fraction) * (1.0 - correlations[1])).sqrt();
        ((correlations[whole] * (1.0 - fraction) + correlations[whole + 1] * fraction)
            / normalization.max(1e-12))
        .min(1.0)
    };
    let mut result = (240..=800)
        .map(|quarter| {
            let bpm = quarter as f64 / 4.0;
            let lag = FPS * 60.0 / bpm;
            let periodicity = correlation(lag);
            let prior = 1.0 - 0.16 * ((bpm / 120.0).ln() / 2.0_f64.ln()).abs();
            Candidate {
                bpm,
                periodicity,
                score: (periodicity
                    + 0.35 * correlation(lag * 2.0)
                    + 0.15 * correlation(lag * 3.0))
                    * prior,
            }
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| b.score.total_cmp(&a.score));
    result
}

fn same_tempo(a: f64, b: f64) -> bool {
    (a - b).abs() <= (a * 0.02).max(1.0)
}
fn seconds(frame: usize) -> f64 {
    (frame * HOP + WINDOW / 2) as f64 / RATE as f64
}

/// Isolated transients locate a change more precisely than overlapping windows.
/// A local threshold also works across volume changes.
fn peaks(onset: &[f64]) -> Vec<f64> {
    let mut result = Vec::<usize>::new();
    for i in 1..onset.len().saturating_sub(1) {
        let local_max = onset
            [i.saturating_sub(FPS as usize)..(i + FPS as usize + 1).min(onset.len())]
            .iter()
            .copied()
            .fold(0.0_f64, f64::max);
        if onset[i] < 1e-5
            || onset[i] < local_max * 0.3
            || onset[i] <= onset[i - 1]
            || onset[i] < onset[i + 1]
        {
            continue;
        }
        if let Some(last) = result.last_mut()
            && i - *last < (FPS * 0.16) as usize
        {
            if onset[i] > onset[*last] {
                *last = i;
            }
        } else {
            result.push(i);
        }
    }
    result.into_iter().map(seconds).collect()
}

fn interval_error(delta: f64, bpm: f64) -> f64 {
    let beats = delta * bpm / 60.0;
    if !(0.5..=4.25).contains(&beats) {
        return 0.25;
    }
    (beats - beats.round()).abs().min(0.25)
}

fn boundary(peaks: &[f64], old: f64, new: f64, begin: f64, end: f64) -> f64 {
    let middle = (begin + end) / 2.0;
    let local = peaks
        .iter()
        .copied()
        .filter(|t| *t >= begin - 2.0 && *t <= end + 2.0)
        .collect::<Vec<_>>();
    if local.len() < 4 {
        return middle;
    }
    let errors = local
        .windows(2)
        .map(|p| {
            (
                interval_error(p[1] - p[0], old),
                interval_error(p[1] - p[0], new),
            )
        })
        .collect::<Vec<_>>();
    let mut cost = errors.iter().map(|e| e.1).sum::<f64>();
    let mut best = (f64::INFINITY, middle);
    for (i, &time) in local.iter().enumerate() {
        // Prefer the window midpoint when onset evidence ties, e.g. silence.
        let score = cost + (time - middle).abs() * 1e-6;
        if time >= begin && time <= end && score < best.0 {
            best = (score, time);
        }
        if let Some(error) = errors.get(i) {
            cost += error.0 - error.1;
        }
    }
    best.1
}

fn refine_bpm(peaks: &[f64], bpm: f64, begin: f64, end: f64) -> f64 {
    // Fit the longest consistent pulse chain to reduce long-section drift.
    let mut chain = Vec::<(f64, f64)>::new();
    let mut best = Vec::new();
    for time in peaks.iter().copied().filter(|t| *t >= begin && *t < end) {
        if let Some(&(beat, last)) = chain.last() {
            let beats = (time - last) * bpm / 60.0;
            if (1.0..=4.0).contains(&beats.round()) && (beats - beats.round()).abs() < 0.1 {
                chain.push((beat + beats.round(), time));
            } else {
                if chain.len() > best.len() {
                    best = chain.clone();
                }
                chain = vec![(0.0, time)];
            }
        } else {
            chain.push((0.0, time));
        }
    }
    if chain.len() > best.len() {
        best = chain;
    }
    if best.len() < 6 || best.last().unwrap().1 - best[0].1 < 4.0 {
        return bpm;
    }
    let n = best.len() as f64;
    let x = best.iter().map(|p| p.0).sum::<f64>() / n;
    let y = best.iter().map(|p| p.1).sum::<f64>() / n;
    let slope = best.iter().map(|p| (p.0 - x) * (p.1 - y)).sum::<f64>()
        / best.iter().map(|p| (p.0 - x).powi(2)).sum::<f64>();
    let refined = 60.0 / slope;
    if same_tempo(bpm, refined) {
        (refined * 1000.0).round() / 1000.0
    } else {
        bpm
    }
}

pub fn estimate(samples: &[f32]) -> Estimate {
    let missing = || Estimate {
        note:
            "No stable pulse was measured. A temporary 120 BPM map is suggested; edit it if needed."
                .into(),
        ..Default::default()
    };
    if samples.len() < RATE as usize * 8 {
        return missing();
    }
    let fft = FftPlanner::<f32>::new().plan_fft_forward(WINDOW);
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut bins = vec![Complex::new(0.0, 0.0); WINDOW];
    let mut previous = vec![0.0_f32; WINDOW / 2];
    let mut onset = Vec::new();
    let window: Vec<_> = (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (WINDOW - 1) as f32).cos())
        .collect();
    for start in (0..=samples.len() - WINDOW).step_by(HOP) {
        for (i, bin) in bins.iter_mut().enumerate() {
            let value = samples[start + i];
            let value = if value.is_finite() { value } else { 0.0 };
            *bin = Complex::new(value * window[i], 0.0);
        }
        fft.process_with_scratch(&mut bins, &mut scratch);
        let mut flux = 0.0;
        for i in 4..WINDOW / 2 - 1 {
            let power = (1.0 + bins[i].norm() * 10.0).ln();
            flux += (power - previous[i]).max(0.0);
            previous[i] = power;
        }
        onset.push(flux as f64);
    }
    let raw = onset.clone();
    for (i, value) in onset.iter_mut().enumerate() {
        let begin = i.saturating_sub(8);
        let end = (i + 9).min(raw.len());
        *value = (*value - raw[begin..end].iter().sum::<f64>() / (end - begin) as f64).max(0.0);
    }
    if onset.iter().map(|v| v * v).sum::<f64>() < 1e-6 {
        return missing();
    }
    let overall = candidates(&onset);
    let window = ((LOCAL_SECONDS * FPS) as usize).min(onset.len());
    let step = (STEP_SECONDS * FPS) as usize;
    let mut starts = (0..=onset.len() - window).step_by(step).collect::<Vec<_>>();
    if starts.last() != Some(&(onset.len() - window)) {
        starts.push(onset.len() - window);
    }
    // Three agreeing windows establish a sustained change. A single ambiguous
    // window, fill, dropout or half/double-time alternative cannot add a marker.
    let mut groups = Vec::<Vec<(f64, Candidate)>>::new();
    let mut previous_bpm = None;
    for start in starts {
        let options = candidates(&onset[start..start + window]);
        let mut best = options[0];
        if best.periodicity < 0.2 {
            continue;
        }
        if let Some(previous) = previous_bpm {
            // Keep a consistent beat unit when octave alternatives are similarly
            // supported. Audio alone cannot resolve half/double-time intent.
            for scale in [0.5, 2.0] {
                if same_tempo(best.bpm * scale, previous) {
                    best.bpm *= scale;
                    break;
                }
            }
            if let Some(option) = options.iter().find(|c| {
                same_tempo(c.bpm, previous) && c.score >= best.score * 0.9 && c.periodicity >= 0.2
            }) {
                best = *option;
            }
        }
        let time = seconds(start + window / 2);
        if groups.last().is_none_or(|group| {
            !same_tempo(group[0].1.bpm, best.bpm)
                || time - group.last().unwrap().0 > STEP_SECONDS * 1.6
        }) {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push((time, best));
        previous_bpm = Some(best.bpm);
    }
    let peaks = peaks(&onset);
    let stable = groups
        .iter()
        .filter(|g| g.len() >= 3 || (onset.len() <= window + step && !g.is_empty()))
        .collect::<Vec<_>>();
    let mut segments = Vec::<Segment>::new();
    let mut previous_end = 0.0;
    for group in stable {
        let mut bpms = group.iter().map(|p| p.1.bpm).collect::<Vec<_>>();
        bpms.sort_by(f64::total_cmp);
        let bpm = bpms[bpms.len() / 2];
        if let Some(last) = segments.last()
            && same_tempo(last.bpm, bpm)
        {
            previous_end = group.last().unwrap().0;
            continue;
        }
        let start_seconds = segments.last().map_or(0.0, |last| {
            boundary(
                &peaks,
                last.bpm,
                bpm,
                (previous_end - LOCAL_SECONDS / 2.0).max(last.start_seconds + STEP_SECONDS),
                group[0].0 + LOCAL_SECONDS / 2.0,
            )
        });
        segments.push(Segment {
            start_seconds,
            bpm,
            periodicity: group.iter().map(|p| p.1.periodicity).sum::<f64>() / group.len() as f64,
        });
        previous_end = group.last().unwrap().0;
    }
    if segments.is_empty() {
        if overall[0].periodicity < 0.2 {
            return missing();
        }
        segments.push(Segment {
            start_seconds: 0.0,
            bpm: overall[0].bpm,
            periodicity: overall[0].periodicity,
        });
    }
    for i in 0..segments.len() {
        let end = segments
            .get(i + 1)
            .map_or(samples.len() as f64 / RATE as f64, |s| s.start_seconds);
        segments[i].bpm = refine_bpm(&peaks, segments[i].bpm, segments[i].start_seconds, end);
    }
    let bpm = segments[0].bpm;
    let alternatives = overall
        .iter()
        .filter(|c| (c.bpm - bpm).abs() > 5.0 && c.score > overall[0].score * 0.8)
        .fold(Vec::<f64>::new(), |mut list, c| {
            if list.len() < 3 && list.iter().all(|v| (*v - c.bpm).abs() > 5.0) {
                list.push(c.bpm);
            }
            list
        });
    Estimate {
        bpm: Some(bpm),
        alternatives,
        periodicity: segments[0].periodicity,
        note: format!(
            "Audio-derived proposal: {} tempo change(s). Review Timing against the recording; half/double tempo, weak beats and the first beat may need correction.",
            segments.len() - 1
        ),
        segments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clicks(sections: &[(f64, f64)], duration: f64) -> Vec<f32> {
        let mut samples = vec![0.0; (duration * RATE as f64) as usize];
        for (i, &(start, bpm)) in sections.iter().enumerate() {
            let end = sections.get(i + 1).map_or(duration - 0.1, |s| s.0);
            let mut beat = start;
            while beat < end - 1e-6 {
                let start = (beat * RATE as f64) as usize;
                for j in 0..600.min(samples.len() - start) {
                    samples[start + j] += ((j as f32 * 0.7).sin() + (j as f32 * 1.73).sin())
                        * (-(j as f32) / 90.0).exp();
                }
                beat += 60.0 / bpm;
            }
        }
        samples
    }

    #[test]
    fn constant_tempos_stay_constant_and_silence_has_no_pulse() {
        for bpm in [90.0, 120.0, 150.0] {
            let result = estimate(&clicks(&[(1.0, bpm)], 36.0));
            assert!((result.bpm.unwrap() - bpm).abs() < 0.2, "{bpm}: {result:?}");
            assert_eq!(result.segments.len(), 1, "{result:?}");
        }
        assert!(estimate(&vec![0.0; RATE as usize * 12]).bpm.is_none());
        assert!(
            estimate(&vec![f32::NAN; RATE as usize * 12])
                .segments
                .is_empty()
        );
        assert!(estimate(&[0.0; 10]).bpm.is_none());
    }

    #[test]
    fn detects_multiple_changes_and_maps_audio_times_through_chart_and_midi() {
        let expected = [(0.0, 120.0), (24.0, 90.0), (48.0, 150.0), (72.0, 120.0)];
        let result = estimate(&clicks(&expected, 96.0));
        assert_eq!(result.segments.len(), expected.len(), "{result:?}");
        let map = result.tempo_map().unwrap();
        for ((start, bpm), (segment, tempo)) in
            expected.iter().zip(result.segments.iter().zip(&map.tempos))
        {
            assert!((segment.bpm - bpm).abs() < 0.3, "{result:?}");
            assert!((segment.start_seconds - start).abs() < 0.15, "{result:?}");
            assert!((map.seconds(tempo.tick) - start).abs() < 0.16);
        }
        let chart = crate::chart::Chart::parse(crate::chart::new_chart(
            "Tempo",
            "Test",
            &map,
            &Default::default(),
        ))
        .unwrap();
        let midi = crate::midi::timing(&crate::midi::new_midi(&map, &Default::default()).unwrap())
            .unwrap();
        for second in [1.0, 23.9, 24.1, 47.9, 48.1, 71.9, 72.1, 95.0] {
            let tick = map.tick(second).unwrap();
            assert!((chart.timing.seconds(tick) - second).abs() < 0.002);
            assert!((midi.seconds(tick) - second).abs() < 0.002);
        }
    }

    #[test]
    fn detects_smaller_sustained_change() {
        let result = estimate(&clicks(&[(0.0, 120.0), (30.0, 128.0)], 60.0));
        assert_eq!(result.segments.len(), 2, "{result:?}");
        assert!((result.segments[1].bpm - 128.0).abs() < 0.3, "{result:?}");
        assert!(
            (result.segments[1].start_seconds - 30.0).abs() < 0.15,
            "{result:?}"
        );
    }

    #[test]
    fn silence_volume_changes_and_short_fills_do_not_create_markers() {
        let mut samples = clicks(&[(0.0, 120.0)], 72.0);
        samples[RATE as usize * 24..RATE as usize * 36].fill(0.0);
        for sample in &mut samples[RATE as usize * 36..] {
            *sample *= 0.2;
        }
        let fill = clicks(&[(0.0, 170.0)], 2.0);
        for (sample, extra) in samples[RATE as usize * 50..].iter_mut().zip(fill) {
            *sample += extra;
        }
        let result = estimate(&samples);
        assert_eq!(result.segments.len(), 1, "{result:?}");
        assert!((result.bpm.unwrap() - 120.0).abs() < 0.3);
    }

    #[test]
    fn old_estimates_deserialize_and_malformed_boundaries_are_rejected() {
        let old: Estimate = serde_json::from_str(
            r#"{"bpm":120.0,"alternatives":[],"periodicity":0.5,"note":"legacy"}"#,
        )
        .unwrap();
        assert!(old.segments.is_empty());
        assert!(old.tempo_map().is_err());
        let mut result = estimate(&clicks(&[(0.0, 120.0), (24.0, 90.0)], 48.0));
        result.segments[1].start_seconds = -1.0;
        assert!(result.tempo_map().is_err());
    }

    #[test]
    fn uncorrelated_noise_has_no_measured_map() {
        let mut seed = 17_u32;
        let noise = (0..RATE as usize * 30)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as f64 / u32::MAX as f64 - 0.5) as f32 * 0.2
            })
            .collect::<Vec<_>>();
        let result = estimate(&noise);
        assert!(result.segments.is_empty(), "{result:?}");
    }

    #[test]
    fn browser_pcm_uses_the_same_analysis_clock() {
        let mono = clicks(&[(0.0, 120.0), (24.0, 90.0)], 48.0);
        let stereo = mono
            .iter()
            .flat_map(|sample| [*sample; 8])
            .collect::<Vec<_>>();
        let native = estimate(&mono);
        let browser = estimate_interleaved(&stereo, RATE * 4, 2).unwrap();
        assert_eq!(native.tempo_map().unwrap(), browser.tempo_map().unwrap());
        for (a, b) in native.segments.iter().zip(&browser.segments) {
            assert!((a.start_seconds - b.start_seconds).abs() < 1e-6);
            assert!((a.periodicity - b.periodicity).abs() < 1e-6);
        }
        assert!(estimate_interleaved(&[], 0, 2).is_err());
        assert!(estimate_interleaved(&[0.0], RATE, 2).is_err());
    }

    #[test]
    fn changing_beat_density_does_not_flip_between_octave_tempos() {
        let mut samples = clicks(&[(0.0, 175.0)], 72.0);
        // Alternate-beat accents gradually become the only audible pulse.
        for (i, sample) in samples.iter_mut().enumerate() {
            let time = i as f64 / RATE as f64;
            let beat = (time * 175.0 / 60.0).floor() as usize;
            if (24.0..48.0).contains(&time) && beat % 2 == 1 {
                *sample *= 0.01;
            }
        }
        let result = estimate(&samples);
        assert_eq!(result.segments.len(), 1, "{result:?}");
    }

    #[test]
    fn changes_remain_detectable_with_tonal_backing_and_noise() {
        let mut samples = clicks(&[(0.0, 120.0), (24.0, 90.0), (48.0, 150.0)], 72.0);
        let mut seed = 23_u32;
        for (i, sample) in samples.iter_mut().enumerate() {
            let time = i as f64 / RATE as f64;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed as f64 / u32::MAX as f64 - 0.5) * 0.04;
            let tone = (std::f64::consts::TAU * 220.0 * time).sin() * 0.08
                + (std::f64::consts::TAU * 329.63 * time).sin() * 0.04;
            *sample += (noise + tone) as f32;
        }
        let result = estimate(&samples);
        assert_eq!(result.segments.len(), 3, "{result:?}");
        for (segment, (start, bpm)) in
            result
                .segments
                .iter()
                .zip([(0.0, 120.0), (24.0, 90.0), (48.0, 150.0)])
        {
            assert!((segment.bpm - bpm).abs() < 0.5, "{result:?}");
            assert!((segment.start_seconds - start).abs() < 0.2, "{result:?}");
        }
    }
}
