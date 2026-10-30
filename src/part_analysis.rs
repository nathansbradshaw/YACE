//! Read-only, source-bound Expert/Pro diagnostics shared by the desktop and CLI.
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
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

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
    crate::stem_coverage_review::validate_chart_events(chart, events, duration)?;
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumPartFile {
    pub id: String,
    pub label: String,
    pub filename: String,
    pub sha256: String,
    pub frames: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumPartSet {
    pub source_key: String,
    pub original_audio_sha256: String,
    pub drum_audio_sha256: String,
    pub directory: PathBuf,
    pub parts: Vec<DrumPartFile>,
    pub receipt: crate::larsnet::SeparationReceipt,
}
/// A persisted listening preference, independent of chart notes and approvals.
pub use crate::kit_audition::KitAudition;
pub struct KitClip {
    pub part: String,
    pub start: f64,
    pub end: f64,
    pub wav: Vec<u8>,
}

/// Audio analysis can survive note edits. Never reuse it for changed stems or
/// another recording merely because the song title or duration happens to match.
pub use crate::kit_audition::source_key as kit_source_key;
fn project_drums(store: &Store, project: &Project) -> Result<(PathBuf, String)> {
    let original = store.audio_path(project)?;
    ensure!(
        models::hash_file(&original)? == project.audio_sha256,
        "Project recording changed"
    );
    let manifest = project
        .content
        .stems
        .as_ref()
        .context("Attach aligned drum audio before splitting the kit")?;
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    let asset = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Drums)
        .context("No drum stem is attached")?;
    ensure!(
        models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
        "Attached drum audio changed"
    );
    Ok((asset.path.clone(), asset.sha256.clone()))
}

const KIT_JSON_LIMIT: u64 = 256 * 1024;
fn read_kit_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= KIT_JSON_LIMIT,
        "Invalid or oversized kit cache JSON"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(KIT_JSON_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= KIT_JSON_LIMIT,
        "Kit cache JSON grew while reading"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_cached_kit(
    set: &DrumPartSet,
    cache: &Path,
    project: &Project,
    source_key: &str,
    drum_hash: &str,
) -> Result<()> {
    use crate::larsnet;
    let frames = project.content.stems.as_ref().context("No stems")?.frames;
    ensure!(
        set.source_key == source_key
            && set.original_audio_sha256 == project.audio_sha256
            && set.drum_audio_sha256 == drum_hash
            && set.receipt.source_audio_sha256.as_deref() == Some(drum_hash),
        "Stale kit cache source"
    );
    let receipt = &set.receipt;
    ensure!(
        receipt.version == 1
            && receipt.algorithm == larsnet::ALGORITHM
            && receipt.implementation_sha256 == larsnet::implementation_hash()
            && receipt.sample_rate == 44100
            && receipt.channels == 2
            && receipt.frames == frames
            && receipt.weight_license == larsnet::WEIGHT_LICENSE,
        "Unsupported kit cache receipt"
    );
    let expected: BTreeMap<String, String> = larsnet::MODELS
        .into_iter()
        .map(|(id, hash)| (id.into(), hash.into()))
        .collect();
    ensure!(
        receipt.models == expected && set.parts.len() == 5 && receipt.outputs.len() == 5,
        "Incomplete kit cache or model identities"
    );
    let directory = cache.join("parts");
    ensure!(
        fs::symlink_metadata(&directory)?.is_dir()
            && set.directory.canonicalize()? == directory.canonicalize()?,
        "Kit cache points outside its component folder"
    );
    let native: larsnet::SeparationReceipt =
        read_kit_json(&directory.join(larsnet::RECEIPT_FILENAME))?;
    ensure!(
        serde_json::to_value(&native)? == serde_json::to_value(receipt)?,
        "Kit cache/native receipt mismatch"
    );
    let valid_hash = |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        valid_hash(&receipt.source_pcm_sha256),
        "Invalid kit source PCM identity"
    );
    for (id, label) in [
        ("kick", "Kick"),
        ("snare", "Snare"),
        ("toms", "Toms"),
        ("hihat", "Hi-hat"),
        ("cymbals", "Cymbals"),
    ] {
        ensure!(
            set.parts.iter().filter(|p| p.id == id).count() == 1
                && receipt.outputs.iter().filter(|p| p.part == id).count() == 1,
            "Duplicate or missing kit component"
        );
        let part = set
            .parts
            .iter()
            .find(|p| p.id == id)
            .context("Missing kit part")?;
        let output = receipt
            .outputs
            .iter()
            .find(|p| p.part == id)
            .context("Missing native part receipt")?;
        ensure!(
            part.label == label
                && part.filename == format!("{id}.wav")
                && output.filename == part.filename
                && part.frames == frames
                && output.frames == frames
                && output.wav_sha256.as_ref() == Some(&part.sha256)
                && valid_hash(&part.sha256)
                && valid_hash(&output.pcm_sha256),
            "Invalid cached component metadata"
        );
        let path = directory.join(&part.filename);
        let metadata = fs::symlink_metadata(&path)?;
        // Native output has a 44-byte IEEE-float RIFF header and eight bytes
        // per stereo frame. Check the frame count without decoding the audio.
        let data_bytes = u32::try_from(frames.checked_mul(8).context("Kit frame count overflow")?)?;
        let riff_bytes = data_bytes
            .checked_add(36)
            .context("Kit RIFF size overflow")?;
        ensure!(
            metadata.is_file() && metadata.len() == u64::from(data_bytes) + 44,
            "Cached component has another frame count"
        );
        let mut header = [0u8; 44];
        fs::File::open(&path)?.read_exact(&mut header)?;
        ensure!(
            &header[0..4] == b"RIFF"
                && &header[8..16] == b"WAVEfmt "
                && u32::from_le_bytes(header[4..8].try_into()?) == riff_bytes
                && u32::from_le_bytes(header[16..20].try_into()?) == 16
                && u16::from_le_bytes(header[20..22].try_into()?) == 3
                && u16::from_le_bytes(header[22..24].try_into()?) == 2
                && u32::from_le_bytes(header[24..28].try_into()?) == 44100
                && u32::from_le_bytes(header[28..32].try_into()?) == 44100 * 8
                && u16::from_le_bytes(header[32..34].try_into()?) == 8
                && u16::from_le_bytes(header[34..36].try_into()?) == 32
                && &header[36..40] == b"data"
                && u32::from_le_bytes(header[40..44].try_into()?) == data_bytes,
            "Invalid native kit audio header/clock"
        );
        ensure!(
            models::hash_file(&path)? == part.sha256,
            "Cached kit audio changed"
        );
    }
    Ok(())
}

