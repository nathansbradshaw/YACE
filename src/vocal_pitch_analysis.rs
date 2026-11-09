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
    #[serde(default)]
    pub start: u64,
    #[serde(default)]
    pub end: u64,
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
    #[serde(default)]
    pub model_implementation_sha256: String,
    pub timing: TempoMap,
    pub range: TimeRange,
    pub inspected: usize,
    pub unresolved: usize,
    pub corrections: Vec<Correction>,
    pub applied: bool,
    /// Applied note witnesses carried across later checks with identical inputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_evidence: Vec<crate::vocal_generation::NoteEvidence>,
    #[serde(default)]
    pub section: crate::vocal_section_analysis::Section,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive: Option<Box<crate::vocal_adaptive::Report>>,
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
    let section =
        crate::vocal_section_analysis::compare(chart, timing, generation, &frames, range)?;
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
                start: note.start,
                end: note.end,
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
        model_implementation_sha256: crate::vocal_rmvpe::implementation_hash(),
        timing: timing.clone(),
        range,
        inspected,
        unresolved,
        corrections,
        applied: false,
        retained_evidence: vec![],
        section,
        adaptive: None,
    })
}
impl Analysis {
    fn current_profile(&self) -> bool {
        static ADAPTIVE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        (self.algorithm == ANALYSIS_ALGORITHM && self.adaptive.is_none())
            || (self.algorithm == crate::vocal_adaptive::ALGORITHM
                && self.adaptive.as_ref().is_some_and(|r| {
                    r.algorithm == crate::vocal_adaptive_planner::ALGORITHM
                        && &r.implementation_sha256
                            == ADAPTIVE.get_or_init(crate::vocal_adaptive::implementation_hash)
                }))
    }
    pub fn validate(&self, project: &Project, timing: &TempoMap) -> Result<()> {
        ensure!(
            self.version == 1
                && ((self.algorithm == ANALYSIS_ALGORITHM && self.adaptive.is_none())
                    || (self.algorithm == crate::vocal_adaptive::ALGORITHM
                        && self.adaptive.is_some()))
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
        ensure!(
            self.model_implementation_sha256 == crate::vocal_rmvpe::implementation_hash(),
            "Vocal comparison uses an earlier model implementation; compare again"
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
                    && n.start == c.start
                    && n.end == c.end
                    && timing.seconds(n.start) >= self.range.start
                    && timing.seconds(n.end) <= self.range.end,
                "Compared note moved or changed"
            );
        }
        ensure!(
            self.section.applied_repairs.is_empty(),
            "Section report is already applied"
        );
        ensure!(
            self.section.repairs.len() <= 200_000
                && self.section.findings.len() <= 2000
                && self.section.contour.len() <= 2000,
            "Section report exceeds budget"
        );
        if !self.section.algorithm.is_empty() {
            ensure!(
                self.section.algorithm
                    == if self.adaptive.is_some() {
                        crate::vocal_section_analysis::ADAPTIVE_ALGORITHM
                    } else {
                        crate::vocal_section_analysis::ALGORITHM
                    },
                "Unknown section comparison algorithm"
            );
        } else {
            ensure!(
                self.section.repairs.is_empty()
                    && self.section.findings.is_empty()
                    && self.section.contour.is_empty(),
                "Missing section algorithm"
            );
        }
        if let Some(report) = &self.adaptive {
            report.validate(self.range, project.duration)?;
        }
        ensure!(
            self.section.voiced_seconds.is_finite()
                && self.section.uncharted_seconds.is_finite()
                && self.section.voiced_seconds >= 0.
                && self.section.uncharted_seconds >= 0.
                && self.section.uncharted_seconds <= self.section.voiced_seconds + 0.01
                && self.section.voiced_seconds <= self.range.end - self.range.start + 0.01,
            "Invalid section coverage"
        );
        for r in &self.section.repairs {
            ensure!(!r.after.is_empty(), "Empty section repair");
            let (a, b) = r.bounds();
            ensure!(
                timing.seconds(a) >= self.range.start - 0.002
                    && timing.seconds(b) <= self.range.end + 0.002,
                "Section repair exceeds selected range"
            );
        }
        crate::vocal_section_analysis::apply(chart, &self.section.repairs)?;
        ensure!(
            self.section
                .contour
                .windows(2)
                .all(|w| w[0].seconds < w[1].seconds),
            "Unordered section contour"
        );
        for p in &self.section.contour {
            ensure!(
                p.seconds.is_finite()
                    && p.seconds >= self.range.start
                    && p.seconds < self.range.end
                    && p.midi
                        .is_none_or(|m| m.is_finite() && (0. ..=127.).contains(&m)),
                "Invalid section contour"
            );
        }
        for f in &self.section.findings {
            ensure!(
                f.start.is_finite()
                    && f.end.is_finite()
                    && f.start >= self.range.start
                    && f.start < f.end
                    && f.end <= self.range.end + 0.001
                    && f.detail.len() <= 2000,
                "Invalid section finding"
            );
        }
        Ok(())
    }
}
/// Keep only still-matching applied witnesses from the project's prior check.
/// The caller discards any incoming worker-supplied retained evidence first.
pub(crate) fn retain_prior_evidence(project: &Project, next: &mut Analysis) -> Result<()> {
    let Some(prior) = project.content.vocal_pitch_analysis.as_ref().filter(|a| {
        a.version == next.version
            && a.current_profile()
            && next.current_profile()
            && a.audio_sha256 == next.audio_sha256
            && a.stem_sha256 == next.stem_sha256
            && a.model_sha256 == next.model_sha256
            && a.model_implementation_sha256 == next.model_implementation_sha256
            && a.timing == next.timing
    }) else {
        return Ok(());
    };
    ensure!(
        prior.retained_evidence.len() <= 200_000 && prior.corrections.len() <= 200_000,
        "Too much retained vocal pitch evidence"
    );
    let keys: std::collections::BTreeSet<_> = project
        .content
        .vocals
        .as_ref()
        .into_iter()
        .flat_map(|c| &c.notes)
        .filter_map(|n| match n.tone {
            VocalTone::Pitched { midi } => Some((n.start, n.end, midi)),
            _ => None,
        })
        .collect();
    let mut retained = std::collections::BTreeMap::new();
    for e in prior
        .retained_evidence
        .iter()
        .cloned()
        .chain(
            prior
                .corrections
                .iter()
                .filter(|_| prior.applied && prior.section.applied_pitch)
                .map(correction_evidence),
        )
        .chain(
            prior
                .section
                .applied_repairs
                .iter()
                .filter(|_| prior.applied)
                .filter_map(|i| prior.section.repairs.get(*i))
                .flat_map(|r| r.evidence()),
        )
    {
        let key = (e.start, e.end, e.selected_midi);
        if keys.contains(&key) {
            retained.insert(key, e);
        }
    }
    ensure!(
        retained.len() <= 200_000,
        "Too much retained vocal pitch evidence"
    );
    next.retained_evidence = retained.into_values().collect();
    Ok(())
}
fn correction_evidence(c: &Correction) -> crate::vocal_generation::NoteEvidence {
    crate::vocal_generation::NoteEvidence {
        start: c.start,
        end: c.end,
        selected_midi: c.after,
        alternatives: vec![c.before],
        models: vec![],
        support: vec![crate::vocal_generation::PitchSupport::ContourPath],
        uncertain: true,
        reasons: vec![format!(
            "Independent RMVPE comparison: {} supporting frames. The previous pitch remains an alternative. Model support is not a calibrated correctness probability or singer-identity guarantee.",
            c.support_frames
        )],
    }
}
/// Retuned notes remain visibly uncertain drafts. Preserve the previous pitch
/// as an alternative without rewriting the original generation's diagnostics.
pub fn current_evidence(
    project: &Project,
    timing: &TempoMap,
    note: &crate::vocals::VocalNote,
) -> Option<crate::vocal_generation::NoteEvidence> {
    static IMPLEMENTATION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let implementation = IMPLEMENTATION.get_or_init(crate::vocal_rmvpe::implementation_hash);
    if let Some(a) = project.content.vocal_pitch_analysis.as_ref().filter(|a| {
        a.current_profile()
            && a.audio_sha256 == project.audio_sha256
            && a.timing == *timing
            && a.model_sha256 == crate::vocal_rmvpe::SHA256
            && &a.model_implementation_sha256 == implementation
            && project.content.stems.as_ref().is_some_and(|m| {
                m.stems.iter().any(|s| {
                    s.id == crate::stem_review::StemId::Vocals && s.sha256 == a.stem_sha256
                })
            })
    }) {
        if let Some(c) = a
            .corrections
            .iter()
            .filter(|_| a.applied && a.section.applied_pitch)
            .find(|c| {
                c.start == note.start
                    && c.end == note.end
                    && note.tone == VocalTone::Pitched { midi: c.after }
            })
        {
            return Some(correction_evidence(c));
        }
        if a.applied {
            for r in a
                .section
                .applied_repairs
                .iter()
                .filter_map(|i| a.section.repairs.get(*i))
            {
                if let Some(e) = r.evidence().into_iter().find(|e| {
                    e.start == note.start
                        && e.end == note.end
                        && note.tone
                            == VocalTone::Pitched {
                                midi: e.selected_midi,
                            }
                }) {
                    return Some(e);
                }
            }
        }
        if let VocalTone::Pitched { midi } = note.tone
            && let Ok(index) = a
                .retained_evidence
                .binary_search_by_key(&(note.start, note.end, midi), |e| {
                    (e.start, e.end, e.selected_midi)
                })
        {
            return Some(a.retained_evidence[index].clone());
        }
    }
    project
        .content
        .vocal_review
        .as_ref()?
        .evidence_for(project, timing, note)
        .cloned()
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
    analyze_configured(
        project,
        audio,
        timing,
        range,
        model_directory,
        control,
        false,
    )
}
#[cfg(not(target_arch = "wasm32"))]
pub fn analyze_configured(
    project: &Project,
    audio: &std::path::Path,
    timing: &TempoMap,
    range: Option<TimeRange>,
    model_directory: &std::path::Path,
    control: std::sync::Arc<crate::clean_recognition::Control>,
    adaptive: bool,
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
    if !adaptive {
        return propose(project, timing, &g, range);
    }
    let pcm = crate::models::decode(&stem.path, 16000, 1)?;
    let source = crate::models::decode(audio, 16000, 2)?;
    let vocals = crate::models::decode(&stem.path, 16000, 2)?;
    let mut review =
        crate::vocal_adaptive::Review::new(project, timing, &g, range, pcm, &source, &vocals)?;
    drop(source);
    drop(vocals);
    let mut session = adaptive_session(&model)?;
    while let Some(window) = review.next_window()? {
        ensure!(
            !control.is_cancelled(),
            "Vocal comparison cancelled; current notes were kept"
        );
        if review.pass_count() > 0
            && review
                .pass_count()
                .is_multiple_of(crate::vocal_adaptive::INFERENCE_BATCH)
        {
            drop(session);
            session = adaptive_session(&model)?;
        }
        control.update(format!(
            "Deep vocal review · pass {} · {:.2}–{:.2}s · depth {}",
            review.pass_count() + 1,
            window.start as f64 * 0.01,
            (window.end as f64 * 0.01).min(project.duration),
            window.depth
        ));
        let values =
            crate::rmvpe_frontend::infer(&mut session, review.pcm(), window.start, window.end)?;
        let frames = crate::vocal_rmvpe::decode_chunk(review.pcm(), window.start, &values)?;
        review.complete(&window, frames)?;
    }
    ensure!(
        !control.is_cancelled()
            && crate::models::hash_file(audio)? == project.audio_sha256
            && crate::models::hash_file(&stem.path)? == stem.sha256
            && crate::models::hash_file(&model)? == crate::vocal_rmvpe::SHA256,
        "Vocal sources changed or comparison cancelled"
    );
    review.finish()
}

#[cfg(not(target_arch = "wasm32"))]
fn adaptive_session(model: &std::path::Path) -> Result<ort::session::Session> {
    Ok(ort::session::Session::builder()?
        .with_intra_threads(2)?
        .with_memory_pattern(false)?
        .with_execution_providers([ort::execution_providers::CPUExecutionProvider::default()
            .with_arena_allocator(false)
            .build()])?
        .commit_from_file(model)?)
}
