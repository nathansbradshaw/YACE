//! Shared inference decoding and source-supported recovery for both clients.
use crate::mapping::{
    BassShortCandidate, DrumHit, PitchCandidateEvidence, PitchCandidateSource, PitchNote, Voice,
};
pub fn drum_peaks(activations: &[f32]) -> Vec<DrumHit> {
    drum_peaks_with_thresholds(activations, [0.22, 0.24, 0.32, 0.22, 0.30], [0.; 5])
}

fn drum_peaks_with_thresholds(
    activations: &[f32],
    thresholds: [f32; 5],
    minimum: [f32; 5],
) -> Vec<DrumHit> {
    let frames = activations.len() / 5;
    let voices = [
        Voice::Kick,
        Voice::Snare,
        Voice::Tom,
        Voice::HiHat,
        Voice::Cymbal,
    ];
    let mut hits = Vec::new();
    for class in 0..5 {
        let mut last: Option<usize> = None;
        for frame in 0..frames {
            let value = activations[frame * 5 + class];
            let start = frame.saturating_sub(10);
            let end = (frame + 2).min(frames);
            let average: f32 = (start..end)
                .map(|i| activations[i * 5 + class])
                .sum::<f32>()
                / (end - start) as f32;
            if !value.is_finite() || value < minimum[class] || value < average + thresholds[class] {
                continue;
            }
            if (frame.saturating_sub(2)..(frame + 2).min(frames)).any(|i| {
                activations[i * 5 + class] > value
                    || (i < frame && activations[i * 5 + class] == value)
            }) {
                continue;
            }
            if last.is_some_and(|prev| frame - prev <= 2) {
                continue;
            }
            hits.push(DrumHit {
                time: frame as f64 / 100.0,
                voice: voices[class],
                confidence: value.clamp(0.0, 1.0),
            });
            last = Some(frame);
        }
    }
    // Keep the strongest peak in each class's suppression window.
    // Broad cymbal peaks can otherwise become duplicate hits 50 ms apart.
    hits.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then(a.time.total_cmp(&b.time))
    });
    let mut selected: Vec<DrumHit> = Vec::new();
    for hit in hits {
        if !selected
            .iter()
            .any(|old| old.voice == hit.voice && (old.time - hit.time).abs() < 0.060 - 1e-9)
        {
            selected.push(hit);
        }
    }
    selected.sort_by(|a, b| a.time.total_cmp(&b.time));
    selected
}

/// A strong local class peak may fall below the fixed contrast threshold in
/// dense percussion. Require an independent attack rather than a genre grid.
pub fn supported_drum_peaks(
    activations: &[f32],
    audio_22050: &[f32],
) -> (Vec<DrumHit>, Vec<DrumHit>) {
    let mut hits = drum_peaks(activations);
    let attacks = crate::review::attacks(audio_22050);
    let candidates =
        drum_peaks_with_thresholds(activations, [0.10; 5], [0.50, 0.50, 0.60, 0.55, 0.60]);
    let mut recovered = Vec::new();
    for hit in candidates {
        if crate::review::nearest(hit.time, &attacks)
            .is_some_and(|a| (a.time - hit.time).abs() <= 0.025)
            && !hits
                .iter()
                .any(|h| h.voice == hit.voice && (h.time - hit.time).abs() < 0.060 - 1e-9)
        {
            recovered.push(hit.clone());
            hits.push(hit);
        }
    }
    hits.sort_by(|a, b| a.time.total_cmp(&b.time));
    (hits, recovered)
}

pub struct ShortPitchInference {
    pub candidates: Vec<PitchCandidateEvidence>,
    pub short_candidates: Vec<BassShortCandidate>,
}
pub type BassPitchInference = ShortPitchInference;

#[cfg(test)]
#[path = "guitar_coverage_tests.rs"]
mod guitar_coverage_tests;
#[path = "guitar_harmonics.rs"]
mod guitar_harmonics;
#[path = "guitar_polyphonic.rs"]
mod guitar_polyphonic;

