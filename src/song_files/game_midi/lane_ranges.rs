//! Preserve physical marker ownership independently of normalized note tiers.
use super::{LaneScope, NoteAttributes, PhysicalLaneRange, decode, hash, merge};
use crate::{chart::Chart, editor::Project, lane_ranges::Edit, storage as fs};
use anyhow::{Context, Result, ensure};
use midly::{
    MidiMessage, Smf, TrackEventKind,
    num::{u4, u7},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneState {
    pub source_sha256: String,
    pub ranges: Vec<PhysicalLaneRange>,
}

pub(super) fn record_scope(
    record: &PhysicalLaneRange,
    suffix: &str,
    difficulty: Option<usize>,
) -> bool {
    if !crate::lane_ranges::supports(&format!("Expert{suffix}"), record.range.kind)
        || record.attributes.zero_length
        || record.attributes.talkie_pitch.is_some()
    {
        return false;
    }
    match (&record.scope, difficulty) {
        (LaneScope::Dedicated { track }, Some(tier)) => {
            suffix == "ProKeys"
                && record.range.kind == crate::lane_ranges::Kind::Trill
                && *track == format!("{}{suffix}", ["Easy", "Medium", "Hard", "Expert"][tier])
        }
        (LaneScope::Expert, None) => !(41..=50).contains(&record.attributes.velocity),
        (LaneScope::ExpertHard, None) => (41..=50).contains(&record.attributes.velocity),
        _ => false,
    }
}

pub fn lane_state(song: &Path) -> Result<Option<LaneState>> {
    let Some((_, directory)) = super::source(song)? else {
        return Ok(None);
    };
    let bytes = fs::read(directory.join("notes.mid"))?;
    Ok(Some(LaneState {
        source_sha256: hash(&bytes),
        ranges: super::decoded_lane_sources(&bytes)?,
    }))
}

/// A YACE game export carries a hash-bound working MIDI beside its editable
/// chart. Reconstruct the authored physical view from those actual bytes,
/// including explicit removal, while retaining the immutable original.
pub fn read_lane_state(song: &Path) -> Result<Option<LaneState>> {
    let working = crate::song_layout::resolve(song, "chart-engine-working");
    let path = working.join("game-notes.mid");
    if !fs::is_file(&path) {
        return Ok(None);
    }
    let Ok(Some(original)) = lane_state(song) else {
        return Ok(None);
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        version: u32,
        master_sha256: String,
        midi_sha256: String,
    }
    let binding: Binding = serde_json::from_slice(&fs::read(working.join("binding.json"))?)?;
    let bytes = fs::read(path)?;
    ensure!(
        binding.version == 1
            && binding.midi_sha256 == hash(&bytes)
            && binding.master_sha256 == hash(&fs::read(working.join("notes.chart"))?),
        "Working MIDI lane binding changed; source retained"
    );
    let ranges = super::decoded_lane_sources(&bytes)?;
    if ranges == original.ranges {
        return Ok(None);
    }
    Ok(Some(LaneState {
        source_sha256: original.source_sha256,
        ranges,
    }))
}

pub fn edit_lane_ranges(
    project: &mut Project,
    chart: &Chart,
    song: &Path,
    owner: &str,
    scope: &LaneScope,
    edit: &Edit,
) -> Result<()> {
    let original =
        lane_state(song)?.context("MIDI lane editing requires an imported MIDI source")?;
    let mut state = project
        .content
        .midi_lane_ranges
        .clone()
        .unwrap_or_else(|| original.clone());
    ensure!(
        state.source_sha256 == original.source_sha256,
        "MIDI lane source changed; project retained"
    );
    let (suffix, difficulty) = decode::part(owner).context("Unsupported MIDI lane owner")?;
    let track = match scope {
        LaneScope::Expert | LaneScope::ExpertHard => format!("Expert{suffix}"),
        LaneScope::Dedicated { track } => track.clone(),
    };
    ensure!(
        project.content.tracks.contains_key(&track)
            || *scope == LaneScope::ExpertHard
                && project
                    .content
                    .tracks
                    .contains_key(&format!("Hard{suffix}")),
        "Choose an existing instrument track"
    );
    let current: Vec<_> = state
        .ranges
        .iter()
        .filter(|r| r.owner == owner && r.scope == *scope)
        .map(|r| r.range.clone())
        .collect();
    crate::lane_ranges::edited(&track, &current, edit)?;
    let old = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { range } => Some(range),
    };
    let attributes = old
        .and_then(|old| {
            state
                .ranges
                .iter()
                .find(|r| r.owner == owner && r.scope == *scope && r.range == *old)
        })
        .map(|r| r.attributes.clone())
        .unwrap_or(NoteAttributes {
            channel: 0,
            velocity: if *scope == LaneScope::ExpertHard {
                45
            } else {
                100
            },
            off_velocity: 0,
            off_as_note_on: false,
            zero_length: false,
            talkie_pitch: None,
        });
    state
        .ranges
        .retain(|r| !(r.owner == owner && r.scope == *scope && old == Some(&r.range)));
    if let Edit::Put { range, .. } = edit {
        let record = PhysicalLaneRange {
            owner: owner.into(),
            scope: scope.clone(),
            range: range.clone(),
            attributes,
        };
        ensure!(
            record_scope(&record, suffix, difficulty),
            "This scope is not supported by the MIDI owner"
        );
        state.ranges.push(record);
    }
    state
        .ranges
        .sort_by_key(|r| (r.owner.clone(), r.range.start, r.range.kind, r.range.end));
    validate(&state, project, chart)?;
    // Prove the candidate can be encoded before accepting a saved command.
    let (_, directory) = super::source(song)?.context("Missing MIDI source")?;
    rewrite(
        &fs::read(directory.join("notes.mid"))?,
        &original.ranges,
        &state.ranges,
    )?;
    project.content.midi_lane_ranges = Some(state);
    Ok(())
}