/// Restore the newest intact source-bound five-part cache without loading a
/// model, decoding full audio, changing notes, or writing a review decision.
/// Invalid/stale candidates are skipped; a changed current recording or stem
/// remains an error instead of masquerading as an absent cache.
pub fn load_project_drum_parts(store: &Store, project: &Project) -> Result<Option<DrumPartSet>> {
    if project
        .content
        .stems
        .as_ref()
        .is_none_or(|m| !m.stems.iter().any(|s| s.id == StemId::Drums))
    {
        return Ok(None);
    }
    let (_, drum_hash) = project_drums(store, project)?;
    let source_key = kit_source_key(project)?;
    let analysis = store.root.join("analysis");
    let entries = match fs::read_dir(&analysis) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if !entry.file_name().to_string_lossy().starts_with("kit-")
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let path = entry.path().join("kit-parts.json");
        if let Ok(meta) = fs::symlink_metadata(&path) {
            candidates.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), path));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut selected = None;
    for (_, path) in candidates {
        let Ok(set) = read_kit_json::<DrumPartSet>(&path) else {
            continue;
        };
        if validate_cached_kit(
            &set,
            path.parent().context("Missing kit cache folder")?,
            project,
            &source_key,
            &drum_hash,
        )
        .is_ok()
        {
            selected = Some(set);
            break;
        }
    }
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed while loading kit cache"
    );
    Ok(selected)
}

/// Select a complete current cache. Call on a worker: all five WAV hashes are
/// checked, and the descriptor never admits an arbitrary caller-provided file.
pub fn select_kit_audition(store: &Store, project: &Project, part: &str) -> Result<KitAudition> {
    let set = load_project_drum_parts(store, project)?.context("Split the drum kit first")?;
    let file = set
        .parts
        .iter()
        .find(|file| file.id == part)
        .context("Unknown kit part")?;
    Ok(KitAudition {
        source_key: set.source_key.clone(),
        kit: set
            .directory
            .parent()
            .context("Missing kit cache directory")?
            .join("kit-parts.json"),
        part: file.id.clone(),
        label: file.label.clone(),
        audio_sha256: file.sha256.clone(),
        frames: file.frames,
    })
}