pub struct GuitarPitchInference {
    pub candidates: Vec<PitchCandidateEvidence>,
    pub short_candidates: Vec<BassShortCandidate>,
    pub polyphonic_notes: Vec<crate::mapping::GuitarPolyphonicNote>,
    pub diagnostics: std::collections::BTreeMap<String, usize>,
}

pub fn guitar_from_activations(
    activations: PitchActivations,
    pitch_audio: &[f32],
    source: crate::mapping::GuitarSource,
) -> GuitarPitchInference {
    let short = recover_short_plucks(&activations, pitch_audio, 40..=88);
    let (diagnostics, proposed, polyphonic_notes) =
        if source == crate::mapping::GuitarSource::Guitar {
            guitar_polyphonic::recover(
                &activations,
                pitch_audio,
                &short.candidates,
                &short.short_candidates,
            )
        } else {
            (Default::default(), Vec::new(), Vec::new())
        };
    let mut candidates = short.candidates;
    candidates.extend(proposed);
    candidates.sort_by(|a, b| {
        a.candidate
            .start
            .total_cmp(&b.candidate.start)
            .then(a.candidate.pitch.cmp(&b.candidate.pitch))
    });
    GuitarPitchInference {
        candidates,
        short_candidates: short.short_candidates,
        polyphonic_notes,
        diagnostics,
    }
}
pub fn recover_short_bass(
    activations: &PitchActivations,
    pitch_audio: &[f32],
) -> BassPitchInference {
    recover_short_plucks(activations, pitch_audio, 24..=67)
}

fn recover_short_plucks(
    activations: &PitchActivations,
    pitch_audio: &[f32],
    pitches: std::ops::RangeInclusive<u8>,
) -> ShortPitchInference {
    let duration = activations.audio.len() as f64 / 22050.;
    let baseline = clip_pitch_candidates(
        decode_pitch_with_evidence(&activations.frames, &activations.onsets),
        duration,
    );
    let hypotheses: Vec<_> = clip_pitch_candidates(
        decode_pitch_with_limits(&activations.frames, &activations.onsets, 4, 11),
        duration,
    )
    .into_iter()
    .filter(|evidence| evidence.source == PitchCandidateSource::NetworkOnset)
    .collect();
    let peaks = crate::review::attacks(&activations.audio);
    let mut short_candidates: Vec<_> = hypotheses
        .iter()
        .map(|evidence| {
            let support = short_pluck_support(
                evidence,
                &baseline,
                &hypotheses,
                pitch_audio,
                &peaks,
                &pitches,
            );
            BassShortCandidate {
                evidence: evidence.clone(),
                retained: support.reason == "source_supported",
                reason: support.reason.into(),
                source_attack_seconds: support.source_flux_seconds,
                periodic_pitch: support.periodic_pitch,
                periodicity: support.periodicity,
                fundamental_fraction: support.fundamental_fraction,
                window_start: support.window_start,
                window_end: support.window_end,
            }
        })
        .collect();
    let mut rank: Vec<_> = (0..short_candidates.len()).collect();
    rank.sort_by(|a, b| {
        short_candidates[*b]
            .evidence
            .onset_score
            .total_cmp(&short_candidates[*a].evidence.onset_score)
            .then(
                short_candidates[*b]
                    .evidence
                    .candidate
                    .confidence
                    .total_cmp(&short_candidates[*a].evidence.candidate.confidence),
            )
    });
    let mut candidates = baseline;
    for index in rank {
        let short = &mut short_candidates[index];
        if !short.retained {
            continue;
        }
        if candidates.iter().any(|previous| {
            (previous.candidate.start - short.evidence.candidate.start).abs() < 0.055
        }) {
            short.retained = false;
            short.reason = "near_existing_attack".into();
            continue;
        }
        candidates.push(short.evidence.clone());
    }
    candidates.sort_by(|a, b| {
        a.candidate
            .start
            .total_cmp(&b.candidate.start)
            .then(a.candidate.pitch.cmp(&b.candidate.pitch))
    });
    BassPitchInference {
        candidates,
        short_candidates,
    }
}

