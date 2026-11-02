//! Source-bound, batch-applicable pitch corrections. Lyrics, timing and MIDI
//! performance attributes are preserved. A report is invalidated by source,
//! tempo or vocal edits while analysis is running or pending review.
use crate::{
    editor::{Project, TimeRange},
    timing::TempoMap,
    vocal_generation::{COMPARISON_ALGORITHM, Generation},
    vocals::{VocalChart, VocalTone},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const ANALYSIS_ALGORITHM: &str = "rmvpe-note-comparison-v1-weighted3.2-80percent-raw50percent";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Correction {
    pub index: usize,
    pub before: u8,
    pub after: u8,
    pub support_frames: usize,
    pub support_fraction: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub version: u32,
    pub algorithm: String,
    pub audio_sha256: String,
    pub stem_sha256: String,
    pub chart_sha256: String,
    pub generation_sha256: String,
    pub model_sha256: String,
    pub timing: TempoMap,
    pub range: TimeRange,
    pub inspected: usize,
    pub unresolved: usize,
    pub corrections: Vec<Correction>,
    pub applied: bool,
}
pub fn chart_hash(chart: &VocalChart) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(chart)?)))
}
pub fn propose(
    project: &Project,
    timing: &TempoMap,
    generation: &Generation,
    range: Option<TimeRange>,
) -> Result<Analysis> {
    let chart = project
        .content
        .vocals
        .as_ref()
        .context("No vocal notes to compare")?;
    let stem = project
        .content
        .stems
        .as_ref()
        .and_then(|m| {
            m.stems
                .iter()
                .find(|s| s.id == crate::stem_review::StemId::Vocals)
        })
        .context("Attach a vocal stem before comparing pitches")?;
    ensure!(
        generation.algorithm == COMPARISON_ALGORITHM
            && generation.audio_sha256 == project.audio_sha256
            && generation.vocal_stem_sha256 == stem.sha256
            && generation.timing.as_ref() == Some(timing)
            && (generation.duration_seconds - project.duration).abs() <= 1. / 44100.,
        "Pitch comparison belongs to different source, timing or algorithm"
    );
    crate::vocal_rmvpe::validate(
        generation
            .rmvpe_pitch
            .as_ref()
            .context("Missing independent pitch comparison")?,
        &project.audio_sha256,
        &stem.sha256,
        project.duration,
    )?;
    let range = range.unwrap_or(TimeRange {
        start: 0.,
        end: project.duration,
    });
    ensure!(
        range.start.is_finite()
            && range.end.is_finite()
            && 0. <= range.start
            && range.start < range.end
            && range.end <= project.duration,
        "Invalid vocal comparison range"
    );
    let mut corrections = Vec::new();
    let (mut inspected, mut unresolved) = (0, 0);
    // Existing notes establish the intervals. Compare their pitches against a
    // dense independent contour, including quiet intervals the DSP mask missed.
    // This comparator cannot create, delete or move notes.
    let evidence = generation.rmvpe_pitch.as_ref().unwrap();
    let mut frames: Vec<_> = evidence
        .frames
        .iter()
        .map(|f| {
            let midi = f
                .frequency_hz
                .filter(|_| f.confidence > 0.03)
                .map(|hz| 69. + 12. * (hz / 440.).log2())
                .filter(|p| (35.5..84.5).contains(p));
            crate::vocal_generation::PitchFrame {
                time_seconds: f.time_seconds,
                midi,
                yin_midi: None,
                mcleod_midi: None,
                clarity: f.confidence as f64,
                rms: f.rms,
                local_rms: Some(f.rms),
                local_energy_active: None,
                support: crate::vocal_generation::PitchSupport::ContourPath,
            }
        })
        .collect();
    let weights: Vec<_> = evidence
        .frames
        .iter()
        .map(|f| (f.confidence as f64).max(0.1))
        .collect();
    crate::vocal_pitch_tracking::weighted_note_path(&mut frames, 3.2, &weights, &|| false)?;
    for (index, note) in chart.notes.iter().enumerate() {
        let VocalTone::Pitched { midi: before } = note.tone else {
            continue;
        };
        let start = timing.seconds(note.start);
        let end = timing.seconds(note.end);
        // Partial notes at selection edges are preserved, never clipped.
        if start < range.start || end > range.end {
            continue;
        }
        inspected += 1;
        let trim = ((end - start) * 0.1).min(0.030);
        let a = frames.partition_point(|f| f.time_seconds < start + trim);
        let b = frames.partition_point(|f| f.time_seconds < end - trim);
        let mut votes = [0usize; 49];
        let mut total = 0;
        for f in &frames[a..b] {
            if let Some(p) = f.midi.filter(|p| (35.5..84.5).contains(p)) {
                votes[(p.round() as usize).clamp(36, 84) - 36] += 1;
                total += 1;
            }
        }
        let winner = (0..49).max_by_key(|&i| votes[i]).unwrap();
        let support = votes[winner];
        let after = winner as u8 + 36;
        let fraction = if total > 0 {
            support as f64 / total as f64
        } else {
            0.
        };
        let covered = total as f64 * 0.01;
        let physical_support = evidence.frames[a..b]
            .iter()
            .filter(|f| {
                f.confidence >= 0.1
                    && f.frequency_hz.is_some_and(|hz| {
                        (69. + 12. * (hz / 440.).log2() - after as f64).abs() <= 0.75
                    })
            })
            .count();
        if total < 4
            || covered < (end - start - 2. * trim) * 0.4
            || fraction < 0.8
            || physical_support < 4
            || physical_support * 2 < b - a
        {
            unresolved += 1;
            continue;
        }
        if before != after {
            corrections.push(Correction {
                index,
                before,
                after,
                support_frames: support,
                support_fraction: fraction,
            });
        }
    }
    Ok(Analysis {
        version: 1,
        algorithm: ANALYSIS_ALGORITHM.into(),
        audio_sha256: project.audio_sha256.clone(),
        stem_sha256: stem.sha256.clone(),
        chart_sha256: chart_hash(chart)?,
        generation_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(generation)?)),
        model_sha256: crate::vocal_rmvpe::SHA256.into(),
        timing: timing.clone(),
        range,
        inspected,
        unresolved,
        corrections,
        applied: false,
    })
}
impl Analysis {
    pub fn validate(&self, project: &Project, timing: &TempoMap) -> Result<()> {
        ensure!(
            self.version == 1
                && self.algorithm == ANALYSIS_ALGORITHM
                && self.generation_sha256.len() == 64
                && self
                    .generation_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
                && !self.applied
                && self.audio_sha256 == project.audio_sha256
                && self.timing == *timing
                && self.model_sha256 == crate::vocal_rmvpe::SHA256,
            "Vocal analysis is stale or already applied; compare again"
        );
        let chart = project
            .content
            .vocals
            .as_ref()
            .context("No current vocal chart")?;
        ensure!(
            self.chart_sha256 == chart_hash(chart)?,
            "Vocal notes or lyrics changed after comparison; compare again"
        );
        let stem = project
            .content
            .stems
            .as_ref()
            .and_then(|m| {
                m.stems
                    .iter()
                    .find(|s| s.id == crate::stem_review::StemId::Vocals)
            })
            .context("The compared vocal stem is unavailable")?;
        ensure!(
            self.stem_sha256 == stem.sha256
                && self.range.start.is_finite()
                && self.range.end.is_finite()
                && self.range.start >= 0.
                && self.range.start < self.range.end
                && self.range.end <= project.duration
                && self.inspected <= chart.notes.len()
                && self.unresolved <= self.inspected
                && self.corrections.len() <= self.inspected,
            "Invalid vocal comparison scope"
        );
        let mut indices = std::collections::BTreeSet::new();
        for c in &self.corrections {
            ensure!(
                indices.insert(c.index)
                    && (36..=84).contains(&c.after)
                    && c.before != c.after
                    && c.support_frames >= 4
                    && c.support_fraction.is_finite()
                    && (0.8..=1.).contains(&c.support_fraction),
                "Invalid pitch correction evidence"
            );
            let n = chart
                .notes
                .get(c.index)
                .context("Compared note no longer exists")?;
            ensure!(
                n.tone == VocalTone::Pitched { midi: c.before }
                    && timing.seconds(n.start) >= self.range.start
                    && timing.seconds(n.end) <= self.range.end,
                "Compared note moved or changed"
            );
        }
        Ok(())
    }
}
pub fn note_name(midi: u8) -> String {
    format!(
        "{}{}",
        [
            "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"
        ][midi as usize % 12],
        midi as i16 / 12 - 1
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn analyze(
    project: &Project,
    audio: &std::path::Path,
    timing: &TempoMap,
    range: Option<TimeRange>,
    model_directory: &std::path::Path,
    control: std::sync::Arc<crate::clean_recognition::Control>,
) -> Result<Analysis> {
    let manifest = project
        .content
        .stems
        .as_ref()
        .context("Attach stems before comparing pitches")?;
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == crate::stem_review::StemId::Vocals)
        .context("No vocal stem")?;
    let model = crate::vocal_rmvpe::prepare(model_directory, true, &|| control.is_cancelled())?;
    let binding = crate::clean::StemAlignment {
        audio_sha256: project.audio_sha256.clone(),
        stem_sha256: stem.sha256.clone(),
        evidence: manifest.alignment.details.clone(),
    };
    control.update("Comparing vocal pitch with the recording…");
    let e = crate::vocal_rmvpe::analyze(audio, &stem.path, &binding, &model, &|| {
        control.is_cancelled()
    })?;
    control.update("Finding stable notes and supported pitch corrections…");
    let g = crate::vocal_generation::generate_cancellable(
        crate::vocal_generation::Request {
            audio,
            vocal_stem: &stem.path,
            alignment: &binding,
            timing,
            words: None,
            lyrics: None,
            policy: &crate::vocal_generation::Policy {
                analysis_mode: crate::vocal_generation::AnalysisMode::HighAccuracy,
                ..Default::default()
            },
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: Some(&e),
        },
        &|| control.is_cancelled(),
    )?;
    propose(project, timing, &g, range)
}