/// Validate and decode one full-length component on the recording clock. The
/// transport performs rate conversion separately; no offset or trim is applied.
pub fn validated_kit_audition_path(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
) -> Result<PathBuf> {
    ensure!(
        selected.source_key == kit_source_key(project)?,
        "Kit audition belongs to different source audio"
    );
    let (_, drum_hash) = project_drums(store, project)?;
    let kit = selected.kit.canonicalize()?;
    let cache = kit.parent().context("Missing kit cache")?;
    let analysis = store.root.join("analysis").canonicalize()?;
    ensure!(
        kit.file_name().is_some_and(|name| name == "kit-parts.json")
            && cache.parent() == Some(analysis.as_path())
            && cache
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("kit-")),
        "Kit audition must use a saved cache in this project"
    );
    let set: DrumPartSet = read_kit_json(&kit)?;
    validate_cached_kit(&set, cache, project, &selected.source_key, &drum_hash)?;
    let file = set
        .parts
        .iter()
        .find(|file| file.id == selected.part)
        .context("Unknown kit part")?;
    ensure!(
        file.label == selected.label
            && file.sha256 == selected.audio_sha256
            && file.frames == selected.frames,
        "Kit audition descriptor changed"
    );
    let path = set.directory.join(&file.filename);
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed while validating kit audition"
    );
    Ok(path)
}

pub fn load_kit_audition(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
) -> Result<(PathBuf, Vec<f32>)> {
    let path = validated_kit_audition_path(store, project, selected)?;
    let (_, drum_hash) = project_drums(store, project)?;
    let pcm = models::decode(&path, 44100, 2)?;
    ensure!(
        pcm.iter().all(|sample| sample.is_finite()),
        "Kit audition contains nonfinite PCM"
    );
    ensure!(
        pcm.len() as u64 == selected.frames * 2,
        "Kit audition has another sample clock"
    );
    ensure!(
        models::hash_file(&path)? == selected.audio_sha256,
        "Kit component changed during decode"
    );
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed during kit decode"
    );
    Ok((path, pcm))
}

/// Prepare the component for the exact already prepared shared transport.
/// Source frames are exact at normal speed; atempo's bounded trailing padding
/// follows the same policy as the ordinary instrument mixer.
pub fn prepare_kit_audition(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
    rate: f64,
    transport_samples: usize,
) -> Result<Vec<f32>> {
    ensure!(
        rate.is_finite()
            && (0.5..=1.5).contains(&rate)
            && transport_samples > 0
            && transport_samples.is_multiple_of(2),
        "Invalid kit transport clock"
    );
    let (path, pcm) = load_kit_audition(store, project, selected)?;
    let mut pcm = if rate == 1.0 {
        pcm
    } else {
        models::decode_at_tempo(&path, 44100, 2, rate)?
    };
    ensure!(
        pcm.iter().all(|sample| sample.is_finite()),
        "Practice kit audition contains nonfinite PCM"
    );
    ensure!(
        models::hash_file(&path)? == selected.audio_sha256,
        "Kit audio changed during practice preparation"
    );
    if rate == 1.0 {
        ensure!(
            pcm.len() == transport_samples,
            "Kit source and shared transport have different frame counts"
        );
    } else {
        ensure!(
            pcm.len().abs_diff(transport_samples) <= ((44100.0 * 0.10 / rate) as usize) * 2,
            "Tempo-converted kit duration differs by over 100 ms"
        );
    }
    pcm.resize(transport_samples, 0.0);
    Ok(pcm)
}