pub struct PitchActivations {
    pub frames: Vec<f32>,
    pub onsets: Vec<f32>,
    pub audio: Vec<f32>,
}

pub fn clip_pitch_candidates(
    mut decoded: Vec<PitchCandidateEvidence>,
    duration: f64,
) -> Vec<PitchCandidateEvidence> {
    decoded.retain(|n| n.candidate.start < duration);
    for n in &mut decoded {
        n.candidate.end = n.candidate.end.min(duration);
    }
    decoded.retain(|n| n.candidate.end > n.candidate.start);
    decoded
}

pub fn decode_pitch(frames: &[f32], onsets: &[f32]) -> Vec<PitchNote> {
    decode_pitch_with_evidence(frames, onsets)
        .into_iter()
        .map(|evidence| evidence.candidate)
        .collect()
}

pub fn decode_pitch_with_evidence(frames: &[f32], onsets: &[f32]) -> Vec<PitchCandidateEvidence> {
    decode_pitch_with_limits(frames, onsets, 11, usize::MAX)
}

/// Frame bounds affect returned candidates, with the ordinary consumption order
/// retained for long notes. The default pass remains bit-for-bit compatible.
fn decode_pitch_with_limits(
    frames: &[f32],
    onsets: &[f32],
    minimum_frames: usize,
    maximum_frames: usize,
) -> Vec<PitchCandidateEvidence> {
    let count = frames.len() / 88;
    if count < 3 || frames.len() != onsets.len() {
        return Vec::new();
    }
    let mut inferred = onsets.to_vec();
    let mut differences = vec![0.0f32; frames.len()];
    let mut maximum = 0.0f32;
    for t in 2..count {
        for p in 0..88 {
            let d = (frames[t * 88 + p] - frames[(t - 1) * 88 + p])
                .min(frames[t * 88 + p] - frames[(t - 2) * 88 + p])
                .max(0.0);
            differences[t * 88 + p] = d;
            maximum = maximum.max(d);
        }
    }
    let onset_max = onsets.iter().copied().fold(0.0f32, f32::max);
    if maximum > 0.0 {
        for (i, d) in differences.iter().enumerate() {
            inferred[i] = inferred[i].max(d / maximum * onset_max);
        }
    }
    let mut remaining = frames.to_vec();
    let mut out = Vec::new();
    let time = pitch_frame_seconds;
    for start in (1..count - 1).rev() {
        for p in (0..88).rev() {
            let v = inferred[start * 88 + p];
            if v < 0.5 || v <= inferred[(start - 1) * 88 + p] || v <= inferred[(start + 1) * 88 + p]
            {
                continue;
            }
            let mut end = start + 1;
            let mut below = 0;
            while end < count - 1 && below < 11 {
                if remaining[end * 88 + p] < 0.3 {
                    below += 1;
                } else {
                    below = 0;
                }
                end += 1;
            }
            end -= below;
            if end.saturating_sub(start) <= minimum_frames {
                continue;
            }
            let amplitude =
                (start..end).map(|t| frames[t * 88 + p]).sum::<f32>() / (end - start) as f32;
            for t in start..end {
                for neighbor in p.saturating_sub(1)..=(p + 1).min(87) {
                    remaining[t * 88 + neighbor] = 0.0;
                }
            }
            if end - start > maximum_frames {
                continue;
            }
            out.push(PitchCandidateEvidence {
                candidate: PitchNote {
                    start: time(start),
                    end: time(end),
                    pitch: p as u8 + 21,
                    confidence: amplitude.clamp(0.0, 1.0),
                },
                source: if onsets[start * 88 + p] >= 0.5 {
                    PitchCandidateSource::NetworkOnset
                } else {
                    PitchCandidateSource::InferredFrameRise
                },
                onset_score: onsets[start * 88 + p].clamp(0., 1.),
                inferred_onset_score: inferred[start * 88 + p].clamp(0., 1.),
            });
        }
    }
    // Sort Melodia's peak candidates once. Each step only removes energy,
    // so this preserves the selection order without rescanning the whole matrix.
    let mut candidates: Vec<_> = remaining
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0.3)
        .map(|(i, v)| (i, *v))
        .collect();
    candidates.sort_by(|(ia, a), (ib, b)| b.total_cmp(a).then(ia.cmp(ib)));
    for (index, _) in candidates {
        if remaining[index] <= 0.3 {
            continue;
        }
        let middle = index / 88;
        let pitch = index % 88;
        remaining[index] = 0.0;
        let mut i = middle + 1;
        let mut below = 0;
        while i < count - 1 && below < 11 {
            if remaining[i * 88 + pitch] < 0.3 {
                below += 1;
            } else {
                below = 0;
            }
            for neighbor in pitch.saturating_sub(1)..=(pitch + 1).min(87) {
                remaining[i * 88 + neighbor] = 0.0;
            }
            i += 1;
        }
        let end = i.saturating_sub(1 + below);
        let mut i = middle as isize - 1;
        let mut below = 0;
        while i > 0 && below < 11 {
            if remaining[i as usize * 88 + pitch] < 0.3 {
                below += 1;
            } else {
                below = 0;
            }
            for neighbor in pitch.saturating_sub(1)..=(pitch + 1).min(87) {
                remaining[i as usize * 88 + neighbor] = 0.0;
            }
            i -= 1;
        }
        let start = (i + 1 + below as isize).max(0) as usize;
        if end.saturating_sub(start) <= minimum_frames || end.saturating_sub(start) > maximum_frames
        {
            continue;
        }
        let amplitude =
            (start..end).map(|t| frames[t * 88 + pitch]).sum::<f32>() / (end - start) as f32;
        out.push(PitchCandidateEvidence {
            candidate: PitchNote {
                start: time(start),
                end: time(end),
                pitch: pitch as u8 + 21,
                confidence: amplitude.clamp(0.0, 1.0),
            },
            source: PitchCandidateSource::FrameRecovery,
            onset_score: onsets[start * 88 + pitch].clamp(0., 1.),
            inferred_onset_score: inferred[start * 88 + pitch].clamp(0., 1.),
        });
    }
    out.sort_by(|a, b| {
        a.candidate
            .start
            .total_cmp(&b.candidate.start)
            .then(a.candidate.pitch.cmp(&b.candidate.pitch))
    });
    out
}

