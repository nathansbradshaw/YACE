//! Lead overdrive and HARM1's shared harmony overdrive. HARM2/3 controls stay
//! opaque: the target parser copies HARM1 power to both dependent singers.
pub use crate::vocal_percussion::Part;
use crate::{
    song_files::game_midi::{self, NoteAttributes},
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use midly::{
    Format, MetaMessage as Meta, MidiMessage as Note, Smf, TrackEvent, TrackEventKind as Event,
    num::{u4, u7},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: usize = 2048;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub range: Range,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<NoteAttributes>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub owner: String,
    pub phrases: Vec<Phrase>,
}
/// None in legacy project content inherits the verified source. An authored
/// state, including empty tracks, is an explicit replacement and survives Undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub resolution: u32,
    pub source_sha256: Option<String>,
    pub tracks: BTreeMap<Part, Track>,
}
impl State {
    pub fn empty(resolution: u32, source_sha256: Option<String>) -> Self {
        Self {
            version: 1,
            resolution,
            source_sha256,
            tracks: BTreeMap::new(),
        }
    }
    pub fn validate_shape(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported vocal overdrive version or resolution"
        );
        ensure!(
            self.source_sha256
                .as_ref()
                .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid vocal overdrive source fingerprint"
        );
        for (part, track) in &self.tracks {
            ensure!(
                Part::from_midi(&track.owner) == Some(*part),
                "Vocal overdrive physical owner does not match its part"
            );
            ensure!(
                track.phrases.len() <= LIMIT,
                "A vocal part supports up to 2,048 overdrive ranges"
            );
            ensure!(
                track
                    .phrases
                    .windows(2)
                    .all(|p| p[0].range.end <= p[1].range.start),
                "Vocal overdrive ranges must be ordered and non-overlapping"
            );
            for phrase in &track.phrases {
                ensure!(
                    phrase.range.start < phrase.range.end
                        && phrase.range.end <= u64::from(u32::MAX),
                    "Invalid or empty vocal overdrive range"
                );
                if let Some(attributes) = &phrase.midi {
                    attributes.validate()?;
                    ensure!(
                        !attributes.zero_length && attributes.talkie_pitch.is_none(),
                        "Overdrive marker attributes cannot describe a zero-length hit or talkie"
                    );
                }
            }
        }
        Ok(())
    }
    pub fn validate(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.validate_shape()?;
        ensure!(
            self.resolution == timing.resolution,
            "Vocal overdrive/project resolution mismatch"
        );
        ensure!(
            duration.is_finite() && duration > 0.,
            "Invalid vocal overdrive timeline duration"
        );
        ensure!(
            self.tracks.values().flat_map(|t| &t.phrases).all(|p| {
                let end = timing.seconds(p.range.end);
                end.is_finite() && end <= duration + 0.1
            }),
            "Vocal overdrive extends beyond the recording or timeline"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Phrase>,
        range: Range,
    },
    Remove {
        phrase: Phrase,
    },
}
pub fn edited(state: &State, part: Part, edit: &Edit) -> Result<State> {
    state.validate_shape()?;
    let mut next = state.clone();
    let track = next.tracks.entry(part).or_insert_with(|| Track {
        owner: part.track().into(),
        phrases: Vec::new(),
    });
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { phrase } => Some(phrase),
    };
    if let Some(previous) = previous {
        let index = track
            .phrases
            .iter()
            .position(|p| p == previous)
            .context("The vocal overdrive range changed. Select it again.")?;
        track.phrases.remove(index);
    }
    if let Edit::Put { range, .. } = edit {
        track.phrases.push(Phrase {
            range: range.clone(),
            midi: previous.and_then(|p| p.midi.clone()),
        });
    }
    track.phrases.sort_by_key(|p| (p.range.start, p.range.end));
    next.validate_shape()?;
    Ok(next)
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn owner(track: &[TrackEvent<'_>]) -> Result<Option<(Part, String)>> {
    let names: Vec<_> = track
        .iter()
        .filter_map(|e| match e.kind {
            Event::Meta(Meta::TrackName(name)) => Some(std::str::from_utf8(name).ok()),
            _ => None,
        })
        .collect();
    let part = names
        .iter()
        .flatten()
        .find_map(|name| Part::from_midi(name));
    let Some(part) = part else { return Ok(None) };
    ensure!(
        names.len() == 1,
        "Ambiguous vocal overdrive owner aliases; original retained"
    );
    let name = names[0].context("Invalid vocal owner label")?;
    ensure!(
        track
            .last()
            .is_some_and(|e| matches!(e.kind, Event::Meta(Meta::EndOfTrack)))
            && track
                .iter()
                .filter(|e| matches!(e.kind, Event::Meta(Meta::EndOfTrack)))
                .count()
                == 1,
        "Vocal overdrive owner has incomplete end-of-track data; original retained"
    );
    Ok(Some((part, name.into())))
}
fn edge(tick: u64, event: Event<'_>) -> Option<(u64, u8, bool)> {
    match event {
        Event::Midi {
            channel,
            message: Note::NoteOn { key, vel },
        } if key.as_int() == 116 => Some((tick, channel.as_int(), vel.as_int() > 0)),
        Event::Midi {
            channel,
            message: Note::NoteOff { key, .. },
        } if key.as_int() == 116 => Some((tick, channel.as_int(), false)),
        _ => None,
    }
}
pub fn import_midi(bytes: &[u8]) -> Result<State> {
    game_midi::validate_midi_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    ensure!(
        midi.header.format != Format::Sequential,
        "Sequential vocal MIDI is not editable"
    );
    let midly::Timing::Metrical(ppq) = midi.header.timing else {
        anyhow::bail!("Vocal overdrive requires metrical MIDI timing")
    };
    let mut state = State::empty(u32::from(ppq.as_int()), Some(hash(bytes)));
    for events in &midi.tracks {
        let Some((part, name)) = owner(events)? else {
            continue;
        };
        ensure!(
            !state.tracks.contains_key(&part),
            "Duplicate vocal overdrive owners; source retained"
        );
        let mut active = BTreeMap::<u8, (u64, u8)>::new();
        let mut phrases = Vec::new();
        for (tick, event) in game_midi::midi_events(events) {
            let Some((_, channel, on)) = edge(tick, event) else {
                continue;
            };
            if on {
                let Event::Midi {
                    message: Note::NoteOn { vel, .. },
                    ..
                } = event
                else {
                    unreachable!()
                };
                ensure!(
                    active.insert(channel, (tick, vel.as_int())).is_none(),
                    "Overlapping same-channel vocal overdrive markers; source retained"
                );
            } else {
                let (start, velocity) = active
                    .remove(&channel)
                    .context("Unmatched vocal overdrive marker end; source retained")?;
                let (off_velocity, off_as_note_on) = match event {
                    Event::Midi {
                        message: Note::NoteOff { vel, .. },
                        ..
                    } => (vel.as_int(), false),
                    _ => (0, true),
                };
                phrases.push(Phrase {
                    range: Range { start, end: tick },
                    midi: Some(NoteAttributes {
                        channel,
                        velocity,
                        off_velocity,
                        off_as_note_on,
                        zero_length: start == tick,
                        talkie_pitch: None,
                    }),
                });
            }
        }
        ensure!(
            active.is_empty(),
            "Unterminated vocal overdrive marker; source retained"
        );
        phrases.sort_by_key(|p| (p.range.start, p.range.end));
        state.tracks.insert(
            part,
            Track {
                owner: name,
                phrases,
            },
        );
    }
    state.validate_shape()?;
    Ok(state)
}

fn nonempty(state: &State) -> BTreeMap<Part, &Track> {
    state
        .tracks
        .iter()
        .filter(|(_, t)| !t.phrases.is_empty())
        .map(|(p, t)| (*p, t))
        .collect()
}
fn effective_attributes(p: &Phrase) -> NoteAttributes {
    p.midi.clone().unwrap_or(NoteAttributes {
        channel: 0,
        velocity: 100,
        off_velocity: 0,
        off_as_note_on: false,
        zero_length: false,
        talkie_pitch: None,
    })
}
fn equivalent(before: &Track, after: &Track) -> bool {
    before.owner == after.owner
        && before.phrases.len() == after.phrases.len()
        && before
            .phrases
            .iter()
            .zip(&after.phrases)
            .all(|(a, b)| a.range == b.range && effective_attributes(a) == effective_attributes(b))
}
fn same(before: &State, after: &State) -> bool {
    let a = nonempty(before);
    let b = nonempty(after);
    a.len() == b.len()
        && a.iter()
            .all(|(p, a)| b.get(p).is_some_and(|b| equivalent(a, b)))
}
type EncodedEvent<'a> = (u64, u8, usize, Event<'a>);
fn marker_events(phrases: &[Phrase]) -> Vec<EncodedEvent<'static>> {
    let mut events = Vec::new();
    let key = u7::new(116);
    for (index, phrase) in phrases.iter().enumerate() {
        let a = effective_attributes(phrase);
        let channel = u4::new(a.channel);
        events.push((
            phrase.range.start,
            2,
            index,
            Event::Midi {
                channel,
                message: Note::NoteOn {
                    key,
                    vel: u7::new(a.velocity),
                },
            },
        ));
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
        events.push((phrase.range.end, 0, index, Event::Midi { channel, message }));
    }
    events
}

