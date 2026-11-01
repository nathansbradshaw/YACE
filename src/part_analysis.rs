//! Read-only, source-bound Expert/Pro diagnostics shared by the desktop and CLI.
use crate::storage as fs;
use crate::{
    chart::Chart,
    mapping::{Transcription, Voice},
    models,
    stem_review::StemManifest,
};
use crate::{
    editor::{Project, Store},
    stem_review::StemId,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use std::{io::Read, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StageCounts {
    pub audio_attacks: Option<usize>,
    pub saved_hypotheses: Option<usize>,
    pub retained_attacks: Option<usize>,
    pub chart_attacks: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingStage {
    AudioOnly,
    Filtered,
    Mapping,
    Articulation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub stage: FindingStage,
    pub summary: String,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionReport {
    pub id: String,
    pub label: String,
    pub start: f64,
    pub end: f64,
    pub counts: StageCounts,
    pub findings: Vec<Finding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubpartReport {
    pub label: String,
    pub attacks: usize,
    pub rms: f64,
    pub interpretation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartReport {
    pub id: StemId,
    pub label: String,
    pub track: String,
    pub source_available: bool,
    pub counts: StageCounts,
    pub sections: Vec<SectionReport>,
    pub subparts: Vec<SubpartReport>,
    pub limits: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub version: u32,
    pub algorithm: String,
    pub implementation_sha256: String,
    pub source_audio_sha256: String,
    pub chart_sha256: String,
    pub transcription_sha256: Option<String>,
    pub stem_sha256: BTreeMap<String, String>,
    pub duration: f64,
    pub project_key: Option<String>,
    pub parts: Vec<PartReport>,
    pub limits: Vec<String>,
}

/// Cheap content identity: transport, selection, decisions and flag visibility
/// do not invalidate audio/chart diagnostics. Current notes/stems do.
pub fn project_key(project: &Project) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &project.source_chart_sha256,
            &project.audio_sha256,
            &project.content.tracks,
            &project.content.stems,
        ))?)
    ))
}

const TOLERANCE: f64 = 0.060;

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn unique(mut times: Vec<f64>) -> Vec<f64> {
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    times
}
fn nearby(times: &[f64], time: f64) -> bool {
    let i = times.partition_point(|t| *t < time);
    times.get(i).is_some_and(|t| (*t - time).abs() <= TOLERANCE)
        || i.checked_sub(1)
            .and_then(|j| times.get(j))
            .is_some_and(|t| (*t - time).abs() <= TOLERANCE)
}
fn counts(
    audio: Option<&[f64]>,
    saved: Option<&[f64]>,
    retained: Option<&[f64]>,
    chart: &[f64],
    start: f64,
    end: f64,
) -> StageCounts {
    let count =
        |times: &[f64]| times.partition_point(|t| *t < end) - times.partition_point(|t| *t < start);
    StageCounts {
        audio_attacks: audio.map(count),
        saved_hypotheses: saved.map(count),
        retained_attacks: retained.map(count),
        chart_attacks: count(chart),
    }
}

/// Named chart sections are subdivided into bounded listening windows. Without
/// authored names, use explicit time windows, never invented verse/chorus labels.
fn windows(chart: &Chart, duration: f64) -> Vec<(String, f64, f64)> {
    let mut boundaries = vec![(0.0, "Section".to_owned())];
    if let Some(section) = chart.sections.iter().find(|s| s.name == "Events") {
        for line in chart.raw[section.body_start..section.body_end].lines() {
            if let Some((tick, event)) = line.split_once('=')
                && let Ok(tick) = tick.trim().parse::<u64>()
                && let Some(name) = event
                    .trim()
                    .strip_prefix("E \"section ")
                    .and_then(|e| e.strip_suffix('"'))
            {
                let seconds = chart.timing.seconds(tick).clamp(0., duration);
                if seconds < duration {
                    if seconds == 0. {
                        boundaries[0].1 = name.trim().to_owned();
                    } else {
                        boundaries.push((seconds, name.trim().to_owned()));
                    }
                }
            }
        }
    }
    boundaries.sort_by(|a, b| a.0.total_cmp(&b.0));
    boundaries.dedup_by(|a, b| (a.0 - b.0).abs() < 0.001);
    let mut result = Vec::new();
    for (index, (start, label)) in boundaries.iter().enumerate() {
        let end = boundaries.get(index + 1).map_or(duration, |b| b.0);
        let mut at = *start;
        let mut piece = 1;
        while at < end {
            let next = (at + 12.).min(end);
            result.push((
                if end - start > 12. {
                    format!("{label} · {piece}")
                } else {
                    label.clone()
                },
                at,
                next,
            ));
            at = next;
            piece += 1;
        }
    }
    result
}

