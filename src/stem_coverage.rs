//! Compare a stem's activity with the chart.
//! Flag possible gaps for listening; leave authored notes unchanged.
use crate::{
    chart::{Chart, Note},
    mapping::{PitchNote, Transcription},
    models, refine, review,
    stem_review::{StemId, StemManifest},
};
use anyhow::{Result, ensure};
use pitch_detection::detector::{PitchDetector, mcleod::McLeodDetector, yin::YINDetector};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub attack_tolerance_seconds: f64,
    pub long_gap_seconds: f64,
    pub minimum_rms: f64,
    pub minimum_relative_rms: f64,
    pub minimum_flux_fraction: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            attack_tolerance_seconds: 0.080,
            long_gap_seconds: 0.7,
            minimum_rms: 0.006,
            minimum_relative_rms: 0.18,
            minimum_flux_fraction: 0.12,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.attack_tolerance_seconds.is_finite()
                && (0.02..=0.2).contains(&self.attack_tolerance_seconds)
                && self.long_gap_seconds.is_finite()
                && (0.3..=10.).contains(&self.long_gap_seconds)
                && self.minimum_rms.is_finite()
                && self.minimum_rms > 0.
                && self.minimum_relative_rms.is_finite()
                && (0.0..=1.).contains(&self.minimum_relative_rms)
                && self.minimum_flux_fraction.is_finite()
                && (0.0..=1.).contains(&self.minimum_flux_fraction),
            "Invalid stem coverage policy"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchSupport {
    pub midi: f64,
    pub periodicity: f64,
    pub fundamental_fraction: f64,
    pub competitor_fundamental_ratio: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub seconds: f64,
    pub tick: u64,
    pub gap_start: f64,
    pub gap_end: f64,
    pub nearest_expert_attack_seconds: Option<f64>,
    pub flux: f32,
    pub rms: f64,
    pub envelope_ratio: f64,
    pub pitch: Option<PitchSupport>,
    pub transcription_event_present: bool,
    pub interpretation: String,
    /// A proposal in recording coordinates; no five-fret lane is invented.
    /// Reconcile with the entire song's mapping before deriving chart notes.
    pub proposed_bass_event: Option<PitchNote>,
    pub review_required: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TierCoverage {
    pub track: String,
    pub attack_positions: usize,
    pub expert_positions_not_in_tier: usize,
    pub tier_positions_not_in_expert: usize,
    pub interpretation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PitchWindowOwnership {
    pub start: f64,
    pub end: f64,
    pub candidate_end: f64,
    pub competing_candidate_start: Option<f64>,
    pub owned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlignmentObservation {
    pub tick: u64,
    pub seconds: f64,
    pub nearest_flux_seconds: Option<f64>,
    /// Positive means the audio flux peak follows the chart attack.
    pub audio_minus_chart_seconds: Option<f64>,
    pub supported_attack: bool,
    pub raw_transcription_seconds: Option<f64>,
    pub raw_transcription_pitch: Option<u8>,
    pub raw_minus_chart_seconds: Option<f64>,
    pub periodic_pitch: Option<PitchSupport>,
    pub periodic_minus_raw_semitones: Option<f64>,
    /// Stable later audio does not prove the pitch of an earlier short event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_window: Option<PitchWindowOwnership>,
    pub proposed_attack_tick: Option<u64>,
    pub review_required: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimingSummary {
    pub supported_attacks: usize,
    pub median_audio_minus_chart_seconds: Option<f64>,
    pub p90_absolute_audio_minus_chart_seconds: Option<f64>,
    pub supported_residuals_over_50ms: usize,
    pub raw_mapping_comparisons: usize,
    pub median_raw_minus_chart_seconds: Option<f64>,
    pub stable_pitch_comparisons: usize,
    pub pitch_disagreements_over_one_semitone: usize,
    #[serde(default)]
    pub owned_pitch_comparisons: usize,
    #[serde(default)]
    pub owned_pitch_disagreements: usize,
    #[serde(default)]
    pub inconclusive_pitch_windows: usize,
    pub interpretation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackCoverage {
    pub stem: StemId,
    pub expert_track: String,
    pub expert_attack_positions: usize,
    pub transcription_scope: String,
    pub flux_peaks: usize,
    pub strong_peaks_in_long_gaps: usize,
    pub rejected_bass_ambiguity: usize,
    pub candidates: Vec<Candidate>,
    pub tiers: Vec<TierCoverage>,
    pub alignment: Vec<AlignmentObservation>,
    pub timing: TimingSummary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverageReport {
    pub version: u32,
    pub source_audio_sha256: String,
    pub chart_sha256: String,
    pub transcription_sha256: Option<String>,
    pub manifest: StemManifest,
    pub policy: Policy,
    pub tracks: Vec<TrackCoverage>,
    pub limits: Vec<String>,
}

fn rms(audio: &[f32], rate: f64, start: f64, end: f64) -> f64 {
    let lo = ((start.max(0.) * rate) as usize).min(audio.len());
    let hi = ((end.max(0.) * rate) as usize).min(audio.len());
    if hi <= lo {
        return 0.;
    }
    (audio[lo..hi]
        .iter()
        .map(|v| (*v as f64).powi(2))
        .sum::<f64>()
        / (hi - lo) as f64)
        .sqrt()
}
fn power(signal: &[f32], frequency: f64, center: f64) -> f64 {
    let start = (center * 11025.) as isize - 1024;
    if start < 0 || start as usize + 2048 > signal.len() {
        return 0.;
    }
    let (mut re, mut im) = (0., 0.);
    for (i, sample) in signal[start as usize..start as usize + 2048]
        .iter()
        .enumerate()
    {
        let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / 2047.).cos();
        let phase = std::f64::consts::TAU * frequency * i as f64 / 11025.;
        re += *sample as f64 * window * phase.cos();
        im += *sample as f64 * window * phase.sin();
    }
    re * re + im * im
}
fn bass_support(audio: &[f32], competitors: &[&[f32]], time: f64) -> Option<PitchSupport> {
    let mut yin = YINDetector::<f64>::new(2048, 1024);
    let mut mcleod = McLeodDetector::<f64>::new(2048, 1024);
    let mut estimates = Vec::new();
    for center in [time + 0.12, time + 0.22] {
        let sample = (center * 11025.) as usize;
        if sample < 1024 || sample + 1024 > audio.len() {
            return None;
        }
        let signal: Vec<_> = audio[sample - 1024..sample + 1024]
            .iter()
            .map(|value| *value as f64)
            .collect();
        let y = yin.get_pitch(&signal, 11025, 0.01, 0.92)?;
        let m = mcleod.get_pitch(&signal, 11025, 0.01, 0.90)?;
        if !(32.0..250.0).contains(&y.frequency) || !m.clarity.is_finite() || m.clarity < 0.90 {
            return None;
        }
        let periodicity = refine::periodicity(&signal, y.frequency, 11025.);
        let midi = 69. + 12. * (y.frequency / 440.).log2();
        let difference = 12. * (m.frequency / y.frequency).log2();
        if !periodicity.is_finite()
            || periodicity < 0.94
            || ![0., 12., 19.01955, 24.]
                .iter()
                .any(|h| (difference - h).abs() < 0.30)
            || (midi - midi.round()).abs() > 0.30
        {
            return None;
        }
        let fundamental = power(audio, y.frequency, center);
        let strongest = (2..=5)
            .map(|harmonic| power(audio, y.frequency * harmonic as f64, center))
            .fold(fundamental, f64::max);
        let fraction = fundamental / strongest.max(f64::EPSILON);
        let competitor = competitors
            .iter()
            .map(|signal| power(signal, y.frequency, center))
            .fold(0., f64::max)
            / fundamental.max(f64::EPSILON);
        if fraction < 0.12 || competitor > 0.67 {
            return None;
        }
        estimates.push((midi, periodicity, fraction, competitor));
    }
    if (estimates[0].0 - estimates[1].0).abs() > 0.3 {
        return None;
    }
    Some(PitchSupport {
        midi: (estimates[0].0 + estimates[1].0) * 0.5,
        periodicity: estimates[0].1.min(estimates[1].1),
        fundamental_fraction: estimates[0].2.min(estimates[1].2),
        competitor_fundamental_ratio: estimates[0].3.max(estimates[1].3),
    })
}
fn positions(chart: &Chart, track: &str) -> Result<Vec<f64>> {
    let drums = track.ends_with("Drums");
    let mut ticks: Vec<_> = chart
        .notes(track)?
        .into_iter()
        .filter(|note| note.lane <= 4 || (!drums && note.lane == 7))
        .map(|note| note.tick)
        .collect();
    ticks.sort_unstable();
    ticks.dedup();
    Ok(ticks
        .into_iter()
        .map(|tick| chart.timing.seconds(tick))
        .collect())
}
fn distance(time: f64, attacks: &[f64]) -> Option<f64> {
    let index = attacks.partition_point(|attack| *attack < time);
    [index.checked_sub(1), Some(index)]
        .into_iter()
        .flatten()
        .filter_map(|index| attacks.get(index))
        .map(|attack| (time - attack).abs())
        .min_by(f64::total_cmp)
}

/// Validate an entire Expert bass attack move without changing any notes.
/// Returns the complete same-tick group with playable sustain ENDs preserved.
/// This checks geometry only, not the identity/timing of the audible attack.
pub fn safe_bass_attack_move(chart: &Chart, before: u64, after: u64) -> Result<Vec<Note>> {
    ensure!(before != after, "Attack move has no timing change");
    let notes = chart.notes("ExpertDoubleBass")?;
    let playable = |lane: u8| lane <= 4 || lane == 7;
    let group: Vec<_> = notes.iter().filter(|note| note.tick == before).collect();
    ensure!(
        group.iter().any(|note| playable(note.lane)),
        "No Expert bass attack at source tick"
    );
    ensure!(
        group
            .iter()
            .all(|note| playable(note.lane) || (5..=6).contains(&note.lane)),
        "Unsupported same-tick control marker; preserve manually"
    );
    let moved_time = chart.timing.seconds(after);
    ensure!(
        moved_time.is_finite() && moved_time >= 0.,
        "Moved attack precedes recording"
    );
    for other in notes
        .iter()
        .filter(|note| note.tick != before && playable(note.lane))
    {
        ensure!(
            (chart.timing.seconds(other.tick) - moved_time).abs() >= 0.055,
            "Moved attack collides with another bass attack within55ms"
        );
        for target in group.iter().filter(|note| playable(note.lane)) {
            if other.lane == target.lane && other.tick < after {
                ensure!(
                    other
                        .tick
                        .checked_add(other.sustain)
                        .is_some_and(|end| end <= after),
                    "Moved attack overlaps preceding same-lane sustain; prior hold must not be auto-trimmed"
                );
            }
            if other.lane == target.lane && other.tick > after && target.sustain > 0 {
                ensure!(
                    target
                        .tick
                        .checked_add(target.sustain)
                        .is_some_and(|end| end <= other.tick),
                    "Preserved sustain end overlaps the next same-lane attack"
                );
            }
        }
    }
    group
        .into_iter()
        .map(|note| {
            let mut moved = note.clone();
            moved.tick = after;
            if playable(note.lane) && note.sustain > 0 {
                let end = note
                    .tick
                    .checked_add(note.sustain)
                    .ok_or_else(|| anyhow::anyhow!("Bass sustain overflows"))?;
                ensure!(
                    after < end,
                    "Moved attack would pass its original sustain end"
                );
                moved.sustain = end - after;
            }
            Ok(moved)
        })
        .collect()
}

/// Signals have fixed rates: target spectral signal 22050 mono, bass-pitch and
/// competitor signals 11025 mono. Callers must provide identical source clocks.
/// Guitar is diagnosed without patch proposals to protect authored Expert.
pub fn analyze_stem(
    chart: &Chart,
    stem: StemId,
    spectral_audio: &[f32],
    pitch_audio: &[f32],
    competitors: &[&[f32]],
    events: Option<&Transcription>,
    policy: &Policy,
) -> Result<TrackCoverage> {
    policy.validate()?;
    ensure!(
        stem != StemId::Guitar
            || events
                .is_none_or(|events| events.guitar_source == crate::mapping::GuitarSource::Guitar),
        "Guitar-stem coverage cannot assess a chart arranged from another melody source"
    );
    ensure!(
        spectral_audio
            .iter()
            .chain(pitch_audio)
            .chain(competitors.iter().flat_map(|a| a.iter()))
            .all(|v| v.is_finite()),
        "Coverage audio contains nonfinite values"
    );
    ensure!(
        (spectral_audio.len() as f64 / 22050. - pitch_audio.len() as f64 / 11025.).abs() < 0.002,
        "Coverage signals have inconsistent source durations"
    );
    ensure!(
        competitors
            .iter()
            .all(|audio| audio.len() == pitch_audio.len()),
        "Competing stems must share source sample coordinates"
    );
    let suffix = match stem {
        StemId::Bass => "DoubleBass",
        StemId::Drums => "Drums",
        StemId::Guitar => "Single",
        _ => anyhow::bail!("Coverage currently supports bass, drums and guitar"),
    };
    let expert_track = format!("Expert{suffix}");
    let expert = positions(chart, &expert_track)?;
    let transcription_available = events.is_some_and(|events| match stem {
        StemId::Bass => !events.bass.is_empty(),
        StemId::Drums => !events.drums.is_empty(),
        StemId::Guitar => !events.guitar.is_empty(),
        _ => false,
    });
    let duration = spectral_audio.len() as f64 / 22050.;
    let peaks = review::attacks(spectral_audio);
    let strongest = peaks.iter().map(|peak| peak.strength).fold(0., f32::max);
    let global_rms = rms(spectral_audio, 22050., 0., duration);
    let mut out = TrackCoverage {
        stem,
        expert_track,
        expert_attack_positions: expert.len(),
        transcription_scope: if transcription_available { "Nonempty instrument event list supplied; inspect its provenance and filtering." } else { "Instrument transcription scope unproven: absent or empty events may mean this part was never transcribed." }.into(),
        flux_peaks: peaks.len(),
        strong_peaks_in_long_gaps: 0,
        rejected_bass_ambiguity: 0,
        candidates: Vec::new(),
        tiers: Vec::new(),
        alignment: Vec::new(),
        timing: TimingSummary::default(),
    };
    for tier in ["Easy", "Medium", "Hard"] {
        let track = format!("{tier}{suffix}");
        let attacks = positions(chart, &track)?;
        out.tiers.push(TierCoverage { track, attack_positions: attacks.len(), expert_positions_not_in_tier: expert.iter().filter(|t| distance(**t, &attacks).is_none_or(|d| d > 0.001)).count(), tier_positions_not_in_expert: attacks.iter().filter(|t| distance(**t, &expert).is_none_or(|d| d > 0.001)).count(), interpretation: "Expert-relative arrangement differences; lower tiers intentionally omit notes. No independent audio additions are proposed for reduced tiers.".into() });
    }
    if stem == StemId::Bass {
        let notes = chart.notes(&out.expert_track)?;
        let mapped_sources = events.map(|events| {
            crate::mapping::bass_monophonic_chart_sources(&events.bass, &chart.timing, &notes)
        });
        let mut ticks: Vec<_> = notes
            .iter()
            .filter(|note| note.lane <= 4 || note.lane == 7)
            .map(|note| note.tick)
            .collect();
        ticks.sort_unstable();
        ticks.dedup();
        for (index, time) in expert.iter().copied().enumerate() {
            let nearest =
                review::nearest(time, &peaks).filter(|peak| (peak.time - time).abs() <= 0.18);
            let supported_attack = nearest.is_some_and(|peak| {
                let level = rms(spectral_audio, 22050., peak.time, peak.time + 0.08);
                let before = rms(spectral_audio, 22050., peak.time - 0.08, peak.time - 0.02);
                peak.strength >= strongest * policy.minimum_flux_fraction as f32
                    && level
                        >= policy
                            .minimum_rms
                            .max(global_rms * policy.minimum_relative_rms)
                    && level / before.max(0.0001) >= 1.1
            });
            let raw = mapped_sources
                .as_ref()
                .and_then(|sources| sources.get(&ticks[index]));
            let next = expert.get(index + 1).copied().unwrap_or(duration);
            // Leave fast notes unjudged rather than sampling the following note.
            let pitch_start = nearest
                .filter(|_| supported_attack)
                .map_or(time, |peak| peak.time);
            let periodic_pitch = if next - pitch_start >= 0.33 {
                bass_support(pitch_audio, competitors, pitch_start)
            } else {
                None
            };
            let pitch_window = raw.filter(|_| periodic_pitch.is_some()).map(|raw| {
                let start = pitch_start + 0.12 - 1024. / 11025.;
                let end = pitch_start + 0.22 + 1024. / 11025.;
                let competing = events.and_then(|events| {
                    events
                        .bass
                        .iter()
                        .filter(|other| other.start > raw.start + 0.025 && other.pitch != raw.pitch)
                        .map(|other| other.start)
                        .min_by(f64::total_cmp)
                });
                PitchWindowOwnership {
                    start,
                    end,
                    candidate_end: raw.end,
                    competing_candidate_start: competing,
                    owned: start >= raw.start
                        && end <= raw.end
                        && competing.is_none_or(|other| other >= end),
                }
            });
            let error = periodic_pitch
                .as_ref()
                .zip(raw)
                .map(|(pitch, raw)| pitch.midi - raw.pitch as f64);
            let residual = nearest.map(|peak| peak.time - time);
            let proposed_attack_tick = if supported_attack
                && periodic_pitch.is_some()
                && pitch_window.as_ref().is_some_and(|window| window.owned)
                && residual.is_some_and(|r| r.abs() > 0.05)
            {
                nearest.and_then(|peak| {
                    let collision = expert
                        .iter()
                        .enumerate()
                        .any(|(other, t)| other != index && (peak.time - t).abs() < 0.055);
                    (!collision)
                        .then(|| chart.timing.tick(peak.time))
                        .flatten()
                        .filter(|after| safe_bass_attack_move(chart, ticks[index], *after).is_ok())
                })
            } else {
                None
            };
            out.alignment.push(AlignmentObservation {
                tick: ticks[index],
                seconds: time,
                nearest_flux_seconds: nearest.map(|peak| peak.time),
                audio_minus_chart_seconds: residual,
                supported_attack,
                raw_transcription_seconds: raw.map(|note| note.start),
                raw_transcription_pitch: raw.map(|note| note.pitch),
                raw_minus_chart_seconds: raw.map(|note| note.start - time),
                periodic_pitch,
                periodic_minus_raw_semitones: error,
                pitch_window,
                proposed_attack_tick,
                review_required: true,
            });
        }
        let mut signed: Vec<_> = out
            .alignment
            .iter()
            .filter(|observation| observation.supported_attack)
            .filter_map(|observation| observation.audio_minus_chart_seconds)
            .collect();
        signed.sort_by(f64::total_cmp);
        let mut absolute: Vec<_> = signed.iter().map(|value| value.abs()).collect();
        absolute.sort_by(f64::total_cmp);
        let mut raw_residuals: Vec<_> = out
            .alignment
            .iter()
            .filter_map(|observation| observation.raw_minus_chart_seconds)
            .collect();
        raw_residuals.sort_by(f64::total_cmp);
        out.timing = TimingSummary {
            supported_attacks: signed.len(),
            median_audio_minus_chart_seconds: signed.get(signed.len()/2).copied(),
            p90_absolute_audio_minus_chart_seconds: absolute.get(absolute.len().saturating_sub(1)*9/10).copied(),
            supported_residuals_over_50ms: signed.iter().filter(|r|r.abs()>0.05).count(),
            raw_mapping_comparisons: raw_residuals.len(),
            median_raw_minus_chart_seconds: raw_residuals.get(raw_residuals.len()/2).copied(),
            stable_pitch_comparisons: out.alignment.iter().filter(|o|o.periodic_minus_raw_semitones.is_some()).count(),
            pitch_disagreements_over_one_semitone: out.alignment.iter().filter(|o|o.periodic_minus_raw_semitones.is_some_and(|error|error.abs()>1.)).count(),
            owned_pitch_comparisons:out.alignment.iter().filter(|o|o.pitch_window.as_ref().is_some_and(|window|window.owned)).count(),
            owned_pitch_disagreements:out.alignment.iter().filter(|o|o.pitch_window.as_ref().is_some_and(|window|window.owned)&&o.periodic_minus_raw_semitones.is_some_and(|error|error.abs()>1.)).count(),
            inconclusive_pitch_windows:out.alignment.iter().filter(|o|o.pitch_window.as_ref().is_some_and(|window|!window.owned)).count(),
            interpretation: "Flux residuals describe onset consistency, not fitted offsets or musical truth. Stable periodic-window counts retain all measured comparisons; only owned_pitch_comparisons can be attributed within the selected model event. Cross-boundary windows remain flagged and inconclusive, not proven wrong notes.".into(),
        };
    }
    for peak in peaks {
        let nearest = distance(peak.time, &expert);
        if nearest.is_some_and(|d| d <= policy.attack_tolerance_seconds) {
            continue;
        }
        if out
            .candidates
            .last()
            .is_some_and(|candidate| peak.time - candidate.seconds < 0.080)
        {
            continue;
        }
        let index = expert.partition_point(|t| *t < peak.time);
        let start = index.checked_sub(1).map_or(0., |i| expert[i].max(0.));
        let end = expert.get(index).copied().unwrap_or(duration).min(duration);
        if end - start < policy.long_gap_seconds {
            continue;
        }
        let level = rms(spectral_audio, 22050., peak.time, peak.time + 0.1);
        let before = rms(spectral_audio, 22050., peak.time - 0.1, peak.time - 0.025);
        let envelope_ratio = level / before.max(0.0001);
        if level
            < policy
                .minimum_rms
                .max(global_rms * policy.minimum_relative_rms)
            || peak.strength < strongest * policy.minimum_flux_fraction as f32
            || envelope_ratio < 1.10
        {
            continue;
        }
        out.strong_peaks_in_long_gaps += 1;
        let pitch = if stem == StemId::Bass {
            bass_support(pitch_audio, competitors, peak.time)
        } else {
            None
        };
        if stem == StemId::Bass && pitch.is_none() {
            out.rejected_bass_ambiguity += 1;
            continue;
        }
        let Some(tick) = chart.timing.tick(peak.time) else {
            continue;
        };
        let event_present = events.is_some_and(|events| match stem {
            StemId::Bass => events
                .bass
                .iter()
                .any(|n| (n.start - peak.time).abs() <= policy.attack_tolerance_seconds),
            StemId::Guitar => events
                .guitar
                .iter()
                .any(|n| (n.start - peak.time).abs() <= policy.attack_tolerance_seconds),
            StemId::Drums => events
                .drums
                .iter()
                .any(|n| (n.time - peak.time).abs() <= policy.attack_tolerance_seconds),
            _ => false,
        });
        let proposed_bass_event = if !event_present {
            pitch.as_ref().map(|p| PitchNote {
                start: peak.time,
                end: (peak.time + 0.30).min(end),
                pitch: p.midi.round() as u8,
                confidence: 0.0,
            })
        } else {
            None
        };
        out.candidates.push(Candidate { seconds: peak.time, tick, gap_start: start, gap_end: end, nearest_expert_attack_seconds: nearest, flux: peak.strength, rms: level, envelope_ratio, pitch, transcription_event_present: event_present, interpretation: if event_present { "An existing transcription attack has no nearby Expert chart attack: inspect filtering/mapping and arrangement choice." } else if transcription_available { "Strong isolated-stem attack absent from supplied transcription and Expert chart: possible transcription gap; stem bleed and detection error remain possible." } else { "Strong isolated-stem attack has no nearby Expert chart attack; transcription availability unknown." }.into(), proposed_bass_event, review_required: true });
    }
    Ok(out)
}

pub fn audit_files(
    source: &Path,
    chart_path: &Path,
    manifest_path: &Path,
    transcription_path: Option<&Path>,
    policy: &Policy,
) -> Result<CoverageReport> {
    policy.validate()?;
    let source_audio_sha256 = models::hash_file(source)?;
    let chart_sha256 = models::hash_file(chart_path)?;
    let chart = Chart::parse(fs::read_to_string(chart_path)?)?;
    let manifest = StemManifest::read(manifest_path)?;
    // Shared attachment validation checks native stereo format, hashes and exact
    // full-source frame counts. Drop large playback buffers before diagnostics.
    drop(manifest.load(source)?);
    let transcription: Option<Transcription> = transcription_path
        .map(|path| -> Result<_> {
            let value: Transcription = serde_json::from_slice(&fs::read(path)?)?;
            value.validate()?;
            if let Some(identity) = &value.isolation {
                identity.validate_binding(&source_audio_sha256, manifest.frames as f64 / 44100.)?;
            }
            Ok(value)
        })
        .transpose()?;
    let transcription_sha256 = transcription_path.map(models::hash_file).transpose()?;
    let mut pitch_signals = BTreeMap::new();
    for asset in &manifest.stems {
        if [StemId::Bass, StemId::Vocals, StemId::Guitar].contains(&asset.id) {
            pitch_signals.insert(asset.id, models::decode(&asset.path, 11025, 1)?);
        }
    }
    let mut tracks = Vec::new();
    for stem in [StemId::Bass, StemId::Drums, StemId::Guitar] {
        if stem == StemId::Guitar
            && transcription
                .as_ref()
                .is_some_and(|events| events.guitar_source != crate::mapping::GuitarSource::Guitar)
        {
            continue;
        }
        let Some(asset) = manifest.stems.iter().find(|asset| asset.id == stem) else {
            continue;
        };
        let spectral = models::decode(&asset.path, 22050, 1)?;
        let fallback;
        let pitch = if let Some(signal) = pitch_signals.get(&stem) {
            signal
        } else {
            fallback = models::decode(&asset.path, 11025, 1)?;
            &fallback
        };
        let competitors: Vec<_> = [StemId::Vocals, StemId::Guitar]
            .into_iter()
            .filter(|id| *id != stem)
            .filter_map(|id| pitch_signals.get(&id).map(Vec::as_slice))
            .collect();
        tracks.push(analyze_stem(
            &chart,
            stem,
            &spectral,
            pitch,
            &competitors,
            transcription.as_ref(),
            policy,
        )?);
    }
    ensure!(
        models::hash_file(source)? == source_audio_sha256
            && models::hash_file(chart_path)? == chart_sha256,
        "Coverage input changed during analysis"
    );
    for asset in &manifest.stems {
        ensure!(
            models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
            "Coverage stem changed during analysis"
        );
    }
    if let Some(path) = transcription_path {
        ensure!(
            Some(models::hash_file(path)?) == transcription_sha256,
            "Transcription changed during analysis"
        );
    }
    Ok(CoverageReport { version: 1, source_audio_sha256, chart_sha256, transcription_sha256, manifest, policy: policy.clone(), tracks, limits: vec![
        "Model-free signal evidence is uncalibrated. Separation bleed, resonances, pitch harmonics, soft/short attacks and real repeated notes can fool or evade these detectors. Matching sample counts do not prove phase alignment.".into(),
        "Flux peaks are centered 1024-sample-window features at 22050Hz, not exact physical attack timestamps. Leading-edge energy can appear before the source onset; proposals retain this uncertainty rather than applying an invented fixed latency correction.".into(),
        "Only strong unmatched attacks inside long Expert attack gaps are considered; absence of candidates does not establish completeness. Existing sustained gems may intentionally cover sound inside a reported gap.".into(),
        "Bass pitch proposals require stable periodicity and measured fundamental energy; louder matching guitar/vocal fundamentals veto candidates, but cannot establish instrument identity. Pitch must be reconciled with the whole-song mapping.".into(),
        "Raw pitch comparisons require a monophonic source candidate selected by the effective Bass mapper and matching the chart lane. A nearer discarded harmonic is not played-note evidence. Polyphonic or unmatched/authored layouts remain unjudged.".into(),
        "Proposal confidence is deliberately zero (unapproved), and its 300ms duration is a bounded listening window, not a measured note end. Do not feed proposals through generation as accepted notes or apply directly to authored tracks.".into(),
        "No lower-tier audio additions or guitar/drum lane guesses are produced. Lower-tier differences are inspected against Expert links; no charts or projects are modified.".into(),
        "Guitar-stem coverage is omitted when saved Guitar events explicitly follow the Other or Piano source; those arrangements need review against their selected source instead.".into(),
    ] })
}