#[derive(Debug)]
struct ShortBassSupport {
    reason: &'static str,
    source_flux_seconds: Option<f64>,
    periodic_pitch: Option<f64>,
    periodicity: Option<f64>,
    fundamental_fraction: Option<f64>,
    window_start: Option<f64>,
    window_end: Option<f64>,
}

fn short_pluck_support(
    evidence: &PitchCandidateEvidence,
    baseline: &[PitchCandidateEvidence],
    hypotheses: &[PitchCandidateEvidence],
    audio: &[f32],
    peaks: &[crate::review::Attack],
    pitches: &std::ops::RangeInclusive<u8>,
) -> ShortBassSupport {
    use pitch_detection::detector::{PitchDetector, yin::YINDetector};
    let mut support = ShortBassSupport {
        reason: "source_supported",
        source_flux_seconds: None,
        periodic_pitch: None,
        periodicity: None,
        fundamental_fraction: None,
        window_start: None,
        window_end: None,
    };
    let note = &evidence.candidate;
    let reject = |mut support: ShortBassSupport, reason| {
        support.reason = reason;
        support
    };
    if evidence.source != PitchCandidateSource::NetworkOnset
        || evidence.onset_score < 0.75
        || note.confidence < 0.5
    {
        return reject(support, "network_support_not_strong");
    }
    if !pitches.contains(&note.pitch) || note.end - note.start < 0.055 {
        return reject(
            support,
            if *pitches == (24..=67) {
                "outside_short_bass_policy"
            } else {
                "outside_short_guitar_policy"
            },
        );
    }
    if baseline.iter().any(|original| {
        original.candidate.pitch == note.pitch
            && (original.candidate.start - note.start).abs() <= 0.025
    }) {
        return reject(support, "already_represented");
    }
    if baseline.iter().any(|original| {
        let lower = &original.candidate;
        lower.start <= note.start + 0.025
            && lower.end >= note.start + 0.025
            && lower.confidence >= note.confidence * 0.65
            && [12, 19, 24, 28, 31, 36].contains(&(note.pitch as i16 - lower.pitch as i16))
    }) {
        return reject(support, "overlapping_lower_harmonic");
    }
    let Some(peak) = crate::review::nearest(note.start, peaks)
        .filter(|peak| (peak.time - note.start).abs() <= 0.035)
    else {
        return reject(support, "no_independent_source_attack");
    };
    support.source_flux_seconds = Some(peak.time);
    let following = baseline
        .iter()
        .chain(hypotheses)
        .map(|e| &e.candidate)
        .filter(|next| next.start > note.start + 0.025)
        .map(|next| next.start)
        .min_by(f64::total_cmp);
    let start = note.start + 0.003;
    let end = note.end.min(following.unwrap_or(note.end)) - 0.003;
    support.window_start = Some(start);
    support.window_end = Some(end);
    let lo = ((start * 11025.).ceil() as usize).min(audio.len());
    let mut hi = ((end.max(0.) * 11025.).floor() as usize).min(audio.len());
    if (hi.saturating_sub(lo)) % 2 == 1 {
        hi = hi.saturating_sub(1);
    }
    let available = hi.saturating_sub(lo);
    // pitch-detection's FFT scratch allocation is only valid for its usual
    // smooth transform sizes. Use a smaller real window, never zero-pad a
    // short event to manufacture cycles or borrow the following note.
    let size = if available == 0 {
        0
    } else {
        1usize << available.ilog2()
    };
    hi = lo + size;
    support.window_start = (hi > lo).then_some(lo as f64 / 11025.);
    support.window_end = (hi > lo).then_some(hi as f64 / 11025.);
    let nominal = 440. * 2f64.powf((note.pitch as f64 - 69.) / 12.);
    if size < 64 || size as f64 * nominal / 11025. < 3.5 {
        return reject(support, "insufficient_owned_cycles");
    }
    let signal: Vec<_> = audio[lo..hi].iter().map(|sample| *sample as f64).collect();
    let rms = (signal.iter().map(|sample| sample * sample).sum::<f64>() / size as f64).sqrt();
    if !rms.is_finite() || rms < 0.006 {
        return reject(support, "source_too_quiet");
    }
    let mut detector = YINDetector::<f64>::new(size, size / 2);
    let Some(pitch) = detector.get_pitch(&signal, 11025, 0.01, 0.94) else {
        return reject(support, "unstable_source_pitch");
    };
    let midi = 69. + 12. * (pitch.frequency / 440.).log2();
    let periodicity = crate::refine::periodicity(&signal, pitch.frequency, 11025.);
    support.periodic_pitch = (midi.is_finite() && (0.0..=127.0).contains(&midi)).then_some(midi);
    support.periodicity =
        (periodicity.is_finite() && (-1.0..=1.0).contains(&periodicity)).then_some(periodicity);
    if !midi.is_finite()
        || (midi - note.pitch as f64).abs() > 0.35
        || !periodicity.is_finite()
        || periodicity < 0.94
        || size as f64 * pitch.frequency / 11025. < 3.5
    {
        return reject(support, "source_pitch_disagrees");
    }
    let power = |harmonic: f64| {
        let (mut re, mut im) = (0., 0.);
        for (index, sample) in signal.iter().enumerate() {
            let window =
                0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / (size - 1) as f64).cos();
            let phase = std::f64::consts::TAU * pitch.frequency * harmonic * index as f64 / 11025.;
            re += sample * window * phase.cos();
            im += sample * window * phase.sin();
        }
        re * re + im * im
    };
    let fundamental = power(1.);
    let strongest = (2..=5)
        .map(|harmonic| power(harmonic as f64))
        .fold(fundamental, f64::max);
    let fraction = fundamental / strongest.max(1e-30);
    support.fundamental_fraction =
        (fraction.is_finite() && (0. ..=1.).contains(&fraction)).then_some(fraction);
    if !fraction.is_finite() || fraction < 0.04 {
        return reject(support, "weak_fundamental");
    }
    support
}