struct StageEvents {
    saved: Option<Vec<f64>>,
    retained: Option<Vec<f64>>,
    rejected: Vec<(f64, String)>,
}
fn event_stages(id: StemId, events: Option<&Transcription>) -> StageEvents {
    let mut out = StageEvents {
        saved: None,
        retained: None,
        rejected: vec![],
    };
    let Some(events) = events else {
        return out;
    };
    if id == StemId::Drums {
        if !events.drums.is_empty() || events.drum_articulation_source.is_some() {
            let times = unique(events.drums.iter().map(|h| h.time).collect());
            // ADTOF's pre-decoding frame tensor is not persisted. Do not call
            // the retained hit list an independent detector-stage observation.
            out.retained = Some(times);
        }
        return out;
    }
    let (notes, candidates, short) = if id == StemId::Guitar {
        (
            &events.guitar,
            &events.guitar_candidates,
            &events.guitar_short_candidates,
        )
    } else {
        (
            &events.bass,
            &events.bass_candidates,
            &events.bass_short_candidates,
        )
    };
    if notes.is_empty() && candidates.is_empty() && short.is_empty() {
        return out;
    }
    out.retained = Some(unique(notes.iter().map(|n| n.start).collect()));
    if !candidates.is_empty() || !short.is_empty() {
        out.saved = Some(unique(
            candidates
                .iter()
                .map(|c| c.candidate.start)
                .chain(short.iter().map(|c| c.evidence.candidate.start))
                .collect(),
        ));
    }
    out.rejected = short
        .iter()
        .filter(|c| !c.retained)
        .map(|c| (c.evidence.candidate.start, c.reason.clone()))
        .collect();
    out
}

fn finding(
    id: StemId,
    stage: FindingStage,
    time: f64,
    duration: f64,
    summary: &str,
    detail: String,
) -> Finding {
    let key = digest(format!("{}:{stage:?}:{time:.9}:{summary}", id.as_str()).as_bytes());
    Finding {
        id: key,
        start: (time - 0.15).max(0.),
        end: (time + 0.35).min(duration),
        stage,
        summary: summary.into(),
        detail,
    }
}

