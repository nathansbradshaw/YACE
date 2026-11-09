//! Read a conservative percussion view without changing the source MIDI.
use super::{Hit, Part, PercussionChart, Phrase, Track};
use crate::{
    song_files::game_midi::{NoteAttributes, validate_midi_complete},
    vocal_harmonies::Part as HarmonyPart,
    vocals::VocalPhrase,
};
use anyhow::{Context, Result, ensure};
use midly::{Format, MetaMessage as M, MidiMessage as N, Smf, TrackEvent, TrackEventKind as K};
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

#[cfg(test)]
#[path = "midi_tests.rs"]
mod tests;