/// Original recording coordinates of the stitched Basic Pitch activations.
/// Each 43,844-sample input window advances by 36,164 samples. We discard
/// fifteen frames at either edge, retaining 142 frames; the initial fifteen
/// frames of audio padding cancel the leading trim. A concatenated frame is
/// therefore not on a uniform 256-sample grid across window boundaries.
/// Use the actual window origin, without upstream's empirical drift offset.
pub fn pitch_frame_seconds(frame: usize) -> f64 {
    const KEPT: usize = 172 - 30;
    const HOP: usize = 43844 - 30 * 256;
    ((frame / KEPT) * HOP + (frame % KEPT) * 256) as f64 / 22050.0
}

#[cfg(test)]
mod bass_short_tests {
    use super::*;

    fn fixture(pitch: u8, start: usize, length: usize, phase: f64) -> (PitchActivations, Vec<f32>) {
        let mut frames = vec![0.; 100.max(start + length + 20) * 88];
        let mut onsets = frames.clone();
        for frame in start..start + length {
            frames[frame * 88 + (pitch - 21) as usize] = 0.8;
        }
        onsets[start * 88 + (pitch - 21) as usize] = 0.9;
        let waveform = |rate: usize| {
            (0..((pitch_frame_seconds(start + length) + 0.2).max(1.) * rate as f64).ceil() as usize)
                .map(|index| {
                    let t = index as f64 / rate as f64;
                    let a = pitch_frame_seconds(start);
                    let b = pitch_frame_seconds(start + length);
                    if t < a || t >= b {
                        0.
                    } else {
                        let f = 440. * 2f64.powf((pitch as f64 - 69.) / 12.);
                        let p = std::f64::consts::TAU * f * (t - a) + phase;
                        (0.3 * ((t - a) / 0.002).min(1.) * (p.sin() + 0.3 * (2. * p).sin())) as f32
                    }
                })
                .collect()
        };
        (
            PitchActivations {
                frames,
                onsets,
                audio: waveform(22050),
            },
            waveform(11025),
        )
    }

