//! Conservative monophonic-bass harmonic correction using the existing Rust
//! YIN/McLeod detectors and measured fundamental energy. YIN review of refined
//! output is a consistency check, not independent accuracy evidence.
use crate::mapping::{
    BassAttackReview, BassCorrection, BassFundamentalSupport, BassHarmonicConflict,
    BassShadowMethod, BassShadowSupport, BassTimingCorrection, Transcription,
};
use anyhow::Result;
use pitch_detection::detector::{PitchDetector, mcleod::McLeodDetector, yin::YINDetector};
use std::path::Path;

pub fn bass(events: &mut Transcription, audio: &Path) -> Result<()> {
    let signal = crate::models::decode(audio, 11025, 1)?;
    correct_bass(events, &signal);
    assess_attacks(events, &crate::models::decode(audio, 22050, 1)?);
    Ok(())
}

pub fn correct_bass(events: &mut Transcription, audio: &[f32]) {
    let previous_corrections = events.bass_corrections.len();
    let mut attacks: Vec<_> = events.bass.iter().map(|note| note.start).collect();
    attacks.sort_by(f64::total_cmp);
    attacks.dedup();
    let source_notes = events.bass.clone();
    for note in &mut events.bass {
        let shadows: Vec<_> = source_notes
            .iter()
            .filter(|later| later.start > note.start + 0.025 && later.start <= note.start + 0.055)
            .filter_map(|later| harmonic_shadow_support(note, later, audio))
            .collect();
        // Keep pitch-check samples inside this note.
        // A window crossing its edges could borrow a neighbour's pitch.
        let next_attack = attacks.iter().copied().find(|time| {
            *time > note.start + 0.025
                && !shadows.iter().any(|support| support.later.start == *time)
        });
        let end = next_attack.map_or(note.end, |next| note.end.min(next));
        let first = ((note.start + 0.012) * 11025.).ceil() as usize;
        let last = (((end - 0.008).max(0.) * 11025.).floor() as usize).min(audio.len());
        let available = last.saturating_sub(first);
        // A long window needs room to move away from the pluck transient.
        // Otherwise three almost identical windows repeat the same corruption.
        let size = if available >= 3072 { 2048 } else { 1024 };
        if available < size + 64 {
            continue;
        }
        let mut detector = McLeodDetector::<f64>::new(size, size / 2);
        let mut yin = YINDetector::<f64>::new(size, size / 2);
        let mut estimates = Vec::new();
        // Limit the inspected sustain to the first 600 ms, but retain two
        // distinct windows. This also admits short, well-supported bass notes.
        let span = available.min((0.6 * 11025.) as usize).max(size);
        for start in [first, first + (span - size) / 2, first + (span - size)] {
            let signal: Vec<_> = audio[start..start + size]
                .iter()
                .map(|v| *v as f64)
                .collect();
            let p = detector.get_pitch(&signal, 11025, 0.01, 0.85);
            let Some(y) = yin.get_pitch(&signal, 11025, 0.01, 0.90) else {
                continue;
            };
            if !(32.0..400.0).contains(&y.frequency) {
                continue;
            }
            // At least 3.5 periods: low pitches need a longer owned interval.
            if size as f64 * y.frequency / 11025. < 3.5 {
                continue;
            }
            let clarity = periodicity(&signal, y.frequency, 11025.0);
            if clarity < 0.94 || !clarity.is_finite() {
                continue;
            }
            let midi = 69.0 + 12.0 * (y.frequency / 440.0).log2();
            if let Some(p) = p {
                if !(32.0..800.0).contains(&p.frequency) || !p.clarity.is_finite() {
                    continue;
                }
                let mcleod_midi = 69.0 + 12.0 * (p.frequency / 440.0).log2();
                if ![0.0, 12.0, 19.01955, 24.0, 27.86314, 31.01955, 36.0]
                    .iter()
                    .any(|h| (mcleod_midi - midi - h).abs() < 0.35)
                {
                    continue;
                }
            }
            // A mathematically periodic subharmonic is not enough: require
            // measurable spectral energy at the proposed fundamental.
            let power = |multiple: f64| {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, s) in signal.iter().enumerate() {
                    let window =
                        0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (size - 1) as f64).cos();
                    let phase = std::f64::consts::TAU * y.frequency * multiple * i as f64 / 11025.0;
                    re += s * window * phase.cos();
                    im += s * window * phase.sin();
                }
                re * re + im * im
            };
            let fundamental = power(1.0);
            let strongest = (2..=5).map(|h| power(h as f64)).fold(fundamental, f64::max);
            if fundamental < strongest * 0.04 {
                continue;
            }
            estimates.push((midi, clarity));
        }
        if estimates.len() < 2
            || estimates
                .iter()
                .any(|estimate| (estimate.0 - estimates[0].0).abs() > 0.30)
        {
            continue;
        }
        let midi =
            estimates.iter().map(|estimate| estimate.0).sum::<f64>() / estimates.len() as f64;
        let rounded = midi.round();
        if (midi - rounded).abs() > 0.35 || !(24.0..=67.0).contains(&rounded) {
            continue;
        }
        let difference = note.pitch as f64 - midi;
        // Only correct an upper harmonic supported by stable periodicity.
        // Never raise a fundamental, or rewrite an unrelated melodic pitch.
        if ![12.0, 19.01955, 24.0, 27.86314, 31.01955, 36.0]
            .iter()
            .any(|h| (difference - h).abs() < 0.35)
        {
            continue;
        }
        events.bass_corrections.push(BassCorrection {
            original: note.clone(),
            revised_pitch: rounded as u8,
            periodic_pitch: midi,
            clarity: estimates
                .iter()
                .map(|estimate| estimate.1)
                .fold(1., f64::min),
        });
        events.bass_shadow_support.extend(shadows);
        note.pitch = rounded as u8;
    }
    let added = events.bass_corrections.len() - previous_corrections;
    if added > 0 {
        events.warnings.push(format!("{added} bass upper-harmonic candidates lowered using two note-owned YIN windows plus measured fundamental energy, with McLeod harmonic-family agreement when available; original events retained in bass_corrections. Windows stop before the next distinct attack. Short or ambiguous notes remain unchanged. YIN review is a consistency check, not independent accuracy evidence. Listening review remains required."));
    }
    correct_attacks(events, audio);
    assess_harmonic_conflicts(events);
}

/// Only unresolved close harmonic pairs are retained. Corrected shadows now
/// have the same fundamental pitch and therefore cannot create a second flag.
fn assess_harmonic_conflicts(events: &mut Transcription) {
    events.bass_harmonic_conflicts = events
        .bass
        .iter()
        .flat_map(|candidate| {
            events
                .bass
                .iter()
                .filter(move |other| is_close_harmonic_pair(candidate, other))
                .map(move |competing_candidate| BassHarmonicConflict {
                    candidate: candidate.clone(),
                    competing_candidate: competing_candidate.clone(),
                })
        })
        .collect();
}

pub(crate) fn is_close_harmonic_pair(
    earlier: &crate::mapping::PitchNote,
    later: &crate::mapping::PitchNote,
) -> bool {
    let gap = later.start - earlier.start;
    (0.025..=0.055).contains(&gap)
        && later.confidence >= earlier.confidence * 1.25
        && earlier.end.min(later.end) - later.start >= 0.10
        && [12., 19., 24., 28., 31., 36.]
            .iter()
            .any(|harmonic| ((earlier.pitch as f64 - later.pitch as f64) - harmonic).abs() < 0.1)
}