fn analyse_bound(
    audio: &Path,
    chart: &Chart,
    manifest: Option<&StemManifest>,
    events: Option<&Transcription>,
    event_hash: Option<String>,
) -> Result<AnalysisReport> {
    let audio_hash = models::hash_file(audio)?;
    let loaded = manifest.map(|m| m.load(audio)).transpose()?;
    let duration = if let Some(m) = manifest {
        m.frames as f64 / m.sample_rate as f64
    } else {
        models::decode(audio, 22050, 1)?.len() as f64 / 22050.
    };
    ensure!(
        duration.is_finite() && duration > 0.,
        "Part analysis requires nonempty audio"
    );
    crate::analysis_validation::validate_chart_events(chart, events, duration)?;
    if let Some(events) = events {
        events.validate()?;
        if let Some(bound) = &events.isolation {
            bound.validate_binding(&audio_hash, duration)?;
            if let Some(current) = manifest {
                for old in &bound.stems {
                    if let Some(now) = current.stems.iter().find(|s| s.id == old.id) {
                        ensure!(
                            now.sha256.eq_ignore_ascii_case(&old.sha256),
                            "Transcription and review use different {} stems",
                            old.id.as_str()
                        );
                    }
                }
            }
        }
    }
    let sections = windows(chart, duration);
    let mut parts = Vec::new();
    for (id, label, track) in [
        (StemId::Guitar, "Guitar", "ExpertSingle"),
        (StemId::Bass, "Bass", "ExpertDoubleBass"),
        (StemId::Drums, "Drums", "ExpertDrums"),
    ] {
        let alternate_guitar = id == StemId::Guitar
            && events.is_some_and(|e| e.guitar_source != crate::mapping::GuitarSource::Guitar);
        let source = (!alternate_guitar)
            .then(|| loaded.as_ref().and_then(|l| l.stems.get(&id)))
            .flatten();
        let mono = source.map(|pcm| {
            pcm.chunks_exact(4)
                .map(|v| (v[0] + v[1] + v[2] + v[3]) * 0.25)
                .collect::<Vec<_>>()
        });
        let attacks: Option<Vec<f64>> = mono
            .as_ref()
            .map(|signal| -> Result<_> {
                if id == StemId::Guitar {
                    Ok(crate::guitar_attacks::detect(signal, 22050)?
                        .into_iter()
                        .map(|a| a.seconds)
                        .collect())
                } else {
                    Ok(crate::review::attacks(signal)
                        .into_iter()
                        .map(|a| a.time)
                        .collect())
                }
            })
            .transpose()?
            .map(unique);
        let subparts = if id == StemId::Drums {
            mono.as_ref().map(|signal|crate::drum_parts::analyse(signal,22050)).transpose()?.unwrap_or_default().into_iter().map(|p|SubpartReport{label:p.part,attacks:p.attacks.len(),rms:p.rms,interpretation:"Overlapping frequency/transient view; not an isolated kit piece or a note count.".into()}).collect()
        } else {
            vec![]
        };
        let stage = event_stages(id, events);
        let chart_notes = chart.notes(track)?;
        let chart_times = unique(
            chart_notes
                .iter()
                .filter(|n| crate::editor::playable(track, n.lane))
                .map(|n| chart.timing.seconds(n.tick))
                .collect(),
        );
        let mut findings = Vec::new();
        if let Some(attacks) = &attacks {
            for time in attacks {
                if nearby(&chart_times, *time) {
                    continue;
                }
                if stage.retained.as_ref().is_some_and(|s| nearby(s, *time)) {
                    continue;
                }
                if stage.saved.as_ref().is_some_and(|s| nearby(s, *time)) {
                    continue;
                }
                findings.push(finding(id,FindingStage::AudioOnly,*time,duration,"Attack without saved event",if stage.retained.is_some(){"An independent audio feature has no nearby retained event or Expert chart attack. It may be a detector gap, bleed, a sustained-note articulation or an intentional arrangement choice."}else{"Audio evidence exists, but transcription scope is unknown. An absent event list cannot establish that the generator missed this part."}.into()));
            }
        }
        for (time, reason) in &stage.rejected {
            if stage.retained.as_ref().is_some_and(|s| nearby(s, *time))
                || nearby(&chart_times, *time)
            {
                continue;
            }
            findings.push(finding(id,FindingStage::Filtered,*time,duration,"Saved hypothesis rejected",format!("Saved short-note rejection: {reason}. Rejection is not proof of an omitted real note; inspect the source and nearby harmonics.")));
        }
        if let Some(retained) = &stage.retained {
            for time in retained {
                if !nearby(&chart_times, *time) {
                    findings.push(finding(id,FindingStage::Mapping,*time,duration,"Retained event outside chart","A retained event has no Expert attack within60ms. Grouping, monophonic selection, chord mapping or an intentional edit may explain the difference; this is not automatically a mapping error.".into()));
                }
            }
        }
        if id == StemId::Drums
            && let Some(events) = events
        {
            let applied_cymbals: std::collections::BTreeSet<_> =
                crate::mapping::played_drum_articulations(events, &chart.timing)
                    .into_iter()
                    .filter(|(index, tick, voice)| {
                        matches!(voice, Voice::Ride | Voice::Crash)
                            && events.drum_articulations.iter().any(|e| {
                                e.hit_index == *index
                                    && e.map_proposal
                                    && e.proposed == Some(*voice)
                            })
                            && chart_notes
                                .iter()
                                .any(|note| note.tick == *tick && note.lane == voice.lane())
                    })
                    .map(|(index, _, _)| index)
                    .collect();
            for (index, hit) in events.drums.iter().enumerate() {
                if hit.voice == Voice::Tom
                    || (hit.voice == Voice::Cymbal && !applied_cymbals.contains(&index))
                {
                    let family = if hit.voice == Voice::Tom {
                        "Generic tom"
                    } else {
                        "Generic cymbal"
                    };
                    let mut issue = finding(
                        id,
                        FindingStage::Articulation,
                        hit.time,
                        duration,
                        "Kit piece unresolved",
                        format!(
                            "{family} evidence does not establish the played Pro register or cymbal family. Inspect source audio; eligibility alone is not an applied chart articulation, and no lane change is proposed."
                        ),
                    );
                    // Simultaneous raw hypotheses have distinct ownership even
                    // when their stage, time and visible summary are identical.
                    issue.id = digest(format!("{}:raw:{index}", issue.id).as_bytes());
                    findings.push(issue);
                }
            }
        }
        findings.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.id.cmp(&b.id)));
        findings.dedup_by(|a, b| a.id == b.id);
        let mut rows = Vec::new();
        for (label, start, end) in &sections {
            let inclusive_end = if *end == duration {
                *end + f64::EPSILON * duration
            } else {
                *end
            };
            rows.push(SectionReport {
                id: digest(format!("{}:{start:.9}:{end:.9}", id.as_str()).as_bytes()),
                label: label.clone(),
                start: *start,
                end: *end,
                counts: counts(
                    attacks.as_deref(),
                    stage.saved.as_deref(),
                    stage.retained.as_deref(),
                    &chart_times,
                    *start,
                    inclusive_end,
                ),
                findings: findings
                    .iter()
                    .filter(|f| f.start < *end && f.end > *start)
                    .cloned()
                    .collect(),
            });
        }
        let mut limits=vec!["Saved hypotheses cover only persisted candidate evidence, not the complete model frame output. Stage counts describe different observations and need not form a decreasing funnel.".into()];
        if source.is_none() {
            limits.push(if alternate_guitar {"This Guitar track uses another melody source; guitar-stem diagnostics are unavailable."} else {"No bound instrument stem is attached. Audio evidence is unavailable, not zero or proof of silence."}.into());
        }
        if events.is_some_and(|e| e.isolation.is_none()) {
            limits.push("Legacy events have no isolated-source receipt. Their recording association is unproven; compare them as supplied observations.".into());
        }
        parts.push(PartReport {
            id,
            label: label.into(),
            track: track.into(),
            source_available: source.is_some(),
            counts: counts(
                attacks.as_deref(),
                stage.saved.as_deref(),
                stage.retained.as_deref(),
                &chart_times,
                0.,
                duration + f64::EPSILON * duration,
            ),
            sections: rows,
            subparts,
            limits,
        });
    }
    ensure!(
        models::hash_file(audio)? == audio_hash,
        "Recording changed during part analysis"
    );
    let stem_sha256 = manifest
        .map(|m| {
            m.stems
                .iter()
                .map(|s| (s.id.as_str().into(), s.sha256.clone()))
                .collect()
        })
        .unwrap_or_default();
    if let Some(manifest) = manifest {
        for asset in &manifest.stems {
            ensure!(
                models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
                "Stem changed during part analysis"
            );
        }
    }
    let implementation_sha256 = digest(
        &[
            include_bytes!("part_analysis.rs").as_slice(),
            include_bytes!("guitar_attacks.rs").as_slice(),
            include_bytes!("drum_parts.rs").as_slice(),
            include_bytes!("review.rs").as_slice(),
        ]
        .concat(),
    );
    Ok(AnalysisReport{version:1,algorithm:"part-analysis-v1".into(),implementation_sha256,source_audio_sha256:audio_hash,chart_sha256:digest(chart.raw.as_bytes()),transcription_sha256:event_hash,stem_sha256,duration,project_key:None,parts,limits:vec!["Expert/Pro diagnostics only. Findings are uncalibrated listening prompts, not missing-note truth, musical approval or permission to edit.".into(),"Frequency bands overlap and contain leakage. Their attacks are signal features, not isolated drum identities; the22.05kHz report analysis caps the Metal band at11.025kHz. Keep the original and instrument audio for comparison.".into(),"Audio features and chart attacks use the original recording clock. No automatic offset fitting or note changes are performed.".into()]})
}

