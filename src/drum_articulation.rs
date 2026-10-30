//! Compare drum sounds while keeping the original hit times.
//! Frequency groups suggest kit pieces for review; they do not identify them.
use crate::mapping::{DrumHit, Voice};
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

pub const ALGORITHM: &str = "relative-drum-articulation-v1";
pub const CYMBAL_MODEL_SHA256: &str =
    "23d4b05ce7e9715bdb4396823d68bc886e8a958fa9f6266c0f7132b78098b521";
/// ONNX sigmoid SIMD rounding only, not a musical admission threshold.
pub const SCORE_EPSILON: f32 = 1e-6;
const RATE: usize = 22050;
const FFT: usize = 2048;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub version: u32,
    pub algorithm: String,
    pub recording_sha256: String,
    pub stem_sha256: String,
    pub source_frames: u64,
}
impl Source {
    pub fn from_manifest(manifest: &crate::stem_review::StemManifest) -> Result<Self> {
        let stem = manifest
            .stems
            .iter()
            .find(|s| s.id == crate::stem_review::StemId::Drums)
            .ok_or_else(|| anyhow::anyhow!("Drum articulation requires a bound drum stem"))?;
        Ok(Self {
            version: 1,
            algorithm: ALGORITHM.into(),
            recording_sha256: manifest.source_audio_sha256.clone(),
            stem_sha256: stem.sha256.clone(),
            source_frames: manifest.frames,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub hit_index: usize,
    pub time: f64,
    pub original: Voice,
    pub original_confidence: f32,
    pub proposed: Option<Voice>,
    /// Policy eligibility, never a human review approval. Experimental DSP
    /// proposals remain false until independently accepted.
    #[serde(default)]
    pub map_proposal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cymbal_model: Option<CymbalModelEvidence>,
    /// Heuristic separation margin, not a probability of correctness.
    pub confidence: f64,
    pub reason: String,
    pub resonance_hz: Option<f64>,
    pub resonance_spread_semitones: Option<f64>,
    pub spectral_peak_fraction: Option<f64>,
    pub spectral_flatness: Option<f64>,
    pub window_start: Option<f64>,
    pub window_end: Option<f64>,
    pub window_samples: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CymbalModelEvidence {
    pub model_sha256: String,
    /// GM pitch order36,38,48,42,51,53,49,75. Per-class maxima in the owned
    /// +/-35ms neighborhood, not a calibrated probability of an instrument.
    pub class_peaks: [f32; 8],
    pub raw_class_peaks: [f32; 8],
    pub clipped_values: usize,
    pub window_start: f64,
    pub window_end: f64,
    pub peak_time: f64,
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn spectrum(audio: &[f32], start: usize, physical: usize, fft: &dyn rustfft::Fft<f64>) -> Vec<f64> {
    let mut buffer: Vec<_> = audio[start..start + physical]
        .iter()
        .enumerate()
        .map(|(i, sample)| {
            Complex::new(
                *sample as f64
                    * (0.5
                        - 0.5 * (std::f64::consts::TAU * i as f64 / (physical - 1) as f64).cos()),
                0.,
            )
        })
        .collect();
    buffer.resize(FFT, Complex::new(0., 0.));
    fft.process(&mut buffer);
    buffer[..=FFT / 2].iter().map(|v| v.norm_sqr()).collect()
}

fn features(audio: &[f32], hits: &[DrumHit]) -> Vec<Evidence> {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT);
    hits.iter()
        .enumerate()
        .filter(|(_, h)| matches!(h.voice, Voice::Tom | Voice::Cymbal))
        .map(|(index, hit)| {
            let mut evidence = Evidence {
                hit_index: index,
                time: hit.time,
                original: hit.voice,
                original_confidence: hit.confidence,
                proposed: None,
                map_proposal: false,
                cymbal_model: None,
                confidence: 0.,
                reason: "insufficient_owned_audio".into(),
                resonance_hz: None,
                resonance_spread_semitones: None,
                spectral_peak_fraction: None,
                spectral_flatness: None,
                window_start: None,
                window_end: None,
                window_samples: None,
            };
            let start = (hit.time * RATE as f64).round() as usize;
            let next = hits
                .iter()
                .filter(|h| h.time > hit.time + 0.025)
                .map(|h| (h.time * RATE as f64).round() as usize)
                .min()
                .unwrap_or(audio.len())
                .min(audio.len());
            let offsets = [55usize, 220, 441];
            let available = next.saturating_sub(start.saturating_add(offsets[2]));
            let Some(physical) = [2048, 1024, 512].into_iter().find(|n| *n <= available) else {
                return evidence;
            };
            evidence.window_start = Some((start + offsets[0]) as f64 / RATE as f64);
            evidence.window_end = Some((start + offsets[2] + physical) as f64 / RATE as f64);
            evidence.window_samples = Some(physical);
            if hits.iter().enumerate().any(|(i, h)| {
                i != index
                    && (h.time - hit.time).abs() <= 0.025
                    && match hit.voice {
                        Voice::Tom => matches!(h.voice, Voice::Kick | Voice::Snare | Voice::Tom),
                        _ => matches!(h.voice, Voice::HiHat | Voice::Cymbal),
                    }
            }) {
                evidence.reason = "overlapping_same_band_attack".into();
                return evidence;
            }
            let powers: Vec<_> = offsets
                .iter()
                .map(|offset| spectrum(audio, start + offset, physical, fft.as_ref()))
                .collect();
            if powers.iter().any(|p| p.iter().sum::<f64>() < 1e-7) {
                evidence.reason = "quiet_owned_audio".into();
                return evidence;
            }
            if hit.voice == Voice::Tom {
                let mut pitches = vec![];
                for power in &powers {
                    let lo = 7usize;
                    let hi = 43usize;
                    let total = power[lo..hi].iter().sum::<f64>();
                    let peak = (lo + 1..hi - 1)
                        .max_by(|a, b| power[*a].total_cmp(&power[*b]))
                        .unwrap();
                    let concentration =
                        power[peak - 1..=peak + 1].iter().sum::<f64>() / total.max(1e-20);
                    let low_fraction = total / power[lo..930].iter().sum::<f64>().max(1e-20);
                    if concentration < 0.35 || low_fraction < 0.20 {
                        evidence.reason = "diffuse_tom_resonance".into();
                        return evidence;
                    }
                    let logs = [
                        power[peak - 1].max(1e-20).ln(),
                        power[peak].max(1e-20).ln(),
                        power[peak + 1].max(1e-20).ln(),
                    ];
                    let displacement = (0.5 * (logs[0] - logs[2])
                        / (logs[0] - 2. * logs[1] + logs[2]))
                        .clamp(-0.5, 0.5);
                    pitches.push((peak as f64 + displacement) * RATE as f64 / FFT as f64);
                }
                let pitch = median(&pitches);
                if pitch * physical as f64 / (RATE as f64) < 3.5 {
                    evidence.reason = "insufficient_owned_cycles".into();
                    return evidence;
                }
                let spread = pitches
                    .iter()
                    .map(|p| (12. * (p / pitch).log2()).abs())
                    .fold(0., f64::max);
                evidence.resonance_hz = Some(pitch);
                evidence.resonance_spread_semitones = Some(spread);
                evidence.reason = if spread <= 1.2 {
                    "unresolved_kit_pitch_groups"
                } else {
                    "unstable_tom_resonance"
                }
                .into();
            } else {
                let mut peaks = vec![];
                let mut flatness = vec![];
                for power in &powers {
                    let band = &power[140..930];
                    let total = band.iter().sum::<f64>();
                    peaks.push(
                        band.windows(3)
                            .map(|b| b.iter().sum::<f64>())
                            .fold(0., f64::max)
                            / total.max(1e-20),
                    );
                    flatness.push(
                        (band.iter().map(|p| p.max(1e-20).ln()).sum::<f64>() / band.len() as f64)
                            .exp()
                            / (total / band.len() as f64).max(1e-20),
                    );
                }
                evidence.spectral_peak_fraction = Some(median(&peaks));
                evidence.spectral_flatness = Some(median(&flatness));
                evidence.reason = "unresolved_cymbal_timbre_groups".into();
            }
            evidence
        })
        .collect()
}

fn group_toms(evidence: &mut [Evidence]) {
    let mut points: Vec<_> = evidence
        .iter()
        .enumerate()
        .filter(|(_, e)| e.reason == "unresolved_kit_pitch_groups")
        .map(|(i, e)| (i, 12. * e.resonance_hz.unwrap().log2()))
        .collect();
    points.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut groups: Vec<Vec<(usize, f64)>> = vec![];
    for point in points {
        if groups
            .last()
            .and_then(|g| g.last())
            .is_none_or(|prev| point.1 - prev.1 >= 2.5)
        {
            groups.push(vec![]);
        }
        groups.last_mut().unwrap().push(point);
    }
    // A singleton or continuous pitch cloud cannot establish a kit register.
    if groups.len() != 3
        || groups
            .iter()
            .any(|g| g.len() < 3 || g.last().unwrap().1 - g[0].1 > 2.5)
    {
        return;
    }
    let centers: Vec<_> = groups
        .iter()
        .map(|g| median(&g.iter().map(|p| p.1).collect::<Vec<_>>()))
        .collect();
    if centers.windows(2).any(|c| c[1] - c[0] < 3.) {
        return;
    }
    for (group_index, group) in groups.iter().enumerate() {
        let voice = if group_index == 0 {
            Voice::LowTom
        } else if group_index == groups.len() - 1 {
            Voice::HighTom
        } else {
            Voice::MidTom
        };
        let gap = centers
            .windows(2)
            .map(|c| c[1] - c[0])
            .fold(f64::INFINITY, f64::min);
        let width = group.last().unwrap().1 - group[0].1;
        for (index, _) in group {
            let e = &mut evidence[*index];
            e.proposed = Some(voice);
            e.confidence =
                (gap / (gap + 1. + width + e.resonance_spread_semitones.unwrap())).clamp(0., 1.);
            e.reason = "relative_repeated_tom_resonance".into();
        }
    }
}

fn group_cymbals(evidence: &mut [Evidence]) {
    let mut points: Vec<_> = evidence
        .iter()
        .enumerate()
        .filter(|(_, e)| e.reason == "unresolved_cymbal_timbre_groups")
        .map(|(i, e)| (i, e.spectral_peak_fraction.unwrap().max(1e-12).ln()))
        .collect();
    points.sort_by(|a, b| a.1.total_cmp(&b.1));
    // Look for two separated timbre populations, never split a unimodal cloud
    // by rank or fill a desired lane count.
    if points.len() < 8 {
        return;
    }
    let Some(split) = (4..=points.len() - 4).max_by(|a, b| {
        (points[*a].1 - points[*a - 1].1).total_cmp(&(points[*b].1 - points[*b - 1].1))
    }) else {
        return;
    };
    if points[split].1 - points[split - 1].1 < 0.7 {
        return;
    }
    let (broad, tonal) = points.split_at(split);
    let broad_flat = median(
        &broad
            .iter()
            .map(|(i, _)| evidence[*i].spectral_flatness.unwrap())
            .collect::<Vec<_>>(),
    );
    let tonal_flat = median(
        &tonal
            .iter()
            .map(|(i, _)| evidence[*i].spectral_flatness.unwrap())
            .collect::<Vec<_>>(),
    );
    if broad_flat < 0.07
        || tonal_flat > broad_flat * 0.65
        || tonal
            .iter()
            .any(|(i, _)| evidence[*i].spectral_peak_fraction.unwrap() < 0.08)
    {
        return;
    }
    for (population, voice) in [(broad, Voice::Crash), (tonal, Voice::Ride)] {
        for (index, _) in population {
            let e = &mut evidence[*index];
            e.proposed = Some(voice);
            let gap = tonal[0].1 - broad.last().unwrap().1;
            e.confidence = ((gap / (1. + gap)) * (1. - tonal_flat / broad_flat))
                .clamp(0., 1.)
                .sqrt();
            e.reason = "separated_broad_and_tonal_cymbal_populations".into();
        }
    }
}

/// No reference targets, tempo or lane counts enter this analysis. PCM must be
/// a finite mono 22.05 kHz source-clock drum stem, not a re-timed excerpt.
pub fn analyze(audio: &[f32], hits: &[DrumHit]) -> Result<Vec<Evidence>> {
    ensure!(
        audio.len() <= RATE * 3600 && audio.iter().all(|s| s.is_finite()),
        "Invalid drum articulation PCM"
    );
    ensure!(
        hits.len() <= 20000
            && hits.iter().all(|h| h.time.is_finite()
                && h.time >= 0.
                && h.time <= audio.len() as f64 / RATE as f64
                && h.confidence.is_finite()
                && (0.0..=1.).contains(&h.confidence)),
        "Invalid drum articulation hit clock"
    );
    let mut evidence = features(audio, hits);
    group_toms(&mut evidence);
    group_cymbals(&mut evidence);
    Ok(evidence)
}

pub fn validate(evidence: &[Evidence], hits: &[DrumHit]) -> Result<()> {
    let mut indices = std::collections::BTreeSet::new();
    for e in evidence {
        ensure!(
            indices.insert(e.hit_index),
            "Duplicate drum articulation evidence"
        );
        let hit = hits
            .get(e.hit_index)
            .ok_or_else(|| anyhow::anyhow!("Drum articulation hit missing"))?;
        ensure!(
            e.time == hit.time
                && e.original == hit.voice
                && e.original_confidence == hit.confidence
                && matches!(hit.voice, Voice::Tom | Voice::Cymbal)
                && e.confidence.is_finite()
                && (0.0..=1.).contains(&e.confidence)
                && !e.reason.trim().is_empty(),
            "Drum articulation source changed"
        );
        ensure!(
            e.proposed.is_none_or(|v| match hit.voice {
                Voice::Tom => matches!(v, Voice::HighTom | Voice::MidTom | Voice::LowTom),
                _ => matches!(v, Voice::Crash | Voice::Ride),
            }),
            "Drum articulation crosses instrument family"
        );
        ensure!(
            [
                e.resonance_hz,
                e.resonance_spread_semitones,
                e.spectral_peak_fraction,
                e.spectral_flatness,
                e.window_start,
                e.window_end
            ]
            .into_iter()
            .flatten()
            .all(|v| v.is_finite() && v >= 0.),
            "Invalid drum articulation measurements"
        );
        ensure!(
            e.resonance_hz
                .is_none_or(|v| v > 0. && v <= RATE as f64 / 2.)
                && e.spectral_peak_fraction.is_none_or(|v| v <= 1.)
                && e.spectral_flatness.is_none_or(|v| v <= 1. + 1e-9),
            "Out-of-range drum articulation measurements"
        );
        ensure!(
            e.window_start.is_some() == e.window_end.is_some()
                && e.window_start.is_some() == e.window_samples.is_some()
                && e.window_start
                    .zip(e.window_end)
                    .is_none_or(|(start, end)| start >= e.time && end > start)
                && e.window_samples
                    .is_none_or(|n| [512, 1024, 2048].contains(&n)),
            "Invalid drum articulation source window"
        );
        ensure!(
            e.proposed.is_none() || e.window_start.is_some() || e.cymbal_model.is_some(),
            "Drum articulation proposal has no source window"
        );
        if let Some(model) = &e.cymbal_model {
            ensure!(
                e.original == Voice::Cymbal
                    && model.model_sha256 == CYMBAL_MODEL_SHA256
                    && model
                        .class_peaks
                        .iter()
                        .all(|s| s.is_finite() && (0.0..=1.).contains(s))
                    && model
                        .raw_class_peaks
                        .iter()
                        .all(|s| s.is_finite() && (-SCORE_EPSILON..=1. + SCORE_EPSILON).contains(s))
                    && model
                        .class_peaks
                        .iter()
                        .zip(model.raw_class_peaks)
                        .all(|(p, raw)| *p == raw.clamp(0., 1.))
                    && model.clipped_values <= 64
                    && [model.window_start, model.window_end, model.peak_time]
                        .iter()
                        .all(|t| t.is_finite() && *t >= 0.)
                    && model.window_start <= e.time
                    && e.time <= model.window_end
                    && e.time - model.window_start <= 0.035 + 1e-9
                    && model.window_end - e.time <= 0.035 + 1e-9
                    && model.window_start <= model.peak_time
                    && model.peak_time <= model.window_end
                    && (model.peak_time - e.time).abs() <= 0.035 + 1e-9,
                "Invalid cymbal model evidence"
            );
        }
        ensure!(
            !e.map_proposal || e.proposed.is_some(),
            "Mapped articulation has no proposal"
        );
    }
    Ok(())
}

pub fn validate_source(
    source: Option<&Source>,
    evidence: &[Evidence],
    manifest: Option<&crate::stem_review::StemManifest>,
) -> Result<()> {
    if evidence.is_empty() {
        return Ok(());
    }
    if let Some(manifest) = manifest {
        let source =
            source.ok_or_else(|| anyhow::anyhow!("Drum articulation source identity missing"))?;
        let expected = Source::from_manifest(manifest)?;
        ensure!(
            source.version == 1
                && source.algorithm == ALGORITHM
                && source.recording_sha256 == expected.recording_sha256
                && source.stem_sha256 == expected.stem_sha256
                && source.source_frames == expected.source_frames,
            "Drum articulation recording/stem changed"
        );
        let duration = manifest.frames as f64 / manifest.sample_rate as f64;
        ensure!(
            evidence.iter().all(|e| e
                .window_end
                .is_none_or(|end| end <= duration + 1. / RATE as f64)
                && e.cymbal_model
                    .as_ref()
                    .is_none_or(|m| m.window_end <= duration + 1. / RATE as f64)),
            "Drum articulation window exceeds source clock"
        );
    } else {
        ensure!(
            source.is_none(),
            "Drum articulation bound source has no manifest"
        );
    }
    Ok(())
}

/// Raw events are retained separately. This only changes the proposed voice.
pub fn proposed_hits(hits: &[DrumHit], evidence: &[Evidence]) -> Vec<DrumHit> {
    let mut result = hits.to_vec();
    if validate(evidence, hits).is_err() {
        return result;
    }
    for e in evidence {
        if let Some(voice) = e.proposed.filter(|_| e.map_proposal) {
            result[e.hit_index].voice = voice;
        }
    }
    result
}

/// Classifies only already detected generic cymbal attacks. The detailed model
/// cannot create, remove, retime, rescore or relabel another broad family.
pub fn classify_cymbals(
    frames: &[[f32; 8]],
    hits: &[DrumHit],
    evidence: &mut [Evidence],
    duration: f64,
) -> Result<()> {
    let invalid = frames.iter().enumerate().find_map(|(frame, scores)| {
        scores
            .iter()
            .enumerate()
            .find(|(_, s)| !s.is_finite() || !(-SCORE_EPSILON..=1. + SCORE_EPSILON).contains(*s))
            .map(|(class, value)| (frame, class, *value))
    });
    ensure!(
        duration.is_finite()
            && duration > 0.
            && !frames.is_empty()
            && frames.len() <= 360001
            && frames
                .iter()
                .flatten()
                .all(|s| s.is_finite() && (-SCORE_EPSILON..=1. + SCORE_EPSILON).contains(s)),
        "Invalid detailed drum frame scores: first invalid={invalid:?} frames={} duration={duration}",
        frames.len()
    );
    let final_time = (frames.len() - 1) as f64 / 100.;
    ensure!(
        final_time <= duration + 1e-8 && duration - final_time < 0.01000001,
        "Detailed drum frame/source clock mismatch"
    );
    validate(evidence, hits)?;
    for e in evidence.iter_mut().filter(|e| e.original == Voice::Cymbal) {
        let mut lo = (e.time - 0.035).max(0.);
        let mut hi = (e.time + 0.035).min(duration);
        for h in hits
            .iter()
            .filter(|h| h.voice == Voice::Cymbal && h.time != e.time)
        {
            if h.time < e.time {
                lo = lo.max((e.time + h.time) / 2.);
            } else {
                hi = hi.min((e.time + h.time) / 2.);
            }
        }
        let owned: Vec<_> = frames
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let time = *i as f64 / 100.;
                time >= lo - 1e-9
                    && time <= hi + 1e-9
                    && !hits.iter().enumerate().any(|(index, h)| {
                        h.voice == Voice::Cymbal
                            && index != e.hit_index
                            && ((h.time - time).abs() < (e.time - time).abs() - 1e-9
                                || ((h.time - time).abs() - (e.time - time).abs()).abs() <= 1e-9
                                    && index < e.hit_index)
                    })
            })
            .collect();
        if owned.is_empty() {
            e.reason = "detailed_model_no_owned_frame".into();
            continue;
        }
        let mut raw_peaks = [f32::NEG_INFINITY; 8];
        let mut clipped_values = 0usize;
        for (_, frame) in &owned {
            for (p, s) in raw_peaks.iter_mut().zip(frame.iter()) {
                *p = p.max(*s);
                clipped_values += usize::from(!(0.0..=1.).contains(s));
            }
        }
        let peaks = raw_peaks.map(|s| s.clamp(0., 1.));
        // Training pitch53 combines ride bell and cowbell. It is a competing
        // ambiguous class, never evidence for Ride by itself.
        let ride = peaks[4];
        let crash = peaks[6];
        let (column, voice, score, other) = if ride >= crash {
            (4, Voice::Ride, ride, crash)
        } else {
            (6, Voice::Crash, crash, ride)
        };
        let noncymbal = [0usize, 1, 2, 3, 5, 7]
            .into_iter()
            .map(|i| peaks[i])
            .fold(0., f32::max);
        let peak = owned
            .iter()
            .max_by(|a, b| {
                a.1[column].total_cmp(&b.1[column]).then_with(|| {
                    let da = (a.0 as f64 / 100. - e.time).abs();
                    let db = (b.0 as f64 / 100. - e.time).abs();
                    db.total_cmp(&da)
                })
            })
            .unwrap()
            .0 as f64
            / 100.;
        e.cymbal_model = Some(CymbalModelEvidence {
            model_sha256: CYMBAL_MODEL_SHA256.into(),
            class_peaks: peaks,
            raw_class_peaks: raw_peaks,
            clipped_values,
            window_start: lo,
            window_end: hi,
            peak_time: peak,
        });
        e.proposed = None;
        e.map_proposal = false;
        e.confidence = 0.;
        e.reason = "detailed_model_ambiguous".into();
        if peaks[5] >= score {
            e.reason = "ambiguous_ride_bell_or_cowbell".into();
        }
        if score >= 0.6 && score - other >= 0.15 && score - noncymbal >= 0.10 {
            e.proposed = Some(voice);
            e.map_proposal = true;
            e.confidence = (score - other.max(noncymbal)) as f64;
            e.reason = "detailed_model_owned_cymbal_vote".into();
        }
    }
    validate(evidence, hits)
}

/// Native pinned ONNX;20-second cores with3 seconds of source context. Only
/// central frames are retained, on the original441-sample/100Hz source clock.
pub fn infer_cymbals(
    audio: &std::path::Path,
    model: &std::path::Path,
    hits: &[DrumHit],
    evidence: &mut [Evidence],
) -> Result<()> {
    let (frames, duration) = infer_cymbal_frames(audio, model)?;
    classify_cymbals(&frames, hits, evidence, duration)
}

/// Raw pinned-checkpoint outputs for independent source-bound diagnostics.
/// Consumers must validate probabilities before treating them as evidence.
pub fn infer_cymbal_frames(
    audio: &std::path::Path,
    model: &std::path::Path,
) -> Result<(Vec<[f32; 8]>, f64)> {
    use ort::{session::Session, value::Tensor};
    ensure!(
        crate::models::hash_file(model)? == CYMBAL_MODEL_SHA256,
        "Detailed drum model checksum mismatch"
    );
    let samples = crate::models::decode(audio, 44100, 1)?;
    ensure!(
        !samples.is_empty() && samples.len() <= 44100 * 3600,
        "Invalid detailed drum source length"
    );
    let input_peak = samples.iter().map(|s| s.abs()).fold(0., f32::max);
    let out_of_range = samples.iter().filter(|s| s.abs() > 1.).count();
    eprintln!(
        "Detailed cymbal input: peak={input_peak:.9}, out_of_nominal_range_samples={out_of_range}, normalization=none"
    );
    let count = samples.len() / 441 + 1;
    let mut frames = vec![[0.; 8]; count];
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model)?;
    for core in (0..samples.len()).step_by(20 * 44100) {
        let start = core.saturating_sub(3 * 44100);
        let end = (core + 23 * 44100).min(samples.len());
        let mut chunk = samples[start..end].to_vec();
        chunk.resize(chunk.len().max(44100), 0.);
        let expected_frames = chunk.len() / 441 + 1;
        let tensor = Tensor::from_array(([1, chunk.len()], chunk.into_boxed_slice()))?;
        let outputs = session.run(ort::inputs!["waveform"=>tensor])?;
        let (shape, scores) = outputs["onset_probs"].try_extract_tensor::<f32>()?;
        ensure!(
            shape.len() == 3
                && shape[0] == 1
                && shape[1] == expected_frames as i64
                && shape[2] == 8,
            "Unexpected detailed drum output shape"
        );
        let local = (core - start) / 441;
        let global = core / 441;
        let take = if core + 20 * 44100 >= samples.len() {
            count - global
        } else {
            2000.min(count - global)
        };
        ensure!(
            (local + take) * 8 <= scores.len(),
            "Detailed drum output shorter than source clock"
        );
        for (frame, values) in frames[global..global + take]
            .iter_mut()
            .zip(scores[local * 8..(local + take) * 8].chunks_exact(8))
        {
            frame.copy_from_slice(values);
        }
        eprintln!(
            "Detailed cymbals: {:.1}/{:.1} seconds",
            (core + 20 * 44100).min(samples.len()) as f64 / 44100.,
            samples.len() as f64 / 44100.
        );
    }
    ensure!(
        crate::models::hash_file(model)? == CYMBAL_MODEL_SHA256,
        "Detailed drum model changed during inference"
    );
    Ok((frames, samples.len() as f64 / 44100.))
}
