//! Derive easier tracks from Expert while keeping manual exceptions.
//!
//! Use the autochart tier rules and retain Expert's fret layout. Parent links
//! explain where notes came from; they do not approve the arrangement.
use crate::{
    chart::{Note, Tracks},
    editor::{Content, EditorNote, Project},
    generated_rules::{self, RuleReport},
    mapping::{self, Voice},
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_ID: &str = "expert-master-fixed-tier-v1";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkState {
    pub links: BTreeMap<String, TrackLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAncestry {
    pub parent_ids: Vec<String>,
    pub source_tick: u64,
    pub source_lanes: Vec<u8>,
    pub mapped_lane: u8,
    pub role: String,
    pub rule_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualOverride {
    pub note: Option<EditorNote>,
    pub ancestry: Option<NoteAncestry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackLink {
    pub enabled: bool,
    pub expert_track: String,
    pub rule_id: String,
    pub ancestry: BTreeMap<String, NoteAncestry>,
    pub overrides: BTreeMap<String, ManualOverride>,
    #[serde(default)]
    pub findings: Vec<LinkFinding>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkFinding {
    pub code: String,
    pub note_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverrideView {
    pub note_id: String,
    pub suppressed: bool,
    pub orphaned: bool,
    pub ancestry: Option<NoteAncestry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrackSnapshot {
    pub track: String,
    pub expert_track: String,
    pub linked: bool,
    pub has_link: bool,
    pub can_enable: bool,
    pub generated: bool,
    pub rule_id: String,
    pub override_count: usize,
    pub ancestry: BTreeMap<String, NoteAncestry>,
    pub overrides: Vec<OverrideView>,
    pub findings: Vec<LinkFinding>,
    pub rules: RuleReport,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Snapshot {
    pub tracks: Vec<TrackSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LinkPreview {
    pub track: String,
    pub expert_track: String,
    pub before_count: usize,
    pub after_count: usize,
    pub added_count: usize,
    pub removed_count: usize,
    pub changed_count: usize,
    pub retained_overrides: usize,
    pub rules: RuleReport,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReductionLineage {
    pub expert_track: String,
    pub master_origin: String,
    pub rule_id: String,
    pub master_note_events_sha256: String,
    pub master_note_events: usize,
    pub lower_note_events: usize,
}

/// Derive generated lower tracks from the Expert track that will be exported.
/// Keep authored lower tracks and Expert notes unchanged.
/// Internal parent IDs guide reduction; receipts identify the source by hashes.
pub fn reduce_generated(
    generated: &Tracks,
    authored: &Tracks,
    timing: &TempoMap,
    duration: f64,
) -> Result<(Tracks, BTreeMap<String, ReductionLineage>)> {
    let mut effective = generated.clone();
    for (track, notes) in authored.iter().filter(|(_, notes)| !notes.is_empty()) {
        effective.insert(track.clone(), notes.clone());
    }
    let content = Content {
        track_origins: Default::default(),
        vocals: None,
        vocal_review: None,
        tracks: effective
            .into_iter()
            .map(|(track, notes)| {
                let indexed = notes
                    .into_iter()
                    .enumerate()
                    .map(|(index, note)| EditorNote {
                        id: format!("reduction-input-{track}-{index}"),
                        note,
                    })
                    .collect();
                (track, indexed)
            })
            .collect(),
        difficulty_links: Default::default(),
        decisions: vec![],
        hidden_flags: Default::default(),
        stems: None,
        keyboard_stems: None,
        stem_mix: Default::default(),
    };
    let mut output = generated.clone();
    let mut lineage = BTreeMap::new();
    for track in generated.keys() {
        let Ok((expert, _, _)) = family(track) else {
            continue;
        };
        if authored.get(track).is_some_and(|notes| !notes.is_empty()) {
            continue;
        }
        let master = content
            .tracks
            .get(&expert)
            .context("Generated lower difficulty has no effective Expert master")?;
        let result = derive(&content, track, timing)?;
        gate(track, &result.notes, timing, duration)?;
        let mut master_notes: Vec<_> = master.iter().map(|n| n.note.clone()).collect();
        master_notes.sort();
        let bytes = serde_json::to_vec(&master_notes)?;
        lineage.insert(
            track.clone(),
            ReductionLineage {
                expert_track: expert.clone(),
                master_origin: if authored.get(&expert).is_some_and(|notes| !notes.is_empty()) {
                    "authored_chart"
                } else {
                    "generated_expert"
                }
                .into(),
                rule_id: RULE_ID.into(),
                master_note_events_sha256: format!("{:x}", Sha256::digest(bytes)),
                master_note_events: master_notes.len(),
                lower_note_events: result.notes.len(),
            },
        );
        output.insert(
            track.clone(),
            result.notes.into_iter().map(|n| n.note).collect(),
        );
    }
    Ok((output, lineage))
}

#[derive(Clone, Debug)]
struct Derived {
    notes: Vec<EditorNote>,
    ancestry: BTreeMap<String, NoteAncestry>,
    findings: Vec<LinkFinding>,
}

fn family(track: &str) -> Result<(String, usize, bool)> {
    let (tier, suffix) = ["Easy", "Medium", "Hard"]
        .into_iter()
        .enumerate()
        .find_map(|(tier, prefix)| track.strip_prefix(prefix).map(|suffix| (tier, suffix)))
        .context("Link target must be an Easy, Medium or Hard track")?;
    ensure!(
        matches!(suffix, "Single" | "DoubleBass" | "Drums"),
        "Unsupported linked instrument"
    );
    Ok((format!("Expert{suffix}"), tier, suffix == "Drums"))
}

fn origin(parents: &[&EditorNote], lane: u8, role: &str, rules: &[&str]) -> NoteAncestry {
    NoteAncestry {
        parent_ids: parents
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        source_tick: parents.iter().map(|p| p.note.tick).min().unwrap_or(0),
        source_lanes: parents
            .iter()
            .map(|p| p.note.lane)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        mapped_lane: lane,
        role: role.into(),
        rule_ids: std::iter::once(RULE_ID)
            .chain(rules.iter().copied())
            .map(str::to_owned)
            .collect(),
    }
}

fn push(out: &mut Derived, track: &str, note: Note, origin: NoteAncestry) {
    // Keep tick, lane and chart hash out of the child ID.
    // Moving its Expert parent must preserve the child's manual override.
    let bytes = serde_json::to_vec(&(RULE_ID, track, &origin.parent_ids, &origin.role))
        .expect("serializable ancestry");
    let id = format!("derived-{:x}", Sha256::digest(bytes));
    out.ancestry.insert(id.clone(), origin);
    out.notes.push(EditorNote { id, note });
}

fn derive(content: &Content, track: &str, timing: &TempoMap) -> Result<Derived> {
    let (expert, tier, drums) = family(track)?;
    let source = content
        .tracks
        .get(&expert)
        .context("This instrument has no Expert master")?;
    let controls: BTreeMap<_, _> = source
        .iter()
        .map(|n| ((n.note.tick, n.note.lane), n))
        .collect();
    let mut out = Derived {
        notes: vec![],
        ancestry: BTreeMap::new(),
        findings: vec![],
    };
    if drums {
        let has_pro_cymbals = source.iter().any(|n| (66..=68).contains(&n.note.lane));
        let mut slots = BTreeSet::new();
        let mut groups: Vec<Vec<(&EditorNote, Voice)>> = vec![];
        let mut sorted: Vec<_> = source.iter().filter(|n| n.note.lane <= 4).collect();
        sorted.sort_by_key(|n| (n.note.tick, n.note.lane));
        for item in sorted {
            let lane = item.note.lane;
            let cymbal = controls.contains_key(&(item.note.tick, lane + 64));
            let voice = match (lane, cymbal) {
                (0, _) => Voice::Kick,
                (1, _) => Voice::Snare,
                (2, true) => Voice::HiHat,
                (3, true) => Voice::Ride,
                (4, true) => Voice::Crash,
                // A standard four-pad chart has no tom/cymbal distinction.
                // Retain its yellow timekeeping lane without inventing a Pro
                // cymbal marker; explicit Pro charts can distinguish high toms.
                (2, false) if !has_pro_cymbals => Voice::HiHat,
                (2, false) => Voice::HighTom,
                (3, false) => Voice::MidTom,
                _ => Voice::LowTom,
            };
            // Explicit ghosts are reliable source flags; absent MIDI velocity
            // is never fabricated. OCTAVE's lower-tier policy drops ghosts.
            if controls.contains_key(&(item.note.tick, lane + 39))
                || !mapping::drum_tier_allows(voice, tier)
            {
                continue;
            }
            if let Some((layer, grid)) = mapping::drum_layer_grid(voice, tier, timing.resolution) {
                let slot = (item.note.tick as f64 / grid as f64).round() as u64;
                if !slots.insert((layer, slot)) {
                    continue;
                }
            }
            if groups.last().is_none_or(|g| {
                timing.seconds(item.note.tick) - timing.seconds(g[0].0.note.tick) > 0.020 + 1e-9
            }) {
                groups.push(vec![]);
            }
            groups.last_mut().unwrap().push((item, voice));
        }
        for group in groups {
            let snare = group.iter().any(|(_, voice)| *voice == Voice::Snare);
            let mut hands: Vec<_> = group.iter().filter(|(n, _)| n.note.lane != 0).collect();
            hands.sort_by_key(|(n, voice)| {
                (
                    if *voice == Voice::Snare {
                        0
                    } else if !voice.cymbal() {
                        1
                    } else {
                        2
                    },
                    n.note.lane,
                    n.note.tick,
                )
            });
            let kept = group
                .iter()
                .filter(|(n, _)| n.note.lane == 0 && !(tier < 2 && snare))
                .chain(hands.into_iter().take(2));
            for (item, _) in kept {
                let note = Note {
                    tick: item.note.tick,
                    lane: item.note.lane,
                    sustain: 0,
                };
                push(
                    &mut out,
                    track,
                    note.clone(),
                    origin(
                        &[item],
                        note.lane,
                        "gem",
                        &[
                            "drum_layer_slots",
                            "two_hands",
                            "no_drum_sustain",
                            "drop_explicit_ghosts",
                            "omit_expert_plus_kick",
                        ],
                    ),
                );
                if !has_pro_cymbals && note.lane == 2 && tier < 2 {
                    out.findings.push(LinkFinding { code: "standard_yellow_role_review".into(), note_id: out.notes.last().map(|n| n.id.clone()), reason: "Standard drum source does not distinguish yellow tom from hi-hat. Yellow was retained as a timekeeping lane without adding a Pro cymbal marker; inspect its musical role.".into() });
                }
                for (offset, role) in [(64, "cymbal"), (33, "accent")] {
                    if controls.contains_key(&(note.tick, note.lane + offset))
                        && (1..=4).contains(&note.lane)
                        && (offset != 64 || note.lane >= 2)
                    {
                        push(
                            &mut out,
                            track,
                            Note {
                                tick: note.tick,
                                lane: note.lane + offset,
                                sustain: 0,
                            },
                            origin(
                                &[item],
                                note.lane + offset,
                                role,
                                &["preserve_source_articulation"],
                            ),
                        );
                    }
                }
            }
        }
    } else {
        let policy = mapping::reduction_policy(tier, expert.ends_with("DoubleBass"));
        let mut groups: BTreeMap<u64, Vec<&EditorNote>> = BTreeMap::new();
        for item in source
            .iter()
            .filter(|n| n.note.lane <= 4 || n.note.lane == 7)
        {
            item.note
                .tick
                .checked_add(item.note.sustain)
                .context("Expert sustain overflow")?;
            groups.entry(item.note.tick).or_default().push(item);
        }
        let mut kept = vec![];
        let mut last = None;
        let mut last_block = None;
        let block_ticks = (timing.resolution as u64).saturating_mul(4).max(1);
        for (tick, group) in groups {
            // Reset density budgeting every four quarter notes. Editing one
            // phrase cannot phase-shift a greedy reducer across the whole song.
            let block = tick / block_ticks;
            if last_block != Some(block) {
                last = None;
            }
            last_block = Some(block);
            if last.is_some_and(|prev| {
                timing.seconds(tick) - timing.seconds(prev) < policy.min_gap_seconds
            }) {
                continue;
            }
            let mut lanes: BTreeMap<u8, Vec<&EditorNote>> = BTreeMap::new();
            for item in group {
                let lane = if item.note.lane == 7 {
                    7
                } else {
                    policy.lanes[item.note.lane as usize]
                };
                lanes.entry(lane).or_default().push(item);
            }
            kept.push((
                tick,
                lanes.into_iter().take(policy.max_chord).collect::<Vec<_>>(),
            ));
            last = Some(tick);
        }
        for (index, (tick, lanes)) in kept.iter().enumerate() {
            let next = kept.get(index + 1).map(|g| g.0);
            let mut parents = vec![];
            for (lane, contributing) in lanes {
                parents.extend(contributing.iter().copied());
                let length = contributing
                    .iter()
                    .map(|n| n.note.sustain)
                    .max()
                    .unwrap_or(0);
                let length = next.map_or(length, |next| {
                    length.min(next.saturating_sub(*tick).saturating_sub(
                        (timing.resolution / policy.sustain_gap_divisor).max(1) as u64,
                    ))
                });
                let sustain = if timing.seconds(tick.saturating_add(length)) - timing.seconds(*tick)
                    >= policy.min_sustain_seconds
                {
                    length
                } else {
                    0
                };
                push(
                    &mut out,
                    track,
                    Note {
                        tick: *tick,
                        lane: *lane,
                        sustain,
                    },
                    origin(
                        contributing,
                        *lane,
                        "gem",
                        &[
                            "fixed_tier_lane_compression",
                            "tier_attack_spacing",
                            "tier_chord_limit",
                            "sustain_gap_ppq8",
                            "sustain_min_200ms",
                            "local_four_quarter_density_budget",
                        ],
                    ),
                );
            }
            for (lane, role) in [(5, "forced"), (6, "tap")] {
                if controls.contains_key(&(*tick, lane)) {
                    push(
                        &mut out,
                        track,
                        Note {
                            tick: *tick,
                            lane,
                            sustain: 0,
                        },
                        origin(&parents, lane, role, &["preserve_source_control"]),
                    );
                    if lane == 5 {
                        out.findings.push(LinkFinding { code: "forced_context_review".into(), note_id: out.notes.last().map(|n| n.id.clone()), reason: "Expert forced-toggle marker retained. Automatic HOPO context may change after reduction; inspect the resulting lower-tier strum/HOPO behavior.".into() });
                    }
                }
            }
        }
    }
    out.notes
        .sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn overlaps(a: &NoteAncestry, b: &NoteAncestry) -> bool {
    a.parent_ids.iter().any(|id| b.parent_ids.contains(id))
}

fn marker_has_target(note: &Note, notes: &[EditorNote], drums: bool) -> bool {
    if drums {
        let lane = match note.lane {
            66..=68 => Some(note.lane - 64),
            34..=37 => Some(note.lane - 33),
            40..=43 => Some(note.lane - 39),
            _ => None,
        };
        lane.is_none_or(|lane| {
            notes
                .iter()
                .any(|n| n.note.tick == note.tick && n.note.lane == lane)
        })
    } else {
        !(5..=6).contains(&note.lane)
            || notes
                .iter()
                .any(|n| n.note.tick == note.tick && (n.note.lane <= 4 || n.note.lane == 7))
    }
}

fn apply_overrides(mut base: Derived, link: &TrackLink, drums: bool) -> Derived {
    for (id, manual) in &link.overrides {
        let mut blocked = BTreeSet::new();
        for item in &base.notes {
            let anchored = manual
                .ancestry
                .as_ref()
                .zip(base.ancestry.get(&item.id))
                .is_some_and(|(a, b)| overlaps(a, b) && (a.role == b.role || a.role == "gem"));
            let collision = manual.note.as_ref().is_some_and(|m| {
                m.note.tick == item.note.tick
                    && (m.note.lane == item.note.lane
                        || (!drums
                            && (m.note.lane == 7) != (item.note.lane == 7)
                            && (m.note.lane <= 4 || m.note.lane == 7)
                            && (item.note.lane <= 4 || item.note.lane == 7)))
            });
            if item.id == *id || anchored || collision {
                blocked.insert(item.id.clone());
                if collision && !anchored && item.id != *id {
                    base.findings.push(LinkFinding { code: "manual_collision".into(), note_id: Some(id.clone()), reason: "A newly derived note occupies this manual note's position or conflicts with its open/colored attack. The manual edit was retained and the conflicting derived gem omitted.".into() });
                }
            }
        }
        base.notes.retain(|n| !blocked.contains(&n.id));
        base.ancestry.retain(|id, _| !blocked.contains(id));
    }
    for (id, manual) in &link.overrides {
        if let Some(note) = &manual.note {
            base.notes.push(note.clone());
            if let Some(ancestry) = &manual.ancestry {
                base.ancestry.insert(id.clone(), ancestry.clone());
            }
        }
    }
    // Generated controls may lose their target to a manual suppression. Do not
    // leave orphan markers; explicitly edited markers remain subject to gate.
    let remove: BTreeSet<_> = base
        .notes
        .iter()
        .filter(|n| {
            !link.overrides.contains_key(&n.id) && !marker_has_target(&n.note, &base.notes, drums)
        })
        .map(|n| n.id.clone())
        .collect();
    base.notes.retain(|n| !remove.contains(&n.id));
    base.ancestry.retain(|id, _| !remove.contains(id));
    base.notes
        .sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
    base
}

fn gate(track: &str, notes: &[EditorNote], timing: &TempoMap, duration: f64) -> Result<()> {
    let report = generated_rules::assess(
        &Tracks::from([(track.into(), notes.iter().map(|n| n.note.clone()).collect())]),
        timing,
        duration,
    );
    ensure!(
        !report.has_errors(),
        "Linked difficulty violates structural rules: {}",
        serde_json::to_string(&report)?
    );
    Ok(())
}

fn refresh(content: &mut Content, track: &str, timing: &TempoMap, duration: f64) -> Result<()> {
    let link = content
        .difficulty_links
        .links
        .get(track)
        .context("Track has no difficulty link")?;
    let result = apply_overrides(
        derive(content, track, timing)?,
        link,
        track.ends_with("Drums"),
    );
    gate(track, &result.notes, timing, duration)?;
    let link = content.difficulty_links.links.get_mut(track).unwrap();
    link.ancestry = result.ancestry;
    link.findings = result.findings;
    content.tracks.insert(track.into(), result.notes);
    Ok(())
}

/// Explicit enable replaces that lower track only. Disable preserves notes and
/// exceptions; re-enable rebases them on the current Expert master.
pub fn set_enabled(
    content: &mut Content,
    track: &str,
    enabled: bool,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    let (expert, _, drums) = family(track)?;
    ensure!(
        content.tracks.contains_key(&expert),
        "This instrument has no Expert master"
    );
    if !enabled {
        let link = content
            .difficulty_links
            .links
            .get_mut(track)
            .context("Track has no difficulty link")?;
        link.enabled = false;
        return Ok(());
    }
    if !content.tracks.contains_key(track) {
        let result = derive(content, track, timing)?;
        ensure!(
            result
                .notes
                .iter()
                .any(|note| is_gem(drums, note.note.lane)),
            "Expert produces no playable notes for this lower difficulty"
        );
        gate(track, &result.notes, timing, duration)?;
        content.difficulty_links.links.insert(
            track.into(),
            TrackLink {
                enabled: true,
                expert_track: expert,
                rule_id: RULE_ID.into(),
                ancestry: result.ancestry,
                overrides: BTreeMap::new(),
                findings: result.findings,
            },
        );
        content.tracks.insert(track.into(), result.notes);
        return Ok(());
    }
    content
        .difficulty_links
        .links
        .entry(track.into())
        .or_insert(TrackLink {
            enabled: true,
            expert_track: expert,
            rule_id: RULE_ID.into(),
            ancestry: BTreeMap::new(),
            overrides: BTreeMap::new(),
            findings: vec![],
        })
        .enabled = true;
    refresh(content, track, timing, duration)
}

/// New generated projects opt into consistent reductions. Authored lower tracks
/// and old project files remain independent unless explicitly enabled.
pub fn enable_generated(
    content: &mut Content,
    generated: Option<&BTreeSet<String>>,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    for track in generated.into_iter().flatten() {
        if family(track).is_ok_and(|(expert, _, _)| content.tracks.contains_key(&expert)) {
            ensure!(
                content.tracks.contains_key(track),
                "Generated provenance names a missing lower track"
            );
            set_enabled(content, track, true, timing, duration)?;
        }
    }
    Ok(())
}

/// Capture only a linked easier track's local human/AI note edits. This never
/// modifies any source or sibling track, and remains active while paused.
fn record_overrides(content: &mut Content, before: &Content, track: &str) {
    let old: BTreeMap<_, _> = before.tracks[track]
        .iter()
        .map(|n| (n.id.clone(), n))
        .collect();
    let new: BTreeMap<_, _> = content.tracks[track]
        .iter()
        .map(|n| (n.id.clone(), n.clone()))
        .collect();
    let ids: BTreeSet<_> = old.keys().chain(new.keys()).cloned().collect();
    let changed: BTreeSet<_> = ids
        .iter()
        .filter(|id| old.get(*id).map(|n| &n.note) != new.get(*id).map(|n| &n.note))
        .cloned()
        .collect();
    let ticks: BTreeSet<_> = changed
        .iter()
        .flat_map(|id| {
            old.get(id)
                .map(|n| n.note.tick)
                .into_iter()
                .chain(new.get(id).map(|n| n.note.tick))
        })
        .collect();
    let drums = track.ends_with("Drums");
    let touched: BTreeSet<_> = ids
        .into_iter()
        .filter(|id| {
            changed.contains(id)
                || old
                    .get(id)
                    .is_some_and(|n| ticks.contains(&n.note.tick) && !is_gem(drums, n.note.lane))
                || new
                    .get(id)
                    .is_some_and(|n| ticks.contains(&n.note.tick) && !is_gem(drums, n.note.lane))
        })
        .collect();
    let link = content.difficulty_links.links.get_mut(track).unwrap();
    for id in touched {
        let ancestry = link
            .ancestry
            .get(&id)
            .cloned()
            .or_else(|| link.overrides.get(&id).and_then(|o| o.ancestry.clone()));
        let note = new.get(&id).cloned();
        if note.is_none() && ancestry.is_none() {
            link.overrides.remove(&id);
        } else {
            link.overrides
                .insert(id.clone(), ManualOverride { note, ancestry });
        }
        if !new.contains_key(&id) {
            link.ancestry.remove(&id);
        }
    }
}

fn is_gem(drums: bool, lane: u8) -> bool {
    lane <= 4 || if drums { lane == 32 } else { lane == 7 }
}

pub fn after_note_edit(
    content: &mut Content,
    before: &Content,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    let links: Vec<_> = content.difficulty_links.links.keys().cloned().collect();
    for track in &links {
        if before.tracks.get(track) != content.tracks.get(track) {
            record_overrides(content, before, track);
        }
    }
    for track in &links {
        let link = &content.difficulty_links.links[track];
        if link.enabled
            && (before.tracks.get(&link.expert_track) != content.tracks.get(&link.expert_track)
                || before.tracks.get(track) != content.tracks.get(track))
        {
            refresh(content, track, timing, duration)?;
        }
    }
    Ok(())
}

pub fn clear_overrides(
    content: &mut Content,
    track: &str,
    note_id: Option<&str>,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    let link = content
        .difficulty_links
        .links
        .get_mut(track)
        .context("Track has no difficulty link")?;
    ensure!(
        link.enabled,
        "Resume this difficulty link before resetting its overrides"
    );
    if let Some(id) = note_id {
        let target = link
            .overrides
            .get(id)
            .cloned()
            .context("This note has no local override")?;
        let drums = track.ends_with("Drums");
        let is_control = |o: &ManualOverride| {
            o.ancestry.as_ref().is_some_and(|a| a.role != "gem")
                || o.note.as_ref().is_some_and(|n| !is_gem(drums, n.note.lane))
        };
        let companions: Vec<_> = link
            .overrides
            .iter()
            .filter(|(key, o)| {
                if key.as_str() == id {
                    return true;
                }
                if !is_control(o) {
                    return false;
                }
                let ancestral = target
                    .ancestry
                    .as_ref()
                    .filter(|a| a.role == "gem")
                    .zip(o.ancestry.as_ref())
                    .is_some_and(|(a, b)| overlaps(a, b));
                let positioned = target
                    .note
                    .as_ref()
                    .filter(|n| is_gem(drums, n.note.lane))
                    .zip(o.note.as_ref())
                    .is_some_and(|(gem, control)| {
                        gem.note.tick == control.note.tick
                            && if drums {
                                [gem.note.lane + 64, gem.note.lane + 33, gem.note.lane + 39]
                                    .contains(&control.note.lane)
                            } else {
                                (5..=6).contains(&control.note.lane)
                            }
                    });
                ancestral || positioned
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in companions {
            link.overrides.remove(&id);
        }
    } else {
        link.overrides.clear();
    }
    refresh(content, track, timing, duration)
}

pub fn preview(project: &Project, track: &str, timing: &TempoMap) -> Result<LinkPreview> {
    let (expert_track, _, drums) = family(track)?;
    let base = derive(&project.content, track, timing)?;
    let result = if let Some(link) = project.content.difficulty_links.links.get(track) {
        apply_overrides(base, link, drums)
    } else {
        base
    };
    if !project.content.tracks.contains_key(track) {
        ensure!(
            result
                .notes
                .iter()
                .any(|note| is_gem(drums, note.note.lane)),
            "Expert produces no playable notes for this lower difficulty"
        );
    }
    let before: BTreeMap<_, _> = project
        .content
        .tracks
        .get(track)
        .into_iter()
        .flatten()
        .map(|n| (&n.id, &n.note))
        .collect();
    let after: BTreeMap<_, _> = result.notes.iter().map(|n| (&n.id, &n.note)).collect();
    // Linking changes note IDs. Count otherwise identical notes as unchanged
    // so the confirmation shows musical edits rather than ID replacements.
    let unmatched_before: BTreeSet<_> = before
        .iter()
        .filter(|(id, _)| !after.contains_key(*id))
        .map(|(_, note)| *note)
        .collect();
    let unmatched_after: BTreeSet<_> = after
        .iter()
        .filter(|(id, _)| !before.contains_key(*id))
        .map(|(_, note)| *note)
        .collect();
    let equal_notes = unmatched_before.intersection(&unmatched_after).count();
    Ok(LinkPreview {
        track: track.into(),
        expert_track,
        before_count: before.len(),
        after_count: after.len(),
        added_count: after.keys().filter(|id| !before.contains_key(*id)).count() - equal_notes,
        removed_count: before.keys().filter(|id| !after.contains_key(*id)).count() - equal_notes,
        changed_count: after.iter().fold(0, |count, (id, note)| {
            count + usize::from(before.get(id).is_some_and(|old| *old != *note))
        }),
        retained_overrides: project
            .content
            .difficulty_links
            .links
            .get(track)
            .map_or(0, |l| l.overrides.len()),
        rules: generated_rules::assess(
            &Tracks::from([(
                track.into(),
                result.notes.into_iter().map(|n| n.note).collect(),
            )]),
            timing,
            project.duration,
        ),
    })
}

pub fn snapshot(project: &Project, timing: &TempoMap) -> Snapshot {
    let mut tracks = vec![];
    for (track, notes) in &project.content.tracks {
        let Ok((expert_track, _, _)) = family(track) else {
            continue;
        };
        let Some(expert) = project.content.tracks.get(&expert_track) else {
            continue;
        };
        let ids: BTreeSet<_> = expert.iter().map(|n| n.id.as_str()).collect();
        let link = project.content.difficulty_links.links.get(track);
        let mut findings = link.map_or_else(Vec::new, |l| l.findings.clone());
        let overrides: Vec<_> = link.into_iter().flat_map(|l| &l.overrides).map(|(id,o)| {
            let orphaned = o.ancestry.as_ref().is_some_and(|a| a.parent_ids.iter().any(|id| !ids.contains(id.as_str())));
            if orphaned { findings.push(LinkFinding { code: "orphan_override".into(), note_id: Some(id.clone()), reason: "Expert parent was removed. This local note or suppression is retained until explicitly reset; inspect its musical relationship.".into() }); }
            OverrideView { note_id: id.clone(), suppressed: o.note.is_none(), orphaned, ancestry: o.ancestry.clone() }
        }).collect();
        tracks.push(TrackSnapshot {
            track: track.clone(),
            expert_track,
            linked: link.is_some_and(|l| l.enabled),
            has_link: link.is_some(),
            can_enable: true,
            generated: project
                .review
                .generated_tracks
                .as_ref()
                .is_some_and(|g| g.contains(track)),
            rule_id: RULE_ID.into(),
            override_count: overrides.len(),
            ancestry: link.map_or_else(BTreeMap::new, |l| l.ancestry.clone()),
            overrides,
            findings,
            rules: generated_rules::assess(
                &if link.is_some() {
                    Tracks::from([(
                        track.clone(),
                        notes.iter().map(|n| n.note.clone()).collect(),
                    )])
                } else {
                    // The generated-data profile must not condemn untouched
                    // authored control lanes. Enabling previews the actual
                    // proposed reduction and assesses that generated result.
                    Tracks::new()
                },
                timing,
                project.duration,
            ),
        });
    }
    Snapshot { tracks }
}

pub fn validate(content: &Content, timing: &TempoMap, duration: f64) -> Result<()> {
    for (track, link) in &content.difficulty_links.links {
        let (expert, _, _) = family(track)?;
        ensure!(
            expert == link.expert_track
                && link.rule_id == RULE_ID
                && content.tracks.contains_key(track)
                && content.tracks.contains_key(&expert),
            "Invalid difficulty link scope or rule version"
        );
        let current: BTreeSet<_> = content.tracks[track]
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        for (id, a) in &link.ancestry {
            ensure!(
                current.contains(id.as_str())
                    && !a.parent_ids.is_empty()
                    && a.parent_ids.iter().all(|p| !p.is_empty())
                    && a.parent_ids.iter().collect::<BTreeSet<_>>().len() == a.parent_ids.len()
                    && !a.rule_ids.is_empty(),
                "Invalid note ancestry"
            );
        }
        for (id, o) in &link.overrides {
            ensure!(
                !id.is_empty()
                    && o.note.as_ref().is_none_or(|n| n.id == *id)
                    && (o.note.is_some() || o.ancestry.is_some()),
                "Invalid difficulty override"
            );
        }
        if link.enabled {
            gate(track, &content.tracks[track], timing, duration)?;
        }
    }
    Ok(())
}