/// A weak upper detection can precede the stronger fundamental activation by
/// a few model frames. Treating that activation as a new physical attack both
/// blocks pitch evidence and lets the 55 ms chart floor retain the wrong octave.
/// Never infer this from overlap alone: the lower fundamental must already be
/// measurable strictly BEFORE its predicted onset, and the ordinary two-window
/// periodicity checks still decide whether to change the earlier event.
#[cfg(test)]
fn harmonic_shadow(
    earlier: &crate::mapping::PitchNote,
    later: &crate::mapping::PitchNote,
    audio: &[f32],
) -> bool {
    harmonic_shadow_support(earlier, later, audio).is_some()
}

fn harmonic_shadow_support(
    earlier: &crate::mapping::PitchNote,
    later: &crate::mapping::PitchNote,
    audio: &[f32],
) -> Option<BassShadowSupport> {
    if !is_close_harmonic_pair(earlier, later) {
        return None;
    }
    let frequency = 440. * 2f64.powf((later.pitch as f64 - 69.) / 12.);
    let lo = ((earlier.start + 0.003) * 11025.).ceil() as usize;
    let hi = (((later.start - 0.003).max(0.) * 11025.).floor() as usize).min(audio.len());
    if hi <= lo || (hi - lo) as f64 * frequency / 11025. < 1.75 {
        return None;
    }
    let signal = &audio[lo..hi];
    let rms = (signal
        .iter()
        .map(|sample| (*sample as f64).powi(2))
        .sum::<f64>()
        / signal.len() as f64)
        .sqrt();
    if rms < 0.006 {
        return None;
    }
    let power = |harmonic: f64| {
        let (mut re, mut im) = (0., 0.);
        for (index, sample) in signal.iter().enumerate() {
            let window = 0.5
                - 0.5 * (std::f64::consts::TAU * index as f64 / (signal.len() - 1) as f64).cos();
            let phase = std::f64::consts::TAU * frequency * harmonic * index as f64 / 11025.;
            re += *sample as f64 * window * phase.cos();
            im += *sample as f64 * window * phase.sin();
        }
        re * re + im * im
    };
    let fundamental = power(1.);
    let strongest = (2..=5)
        .map(|harmonic| power(harmonic as f64))
        .fold(fundamental, f64::max);
    let fraction = fundamental / strongest.max(f64::EPSILON);
    if fundamental > f64::EPSILON && fraction >= 0.12 {
        return Some(BassShadowSupport {
            earlier: earlier.clone(),
            later: later.clone(),
            early_fundamental_fraction: fraction,
            method: BassShadowMethod::StrongProjection,
            joint_fit: None,
        });
    }
    let support = early_fundamental_support(earlier, later, audio)?;
    if support.early_explained < 0.90
        || support.later_explained < 0.95
        || support.fundamental_fraction < 0.04
    {
        return None;
    }
    let method = if support.added_variance_fraction >= 0.02
        && support.phase_error_radians <= std::f64::consts::PI / 6.
    {
        BassShadowMethod::JointPhase
    } else if support.added_variance_fraction >= 0.10
        && support.phase_error_radians <= std::f64::consts::FRAC_PI_2
    {
        BassShadowMethod::JointTransient
    } else {
        return None;
    };
    Some(BassShadowSupport {
        earlier: earlier.clone(),
        later: later.clone(),
        early_fundamental_fraction: fraction,
        method,
        joint_fit: Some(support),
    })
}

#[path = "refine/harmonics.rs"]
mod harmonics;
use harmonics::harmonic_fit;

fn early_fundamental_support(
    earlier: &crate::mapping::PitchNote,
    later: &crate::mapping::PitchNote,
    audio: &[f32],
) -> Option<BassFundamentalSupport> {
    let frequency = 440. * 2f64.powf((later.pitch as f64 - 69.) / 12.);
    let lo = ((earlier.start + 0.003) * 11025.).ceil() as usize;
    let hi = (((later.start - 0.003).max(0.) * 11025.).floor() as usize).min(audio.len());
    let stable = ((later.start + 0.012) * 11025.).ceil() as usize;
    if hi <= lo
        || (hi - lo) as f64 * frequency / 11025. < 1.75
        || stable + 1024 > audio.len()
        || (stable + 1024) as f64 / 11025. > earlier.end.min(later.end) - 0.008
    {
        return None;
    }
    let early = harmonic_fit(&audio[lo..hi], frequency, true)?;
    let without = harmonic_fit(&audio[lo..hi], frequency, false)?;
    let after = harmonic_fit(&audio[stable..stable + 1024], frequency, true)?;
    let phase = (early.phase - after.phase
        + std::f64::consts::TAU * frequency * (stable - lo) as f64 / 11025.)
        .rem_euclid(std::f64::consts::TAU);
    Some(BassFundamentalSupport {
        early_explained: 1. - early.residual / early.variance.max(1e-30),
        later_explained: 1. - after.residual / after.variance.max(1e-30),
        added_variance_fraction: (without.residual - early.residual) / early.variance.max(1e-30),
        fundamental_fraction: early.fundamental_power / early.strongest_power.max(1e-30),
        phase_error_radians: phase.min(std::f64::consts::TAU - phase),
    })
}

/// Sample-domain boundary refinement, deliberately restricted to clear attacks
/// out of a quiet baseline. A centered FFT peak is not an onset timestamp.
/// Ringing, legato, competing transients and pitch-ambiguous attacks are left
/// alone rather than moved to the nearest energy peak.
fn correct_attacks(events: &mut Transcription, audio: &[f32]) {
    const RATE: f64 = 11025.;
    let mut energy = Vec::with_capacity(audio.len() + 1);
    energy.push(0.);
    for sample in audio {
        energy.push(energy.last().unwrap() + (*sample as f64).powi(2));
    }
    let level = |start: usize, end: usize| {
        if end <= start || end >= energy.len() {
            0.
        } else {
            (energy[end] - energy[start]) / (end - start) as f64
        }
    };
    let mut attacks: Vec<_> = events.bass.iter().map(|note| note.start).collect();
    attacks.sort_by(f64::total_cmp);
    attacks.dedup();
    for note in &mut events.bass {
        let preceding = attacks
            .iter()
            .copied()
            .rev()
            .find(|time| *time < note.start - 0.025);
        let following = attacks
            .iter()
            .copied()
            .find(|time| *time > note.start + 0.025);
        let lo = (note.start - 0.09).max(preceding.map_or(0., |time| time + 0.055));
        let hi = (note.start + 0.09)
            .min(following.map_or(note.end, |time| time - 0.055))
            .min(note.end - 0.12);
        if hi <= lo {
            continue;
        }
        let width = (0.025 * RATE) as usize;
        let mut candidates = Vec::new();
        for sample in ((lo * RATE) as usize..=(hi * RATE) as usize).step_by(11) {
            if sample < width || sample + width >= audio.len() {
                continue;
            }
            let before = level(sample - width, sample);
            let after = level(sample, sample + width);
            // At least 12 dB of RMS rise (4:1). Smaller changes on ringing
            // notes have an ill-defined boundary and remain model-timed.
            if after >= 0.000036 && before < after * 0.0625 {
                candidates.push((sample, after - before, before, after));
            }
        }
        let Some(&(peak, strength, before, after)) =
            candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        if candidates
            .iter()
            .any(|candidate| candidate.0.abs_diff(peak) > width && candidate.1 >= strength * 0.75)
        {
            continue;
        }
        // Locate the rising edge in small blocks, then require independent
        // periodic pitch agreement after that edge. This avoids centering a
        // 93/186 ms detector window on a supposed attack.
        let small = (0.004 * RATE) as usize;
        let edge = (peak.saturating_sub(width)..=(peak + small).min(audio.len() - small))
            .step_by(11)
            .find(|sample| level(*sample, *sample + small) >= after * 0.20 + before * 2.);
        let Some(edge) = edge else {
            continue;
        };
        let revised = edge as f64 / RATE;
        if (revised - note.start).abs() < 0.008 || (revised - note.start).abs() > 0.09 {
            continue;
        }
        if preceding.is_some_and(|time| revised - time < 0.055)
            || following.is_some_and(|time| time - revised < 0.055)
        {
            continue;
        }
        let start = edge + (0.016 * RATE) as usize;
        let end = start + 1024;
        if end >= audio.len()
            || end as f64 / RATE > note.end.min(following.unwrap_or(note.end)) - 0.008
        {
            continue;
        }
        let signal: Vec<_> = audio[start..end]
            .iter()
            .map(|sample| *sample as f64)
            .collect();
        let mut detector = YINDetector::<f64>::new(1024, 512);
        let Some(pitch) = detector.get_pitch(&signal, RATE as usize, 0.01, 0.94) else {
            continue;
        };
        let midi = 69. + 12. * (pitch.frequency / 440.).log2();
        if !midi.is_finite()
            || (midi - note.pitch as f64).abs() > 0.35
            || periodicity(&signal, pitch.frequency, RATE) < 0.94
        {
            continue;
        }
        events.bass_timing_corrections.push(BassTimingCorrection {
            original: note.clone(),
            revised_start: revised,
            before_rms: before.sqrt(),
            after_rms: after.sqrt(),
            periodic_pitch: midi,
        });
        note.start = revised;
    }
}

