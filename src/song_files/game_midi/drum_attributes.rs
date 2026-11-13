//! Source-bound physical attributes for authored drum activation and roll ranges.
use super::{NoteAttributes, decode, hash, merge, source};
use crate::{
    chart::Chart,
    drum_ranges::{self, Edit, Kind, Range},
    storage as fs,
};
use anyhow::{Context, Result, ensure};
use midly::{
    MidiMessage as Note, Smf, TrackEventKind as Event,
    num::{u4, u7},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[cfg(test)]
#[path = "drum_attributes_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub range: Range,
    pub attributes: Vec<NoteAttributes>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub source_sha256: String,
    pub groups: Vec<Group>,
}
fn keys(kind: Kind) -> &'static [u8] {
    match kind {
        Kind::FillActivation => &[120, 121, 122, 123, 124],
        Kind::SingleRoll => &[126],
        Kind::DoubleRoll => &[127],
    }
}
fn order(r: &Range) -> (Kind, u64, u64) {
    (r.kind, r.start, r.end)
}
impl State {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source_sha256.len() == 64
                && self.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid drum attribute source binding"
        );
        ensure!(
            self.groups.len() <= 8192,
            "Too many drum marker attribute groups"
        );
        ensure!(
            self.groups
                .windows(2)
                .all(|p| order(&p[0].range) < order(&p[1].range)),
            "Unordered drum marker attribute groups"
        );
        for g in &self.groups {
            ensure!(
                g.range.start < g.range.end
                    && g.range.end <= u64::from(u32::MAX)
                    && g.attributes.len() == keys(g.range.kind).len(),
                "Invalid physical drum marker group"
            );
            for a in &g.attributes {
                a.validate()?;
                ensure!(
                    !a.zero_length && a.talkie_pitch.is_none(),
                    "Invalid physical drum marker attributes"
                );
            }
        }
        Ok(())
    }
}
fn groups(bytes: &[u8]) -> Result<Vec<Group>> {
    merge::validate_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    let tracks: Vec<_> = midi
        .tracks
        .iter()
        .filter(|t| decode::label(t).as_deref() == Some("PART DRUMS"))
        .collect();
    ensure!(tracks.len() <= 1, "Ambiguous physical drum owner");
    let Some(track) = tracks.first() else {
        return Ok(vec![]);
    };
    ensure!(
        track
            .iter()
            .filter(|e| matches!(e.kind, Event::Meta(midly::MetaMessage::TrackName(_))))
            .count()
            == 1
            && track
                .iter()
                .filter(|e| matches!(e.kind, Event::Meta(midly::MetaMessage::EndOfTrack)))
                .count()
                == 1
            && track
                .last()
                .is_some_and(|e| matches!(e.kind, Event::Meta(midly::MetaMessage::EndOfTrack))),
        "Ambiguous or incomplete physical drum owner"
    );
    let spans = decode::spans(track, |k| {
        (120..=124).contains(&k) || matches!(k, 126 | 127)
    })?;
    ensure!(spans.len() <= 40960, "Too many physical drum marker edges");
    let mut grouped = BTreeMap::<(Kind, u64, u64), Vec<(u8, NoteAttributes)>>::new();
    for s in spans {
        let kind = match s.key {
            126 => Kind::SingleRoll,
            127 => Kind::DoubleRoll,
            _ => Kind::FillActivation,
        };
        grouped.entry((kind, s.start, s.end)).or_default().push((
            s.key,
            NoteAttributes {
                channel: s.channel,
                velocity: s.velocity,
                off_velocity: s.off_velocity,
                off_as_note_on: s.off_as_note_on,
                zero_length: false,
                talkie_pitch: None,
            },
        ));
    }
    let mut result = vec![];
    for ((kind, start, end), mut attrs) in grouped {
        attrs.sort_by_key(|(key, _)| *key);
        if start >= end || attrs.iter().map(|(k, _)| *k).collect::<Vec<_>>() != keys(kind) {
            continue;
        }
        result.push(Group {
            range: Range { start, end, kind },
            attributes: attrs.into_iter().map(|(_, a)| a).collect(),
        });
    }
    // Overlapping/multi-channel source groups cannot be mapped to a single
    // authoring range. Keep their original bytes outside the editable metadata.
    let mut ambiguous = BTreeSet::new();
    let mut previous = None::<(Kind, u64, usize)>;
    for (i, g) in result.iter().enumerate() {
        if let Some((kind, end, index)) = previous
            && kind == g.range.kind
            && g.range.start < end
        {
            ambiguous.insert(index);
            ambiguous.insert(i);
            if g.range.end > end {
                previous = Some((kind, g.range.end, i));
            }
        } else {
            previous = Some((g.range.kind, g.range.end, i));
        }
    }
    Ok(result
        .into_iter()
        .enumerate()
        .filter_map(|(i, g)| (!ambiguous.contains(&i)).then_some(g))
        .collect())
}
fn original(song: &Path) -> Result<State> {
    let (_, directory) =
        source(song)?.context("Drum attributes require their retained MIDI source")?;
    let bytes = fs::read(directory.join("notes.mid"))?;
    let state = State {
        source_sha256: hash(&bytes),
        groups: groups(&bytes)?,
    };
    state.validate()?;
    Ok(state)
}
pub fn read(song: &Path) -> Result<Option<State>> {
    let working = crate::song_layout::resolve(song, "chart-engine-working");
    let file = working.join("game-notes.mid");
    if !fs::is_file(&file) || !super::exists(song) {
        return Ok(None);
    }
    // Receipt errors remain fatal; unsupported marker layouts stay opaque.
    source(song)?.context("Missing retained drum MIDI source")?;
    let original = match original(song) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        version: u32,
        master_sha256: String,
        midi_sha256: String,
    }
    let binding: Binding = serde_json::from_slice(&fs::read(working.join("binding.json"))?)?;
    let bytes = fs::read(file)?;
    ensure!(
        binding.version == 1
            && binding.midi_sha256 == hash(&bytes)
            && binding.master_sha256 == hash(&fs::read(working.join("notes.chart"))?),
        "Working drum marker attribute binding changed"
    );
    let current = groups(&bytes)?;
    if current == original.groups {
        return Ok(None);
    }
    let state = State {
        source_sha256: original.source_sha256,
        groups: current,
    };
    state.validate()?;
    Ok(Some(state))
}
fn defaults(range: Range) -> Group {
    let attributes = keys(range.kind)
        .iter()
        .map(|_| NoteAttributes {
            channel: 0,
            velocity: 100,
            off_velocity: 0,
            off_as_note_on: false,
            zero_length: false,
            talkie_pitch: None,
        })
        .collect();
    Group { range, attributes }
}
pub fn edited(
    song: &Path,
    state: Option<&State>,
    chart: &Chart,
    edit: &Edit,
    after: &[Range],
) -> Result<State> {
    let original = original(song)?;
    let state = state.unwrap_or(&original);
    state.validate()?;
    ensure!(
        state.source_sha256 == original.source_sha256,
        "Drum marker attributes belong to another source"
    );
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { range, .. } => Some(range),
        Edit::Paste { .. } => None,
    };
    let source_ranges = drum_ranges::TRACKS
        .into_iter()
        .map(|t| drum_ranges::imported(chart, t))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(previous) = previous {
        ensure!(
            state.groups.iter().any(|g| g.range == *previous) || !source_ranges.contains(previous),
            "Original drum marker attributes are ambiguous; retain the source read-only"
        );
    }
    let mut output = vec![];
    for range in after {
        if let Some(group) = state.groups.iter().find(|g| g.range == *range) {
            output.push(group.clone());
            continue;
        }
        if let Edit::Put {
            previous: Some(previous),
            range: target,
            ..
        } = edit
            && target == range
            && previous.kind == range.kind
            && let Some(group) = state.groups.iter().find(|g| g.range == *previous)
        {
            output.push(Group {
                range: range.clone(),
                attributes: group.attributes.clone(),
            });
            continue;
        }
        if !source_ranges.contains(range) {
            output.push(defaults(range.clone()));
        }
    }
    output.sort_by_key(|g| order(&g.range));
    let result = State {
        source_sha256: original.source_sha256,
        groups: output,
    };
    result.validate()?;
    Ok(result)
}
fn edge(tick: u64, kind: Event<'_>) -> Option<(u64, u8, u8, bool)> {
    match kind {
        Event::Midi {
            channel,
            message: Note::NoteOn { key, vel },
        } => Some((tick, channel.as_int(), key.as_int(), vel.as_int() > 0)),
        Event::Midi {
            channel,
            message: Note::NoteOff { key, .. },
        } => Some((tick, channel.as_int(), key.as_int(), false)),
        _ => None,
    }
}
pub fn export(
    song: &Path,
    state: Option<&State>,
    bytes: &[u8],
    selected: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let Some(state) = state else {
        return Ok(bytes.to_vec());
    };
    state.validate()?;
    ensure!(
        state.source_sha256 == original(song)?.source_sha256,
        "Drum marker attribute source changed"
    );
    if !selected.contains("MIDI: PART DRUMS")
        && !drum_ranges::TRACKS.iter().any(|t| selected.contains(*t))
    {
        return Ok(bytes.to_vec());
    }
    let actual = groups(bytes)?;
    let mut changed = vec![];
    for wanted in &state.groups {
        let before = actual
            .iter()
            .find(|g| g.range == wanted.range)
            .context("Drum marker timeline changed; export refused")?;
        if before.attributes != wanted.attributes {
            changed.push((before, wanted));
        }
    }
    if changed.is_empty() {
        return Ok(bytes.to_vec());
    }
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = merge::chunks(bytes, midi.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    for (track, chunk) in midi.tracks.iter().zip(chunks) {
        if decode::label(track).as_deref() != Some("PART DRUMS") {
            output.extend(chunk);
            continue;
        }
        let mut removed = BTreeSet::new();
        for (before, _) in &changed {
            for (&key, a) in keys(before.range.kind).iter().zip(&before.attributes) {
                removed.insert((before.range.start, a.channel, key, true));
                removed.insert((before.range.end, a.channel, key, false));
            }
        }
        let mut events = vec![];
        for (i, (tick, kind)) in merge::absolute(track).into_iter().enumerate() {
            if edge(tick, kind).is_none_or(|e| !removed.contains(&e)) {
                events.push((tick, 1, i, kind));
            }
        }
        let mut serial = track.len();
        for (_, after) in &changed {
            for (&key, a) in keys(after.range.kind).iter().zip(&after.attributes) {
                let key = u7::new(key);
                let channel = u4::new(a.channel);
                events.push((
                    after.range.start,
                    1,
                    serial,
                    Event::Midi {
                        channel,
                        message: Note::NoteOn {
                            key,
                            vel: u7::new(a.velocity),
                        },
                    },
                ));
                serial += 1;
                let message = if a.off_as_note_on {
                    Note::NoteOn {
                        key,
                        vel: u7::new(0),
                    }
                } else {
                    Note::NoteOff {
                        key,
                        vel: u7::new(a.off_velocity),
                    }
                };
                events.push((after.range.end, 0, serial, Event::Midi { channel, message }));
                serial += 1;
            }
        }
        output.extend(merge::encode(events, midi.header)?);
    }
    let candidate = groups(&output)?;
    ensure!(
        state.groups.iter().all(|wanted| candidate.contains(wanted)),
        "Drum marker attribute verification failed; source retained"
    );
    Ok(output)
}