pub fn validate(state: &LaneState, project: &Project, chart: &Chart) -> Result<()> {
    ensure!(
        state.source_sha256.len() == 64
            && state.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid MIDI lane source fingerprint"
    );
    ensure!(
        state.ranges.len() <= 16384,
        "Too many physical MIDI lane ranges"
    );
    let timing = crate::sync_track::effective(project, &chart.timing)?;
    for (track, ranges) in super::lane_source_view(&state.ranges)? {
        // A physical Expert+Hard range can precede creation of the Hard note tier.
        crate::lane_ranges::validate(&track, &ranges, &timing, project.duration)?;
    }
    Ok(())
}

pub fn export_lane_ranges(
    song: &Path,
    state: &LaneState,
    bytes: &[u8],
    selected: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let original = lane_state(song)?.context("Missing MIDI lane source")?;
    ensure!(
        original.source_sha256 == state.source_sha256,
        "MIDI lane source changed; export refused"
    );
    let midi = Smf::parse(bytes)?;
    let owners: BTreeSet<_> = midi
        .tracks
        .iter()
        .filter_map(|t| decode::label(t))
        .collect();
    let before: Vec<_> = original
        .ranges
        .into_iter()
        .filter(|r| owners.contains(&r.owner))
        .collect();
    let after: Vec<_> = state
        .ranges
        .iter()
        .filter(|r| {
            selected.contains(&format!("MIDI: {}", r.owner))
                || selected.iter().any(|track| {
                    let track = if let Some(suffix) = track
                        .strip_prefix("Medium")
                        .or_else(|| track.strip_prefix("Easy"))
                    {
                        if suffix == "ProKeys" {
                            track.clone()
                        } else {
                            format!("Expert{suffix}")
                        }
                    } else {
                        track.clone()
                    };
                    crate::lane_ranges::midi_owner(&track)
                        .is_some_and(|(owner, _)| owner == r.owner)
                })
        })
        .cloned()
        .collect();
    rewrite(bytes, &before, &after)
}

