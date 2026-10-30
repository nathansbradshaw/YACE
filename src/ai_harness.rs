//! Bounded, deterministic review context and guarded use of the shared editor.
//! No vendor, language model, automatic approval, or independent write path.
use crate::{
    editor::{self, Action, Actor, ActorKind, DecisionStatus, Project, Store},
    generated_rules::{self, Finding},
    models,
    stem_review::{AlignmentEvidence, StemManifest},
    timing::TempoMap,
    vocal_review,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

const MAX_PACKET_BYTES: usize = 512 * 1024;
const DETAIL_LIMIT: usize = 16;
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn short(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub track: Option<String>,
    pub limit: usize,
    pub note_limit: usize,
    pub context_seconds: f64,
    pub include_hidden: bool,
    pub include_approved: bool,
    /// A section ID from this same revision/options; omitted on the first page.
    pub after: Option<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            track: None,
            limit: 3,
            note_limit: 96,
            context_seconds: 1.,
            include_hidden: false,
            include_approved: false,
            after: None,
        }
    }
}
impl Options {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.track
                .as_ref()
                .is_none_or(|track| !track.is_empty() && track.len() <= 100),
            "Invalid review track filter"
        );
        ensure!(
            (1..=8).contains(&self.limit),
            "Review section limit must be 1..8"
        );
        ensure!(
            (1..=256).contains(&self.note_limit),
            "Context note limit must be 1..256"
        );
        ensure!(
            self.context_seconds.is_finite() && (0.0..=3.0).contains(&self.context_seconds),
            "Context must be finite and 0..3 seconds"
        );
        ensure!(
            self.after
                .as_ref()
                .is_none_or(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit())),
            "Invalid section cursor"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioAsset {
    pub instrument: String,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioContext {
    pub mix: AudioAsset,
    pub stems: Vec<AudioAsset>,
    pub alignment: Option<AlignmentEvidence>,
    pub sample_rate: Option<u32>,
    pub frames: Option<u64>,
    pub scope: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteContext {
    pub id: String,
    pub tick: u64,
    pub end_tick: u64,
    pub start: f64,
    pub end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cymbal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<crate::vocals::VocalTone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyric: Option<String>,
    /// Distinguishes displayed words, intentional slides, and independent
    /// measured contours that still belong to the same word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syllable_kind: Option<VocalSyllableKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<Lineage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_children: Vec<LinkedChild>,
    pub omitted_children: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocalSyllableKind {
    Missing,
    Text,
    PitchSlide,
    ContinueWord,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedChild {
    pub track: String,
    pub note_id: String,
    pub linked: bool,
    pub manual_override: bool,
    pub suppressed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lineage {
    pub expert_track: String,
    pub parent_ids: Vec<String>,
    pub omitted_parents: usize,
    pub linked: bool,
    pub manual_override: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueContext {
    pub start: f64,
    pub end: f64,
    pub reason: String,
    pub detail: String,
    pub evidence: BTreeMap<String, f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: String,
    pub region_id: String,
    pub track: String,
    pub start: f64,
    pub end: f64,
    pub context_start: f64,
    pub context_end: f64,
    pub region_content_sha256: Option<String>,
    pub approval_allowed: bool,
    pub source_stale_reason: Option<String>,
    pub hidden: bool,
    pub current_decision: Option<DecisionStatus>,
    pub stale_decision: bool,
    pub issues: Vec<IssueContext>,
    pub omitted_issues: usize,
    pub notes: Vec<NoteContext>,
    pub omitted_notes: usize,
    pub rule_findings: Vec<Finding>,
    pub omitted_rule_findings: usize,
    pub rule_scope: String,
    pub vocal_phrases: Vec<crate::vocals::VocalPhrase>,
    pub omitted_vocal_phrases: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KitAuditionContext {
    pub selected: crate::part_analysis::KitAudition,
    pub audio: AudioAsset,
    pub sample_rate: u32,
    pub scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub version: u32,
    pub packet_sha256: String,
    pub project: PathBuf,
    pub revision: u64,
    pub title: String,
    pub duration_seconds: f64,
    pub source_chart_sha256: String,
    pub chart_sha256: String,
    pub project_state_sha256: String,
    pub review_map_sha256: String,
    pub source_review_chart_current: bool,
    pub vocal_document_sha256: Option<String>,
    pub vocal_review_report_sha256: Option<String>,
    pub vocal_review_stem_sha256: Option<String>,
    pub audio: AudioContext,
    /// Actual Piano analysis when it differs from the primary instrument stems.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_audio: Option<AudioContext>,
    /// Current kit source on the same native playback clock, if selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_audition: Option<KitAuditionContext>,
    pub timing: TempoMap,
    pub options: Options,
    pub total_eligible: usize,
    pub remaining: usize,
    pub next_after: Option<String>,
    pub sections: Vec<Section>,
    pub musical_acceptance: String,
}
impl Packet {
    fn calculated_hash(&self) -> Result<String> {
        let mut value = self.clone();
        value.packet_sha256.clear();
        digest(&value)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported AI review packet");
        self.options.validate()?;
        ensure!(
            self.sections.len() <= self.options.limit
                && serde_json::to_vec_pretty(self)?.len() <= MAX_PACKET_BYTES,
            "AI review packet exceeds bounds"
        );
        ensure!(
            self.packet_sha256 == self.calculated_hash()?,
            "AI review packet checksum mismatch"
        );
        if let Some(keyboard) = &self.keyboard_audio {
            ensure!(
                keyboard.mix.path == self.audio.mix.path
                    && keyboard.mix.sha256 == self.audio.mix.sha256
                    && keyboard.mix.instrument == self.audio.mix.instrument
                    && keyboard.sample_rate == Some(44100)
                    && keyboard.frames.is_some_and(|frames| frames > 0)
                    && self
                        .audio
                        .frames
                        .is_none_or(|frames| Some(frames) == keyboard.frames),
                "Keyboard audio context differs from the primary recording/source clock"
            );
        }
        if let Some(kit) = &self.kit_audition {
            ensure!(
                kit.sample_rate == 44100
                    && kit.selected.frames > 0
                    && kit.audio.sha256 == kit.selected.audio_sha256
                    && kit.audio.instrument == format!("drums:{}", kit.selected.part),
                "Kit audition context differs from its selected source"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    pub packet_sha256: String,
    pub expected_revision: u64,
    /// The harness always constructs ActorKind::Ai; no caller-supplied role.
    pub actor_name: String,
    pub action: Action,
}

pub fn review(store: &Store, options: Options) -> Result<Packet> {
    from_project(store, &store.load()?, options)
}
type ChildIndex = BTreeMap<(String, String), BTreeMap<(String, String), LinkedChild>>;
fn child_index(project: &Project) -> ChildIndex {
    let mut index = ChildIndex::new();
    for (track, link) in &project.content.difficulty_links.links {
        for (id, ancestry) in &link.ancestry {
            for parent in &ancestry.parent_ids {
                index
                    .entry((link.expert_track.clone(), parent.clone()))
                    .or_default()
                    .insert(
                        (track.clone(), id.clone()),
                        LinkedChild {
                            track: track.clone(),
                            note_id: id.clone(),
                            linked: link.enabled,
                            manual_override: link.overrides.contains_key(id),
                            suppressed: link.overrides.get(id).is_some_and(|o| o.note.is_none()),
                        },
                    );
            }
        }
        for (id, local) in &link.overrides {
            if let Some(ancestry) = &local.ancestry {
                for parent in &ancestry.parent_ids {
                    index
                        .entry((link.expert_track.clone(), parent.clone()))
                        .or_default()
                        .insert(
                            (track.clone(), id.clone()),
                            LinkedChild {
                                track: track.clone(),
                                note_id: id.clone(),
                                linked: link.enabled,
                                manual_override: true,
                                suppressed: local.note.is_none(),
                            },
                        );
                }
            }
        }
    }
    index
}
fn bound_audio_context(
    mix: &AudioAsset,
    manifest: Option<&StemManifest>,
    duration: f64,
) -> Result<AudioContext> {
    let mut audio = AudioContext {
        mix: mix.clone(),
        stems: vec![],
        alignment: None,
        sample_rate: None,
        frames: None,
        scope: "Source-time coordinates; checksums verified. Alignment provenance is supplied evidence, not listening acceptance.".into(),
    };
    if let Some(manifest) = manifest {
        manifest.validate_binding(&mix.sha256, duration)?;
        for stem in &manifest.stems {
            let path = stem
                .path
                .canonicalize()
                .context("Attached review stem is missing")?;
            ensure!(
                models::hash_file(&path)? == stem.sha256,
                "Attached {} stem changed; reload/rebind the source",
                stem.id.as_str()
            );
            audio.stems.push(AudioAsset {
                instrument: stem.id.as_str().into(),
                path,
                sha256: stem.sha256.clone(),
            });
        }
        audio.stems.sort_by(|a, b| a.instrument.cmp(&b.instrument));
        audio.alignment = Some(AlignmentEvidence {
            method: short(&manifest.alignment.method, 200),
            details: short(&manifest.alignment.details, 512),
        });
        audio.sample_rate = Some(manifest.sample_rate);
        audio.frames = Some(manifest.frames);
    }
    Ok(audio)
}
fn from_project(store: &Store, project: &Project, options: Options) -> Result<Packet> {
    options.validate()?;
    if let Some(track) = &options.track {
        ensure!(
            project.content.tracks.contains_key(track)
                || (track == vocal_review::TRACK && project.content.vocals.is_some()),
            "Unknown review track; use an exact existing track name"
        );
    }
    let timing = project.timing(store)?;
    let mix = store
        .audio_path(project)?
        .canonicalize()
        .context("Review recording is missing")?;
    ensure!(
        models::hash_file(&mix)? == project.audio_sha256,
        "Review recording changed; source fingerprint mismatch"
    );
    let mix = AudioAsset {
        instrument: "mix".into(),
        path: mix,
        sha256: project.audio_sha256.clone(),
    };
    let audio = bound_audio_context(&mix, project.content.stems.as_ref(), project.duration)?;
    let keyboard_audio = project.content.keyboard_stems.as_ref().map(|manifest| {
        let mut context = bound_audio_context(&mix, Some(manifest), project.duration)?;
        context.scope = "Authoritative Piano/Pro Piano analysis in source-time coordinates, separate from the primary instrument stems. Checksums verified; alignment provenance is not listening acceptance.".into();
        Ok::<_, anyhow::Error>(context)
    }).transpose()?;
    // Apply generation rules only to generated or linked tracks.
    // Authored control lanes can use a wider set of valid events.
    let generated: crate::chart::Tracks = project
        .content
        .tracks
        .iter()
        .filter(|(track, _)| {
            project
                .review
                .generated_tracks
                .as_ref()
                .is_some_and(|s| s.contains(*track))
                || project.content.difficulty_links.links.contains_key(*track)
        })
        .map(|(t, n)| (t.clone(), n.iter().map(|n| n.note.clone()).collect()))
        .collect();
    let rules = generated_rules::assess(&generated, &timing, project.duration);
    let child_index = child_index(project);
    let mut scopes = vec![];
    for region in &project.review.regions {
        for track in &region.tracks {
            scopes.push((region.start, region.end, region.id.clone(), track.clone()));
        }
    }
    if let Some(map) = &project.content.vocal_review {
        for region in &map.regions {
            scopes.push((
                region.start,
                region.end,
                region.id.clone(),
                vocal_review::TRACK.into(),
            ));
        }
    }
    scopes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.3.cmp(&b.3)).then(a.2.cmp(&b.2)));
    let mut eligible = vec![];
    for (start, end, region, track) in scopes {
        if options
            .track
            .as_ref()
            .is_some_and(|filter| filter != &track)
        {
            continue;
        }
        ensure!(
            region.len() <= 512 && track.len() <= 100,
            "Review identity exceeds packet bounds"
        );
        let hidden = project.is_flag_hidden(&region, &track);
        let decision = project.decision(&region, &track, &timing).map(|d| d.status);
        if (hidden && !options.include_hidden)
            || (decision.is_some_and(|s| s != DecisionStatus::Pending) && !options.include_approved)
        {
            continue;
        }
        let id = digest(&(&region, &track))?;
        eligible.push((id, start, end, region, track, hidden, decision));
    }
    let total_eligible = eligible.len();
    let begin = if let Some(after) = &options.after {
        eligible
            .iter()
            .position(|s| &s.0 == after)
            .context("Cursor is absent from current eligible sections; obtain a fresh packet")?
            + 1
    } else {
        0
    };
    let end = (begin + options.limit).min(eligible.len());
    let mut sections = vec![];
    for (id, start, end, region_id, track, hidden, current_decision) in &eligible[begin..end] {
        let context_start = (start - options.context_seconds).max(0.);
        let context_end = (end + options.context_seconds).min(project.duration);
        let mut issues = vec![];
        if track == vocal_review::TRACK {
            let region = project
                .content
                .vocal_review
                .as_ref()
                .and_then(|m| m.region(region_id))
                .context("Vocal flag vanished")?;
            for issue in &region.issues {
                issues.push(IssueContext {
                    start: issue.start_seconds,
                    end: issue.end_seconds,
                    reason: serde_json::to_value(issue.reason)?
                        .as_str()
                        .unwrap_or("vocal_review")
                        .into(),
                    detail: short(&issue.detail, 384),
                    evidence: BTreeMap::new(),
                });
            }
        } else {
            let region = project
                .review
                .regions
                .iter()
                .find(|r| &r.id == region_id)
                .context("Flag vanished")?;
            for issue in region.issues.iter().filter(|i| &i.track == track) {
                issues.push(IssueContext {
                    start: issue.start,
                    end: issue.end,
                    reason: serde_json::to_value(issue.reason)?
                        .as_str()
                        .unwrap_or("review")
                        .into(),
                    detail: issue.reason.description().into(),
                    evidence: issue
                        .evidence
                        .iter()
                        .take(16)
                        .map(|(k, v)| (short(k, 100), *v))
                        .collect(),
                });
            }
        }
        let omitted_issues = issues.len().saturating_sub(DETAIL_LIMIT);
        issues.truncate(DETAIL_LIMIT);
        let mut notes = vec![];
        let mut vocal_phrases = vec![];
        if track == vocal_review::TRACK {
            if let Some(chart) = &project.content.vocals {
                for note in &chart.notes {
                    let a = timing.seconds(note.start);
                    let b = timing.seconds(note.end);
                    if b < context_start || a > context_end {
                        continue;
                    }
                    let lyric = match &note.lyric {
                        crate::vocals::Syllable::Missing => String::new(),
                        crate::vocals::Syllable::Text { text, join_next } => {
                            format!("{}{}", short(text, 128), if *join_next { "-" } else { "" })
                        }
                        crate::vocals::Syllable::Continue => "+".into(),
                        crate::vocals::Syllable::ContinueWord => String::new(),
                    };
                    notes.push(NoteContext {
                        id: format!("vocal-{}", digest(note)?),
                        tick: note.start,
                        end_tick: note.end,
                        start: a,
                        end: b,
                        lane: None,
                        cymbal: None,
                        tone: Some(note.tone.clone()),
                        lyric: Some(lyric),
                        syllable_kind: Some(match note.lyric {
                            crate::vocals::Syllable::Missing => VocalSyllableKind::Missing,
                            crate::vocals::Syllable::Text { .. } => VocalSyllableKind::Text,
                            crate::vocals::Syllable::Continue => VocalSyllableKind::PitchSlide,
                            crate::vocals::Syllable::ContinueWord => {
                                VocalSyllableKind::ContinueWord
                            }
                        }),
                        lineage: None,
                        linked_children: vec![],
                        omitted_children: 0,
                    });
                }
                vocal_phrases = chart
                    .phrases
                    .iter()
                    .filter(|p| {
                        timing.seconds(p.end) >= context_start
                            && timing.seconds(p.start) <= context_end
                    })
                    .cloned()
                    .collect();
            }
        } else if let Some(track_notes) = project.content.tracks.get(track) {
            for item in track_notes {
                let note = &item.note;
                let a = timing.seconds(note.tick);
                let b = timing.seconds(note.tick.saturating_add(note.sustain));
                if b < context_start || a > context_end || !editor::playable(track, note.lane) {
                    continue;
                }
                ensure!(
                    item.id.len() <= 256,
                    "Context note ID exceeds packet bounds"
                );
                let lineage = project
                    .content
                    .difficulty_links
                    .links
                    .get(track)
                    .map(|link| {
                        let ancestry = link.ancestry.get(&item.id);
                        let parents = ancestry.map(|a| a.parent_ids.as_slice()).unwrap_or(&[]);
                        Lineage {
                            expert_track: link.expert_track.clone(),
                            parent_ids: parents.iter().take(8).cloned().collect(),
                            omitted_parents: parents.len().saturating_sub(8),
                            linked: link.enabled,
                            manual_override: link.overrides.contains_key(&item.id),
                        }
                    });
                let children = child_index.get(&(track.clone(), item.id.clone()));
                let omitted_children = children.map_or(0, |c| c.len().saturating_sub(8));
                let linked_children = children
                    .into_iter()
                    .flat_map(|c| c.values())
                    .take(8)
                    .cloned()
                    .collect();
                notes.push(NoteContext {
                    id: item.id.clone(),
                    tick: note.tick,
                    end_tick: note.tick.saturating_add(note.sustain),
                    start: a,
                    end: b,
                    lane: Some(note.lane),
                    cymbal: Some(project.is_cymbal(track, note)),
                    tone: None,
                    lyric: None,
                    syllable_kind: None,
                    lineage,
                    linked_children,
                    omitted_children,
                });
            }
        }
        notes.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.id.cmp(&b.id)));
        let omitted_notes = notes.len().saturating_sub(options.note_limit);
        notes.truncate(options.note_limit);
        let omitted_vocal_phrases = vocal_phrases.len().saturating_sub(DETAIL_LIMIT);
        vocal_phrases.truncate(DETAIL_LIMIT);
        let mut rule_findings: Vec<_> = rules
            .findings
            .iter()
            .filter(|f| &f.track == track && f.seconds >= context_start && f.seconds <= context_end)
            .cloned()
            .collect();
        let omitted_rule_findings = rule_findings.len().saturating_sub(DETAIL_LIMIT);
        rule_findings.truncate(DETAIL_LIMIT);
        let rule_scope = if track == vocal_review::TRACK {
            "Vocal document passed the shared structural validator; acoustic flags remain proposals."
        } else if generated.contains_key(track) {
            "Existing generated-track rules on current notes; findings require musical judgment."
        } else {
            "Authored/source track: generation-only rule profile not applied."
        };
        let (region_content_sha256, source_stale_reason) = match project
            .region_fingerprint(region_id, track, &timing)
        {
            Ok(fingerprint) => (Some(fingerprint), None),
            Err(error) if track == vocal_review::TRACK => (
                None,
                Some(format!(
                    "Vocal source evidence is stale: {}. Regenerate/rebind checks before approval; inspection, navigation and visibility remain available.",
                    short(&error.to_string(), 256)
                )),
            ),
            Err(error) => return Err(error),
        };
        let approval_allowed = region_content_sha256.is_some();
        sections.push(Section {
            id: id.clone(),
            region_id: region_id.clone(),
            track: track.clone(),
            start: *start,
            end: *end,
            context_start,
            context_end,
            region_content_sha256,
            approval_allowed,
            source_stale_reason,
            hidden: *hidden,
            current_decision: *current_decision,
            stale_decision: current_decision.is_none()
                && project.content.decisions.iter().any(|d| {
                    &d.region_id == region_id
                        && &d.track == track
                        && d.status != DecisionStatus::Pending
                }),
            issues,
            omitted_issues,
            notes,
            omitted_notes,
            rule_findings,
            omitted_rule_findings,
            rule_scope: rule_scope.into(),
            vocal_phrases,
            omitted_vocal_phrases,
        });
    }
    let remaining = total_eligible - end;
    let chart_sha256 = project.chart_hash(store)?;
    let kit_audition = project.kit_audition.as_ref().map(|selected| {
        let path = crate::part_analysis::validated_kit_audition_path(store, project, selected)?;
        Ok::<_, anyhow::Error>(KitAuditionContext {
            selected: selected.clone(),
            audio: AudioAsset {
                instrument: format!("drums:{}", selected.part),
                path,
                sha256: selected.audio_sha256.clone(),
            },
            sample_rate: 44100,
            scope: "Estimated kit audio on the original recording clock. Native preview/timeline use this same source; separation is not musical approval.".into(),
        })
    }).transpose()?;
    let mut packet = Packet {
        version: 1,
        packet_sha256: String::new(),
        project: store.root.canonicalize()?,
        revision: project.revision,
        title: short(&project.title, 200),
        duration_seconds: project.duration,
        source_chart_sha256: project.source_chart_sha256.clone(),
        source_review_chart_current: project.review.input_sha256.get("chart") == Some(&chart_sha256),
        chart_sha256,
        project_state_sha256: digest(project)?,
        review_map_sha256: digest(&project.review)?,
        vocal_document_sha256: project.content.vocals.as_ref().map(digest).transpose()?,
        vocal_review_report_sha256: project.content.vocal_review.as_ref().map(|m| m.report_sha256.clone()),
        vocal_review_stem_sha256: project.content.vocal_review.as_ref().map(|m| m.vocal_stem_sha256.clone()),
        audio,
        keyboard_audio,
        kit_audition,
        timing,
        options,
        total_eligible,
        remaining,
        next_after: if remaining > 0 { sections.last().map(|s| s.id.clone()) } else { None },
        sections,
        musical_acceptance: "Unverified. Review musical timing, instrument identity, consistent five-fret phrasing, physical playability and difficulty relationships. Human charts guide quality; exact copying is not a goal. No approval is inferred from checks or model scores.".into(),
    };
    packet.packet_sha256 = packet.calculated_hash()?;
    packet.validate()?;
    Ok(packet)
}

pub fn apply(store: &Store, packet: &Packet, request: ApplyRequest) -> Result<editor::Applied> {
    packet.validate()?;
    ensure!(
        request.packet_sha256 == packet.packet_sha256
            && request.expected_revision == packet.revision,
        "Apply request does not match the review packet/revision"
    );
    ensure!(
        !request.actor_name.trim().is_empty() && request.actor_name.len() <= 200,
        "AI reviewer name is required"
    );
    let command = editor::Request {
        expected_revision: request.expected_revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: request.actor_name,
        },
        action: request.action,
    };
    store.apply_guarded(command, |current| {
        let rebuilt=from_project(store,current,packet.options.clone())?;
        ensure!(rebuilt.packet_sha256==packet.packet_sha256,"Stale or modified AI packet; recording, chart, review state or project changed. Obtain fresh context before applying.");
        Ok(())
    })
}