/// Analyze the current editor snapshot without exporting, saving, changing notes
/// or writing approval decisions. All heavy work belongs on a worker thread.
pub fn analyze_project(store: &Store, project: &Project) -> Result<AnalysisReport> {
    let key = project_key(project)?;
    let audio = store.audio_path(project)?;
    ensure!(
        models::hash_file(&audio)? == project.audio_sha256,
        "Project recording changed"
    );
    let chart = Chart::parse(project.chart_text(store)?)?;
    let event_path = store.root.join("source/chart-engine-transcription.json");
    let (events, hash) = if event_path.is_file() {
        let bytes = fs::read(&event_path)?;
        (
            Some(serde_json::from_slice::<Transcription>(&bytes)?),
            Some(digest(&bytes)),
        )
    } else {
        (None, None)
    };
    let mut result = analyse_bound(
        &audio,
        &chart,
        project.content.stems.as_ref(),
        events.as_ref(),
        hash.clone(),
    )?;
    if let Some(hash) = hash {
        ensure!(
            models::hash_file(&event_path)? == hash,
            "Saved hypotheses changed during analysis"
        );
    }
    result.project_key = Some(key);
    Ok(result)
}

pub fn analyze_files(
    audio: &Path,
    chart_path: &Path,
    manifest_path: Option<&Path>,
    events_path: Option<&Path>,
) -> Result<AnalysisReport> {
    let chart_hash = models::hash_file(chart_path)?;
    let chart = Chart::parse(fs::read_to_string(chart_path)?)?;
    let manifest = manifest_path.map(StemManifest::read).transpose()?;
    let manifest_hash = manifest_path.map(models::hash_file).transpose()?;
    let event_bytes = events_path.map(fs::read).transpose()?;
    let event_hash = event_bytes.as_ref().map(|b| digest(b));
    let events = event_bytes
        .as_ref()
        .map(|b| serde_json::from_slice::<Transcription>(b))
        .transpose()
        .context("Invalid saved instrument events")?;
    let result = analyse_bound(
        audio,
        &chart,
        manifest.as_ref(),
        events.as_ref(),
        event_hash.clone(),
    )?;
    ensure!(
        models::hash_file(chart_path)? == chart_hash,
        "Chart changed during analysis"
    );
    if let Some(path) = manifest_path {
        ensure!(
            Some(models::hash_file(path)?) == manifest_hash,
            "Manifest changed during analysis"
        );
    }
    if let Some(path) = events_path {
        ensure!(
            Some(models::hash_file(path)?) == event_hash,
            "Saved events changed during analysis"
        );
    }
    Ok(result)
}

#[path = "kit_analysis.rs"]
mod kit;
pub use kit::*;