pub(super) fn rewrite(
    bytes: &[u8],
    expected: &[PhysicalLaneRange],
    after: &[PhysicalLaneRange],
) -> Result<Vec<u8>> {
    let actual = super::decoded_lane_sources(bytes)?;
    let canonical =
        |records: &[PhysicalLaneRange]| -> Result<BTreeMap<String, Vec<PhysicalLaneRange>>> {
            super::lane_source_view(records)?;
            let mut result = BTreeMap::<String, Vec<PhysicalLaneRange>>::new();
            for record in records {
                result
                    .entry(record.owner.clone())
                    .or_default()
                    .push(record.clone());
            }
            for ranges in result.values_mut() {
                ranges.sort_by_key(|r| (r.range.start, r.range.kind, r.range.end));
            }
            Ok(result)
        };
    let before = canonical(expected)?;
    let after = canonical(after)?;
    ensure!(
        canonical(&actual)? == before,
        "Physical MIDI lane markers changed; export refused"
    );
    let changed: BTreeSet<_> = before
        .keys()
        .chain(after.keys())
        .filter(|owner| before.get(*owner) != after.get(*owner))
        .cloned()
        .collect();
    if changed.is_empty() {
        return Ok(bytes.to_vec());
    }
    let smf = Smf::parse(bytes)?;
    let (header, chunks) = merge::chunks(bytes, smf.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut seen = BTreeSet::new();
    for (track, chunk) in smf.tracks.iter().zip(chunks) {
        let label = decode::label(track);
        let Some(owner) = label.as_ref().filter(|owner| changed.contains(*owner)) else {
            output.extend(chunk);
            continue;
        };
        seen.insert(owner.clone());
        let dedicated = decode::part(owner)
            .context("Unsupported MIDI lane owner")?
            .1
            .is_some();
        let mut edges = BTreeSet::new();
        for r in before.get(owner).into_iter().flatten() {
            if after.get(owner).is_some_and(|ranges| ranges.contains(r)) {
                continue;
            }
            let key = if r.range.kind == crate::lane_ranges::Kind::Trill {
                127
            } else {
                126
            };
            edges.insert((r.range.start, r.attributes.channel, key, true));
            edges.insert((r.range.end, r.attributes.channel, key, false));
        }
        let mut events = Vec::new();
        for (index, (tick, kind)) in merge::absolute(track).into_iter().enumerate() {
            let edge = match kind {
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn { key, vel },
                } if key.as_int() == 127 || !dedicated && key.as_int() == 126 => {
                    Some((tick, channel.as_int(), key.as_int(), vel.as_int() > 0))
                }
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOff { key, .. },
                } if key.as_int() == 127 || !dedicated && key.as_int() == 126 => {
                    Some((tick, channel.as_int(), key.as_int(), false))
                }
                _ => None,
            };
            if edge.is_none_or(|edge| !edges.contains(&edge)) {
                events.push((tick, 1, index, kind));
            }
        }
        for (index, r) in after.get(owner).into_iter().flatten().enumerate() {
            if before.get(owner).is_some_and(|ranges| ranges.contains(r)) {
                continue;
            }
            let key = u7::new(if r.range.kind == crate::lane_ranges::Kind::Trill {
                127
            } else {
                126
            });
            let channel = u4::new(r.attributes.channel);
            events.push((
                r.range.start,
                2,
                index,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key,
                        vel: u7::new(r.attributes.velocity),
                    },
                },
            ));
            let message = if r.attributes.off_as_note_on {
                MidiMessage::NoteOn {
                    key,
                    vel: u7::new(0),
                }
            } else {
                MidiMessage::NoteOff {
                    key,
                    vel: u7::new(r.attributes.off_velocity),
                }
            };
            events.push((
                r.range.end,
                0,
                index,
                TrackEventKind::Midi { channel, message },
            ));
        }
        output.extend(merge::encode(events, smf.header)?);
    }
    let mut count = smf.tracks.len();
    for owner in changed.difference(&seen) {
        let records = after.get(owner).context("Missing physical lane owner")?;
        let mut events = vec![(
            0,
            1,
            0,
            TrackEventKind::Meta(midly::MetaMessage::TrackName(owner.as_bytes())),
        )];
        for (index, r) in records.iter().enumerate() {
            let key = u7::new(if r.range.kind == crate::lane_ranges::Kind::Trill {
                127
            } else {
                126
            });
            let channel = u4::new(r.attributes.channel);
            events.push((
                r.range.start,
                2,
                index,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::NoteOn {
                        key,
                        vel: u7::new(r.attributes.velocity),
                    },
                },
            ));
            let message = if r.attributes.off_as_note_on {
                MidiMessage::NoteOn {
                    key,
                    vel: u7::new(0),
                }
            } else {
                MidiMessage::NoteOff {
                    key,
                    vel: u7::new(r.attributes.off_velocity),
                }
            };
            events.push((
                r.range.end,
                0,
                index,
                TrackEventKind::Midi { channel, message },
            ));
        }
        output.extend(merge::encode(events, smf.header)?);
        count += 1;
    }
    if count != smf.tracks.len() {
        output[10..12].copy_from_slice(&u16::try_from(count)?.to_be_bytes());
        if smf.header.format == midly::Format::SingleTrack {
            output[8..10].copy_from_slice(&1u16.to_be_bytes());
        }
    }
    let expected: Vec<_> = after.values().flatten().cloned().collect();
    ensure!(
        canonical(&super::decoded_lane_sources(&output)?)? == canonical(&expected)?,
        "MIDI lane verification failed; export refused"
    );
    Ok(output)
}