/// Rewrite only supported key116 edges. Exact input and original range
/// witnesses prevent adopting a stale document; unrelated tracks stay raw.
pub fn rewrite_midi(
    bytes: &[u8],
    expected_sha256: &str,
    before: &State,
    after: &State,
) -> Result<Vec<u8>> {
    ensure!(
        hash(bytes) == expected_sha256,
        "Vocal overdrive MIDI input changed; export refused"
    );
    before.validate_shape()?;
    after.validate_shape()?;
    ensure!(
        before.source_sha256 == after.source_sha256 && before.resolution == after.resolution,
        "Vocal overdrive source binding changed"
    );
    let actual = import_midi(bytes)?;
    ensure!(
        actual.resolution == before.resolution && same(&actual, before),
        "Vocal overdrive original ranges changed; export refused"
    );
    let parts: BTreeSet<_> = before
        .tracks
        .keys()
        .chain(after.tracks.keys())
        .copied()
        .collect();
    let changed: BTreeSet<_> = parts
        .into_iter()
        .filter(
            |part| match (before.tracks.get(part), after.tracks.get(part)) {
                (Some(a), Some(b)) => !equivalent(a, b),
                (None, Some(b)) => !b.phrases.is_empty(),
                (Some(a), None) => !a.phrases.is_empty(),
                _ => false,
            },
        )
        .collect();
    if changed.is_empty() {
        return Ok(bytes.to_vec());
    }
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = game_midi::midi_chunks(bytes, midi.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut seen = BTreeSet::new();
    for (events, chunk) in midi.tracks.iter().zip(chunks) {
        let Some((part, _)) = owner(events)?.filter(|(part, _)| changed.contains(part)) else {
            output.extend(chunk);
            continue;
        };
        seen.insert(part);
        let old = before
            .tracks
            .get(&part)
            .map_or(&[][..], |t| t.phrases.as_slice());
        let next = after
            .tracks
            .get(&part)
            .map_or(&[][..], |t| t.phrases.as_slice());
        let unchanged = |phrase: &Phrase, list: &[Phrase]| {
            list.iter().any(|p| {
                p.range == phrase.range && effective_attributes(p) == effective_attributes(phrase)
            })
        };
        let mut remove = BTreeSet::new();
        for p in old.iter().filter(|p| !unchanged(p, next)) {
            let a = effective_attributes(p);
            remove.insert((p.range.start, a.channel, true));
            remove.insert((p.range.end, a.channel, false));
        }
        let mut keep: Vec<_> = game_midi::midi_events(events)
            .into_iter()
            .enumerate()
            .filter(|(_, (tick, event))| edge(*tick, *event).is_none_or(|e| !remove.contains(&e)))
            .map(|(i, (tick, e))| (tick, 1, i, e))
            .collect();
        let changed: Vec<_> = next
            .iter()
            .filter(|p| !unchanged(p, old))
            .cloned()
            .collect();
        keep.extend(marker_events(&changed));
        output.extend(game_midi::encode_midi_owner(keep, midi.header)?);
    }
    let mut count = midi.tracks.len();
    for part in changed.difference(&seen) {
        let track = after
            .tracks
            .get(part)
            .context("Missing new vocal overdrive owner")?;
        let mut events = vec![(
            0,
            1,
            0,
            Event::Meta(Meta::TrackName(track.owner.as_bytes())),
        )];
        events.extend(marker_events(&track.phrases));
        output.extend(game_midi::encode_midi_owner(events, midi.header)?);
        count += 1;
    }
    if count != midi.tracks.len() {
        output[10..12].copy_from_slice(&u16::try_from(count)?.to_be_bytes());
        if midi.header.format == Format::SingleTrack {
            output[8..10].copy_from_slice(&1u16.to_be_bytes());
        }
    }
    ensure!(
        same(&import_midi(&output)?, after),
        "Vocal overdrive MIDI verification failed; export refused"
    );
    Ok(output)
}

#[cfg(test)]
#[path = "vocal_overdrive_project_tests.rs"]
mod project_tests;
#[cfg(test)]
#[path = "vocal_overdrive_tests.rs"]
mod tests;