    #[test]
    fn source_supported_short_plucks_recover_without_changing_default_decoder() {
        let mut count = 0;
        for pitch in [40, 45, 52, 59] {
            for length in [6, 8, 11] {
                for phase in [0., 0.7, 1.4, 2.1] {
                    let (input, audio) = fixture(pitch, 20, length, phase);
                    assert!(decode_pitch_with_evidence(&input.frames, &input.onsets).is_empty());
                    let recovered = recover_short_bass(&input, &audio);
                    assert_eq!(
                        recovered.candidates.len(),
                        1,
                        "pitch{pitch} length{length} phase{phase}: {:?}",
                        recovered
                            .short_candidates
                            .iter()
                            .map(|c| &c.reason)
                            .collect::<Vec<_>>()
                    );
                    let candidate = &recovered.candidates[0];
                    assert_eq!(candidate.candidate.pitch, pitch);
                    assert_eq!(candidate.candidate.start, pitch_frame_seconds(20));
                    assert_eq!(candidate.source, PitchCandidateSource::NetworkOnset);
                    assert!(recovered.short_candidates[0].retained);
                    count += 1;
                }
            }
        }
        assert_eq!(count, 48);
    }

    #[test]
    fn short_guitar_plucks_keep_source_clock_and_require_pitch_and_attack_support() {
        for pitch in [40, 52, 64, 76, 88] {
            for start in [20, 141, 142] {
                let (input, audio) = fixture(pitch, start, 8, 0.7);
                let result = recover_short_plucks(&input, &audio, 40..=88);
                assert_eq!(
                    result.candidates.len(),
                    1,
                    "{pitch} {start}: {:?}",
                    result
                        .short_candidates
                        .iter()
                        .map(|c| &c.reason)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    result.candidates[0].candidate.start,
                    pitch_frame_seconds(start)
                );
                let (_, wrong) = fixture(pitch - 1, start, 8, 0.7);
                assert!(
                    recover_short_plucks(&input, &wrong, 40..=88)
                        .candidates
                        .is_empty()
                );
                let silent = PitchActivations {
                    audio: vec![0.; input.audio.len()],
                    ..input
                };
                assert!(
                    recover_short_plucks(&silent, &audio, 40..=88)
                        .candidates
                        .is_empty()
                );
            }
        }
        let (input, audio) = fixture(64, 20, 24, 0.7);
        let before = decode_pitch_with_evidence(&input.frames, &input.onsets);
        let after = recover_short_plucks(&input, &audio, 40..=88);
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after.candidates).unwrap()
        );
    }

    #[test]
    fn ringing_harmonics_wrong_pitches_and_weak_model_onsets_are_not_short_notes() {
        let (input, mut audio) = fixture(52, 20, 8, 0.7);
        for (index, sample) in audio.iter_mut().enumerate() {
            *sample =
                (0.3 * (std::f64::consts::TAU * 82.40689 * index as f64 / 11025.).sin()) as f32;
        }
        let mut ringing = input.audio.clone();
        for (index, sample) in ringing.iter_mut().enumerate() {
            *sample =
                (0.3 * (std::f64::consts::TAU * 82.40689 * index as f64 / 22050.).sin()) as f32;
        }
        let no_attack = PitchActivations {
            audio: ringing,
            ..input
        };
        assert!(recover_short_bass(&no_attack, &audio).candidates.is_empty());
        let (mut input, _) = fixture(52, 20, 8, 0.7);
        let (_, wrong) = fixture(40, 20, 8, 0.7);
        assert!(recover_short_bass(&input, &wrong).candidates.is_empty());
        input.onsets[20 * 88 + 31] = 0.6;
        assert!(recover_short_bass(&input, &wrong).candidates.is_empty());
        let (input, audio) = fixture(28, 20, 6, 0.7);
        assert!(recover_short_bass(&input, &audio).candidates.is_empty());
    }

    #[test]
    fn short_recovery_requires_actual_network_onsets_and_keeps_join_clock() {
        for source in [
            PitchCandidateSource::InferredFrameRise,
            PitchCandidateSource::FrameRecovery,
        ] {
            let (mut input, audio) = fixture(45, 20, 8, 0.7);
            input.onsets.fill(0.);
            if source == PitchCandidateSource::InferredFrameRise {
                input.onsets[87] = 0.9;
            }
            let result = recover_short_bass(&input, &audio);
            assert!(result.candidates.is_empty());
            assert!(result.short_candidates.is_empty());
        }
        let (input, audio) = fixture(45, 141, 8, 1.4);
        let result = recover_short_bass(&input, &audio);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].candidate.start,
            pitch_frame_seconds(141)
        );
        assert_eq!(result.candidates[0].candidate.end, pitch_frame_seconds(149));
        let (mut input, audio) = fixture(45, 20, 8, 0.7);
        input.onsets[20 * 88 + 24] = 0.6;
        let result = recover_short_bass(&input, &audio);
        assert!(!result.short_candidates[0].retained);
        assert_eq!(
            result.short_candidates[0].reason,
            "network_support_not_strong"
        );
        let (input, mut audio) = fixture(45, 20, 8, 0.7);
        audio.truncate(100);
        let result = recover_short_bass(&input, &audio);
        assert!(result.candidates.is_empty());
        let events = crate::mapping::Transcription {
            bass_short_candidates: result.short_candidates,
            ..Default::default()
        };
        events.validate().unwrap();
    }

    #[test]
    fn later_attack_cannot_lend_its_cycles_and_original_long_candidates_survive() {
        let (mut input, audio) = fixture(40, 20, 8, 0.7);
        for frame in 23..70 {
            input.frames[frame * 88 + 24] = 0.8;
        }
        input.onsets[23 * 88 + 24] = 0.9;
        let baseline = decode_pitch_with_evidence(&input.frames, &input.onsets);
        assert_eq!(baseline.len(), 1);
        let recovered = recover_short_bass(&input, &audio);
        assert_eq!(
            serde_json::to_value(&baseline).unwrap(),
            serde_json::to_value(&recovered.candidates).unwrap()
        );
        assert!(
            recovered
                .short_candidates
                .iter()
                .any(|c| c.reason == "insufficient_owned_cycles")
        );
    }
}