pub fn prepare_project_drum_parts(
    store: &Store,
    project: &Project,
    model_dir: &Path,
) -> Result<DrumPartSet> {
    let (drums, hash) = project_drums(store, project)?;
    // Full recording/stem frame validation precedes any expensive model run.
    drop(
        project
            .content
            .stems
            .as_ref()
            .context("No stems")?
            .load(&store.audio_path(project)?)?,
    );
    let analysis = store.root.join("analysis");
    fs::create_dir_all(&analysis)?;
    let analysis = analysis.canonicalize()?;
    let stage = tempfile::Builder::new()
        .prefix("kit-")
        .tempdir_in(&analysis)?;
    let output = stage.path().join("parts");
    let receipt = crate::larsnet::separate_files(&drums, model_dir, &output)?;
    ensure!(
        receipt.source_audio_sha256.as_ref() == Some(&hash),
        "Separated drum receipt belongs to another source"
    );
    ensure!(
        receipt.frames == project.content.stems.as_ref().context("No stems")?.frames,
        "Separated kit changed recording length"
    );
    let parts = receipt
        .outputs
        .iter()
        .map(|part| -> Result<_> {
            let label = match part.part.as_str() {
                "kick" => "Kick",
                "snare" => "Snare",
                "toms" => "Toms",
                "hihat" => "Hi-hat",
                "cymbals" => "Cymbals",
                _ => anyhow::bail!("Unexpected separated kit output"),
            };
            Ok(DrumPartFile {
                id: part.part.clone(),
                label: label.into(),
                filename: part.filename.clone(),
                sha256: part
                    .wav_sha256
                    .clone()
                    .context("Separated audio hash missing")?,
                frames: part.frames,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (_, fresh_hash) = project_drums(store, project)?;
    ensure!(fresh_hash == hash, "Drum source changed during separation");
    let result = DrumPartSet {
        source_key: kit_source_key(project)?,
        original_audio_sha256: project.audio_sha256.clone(),
        drum_audio_sha256: hash,
        directory: output,
        parts,
        receipt,
    };
    fs::write(
        stage.path().join("kit-parts.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    let _durable = stage.keep();
    Ok(result)
}

/// Export a bounded listening clip on the original recording clock. All file
/// reads and conversion must run on a worker; no main-player state is changed.
pub fn audition_clip(
    store: &Store,
    project: &Project,
    set: &DrumPartSet,
    part_id: &str,
    start: f64,
    end: f64,
) -> Result<KitClip> {
    ensure!(
        start.is_finite()
            && end.is_finite()
            && start >= 0.
            && end > start
            && end <= project.duration
            && end - start <= 20.,
        "Clip must be a valid recording range of at most20seconds"
    );
    ensure!(
        set.source_key == kit_source_key(project)?
            && set.original_audio_sha256 == project.audio_sha256,
        "Separated kit belongs to another recording or stem attachment"
    );
    let (_, hash) = project_drums(store, project)?;
    ensure!(
        set.drum_audio_sha256 == hash && set.receipt.source_audio_sha256.as_ref() == Some(&hash),
        "Separated kit source receipt changed"
    );
    let part = set
        .parts
        .iter()
        .find(|p| p.id == part_id)
        .context("Unknown separated kit part")?;
    ensure!(
        set.receipt.version == 1
            && set.receipt.algorithm == crate::larsnet::ALGORITHM
            && set.receipt.implementation_sha256 == crate::larsnet::implementation_hash()
            && set.receipt.sample_rate == 44100
            && set.receipt.channels == 2,
        "Unsupported separated kit receipt"
    );
    let expected_models: BTreeMap<String, String> = crate::larsnet::MODELS
        .into_iter()
        .map(|(id, hash)| (id.into(), hash.into()))
        .collect();
    ensure!(
        set.receipt.models == expected_models,
        "Separated kit model identities changed"
    );
    let source_frames = project.content.stems.as_ref().context("No stems")?.frames;
    ensure!(
        set.receipt.frames == source_frames && part.frames == source_frames,
        "Separated kit sample count differs from the attached recording clock"
    );
    let output = set
        .receipt
        .outputs
        .iter()
        .find(|p| p.part == part_id)
        .context("Part missing from native separation receipt")?;
    ensure!(
        output.filename == part.filename
            && output.frames == part.frames
            && output.wav_sha256.as_ref() == Some(&part.sha256),
        "Separated part metadata differs from its native receipt"
    );
    ensure!(
        part.filename == format!("{}.wav", part.id)
            && crate::larsnet::MODELS
                .iter()
                .any(|(name, _)| *name == part.id),
        "Invalid kit output filename"
    );
    let path = set.directory.join(&part.filename);
    ensure!(
        models::hash_file(&path)? == part.sha256,
        "Separated kit audio changed"
    );
    let audio = models::decode(&path, 44100, 2)?;
    ensure!(
        audio.len() / 2 == part.frames as usize && part.frames == set.receipt.frames,
        "Separated kit has another sample count"
    );
    let first = (start * 44100.).floor() as usize;
    let last = ((end * 44100.).ceil() as usize).min(audio.len() / 2);
    ensure!(
        last > first && first < audio.len() / 2,
        "Empty audition range"
    );
    let left: Vec<_> = (first..last).map(|i| audio[i * 2]).collect();
    let right: Vec<_> = (first..last).map(|i| audio[i * 2 + 1]).collect();
    let stage = tempfile::tempdir()?;
    let clip = stage.path().join("clip.wav");
    models::write_audio(&clip, &left, &right)?;
    ensure!(
        models::hash_file(&path)? == part.sha256,
        "Separated kit audio changed during audition"
    );
    Ok(KitClip {
        part: part_id.into(),
        start: first as f64 / 44100.,
        end: last as f64 / 44100.,
        wav: fs::read(clip)?,
    })
}