/// Independent transient support is review evidence, not a deletion rule:
/// gentle real attacks may be absent from the spectral-flux peak list.
fn assess_attacks(events: &mut Transcription, audio_22050: &[f32]) {
    let peaks = crate::review::attacks(audio_22050);
    events.bass_attack_reviews = events
        .bass
        .iter()
        .map(|note| {
            let peak = crate::review::nearest(note.start, &peaks);
            let residual = peak.map(|peak| peak.time - note.start);
            BassAttackReview {
                candidate: note.clone(),
                nearest_attack_seconds: peak.map(|peak| peak.time),
                residual_seconds: residual,
                supported: residual.is_some_and(|residual| residual.abs() <= 0.050),
            }
        })
        .collect();
}

pub(crate) fn periodicity(signal: &[f64], frequency: f64, rate: f64) -> f64 {
    let lag = (rate / frequency).round() as usize;
    let comparison = signal.len() / 2;
    if lag == 0 || comparison == 0 || lag + comparison > signal.len() {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut difference = 0.0;
    for tau in 1..=lag {
        difference = (0..comparison)
            .map(|i| (signal[i] - signal[i + tau]).powi(2))
            .sum::<f64>();
        sum += difference;
    }
    1.0 - difference * lag as f64 / sum.max(f64::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::PitchNote;

    /// Generates candidates from an oracle instrument stem only; reference chart
    /// attacks are deliberately not inputs to this stage of the benchmark.
    #[test]
    #[ignore = "requires explicit CHART_ENGINE_BASS_ORACLE_STEM/MODEL/OUTPUT"]
    fn infer_oracle_bass_candidates() {
        let stem = std::env::var("CHART_ENGINE_BASS_ORACLE_STEM").unwrap();
        let model = std::env::var("CHART_ENGINE_BASS_ORACLE_MODEL").unwrap();
        let output = std::env::var("CHART_ENGINE_BASS_ORACLE_OUTPUT").unwrap();
        let short = std::env::var_os("CHART_ENGINE_BASS_ORACLE_SHORT").is_some();
        let inference = if short {
            crate::models::bass_pitch_with_evidence(Path::new(&stem), Path::new(&model)).unwrap()
        } else {
            crate::models::BassPitchInference {
                candidates: crate::models::pitch_with_evidence(Path::new(&stem), Path::new(&model))
                    .unwrap(),
                short_candidates: vec![],
            }
        };
        let mut events = Transcription {
            bass: inference
                .candidates
                .iter()
                .map(|candidate| candidate.candidate.clone())
                .collect(),
            bass_candidates: inference.candidates,
            bass_short_candidates: inference.short_candidates,
            ..Default::default()
        };
        bass(&mut events, Path::new(&stem)).unwrap();
        events.validate().unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .unwrap();
        serde_json::to_writer_pretty(file, &events).unwrap();
        std::fs::write(
            format!("{output}.source.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "bass_stem_sha256":crate::models::hash_file(Path::new(&stem)).unwrap(),
                "pitch_model_sha256":crate::models::hash_file(Path::new(&model)).unwrap(),
                "events_sha256":crate::models::hash_file(Path::new(&output)).unwrap(),
                "reference_chart_used_for_inference":false,
                "bass_short_decoder":short,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    /// Replay recorded model candidates against the same source, without a
    /// reference chart or a model rerun entering the refinement stage.
    #[test]
    #[ignore = "requires explicit BASS_REPLAY_INPUT/STEM/OUTPUT"]
    fn replay_recorded_bass_candidates() {
        let input = std::env::var("BASS_REPLAY_INPUT").unwrap();
        let stem = std::env::var("BASS_REPLAY_STEM").unwrap();
        let output = std::env::var("BASS_REPLAY_OUTPUT").unwrap();
        let original: Transcription =
            serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
        assert!(!original.bass_candidates.is_empty());
        let mut events = Transcription {
            bass: original
                .bass_candidates
                .iter()
                .map(|item| item.candidate.clone())
                .collect(),
            bass_candidates: original.bass_candidates,
            bass_short_candidates: original.bass_short_candidates,
            ..Default::default()
        };
        bass(&mut events, Path::new(&stem)).unwrap();
        events.validate().unwrap();
        let timing = crate::timing::TempoMap::constant(120.).unwrap();
        let tracks = crate::mapping::tracks(&events, &timing, false).unwrap();
        let chart = crate::chart::new_chart("Source-bound bass replay", "", &timing, &tracks);
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .unwrap();
        serde_json::to_writer_pretty(file, &events).unwrap();
        let chart_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(format!("{output}.chart"))
            .unwrap();
        std::io::Write::write_all(&mut &chart_file, chart.as_bytes()).unwrap();
        let receipt = serde_json::json!({
            "input_events_sha256":crate::models::hash_file(Path::new(&input)).unwrap(),
            "bass_stem_sha256":crate::models::hash_file(Path::new(&stem)).unwrap(),
            "events_sha256":crate::models::hash_file(Path::new(&output)).unwrap(),
            "reference_chart_used_for_inference":false, "mapping_bpm":120.,
            "refine_source_sha256":crate::models::hash_file(Path::new(concat!(env!("CARGO_MANIFEST_DIR"),"/src/refine.rs"))).unwrap(),
        });
        std::fs::write(
            format!("{output}.source.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }

    /// Deliberately diagnostic ablation: test whether treating review-only
    /// spectral-flux absence as a deletion rule actually preserves authored
    /// recall. This is never called by generation or refinement.
    #[test]
    #[ignore = "requires explicit CHART_ENGINE_BASS_FILTER_INPUT/OUTPUT"]
    fn evaluate_flux_deletion_ablation() {
        let input = std::env::var("CHART_ENGINE_BASS_FILTER_INPUT").unwrap();
        let output = std::env::var("CHART_ENGINE_BASS_FILTER_OUTPUT").unwrap();
        let mut events: Transcription =
            serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
        let unsupported: Vec<_> = events
            .bass_attack_reviews
            .iter()
            .filter(|receipt| !receipt.supported)
            .map(|receipt| receipt.candidate.clone())
            .collect();
        let before = events.bass.len();
        events.bass.retain(|note| {
            !unsupported.iter().any(|other| {
                other.start == note.start && other.end == note.end && other.pitch == note.pitch
            })
        });
        events.warnings.push("DIAGNOSTIC ONLY: absent spectral-flux support deleted notes for benchmark ablation; not a production policy.".into());
        events.validate().unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .unwrap();
        serde_json::to_writer_pretty(file, &events).unwrap();
        eprintln!(
            "flux deletion ablation: {before} -> {} raw notes",
            events.bass.len()
        );
    }

    #[test]
    fn missing_flux_support_preserves_a_known_soft_physical_attack() {
        let mut source = vec![0.; 22050];
        for (index, sample) in source.iter_mut().enumerate() {
            let t = index as f64 / 22050.;
            if (0.10..0.12).contains(&t) {
                *sample = (0.7 * (std::f64::consts::TAU * 3179. * t).sin()) as f32;
            } else if (0.5..0.8).contains(&t) {
                *sample = (0.0001 * (std::f64::consts::TAU * 82.40689 * (t - 0.5)).sin()) as f32;
            }
        }
        let note = PitchNote {
            start: 0.5,
            end: 0.8,
            pitch: 40,
            confidence: 0.9,
        };
        let mut events = Transcription {
            bass: vec![note.clone()],
            ..Default::default()
        };
        assess_attacks(&mut events, &source);
        assert!(!events.bass_attack_reviews[0].supported);
        assert_eq!(events.bass.len(), 1);
        assert_eq!(events.bass[0].start, note.start);
    }

    fn tone(start: f64, end: f64, pitch: u8, amplitude: f32, audio: &mut [f32]) {
        let frequency = 440. * 2f32.powf((pitch as f32 - 69.) / 12.);
        for (i, sample) in audio.iter_mut().enumerate() {
            let time = i as f64 / 11025.;
            if (start..end).contains(&time) {
                let phase = std::f32::consts::TAU * frequency * i as f32 / 11025.;
                *sample += amplitude * (phase.sin() + 0.4 * (phase * 2.).sin());
            }
        }
    }

    #[test]
    fn joint_harmonics_separate_a_weak_real_fundamental_from_a_ramping_octave() {
        for phase_step in 0..8 {
            for fundamental in [false, true] {
                let mut audio = vec![0.; 11025];
                let frequency = 440. * 2f64.powf((40. - 69.) / 12.);
                for (index, sample) in audio.iter_mut().enumerate() {
                    let time = index as f64 / 11025.;
                    let phase = std::f64::consts::TAU * frequency * time;
                    if (0.20..0.24).contains(&time) {
                        let envelope = ((time - 0.20) / 0.04).min(1.);
                        *sample = (0.3 * envelope * (2. * phase + phase_step as f64 * 0.7).sin()
                            + if fundamental {
                                0.08 * (phase + 0.3).sin()
                            } else {
                                0.
                            }) as f32;
                    } else if (0.24..0.70).contains(&time) {
                        *sample = (0.3 * (2. * phase + phase_step as f64 * 0.7).sin()
                            + 0.08 * (phase + 0.3).sin()) as f32;
                    }
                }
                let earlier = PitchNote {
                    start: 0.20,
                    end: 0.42,
                    pitch: 52,
                    confidence: 0.3,
                };
                let later = PitchNote {
                    start: 0.24,
                    end: 0.70,
                    pitch: 40,
                    confidence: 0.8,
                };
                let support = early_fundamental_support(&earlier, &later, &audio).unwrap();
                if fundamental {
                    assert!(support.added_variance_fraction > 0.02, "{support:?}");
                    assert!(support.phase_error_radians < 0.01, "{support:?}");
                } else {
                    assert!(support.added_variance_fraction < 1e-8, "{support:?}");
                }
            }
        }
    }

    #[test]
    fn weak_incoherent_lower_bleed_does_not_erase_a_real_octave_attack() {
        for phase in 0..8 {
            let frequency = 440. * 2f64.powf((40. - 69.) / 12.);
            let audio: Vec<_> = (0..11025)
                .map(|index| {
                    let time = index as f64 / 11025.;
                    let angle = std::f64::consts::TAU * frequency * time;
                    if (0.20..0.24).contains(&time) {
                        (0.3 * (2. * angle).sin() + 0.02 * (angle + phase as f64 * 0.7).sin())
                            as f32
                    } else if (0.24..0.70).contains(&time) {
                        (0.3 * angle.sin()) as f32
                    } else {
                        0.
                    }
                })
                .collect();
            let mut events = Transcription {
                bass: vec![
                    PitchNote {
                        start: 0.20,
                        end: 0.42,
                        pitch: 52,
                        confidence: 0.3,
                    },
                    PitchNote {
                        start: 0.24,
                        end: 0.70,
                        pitch: 40,
                        confidence: 0.8,
                    },
                ],
                ..Default::default()
            };
            correct_bass(&mut events, &audio);
            assert_eq!(events.bass[0].pitch, 52, "phase{phase}");
            assert!(events.bass_shadow_support.is_empty());
        }
    }

    #[test]
    fn weak_phase_coherent_fundamentals_have_recoverable_decision_receipts() {
        let frequency = 440. * 2f64.powf((40. - 69.) / 12.);
        let mut joint_methods = 0;
        for phase in 0..8 {
            let audio: Vec<_> = (0..11025)
                .map(|index| {
                    let time = index as f64 / 11025.;
                    let angle = std::f64::consts::TAU * frequency * time;
                    if (0.20..0.70).contains(&time) {
                        (0.3 * (2. * angle + phase as f64 * 0.7).sin() + 0.08 * (angle + 0.3).sin())
                            as f32
                    } else {
                        0.
                    }
                })
                .collect();
            let mut events = Transcription {
                bass: vec![
                    PitchNote {
                        start: 0.20,
                        end: 0.42,
                        pitch: 52,
                        confidence: 0.3,
                    },
                    PitchNote {
                        start: 0.24,
                        end: 0.70,
                        pitch: 40,
                        confidence: 0.8,
                    },
                ],
                ..Default::default()
            };
            correct_bass(&mut events, &audio);
            assert_eq!(events.bass[0].pitch, 40);
            assert_eq!(events.bass_shadow_support.len(), 1);
            assert!(
                events.bass_harmonic_conflicts.is_empty(),
                "a resolved shadow must not retain an unresolved conflict"
            );
            if events.bass_shadow_support[0].joint_fit.is_some() {
                joint_methods += 1;
            }
            events.validate().unwrap();
            let saved = serde_json::to_string(&events).unwrap();
            let restored: Transcription = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored.bass_shadow_support.len(), 1);
        }
        assert!(joint_methods > 0);
    }

    #[test]
    fn unsupported_gentle_repeats_are_flagged_without_deleting_or_moving_notes() {
        let mut events = Transcription {
            bass: [0.2, 0.4, 0.6]
                .into_iter()
                .map(|start| PitchNote {
                    start,
                    end: start + 0.2,
                    pitch: 40,
                    confidence: 0.8,
                })
                .collect(),
            ..Default::default()
        };
        let before = serde_json::to_value(&events.bass).unwrap();
        let audio: Vec<_> = (0..22050)
            .map(|i| (std::f32::consts::TAU * 82.40689 * i as f32 / 22050.).sin() * 0.3)
            .collect();
        assess_attacks(&mut events, &audio);
        assert_eq!(serde_json::to_value(&events.bass).unwrap(), before);
        assert_eq!(events.bass_attack_reviews.len(), 3);
        assert!(
            events
                .bass_attack_reviews
                .iter()
                .any(|review| !review.supported)
        );
        events.validate().unwrap();
    }

    #[test]
    fn short_owned_notes_can_be_corrected_without_borrowing_neighbours() {
        let mut audio = vec![0.; 11025];
        tone(0.20, 0.34, 40, 0.25, &mut audio);
        tone(0.34, 0.70, 43, 0.4, &mut audio);
        let mut events = Transcription {
            bass: vec![
                PitchNote {
                    start: 0.20,
                    end: 0.39,
                    pitch: 52,
                    confidence: 0.8,
                },
                PitchNote {
                    start: 0.34,
                    end: 0.70,
                    pitch: 43,
                    confidence: 0.8,
                },
            ],
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(events.bass[0].pitch, 40);
        assert_eq!(events.bass[1].pitch, 43);
        assert_eq!(events.bass_corrections.len(), 1);
        assert_eq!(events.bass_corrections[0].original.end, 0.39);
    }

    #[test]
    fn next_note_cannot_supply_the_fundamental_for_an_earlier_event() {
        let mut audio = vec![0.; 11025];
        tone(0.20, 0.34, 52, 0.05, &mut audio);
        tone(0.34, 0.80, 40, 0.5, &mut audio);
        let mut events = Transcription {
            bass: vec![
                // A long model release overlaps a much louder lower note.
                PitchNote {
                    start: 0.20,
                    end: 0.80,
                    pitch: 52,
                    confidence: 0.8,
                },
                PitchNote {
                    start: 0.34,
                    end: 0.80,
                    pitch: 40,
                    confidence: 0.8,
                },
            ],
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(events.bass[0].pitch, 52);
        assert!(events.bass_corrections.is_empty());
    }

    #[test]
    fn delayed_harmonic_activation_is_not_a_new_attack_but_real_octave_jump_is() {
        for real_jump in [false, true] {
            let mut audio = vec![0.; 11025];
            if real_jump {
                tone(0.20, 0.24, 52, 0.3, &mut audio);
                tone(0.24, 0.70, 40, 0.5, &mut audio);
            } else {
                tone(0.20, 0.70, 40, 0.3, &mut audio);
            }
            let mut events = Transcription {
                bass: vec![
                    PitchNote {
                        start: 0.20,
                        end: 0.42,
                        pitch: 52,
                        confidence: 0.3,
                    },
                    PitchNote {
                        start: 0.24,
                        end: 0.70,
                        pitch: 40,
                        confidence: 0.8,
                    },
                ],
                ..Default::default()
            };
            correct_bass(&mut events, &audio);
            assert_eq!(events.bass[0].pitch, if real_jump { 52 } else { 40 });
            assert_eq!(events.bass_corrections.len(), if real_jump { 0 } else { 1 });
            assert_eq!(events.bass_harmonic_conflicts.len(), usize::from(real_jump));
            events.validate().unwrap();
            let saved = serde_json::to_string(&events).unwrap();
            let restored: Transcription = serde_json::from_str(&saved).unwrap();
            assert_eq!(
                restored.bass_harmonic_conflicts.len(),
                usize::from(real_jump)
            );
        }
    }

    #[test]
    fn a_noisy_pluck_can_leave_two_agreeing_owned_sustain_windows() {
        let mut audio = vec![0.; 11025];
        tone(0.20, 0.42, 40, 0.3, &mut audio);
        for (index, sample) in audio.iter_mut().enumerate() {
            let time = index as f64 / 11025.;
            if (0.20..0.245).contains(&time) {
                let noise = ((index * 1103515245 + 12345) & 0xffff) as f32 / 32768. - 1.;
                *sample += noise * 0.6;
            }
        }
        let mut events = Transcription {
            bass: vec![PitchNote {
                start: 0.20,
                end: 0.42,
                pitch: 52,
                confidence: 0.8,
            }],
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(events.bass[0].pitch, 40);
        assert_eq!(events.bass_corrections.len(), 1);
    }

    #[test]
    fn contradictory_stable_pitches_inside_a_model_event_veto_correction() {
        let mut audio = vec![0.; 11025];
        tone(0.20, 0.40, 40, 0.3, &mut audio);
        tone(0.40, 0.65, 43, 0.3, &mut audio);
        tone(0.65, 0.90, 40, 0.3, &mut audio);
        let mut events = Transcription {
            bass: vec![PitchNote {
                start: 0.20,
                end: 0.90,
                pitch: 52,
                confidence: 0.8,
            }],
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(events.bass[0].pitch, 52);
        assert!(events.bass_corrections.is_empty());
    }

    #[test]
    fn too_short_low_notes_keep_the_model_pitch() {
        let mut audio = vec![0.; 11025];
        tone(0.20, 0.30, 28, 0.3, &mut audio);
        let mut events = Transcription {
            bass: vec![PitchNote {
                start: 0.20,
                end: 0.30,
                pitch: 40,
                confidence: 0.8,
            }],
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(events.bass[0].pitch, 40);
        assert!(events.bass_corrections.is_empty());
    }

    #[test]
    fn quiet_baseline_attack_is_refined_in_source_time_and_release_is_kept() {
        for predicted in [0.43, 0.57] {
            let mut audio = vec![0.; 11025];
            tone(0.50, 0.85, 40, 0.3, &mut audio);
            let mut events = Transcription {
                bass: vec![PitchNote {
                    start: predicted,
                    end: 0.85,
                    pitch: 40,
                    confidence: 0.8,
                }],
                ..Default::default()
            };
            correct_bass(&mut events, &audio);
            assert!(
                (events.bass[0].start - 0.50).abs() < 0.008,
                "{:?}",
                events.bass[0]
            );
            assert_eq!(events.bass[0].end, 0.85);
            assert_eq!(events.bass_timing_corrections.len(), 1);
            let saved = serde_json::to_value(&events).unwrap();
            correct_bass(&mut events, &audio);
            assert_eq!(serde_json::to_value(&events).unwrap(), saved);
        }
    }

    #[test]
    fn ringing_or_wrong_pitch_attacks_are_not_moved_to_energy_peaks() {
        for (pitch, ringing) in [(40, 0.2), (40, 0.09), (43, 0.)] {
            let mut audio = vec![0.; 11025];
            if ringing > 0. {
                tone(0., 0.50, pitch, ringing, &mut audio);
            }
            tone(0.50, 0.85, pitch, 0.3, &mut audio);
            let mut events = Transcription {
                bass: vec![PitchNote {
                    start: 0.57,
                    end: 0.85,
                    pitch: 40,
                    confidence: 0.8,
                }],
                ..Default::default()
            };
            correct_bass(&mut events, &audio);
            assert_eq!(events.bass[0].start, 0.57);
            assert!(events.bass_timing_corrections.is_empty());
        }
    }

    fn short_attack_events(pitch: u8) -> Transcription {
        Transcription {
            bass: vec![
                PitchNote {
                    start: 0.448,
                    end: 0.85,
                    pitch,
                    confidence: 0.8,
                },
                // Candidate ownership is intentionally shorter than the
                // existing 1024-sample pitch verification window.
                PitchNote {
                    start: 0.600,
                    end: 0.85,
                    pitch,
                    confidence: 0.5,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn short_owned_attack_path_leaves_noise_envelope_modulation_and_bleed_unchanged() {
        for case in 0..5 {
            let mut audio = vec![0.; 11025];
            let mut noise = 0x12345678u32;
            for (i, sample) in audio.iter_mut().enumerate() {
                let time = i as f64 / 11025.;
                let carrier = (std::f64::consts::TAU * 55. * time + 0.4).sin();
                let value = match case {
                    0 if time >= 0.5 => {
                        noise ^= noise << 13;
                        noise ^= noise >> 17;
                        noise ^= noise << 5;
                        0.2 * (noise as f64 / u32::MAX as f64 * 2. - 1.)
                    }
                    1 => {
                        // A continuous carrier dips then rises; it has no new
                        // quiet-baseline pluck and must not be snapped.
                        let amplitude = if (0.44..0.5).contains(&time) {
                            0.03
                        } else {
                            0.15
                        };
                        amplitude * carrier
                    }
                    2 => {
                        let tail = 0.012 * carrier;
                        let higher = if time >= 0.5 {
                            0.2 * (std::f64::consts::TAU * 110. * time).sin()
                        } else {
                            0.
                        };
                        tail + higher
                    }
                    3 if time >= 0.5 => 0.2 * (std::f64::consts::TAU * 110. * time).sin(),
                    4 => {
                        // Legitimate gentle phase-continuous reattack remains
                        // a note; uncertain localization must not snap it.
                        let rise = if time >= 0.5 {
                            0.01 * (1. - (-(time - 0.5) / 0.04).exp())
                        } else {
                            0.
                        };
                        (0.1 + rise) * carrier
                    }
                    _ => 0.,
                };
                *sample = value as f32;
            }
            let mut events = short_attack_events(33);
            correct_attacks(&mut events, &audio);
            assert_eq!(events.bass[0].start, 0.448, "case {case}");
            assert!(events.bass_timing_corrections.is_empty(), "case {case}");
        }
    }

    /// Optional source-bound smoke audit of cached real recordings. No fixed
    /// expected pitch count: these files are evidence, not labelled truth.
    #[test]
    #[ignore = "source-window diagnostic; requires CHART_ENGINE_BASS_INSPECT_TIMES"]
    fn inspect_cached_bass_windows() {
        let events_path = std::env::var("CHART_ENGINE_BASS_EVENTS").unwrap();
        let stem_path = std::env::var("CHART_ENGINE_BASS_STEM").unwrap();
        let mut events: Transcription =
            serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
        if let Ok(model) = std::env::var("CHART_ENGINE_BASS_MODEL") {
            events.bass_candidates =
                crate::models::pitch_with_evidence(Path::new(&stem_path), Path::new(&model))
                    .unwrap();
            events.bass = events
                .bass_candidates
                .iter()
                .map(|e| e.candidate.clone())
                .collect();
        }
        let audio = crate::models::decode(Path::new(&stem_path), 11025, 1).unwrap();
        let times: Vec<f64> = std::env::var("CHART_ENGINE_BASS_INSPECT_TIMES")
            .unwrap()
            .split(',')
            .map(|s| s.parse().unwrap())
            .collect();
        let power = |start: f64, end: f64, frequency: f64| {
            let lo = (start.max(0.) * 11025.).ceil() as usize;
            let hi = ((end.max(0.) * 11025.).floor() as usize).min(audio.len());
            if hi <= lo {
                return 0.;
            }
            let (mut re, mut im) = (0., 0.);
            for (index, sample) in audio[lo..hi].iter().enumerate() {
                let window =
                    0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / (hi - lo - 1) as f64).cos();
                let phase = std::f64::consts::TAU * frequency * index as f64 / 11025.;
                re += *sample as f64 * window * phase.cos();
                im += *sample as f64 * window * phase.sin();
            }
            re * re + im * im
        };
        let windows = |note: &PitchNote, end: f64| {
            let first = ((note.start + 0.012) * 11025.).ceil() as usize;
            let last = (((end - 0.008).max(0.) * 11025.).floor() as usize).min(audio.len());
            let available = last.saturating_sub(first);
            let size = if available >= 3072 { 2048 } else { 1024 };
            if available < size + 64 {
                return Vec::new();
            }
            let span = available.min(6615).max(size);
            [first,first+(span-size)/2,first+span-size].into_iter().map(|start| {
                let signal:Vec<f64>=audio[start..start+size].iter().map(|s|*s as f64).collect();
                let mut yin=YINDetector::<f64>::new(size,size/2);
                let mut mcleod=McLeodDetector::<f64>::new(size,size/2);
                let y=yin.get_pitch(&signal,11025,0.01,0.9);
                let m=mcleod.get_pitch(&signal,11025,0.01,0.85);
                let ys=y.map(|y| {
                    let p=power(start as f64/11025.,(start+size) as f64/11025.,y.frequency);
                    let strongest=(2..=5).map(|h|power(start as f64/11025.,(start+size) as f64/11025.,y.frequency*h as f64)).fold(p,f64::max);
                    serde_json::json!({"midi":69.+12.*(y.frequency/440.).log2(),"periodicity":periodicity(&signal,y.frequency,11025.),"fundamental_fraction":p/strongest.max(1e-30),"cycles":size as f64*y.frequency/11025.})
                });
                serde_json::json!({"start":start as f64/11025.,"end":(start+size) as f64/11025.,"yin":ys,"mcleod_midi":m.map(|m|69.+12.*(m.frequency/440.).log2())})
            }).collect()
        };
        let rows:Vec<_>=times.into_iter().map(|time| {
            let note=events.bass.iter().min_by(|a,b|(a.start-time).abs().total_cmp(&(b.start-time).abs())).unwrap();
            let mut neighbours:Vec<_>=events.bass.iter().filter(|n|(n.start-note.start).abs()<0.20).cloned().collect();
            neighbours.sort_by(|a,b|a.start.total_cmp(&b.start));
            let pairs:Vec<_>=neighbours.iter().filter(|n|n.start>note.start+0.025).map(|later| {
                let frequency=440.*2f64.powf((later.pitch as f64-69.)/12.);
                let p=power(note.start+0.003,later.start-0.003,frequency);
                let strongest=(2..=5).map(|h|power(note.start+0.003,later.start-0.003,frequency*h as f64)).fold(p,f64::max);
                serde_json::json!({"later":later,"gap_ms":(later.start-note.start)*1000.,"pre_cycles":(later.start-note.start-0.006)*frequency,"pre_fundamental_fraction":p/strongest.max(1e-30),"shadow":harmonic_shadow(note,later,&audio),"joint_fit":early_fundamental_support(note,later,&audio)})
            }).collect();
            let next=events.bass.iter().filter(|n|n.start>note.start+0.025).map(|n|n.start).min_by(f64::total_cmp).unwrap_or(note.end);
            let envelope:Vec<_>=(0..90).map(|i| {
                let start=(time-0.2+i as f64*0.005).max(0.);
                let lo=((start*11025.) as usize).min(audio.len());let hi=(((start+0.005)*11025.) as usize).min(audio.len());
                let rms=(audio[lo..hi].iter().map(|s|(*s as f64).powi(2)).sum::<f64>()/(hi-lo).max(1) as f64).sqrt();
                serde_json::json!({"time":start,"rms":rms})
            }).collect();
            serde_json::json!({"target":time,"note":note,"candidate_evidence":events.bass_candidates.iter().filter(|e|e.candidate.start==note.start&&e.candidate.pitch==note.pitch).collect::<Vec<_>>(),"neighbours":neighbours,"harmonic_pairs":pairs,"raw_owned_windows":windows(note,note.end.min(next)),"potential_shadow_windows":windows(note,note.end),"rms_5ms":envelope})
        }).collect();
        let result = serde_json::json!({"events_sha256":crate::models::hash_file(Path::new(&events_path)).unwrap(),"stem_sha256":crate::models::hash_file(Path::new(&stem_path)).unwrap(),"windows":rows});
        let output = std::env::var("CHART_ENGINE_BASS_INSPECT_OUTPUT").unwrap();
        std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "requires CHART_ENGINE_BASS_EVENTS and CHART_ENGINE_BASS_STEM"]
    fn cached_bass_refinement_audit() {
        let events_path = std::env::var("CHART_ENGINE_BASS_EVENTS").unwrap();
        let stem_path = std::env::var("CHART_ENGINE_BASS_STEM").unwrap();
        let mut events: Transcription =
            serde_json::from_slice(&std::fs::read(&events_path).unwrap()).unwrap();
        let comparison_path = std::env::var("CHART_ENGINE_BASS_COMPARE_EVENTS").ok();
        let previous_events: Transcription = comparison_path.as_ref().map_or_else(
            || events.clone(),
            |path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        );
        let chart_path = Path::new(&events_path)
            .parent()
            .unwrap()
            .join("notes.chart");
        let mut previous_chart =
            crate::chart::Chart::parse(std::fs::read_to_string(chart_path).unwrap()).unwrap();
        if comparison_path.is_some() {
            let mut previous_bass = previous_events.clone();
            previous_bass.guitar.clear();
            previous_bass.drums.clear();
            let duration = previous_bass
                .bass
                .iter()
                .map(|note| note.end)
                .fold(0., f64::max);
            let (tracks, _) = crate::difficulty_links::reduce_generated(
                &crate::mapping::tracks(&previous_bass, &previous_chart.timing, false).unwrap(),
                &Default::default(),
                &previous_chart.timing,
                duration,
            )
            .unwrap();
            previous_chart = crate::chart::Chart::parse(crate::chart::new_chart(
                "Previous Bass",
                "",
                &previous_chart.timing,
                &tracks,
            ))
            .unwrap();
        }
        let before = events.bass.len();
        let prior = events.bass_corrections.clone();
        // Replay the same raw model candidates: old post-processing must not
        // hide a formerly unsafe correction from this comparison.
        for correction in &prior {
            if let Some(note) = events.bass.iter_mut().find(|note| {
                note.start == correction.original.start
                    && note.end == correction.original.end
                    && note.pitch == correction.revised_pitch
            }) {
                *note = correction.original.clone();
            }
        }
        events.bass_corrections.clear();
        events.bass_timing_corrections.clear();
        events.bass_shadow_support.clear();
        events.bass_attack_reviews.clear();
        events.bass_harmonic_conflicts.clear();
        let fresh_inference = std::env::var("CHART_ENGINE_BASS_MODEL").ok();
        if let Some(model) = &fresh_inference {
            events.bass_candidates =
                crate::models::pitch_with_evidence(Path::new(&stem_path), Path::new(model))
                    .unwrap();
            events.bass = events
                .bass_candidates
                .iter()
                .map(|evidence| evidence.candidate.clone())
                .collect();
        }
        let raw_count = events.bass.len();
        bass(&mut events, Path::new(&stem_path)).unwrap();
        events.validate().unwrap();
        assert_eq!(events.bass.len(), raw_count);
        let mut bass_only = events.clone();
        bass_only.guitar.clear();
        bass_only.drums.clear();
        let timing = previous_chart.timing.clone();
        let pitch_audio = crate::models::decode(Path::new(&stem_path), 11025, 1).unwrap();
        let spectral_audio = crate::models::decode(Path::new(&stem_path), 22050, 1).unwrap();
        let duration = pitch_audio.len() as f64 / 11025.;
        let (tracks, _) = crate::difficulty_links::reduce_generated(
            &crate::mapping::tracks(&bass_only, &timing, false).unwrap(),
            &Default::default(),
            &timing,
            duration,
        )
        .unwrap();
        let rules = crate::generated_rules::assess(&tracks, &timing, duration);
        assert!(!rules.has_errors(), "{rules:?}");
        let new_chart =
            crate::chart::Chart::parse(crate::chart::new_chart("Bass audit", "", &timing, &tracks))
                .unwrap();
        let generated_review = crate::review::generation_map(
            &new_chart,
            &bass_only,
            duration,
            Default::default(),
            tracks.keys().cloned().collect(),
        )
        .unwrap();
        let stem_dir = Path::new(&stem_path).parent().unwrap();
        let competitors: Vec<_> = ["guitar.wav", "vocals.wav"]
            .iter()
            .map(|name| crate::models::decode(&stem_dir.join(name), 11025, 1).unwrap())
            .collect();
        let competitor_refs: Vec<_> = competitors.iter().map(Vec::as_slice).collect();
        let before_coverage = crate::stem_coverage::analyze_stem(
            &previous_chart,
            crate::stem_review::StemId::Bass,
            &spectral_audio,
            &pitch_audio,
            &competitor_refs,
            Some(&previous_events),
            &Default::default(),
        )
        .unwrap();
        let after_coverage = crate::stem_coverage::analyze_stem(
            &new_chart,
            crate::stem_review::StemId::Bass,
            &spectral_audio,
            &pitch_audio,
            &competitor_refs,
            Some(&events),
            &Default::default(),
        )
        .unwrap();
        let mut used = std::collections::BTreeSet::new();
        let paired: Vec<_> = before_coverage
            .alignment
            .iter()
            .filter(|before| before.supported_attack)
            .filter_map(|before| {
                let candidates: Vec<_> = after_coverage
                    .alignment
                    .iter()
                    .filter(|after| {
                        after.supported_attack
                            && before.nearest_flux_seconds == after.nearest_flux_seconds
                            && (before.seconds - after.seconds).abs() < 0.095
                            && !used.contains(&after.tick)
                    })
                    .collect();
                if candidates.len() != 1 {
                    return None;
                }
                let after = candidates[0];
                used.insert(after.tick);
                Some((before, after))
            })
            .collect();
        let quantiles = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            serde_json::json!({ "count":values.len(),
                "median_absolute_ms": values.get(values.len()/2),
                "p90_absolute_ms": values.get(((values.len().saturating_sub(1)) as f64*0.9).round() as usize),
                "over_50ms":values.iter().filter(|value| **value>50.).count() })
        };
        let paired_timing = serde_json::json!({
            "cohort": "one-to-one supported chart attacks within95ms sharing the same independent flux peak; ambiguous matches excluded",
            "before": quantiles(paired.iter().filter_map(|(before,_)| before.audio_minus_chart_seconds.map(|v|v.abs()*1000.)).collect()),
            "after": quantiles(paired.iter().filter_map(|(_,after)| after.audio_minus_chart_seconds.map(|v|v.abs()*1000.)).collect()),
            "stable_pitch_both": paired.iter().filter(|(before,after)| before.periodic_minus_raw_semitones.is_some()&&after.periodic_minus_raw_semitones.is_some()).count(),
            "pitch_disagreements_before": paired.iter().filter(|(before,after)| after.periodic_minus_raw_semitones.is_some()&&before.periodic_minus_raw_semitones.is_some_and(|v|v.abs()>1.)).count(),
            "pitch_disagreements_after": paired.iter().filter(|(before,after)| before.periodic_minus_raw_semitones.is_some()&&after.periodic_minus_raw_semitones.is_some_and(|v|v.abs()>1.)).count(),
            "owned_pitch_both": paired.iter().filter(|(before,after)| before.pitch_window.as_ref().is_some_and(|w|w.owned)&&after.pitch_window.as_ref().is_some_and(|w|w.owned)).count(),
            "owned_pitch_disagreements_before": paired.iter().filter(|(before,after)| before.pitch_window.as_ref().is_some_and(|w|w.owned)&&after.pitch_window.as_ref().is_some_and(|w|w.owned)&&before.periodic_minus_raw_semitones.is_some_and(|v|v.abs()>1.)).count(),
            "owned_pitch_disagreements_after": paired.iter().filter(|(before,after)| before.pitch_window.as_ref().is_some_and(|w|w.owned)&&after.pitch_window.as_ref().is_some_and(|w|w.owned)&&after.periodic_minus_raw_semitones.is_some_and(|v|v.abs()>1.)).count(),
        });
        if let Ok(path) = std::env::var("CHART_ENGINE_BASS_REFINED_OUTPUT") {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .unwrap();
            serde_json::to_writer_pretty(file, &bass_only).unwrap();
        }
        let flux = crate::review::attacks(&spectral_audio);
        let moved_flux_residuals: Vec<_> = events.bass_timing_corrections.iter().filter_map(|correction| {
            let before = crate::review::nearest(correction.original.start, &flux)?;
            let after = crate::review::nearest(correction.revised_start, &flux)?;
            Some(serde_json::json!({ "original": correction.original.start, "revised": correction.revised_start,
                "before_flux_residual_ms": (before.time-correction.original.start)*1000.,
                "after_flux_residual_ms": (after.time-correction.revised_start)*1000.,
                "same_peak": before.time==after.time, "peak_strength": after.strength,
                "rise_db": 20.*(correction.after_rms/correction.before_rms.max(1e-12)).log10() }))
        }).collect();
        let report = serde_json::json!({
            "events_sha256": crate::models::hash_file(Path::new(&events_path)).unwrap(),
            "comparison_events_sha256": comparison_path.as_ref().map(|path| crate::models::hash_file(Path::new(path)).unwrap()),
            "comparison_mode": if comparison_path.is_some() { "reconstructed_generated_bass_from_previous_transcription" } else { "saved_chart_and_transcription" },
            "stem_sha256": crate::models::hash_file(Path::new(&stem_path)).unwrap(),
            "prior_events": before, "events": raw_count, "fresh_inference": fresh_inference.is_some(), "prior_pitch_corrections": prior.len(),
            "new_pitch_corrections": events.bass_corrections.len(),
            "new_corrections_at_previously_unchanged_attacks": events.bass_corrections.iter().filter(|correction|
                !prior.iter().any(|old| (old.original.start - correction.original.start).abs() < 0.026)).count(),
            "pitch_corrections": events.bass_corrections,
            "timing_corrections": events.bass_timing_corrections,
            "shadow_support": events.bass_shadow_support,
            "harmonic_conflicts": events.bass_harmonic_conflicts,
            "decoder_sources": events.bass_candidates.iter().fold(std::collections::BTreeMap::<String,usize>::new(), |mut counts,e| { *counts.entry(format!("{:?}", e.source)).or_default() += 1; counts }),
            "attack_support": {"candidates":events.bass_attack_reviews.len(),"unsupported_candidates":events.bass_attack_reviews.iter().filter(|o|!o.supported).count()},
            "bass_chart_events_by_tier": tracks.iter().map(|(track, notes)| (track, notes.len())).collect::<std::collections::BTreeMap<_,_>>(),
            "generated_rule_report": rules,
            "generated_review": generated_review,
            "before_independent_coverage": before_coverage,
            "after_independent_coverage": after_coverage,
            "paired_before_after": paired_timing,
            "moved_attack_flux_comparisons": moved_flux_residuals,
            "model_sha256": fresh_inference.as_ref().map(|path| crate::models::hash_file(Path::new(path)).unwrap()),
            "status": "unlabelled_signal_consistency_only"
        });
        if let Ok(path) = std::env::var("CHART_ENGINE_BASS_AUDIT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        println!("{}", serde_json::to_string(&report).unwrap());
    }
    #[test]
    fn harmonic_correction_requires_stable_audio_and_preserves_original() {
        let audio: Vec<_> = (0..11025)
            .map(|i| {
                let phase = std::f32::consts::TAU * 82.40689 * i as f32 / 11025.0;
                0.25 * phase.sin() + 0.12 * (2.0 * phase).sin()
            })
            .collect();
        let mut events = Transcription {
            bass: [52, 40, 43]
                .into_iter()
                .map(|pitch| PitchNote {
                    start: 0.2,
                    end: 0.8,
                    pitch,
                    confidence: 0.8,
                })
                .collect(),
            ..Default::default()
        };
        correct_bass(&mut events, &audio);
        assert_eq!(
            events.bass.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![40, 40, 43]
        );
        assert_eq!(events.bass_corrections.len(), 1);
        assert_eq!(events.bass_corrections[0].original.pitch, 52);
        let saved = serde_json::to_value(&events).unwrap();
        correct_bass(&mut events, &audio);
        assert_eq!(serde_json::to_value(&events).unwrap(), saved);
        let mut silent = Transcription {
            bass: vec![PitchNote {
                start: 0.2,
                end: 0.8,
                pitch: 52,
                confidence: 0.8,
            }],
            ..Default::default()
        };
        correct_bass(&mut silent, &vec![0.0; 11025]);
        assert_eq!(silent.bass[0].pitch, 52);
    }

    #[test]
    fn saved_correction_preserves_full_float_precision() {
        let value = BassCorrection {
            original: PitchNote {
                start: 0.2,
                end: 0.8,
                pitch: 52,
                confidence: 0.8,
            },
            revised_pitch: 40,
            periodic_pitch: 31.100393928214746,
            clarity: 0.9872207893257743,
        };
        let saved = serde_json::to_string(&value).unwrap();
        let loaded: BassCorrection = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            loaded.periodic_pitch.to_bits(),
            value.periodic_pitch.to_bits()
        );
        assert_eq!(loaded.clarity.to_bits(), value.clarity.to_bits());
    }
}