#[cfg(test)]
mod candidate_provenance_tests {
    use super::*;
    #[test]
    fn decoder_retains_network_inferred_and_frame_recovery_evidence() {
        for source in [
            PitchCandidateSource::NetworkOnset,
            PitchCandidateSource::InferredFrameRise,
            PitchCandidateSource::FrameRecovery,
        ] {
            let mut frames = vec![0.; 100 * 88];
            let mut onsets = vec![0.; 100 * 88];
            for frame in 10..50 {
                frames[frame * 88 + 19] = 0.8;
            }
            match source {
                PitchCandidateSource::NetworkOnset => onsets[10 * 88 + 19] = 0.9,
                PitchCandidateSource::InferredFrameRise => onsets[87] = 0.9,
                PitchCandidateSource::FrameRecovery => (),
            }
            let decoded = decode_pitch_with_evidence(&frames, &onsets);
            assert_eq!(decoded.len(), 1);
            assert_eq!(decoded[0].source, source);
            let legacy = decode_pitch(&frames, &onsets);
            assert_eq!(
                serde_json::to_value(&legacy[0]).unwrap(),
                serde_json::to_value(&decoded[0].candidate).unwrap()
            );
            let saved = serde_json::to_string(&decoded).unwrap();
            let restored: Vec<PitchCandidateEvidence> = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored[0].source, source);
            assert_eq!(
                restored[0].onset_score.to_bits(),
                decoded[0].onset_score.to_bits()
            );
        }
    }
}

