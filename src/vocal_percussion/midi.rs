//! Read a conservative percussion view without changing the source MIDI.
use super::{Hit, Part, PercussionChart, Phrase, Track};
use crate::{
    song_files::game_midi::{NoteAttributes, midi_chunks, validate_midi_complete},
    vocal_harmonies::Part as HarmonyPart,
    vocals::VocalPhrase,
};
use anyhow::{Context, Result, ensure};
use midly::{
    Format, Header, MetaMessage as M, MidiMessage as N, Smf, TrackEvent, TrackEventKind as K,
    num::{u4, u7, u28},
};
use std::collections::{BTreeMap, BTreeSet};

fn name<'a>(events: &[TrackEvent<'a>]) -> Result<Option<&'a str>> {
    let names: Vec<_> = events
        .iter()
        .filter_map(|event| match event.kind {
            K::Meta(M::TrackName(name)) => Some(name),
            _ => None,
        })
        .collect();
    let vocal = names
        .iter()
        .filter_map(|name| std::str::from_utf8(name).ok())
        .any(|name| Part::from_midi(name).is_some() || HarmonyPart::from_midi(name).is_some());
    ensure!(
        !vocal || names.len() == 1,
        "Ambiguous vocal track names; source retained"
    );
    Ok(names
        .first()
        .and_then(|name| std::str::from_utf8(name).ok()))
}
#[derive(Clone)]
struct Span {
    start: u64,
    end: u64,
    key: u8,
    attributes: NoteAttributes,
}
fn spans(events: &[TrackEvent<'_>], selected: impl Fn(u8) -> bool) -> Result<Vec<Span>> {
    let mut active = BTreeMap::new();
    let mut result = Vec::new();
    let mut tick = 0u64;
    for event in events {
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .context("Vocal tick overflow")?;
        let K::Midi { channel, message } = event.kind else {
            continue;
        };
        let (key, velocity, on, as_on) = match message {
            N::NoteOn { key, vel } => (key.as_int(), vel.as_int(), vel.as_int() > 0, true),
            N::NoteOff { key, vel } => (key.as_int(), vel.as_int(), false, false),
            _ => continue,
        };
        if !selected(key) {
            continue;
        }
        let channel = channel.as_int();
        if on {
            ensure!(
                active.insert((channel, key), (tick, velocity)).is_none(),
                "Overlapping vocal MIDI key; source retained"
            );
        } else {
            let (start, velocity_on) = active
                .remove(&(channel, key))
                .context("Unmatched vocal MIDI note end; source retained")?;
            result.push(Span {
                start,
                end: tick,
                key,
                attributes: NoteAttributes {
                    channel,
                    velocity: velocity_on,
                    off_velocity: velocity,
                    off_as_note_on: as_on,
                    zero_length: start == tick,
                    talkie_pitch: None,
                },
            });
        }
    }
    ensure!(
        active.is_empty(),
        "Unterminated vocal MIDI note; source retained"
    );
    result.sort_by_key(|span| (span.start, span.end, span.key, span.attributes.channel));
    Ok(result)
}
fn attributes(value: &NoteAttributes) -> Option<NoteAttributes> {
    (!value.is_default()).then(|| value.clone())
}
fn inside(phrase: &VocalPhrase, span: &Span) -> bool {
    if span.start == span.end {
        phrase.start <= span.start && span.start < phrase.end
    } else {
        span.start < phrase.end && phrase.start < span.end
    }
}
fn overlaps_notes(phrases: &[Phrase], notes: &[Span]) -> bool {
    let mut cursor = 0;
    for phrase in phrases {
        while notes.get(cursor).is_some_and(|note| {
            note.end <= phrase.range.start
                && !(note.start == note.end && note.start == phrase.range.start)
        }) {
            cursor += 1;
        }
        if notes
            .get(cursor)
            .is_some_and(|note| inside(&phrase.range, note))
        {
            return true;
        }
    }
    false
}
fn no_lyrics(events: &[TrackEvent<'_>], phrases: &[Phrase]) -> Result<()> {
    let mut cursor = 0;
    let mut tick = 0u64;
    for event in events {
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .context("Vocal tick overflow")?;
        while phrases
            .get(cursor)
            .is_some_and(|phrase| phrase.range.end <= tick)
        {
            cursor += 1;
        }
        if phrases
            .get(cursor)
            .is_some_and(|phrase| phrase.range.start <= tick)
        {
            ensure!(
                !matches!(event.kind, K::Meta(M::Lyric(_)))
                    && !matches!(event.kind, K::Meta(M::Text(text)) if !text.starts_with(b"[")),
                "Lyrics inside a percussion phrase are not editable; source retained"
            );
        }
    }
    Ok(())
}
fn read_track(events: &[TrackEvent<'_>], label: &str) -> Result<Option<Track>> {
    if !events.iter().any(|event| {
        matches!(event.kind,
        K::Midi { message: N::NoteOn { key, vel }, .. }
        if matches!(key.as_int(), 96 | 97) && vel.as_int() > 0)
    }) {
        return Ok(None);
    }
    let notes = spans(events, |key| matches!(key, 96 | 97 | 105 | 106))?;
    let hits: Vec<_> = notes
        .iter()
        .filter(|span| matches!(span.key, 96 | 97))
        .map(|span| Hit {
            start: span.start,
            end: span.end,
            displayed: span.key == 96,
            midi: attributes(&span.attributes),
        })
        .collect();
    let all_phrases: Vec<_> = notes
        .iter()
        .filter(|span| matches!(span.key, 105 | 106))
        .map(|span| Phrase {
            range: VocalPhrase {
                start: span.start,
                end: span.end,
                player: if span.key == 106 { 2 } else { 1 },
            },
            midi: attributes(&span.attributes),
        })
        .collect();
    let mut phrases = Vec::new();
    let mut previous_end = 0;
    let mut percussion_end = 0;
    for phrase in all_phrases {
        let first = hits.partition_point(|hit| hit.start < phrase.range.start);
        let percussion = hits.get(first).is_some_and(|hit| phrase.covers(hit));
        ensure!(
            phrase.range.start >= percussion_end
                && (!percussion || phrase.range.start >= previous_end),
            "Overlapping source percussion/scoring phrases; source retained"
        );
        previous_end = previous_end.max(phrase.range.end);
        if percussion {
            percussion_end = phrase.range.end;
            phrases.push(phrase);
        }
    }
    Ok(Some(Track {
        source: format!("Imported {label} percussion; original MIDI retained separately"),
        phrases,
        hits,
    }))
}

/// Return only supported, dedicated lead/HARM1 percussion phrases. Ambiguous
/// names, mixed singing, unmatched events and uncovered hits refuse the view;
/// callers must preserve the source instead of inventing scoring phrases.
pub fn import_midi(bytes: &[u8]) -> Result<Option<PercussionChart>> {
    validate_midi_complete(bytes)?;
    let source = Smf::parse(bytes)?;
    ensure!(
        source.header.format != Format::Sequential,
        "Sequential MIDI unsupported"
    );
    let mut tracks = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for events in &source.tracks {
        let Some(label) = name(events)? else { continue };
        let Some(part) = Part::from_midi(label) else {
            continue;
        };
        ensure!(
            seen.insert(part),
            "Duplicate vocal percussion owner; source retained"
        );
        if let Some(track) = read_track(events, label)? {
            tracks.insert(part, track);
        }
    }
    if tracks.is_empty() {
        return Ok(None);
    }
    let chart = PercussionChart {
        version: 1,
        resolution: crate::midi::timing(bytes)?.resolution,
        source: "Imported vocal percussion MIDI; original retained separately".into(),
        tracks,
    };
    chart.validate()?;
    for events in &source.tracks {
        let Some(label) = name(events)? else { continue };
        let track = if let Some(part) = Part::from_midi(label) {
            chart.tracks.get(&part)
        } else if matches!(
            HarmonyPart::from_midi(label),
            Some(HarmonyPart::Harm2 | HarmonyPart::Harm3)
        ) {
            chart.tracks.get(&Part::Harm1)
        } else {
            None
        };
        let Some(track) = track else { continue };
        let notes = spans(events, |key| (36..=84).contains(&key))?;
        ensure!(
            !overlaps_notes(&track.phrases, &notes),
            "Percussion overlaps singing in {label}; source retained"
        );
        no_lyrics(events, &track.phrases)?;
    }
    Ok(Some(chart))
}

fn defaults(zero_length: bool) -> NoteAttributes {
    NoteAttributes {
        channel: 0,
        velocity: 100,
        off_velocity: 0,
        off_as_note_on: false,
        zero_length,
        talkie_pitch: None,
    }
}
fn note(channel: u8, key: u8, velocity: u8, on: bool) -> K<'static> {
    let key = u7::new(key);
    let vel = u7::new(velocity);
    K::Midi {
        channel: u4::new(channel),
        message: if on {
            N::NoteOn { key, vel }
        } else {
            N::NoteOff { key, vel }
        },
    }
}
fn encode_track(header: Header, part: Part, track: &Track) -> Result<Vec<u8>> {
    let mut events = vec![(0, 0, K::Meta(M::TrackName(part.track().as_bytes())))];
    for phrase in &track.phrases {
        let attributes = phrase.midi.clone().unwrap_or_else(|| defaults(false));
        let key = if phrase.range.player == 2 { 106 } else { 105 };
        events.push((
            phrase.range.start,
            2,
            note(attributes.channel, key, attributes.velocity, true),
        ));
        events.push((
            phrase.range.end,
            0,
            note(
                attributes.channel,
                key,
                attributes.off_velocity,
                attributes.off_as_note_on,
            ),
        ));
    }
    for hit in &track.hits {
        let zero = hit.start == hit.end;
        let attributes = hit.midi.clone().unwrap_or_else(|| defaults(zero));
        events.push((
            hit.start,
            3,
            note(attributes.channel, hit.key(), attributes.velocity, true),
        ));
        events.push((
            hit.end,
            if zero { 4 } else { 1 },
            note(
                attributes.channel,
                hit.key(),
                attributes.off_velocity,
                attributes.off_as_note_on,
            ),
        ));
    }
    events.sort_by_key(|event| (event.0, event.1));
    let mut previous = 0;
    let mut encoded = Vec::new();
    for (tick, _, kind) in events {
        let delta = tick
            .checked_sub(previous)
            .context("Unordered percussion MIDI")?;
        ensure!(
            delta <= 0x0fff_ffff,
            "Percussion MIDI delta exceeds format range"
        );
        encoded.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    encoded.push(TrackEvent {
        delta: u28::new(0),
        kind: K::Meta(M::EndOfTrack),
    });
    let mut bytes = Vec::new();
    Smf {
        header: Header {
            format: Format::Parallel,
            ..header
        },
        tracks: vec![encoded],
    }
    .write_std(&mut bytes)?;
    Ok(bytes)
}

/// Encode newly authored parts without rewriting any source chunk. Existing
/// vocal owners require the separate preservation-aware edit path; refusing is
/// safer than duplicating a track or replacing its singing/author controls.
pub fn append_midi(original: &[u8], chart: &PercussionChart) -> Result<Vec<u8>> {
    chart.validate()?;
    validate_midi_complete(original)?;
    let source = Smf::parse(original)?;
    ensure!(
        source.header.format != Format::Sequential,
        "Sequential MIDI unsupported"
    );
    ensure!(
        crate::midi::timing(original)?.resolution == chart.resolution,
        "Percussion/source MIDI resolutions differ"
    );
    for events in &source.tracks {
        let Some(label) = name(events)? else { continue };
        ensure!(
            !Part::from_midi(label).is_some_and(|part| chart.tracks.contains_key(&part))
                && !(chart.tracks.contains_key(&Part::Harm1)
                    && HarmonyPart::from_midi(label).is_some()),
            "Existing vocal owner needs preservation-aware percussion editing; source retained"
        );
    }
    let (header, _) = midi_chunks(original, source.tracks.len())?;
    let count = u16::try_from(
        source
            .tracks
            .len()
            .checked_add(chart.tracks.len())
            .context("Too many percussion MIDI tracks")?,
    )?;
    let mut output = original.to_vec();
    output[8..10].copy_from_slice(&1u16.to_be_bytes());
    output[10..12].copy_from_slice(&count.to_be_bytes());
    ensure!(header >= 14, "Invalid MIDI header");
    for (&part, track) in &chart.tracks {
        let encoded = encode_track(source.header, part, track)?;
        let (_, chunks) = midi_chunks(&encoded, 1)?;
        output.extend(chunks[0]);
    }
    Ok(output)
}

#[cfg(test)]
#[path = "midi_tests.rs"]
mod tests;