#[cfg(test)]
mod pitch_clock_tests {
    use super::*;

    #[test]
    fn stitched_frames_keep_their_real_source_window_origins() {
        for window in [0, 1, 2, 100, 500, 5000] {
            for local in [0, 1, 70, 141] {
                let expected = (window * 36164 + local * 256) as f64 / 22050.;
                assert_eq!(pitch_frame_seconds(window * 142 + local), expected);
            }
        }
        for frame in 1..100_000 {
            assert!(pitch_frame_seconds(frame) > pitch_frame_seconds(frame - 1));
        }
        // An activation at the same local frame in each window must follow the
        // input stride even near the end of a long recording.
        let frame = 320 * 142 + 31;
        assert!((pitch_frame_seconds(frame) - (320. * 36164. + 31. * 256.) / 22050.).abs() < 1e-12);
    }

    #[test]
    fn decoder_notes_and_ends_use_source_coordinates_across_a_join() {
        let count = 450;
        let mut frames = vec![0.; count * 88];
        let mut onsets = vec![0.; count * 88];
        for (start, end) in [(130, 165), (300, 333)] {
            for t in start..end {
                frames[t * 88 + 19] = 0.8;
            }
            onsets[start * 88 + 19] = 0.9;
        }
        let notes = decode_pitch(&frames, &onsets);
        assert_eq!(notes.len(), 2);
        for (note, (start, end)) in notes.iter().zip([(130, 165), (300, 333)]) {
            assert_eq!(note.start, pitch_frame_seconds(start));
            assert_eq!(note.end, pitch_frame_seconds(end));
        }
    }
}
