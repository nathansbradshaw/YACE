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
use sha2::{Digest, Sha256};
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
fn complete_owner(events: &[TrackEvent<'_>]) -> Result<()> {
    ensure!(
        events
            .last()
            .is_some_and(|event| matches!(event.kind, K::Meta(M::EndOfTrack)))
            && events
                .iter()
                .filter(|event| matches!(event.kind, K::Meta(M::EndOfTrack)))
                .count()
                == 1,
        "Vocal owner has missing, repeated or non-final end-of-track data; source retained"
    );
    Ok(())
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
    complete_owner(events)?;
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
                midi: None,
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
        let shared = matches!(
            HarmonyPart::from_midi(label),
            Some(HarmonyPart::Harm2 | HarmonyPart::Harm3)
        );
        let track = if let Some(part) = Part::from_midi(label) {
            chart.tracks.get(&part)
        } else if shared {
            chart.tracks.get(&Part::Harm1)
        } else {
            None
        };
        let Some(track) = track else { continue };
        // A shared scoring phrase may also support percussion retained in an
        // opaque secondary part. Do not expose its owner as independently editable.
        let notes = spans(events, |key| {
            (36..=84).contains(&key) || (shared && matches!(key, 96 | 97))
        })?;
        ensure!(
            !overlaps_notes(&track.phrases, &notes),
            "Percussion overlaps singing or unmanaged harmony percussion in {label}; source retained"
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
fn authored_events(track: &Track, hits: bool, phrases: bool) -> Vec<(u64, u8, K<'static>)> {
    let mut events = Vec::new();
    for phrase in track.phrases.iter().filter(|_| phrases) {
        let attributes = phrase.midi.clone().unwrap_or_else(|| defaults(false));
        let key = if phrase.range.player == 2 { 106 } else { 105 };
        events.push((
            phrase.range.start,
            3,
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
    for hit in track.hits.iter().filter(|_| hits) {
        let zero = hit.start == hit.end;
        let attributes = hit.midi.clone().unwrap_or_else(|| defaults(zero));
        events.push((
            hit.start,
            4,
            note(attributes.channel, hit.key(), attributes.velocity, true),
        ));
        events.push((
            hit.end,
            if zero { 5 } else { 1 },
            note(
                attributes.channel,
                hit.key(),
                attributes.off_velocity,
                attributes.off_as_note_on,
            ),
        ));
    }
    events
}
fn encode_events(header: Header, mut events: Vec<(u64, u8, K<'_>)>, end: u64) -> Result<Vec<u8>> {
    let end = end.max(events.iter().map(|event| event.0).max().unwrap_or(0));
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
    let tail = end
        .checked_sub(previous)
        .context("Unordered percussion end")?;
    ensure!(
        tail <= 0x0fff_ffff,
        "Percussion MIDI end delta exceeds format range"
    );
    encoded.push(TrackEvent {
        delta: u28::new(tail as u32),
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
fn encode_track(header: Header, part: Part, track: &Track) -> Result<Vec<u8>> {
    let mut events = vec![(0, 2, K::Meta(M::TrackName(part.track().as_bytes())))];
    events.extend(authored_events(track, true, true));
    encode_events(header, events, 0)
}

fn same_hits(left: &[Hit], right: &[Hit]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            (left.start, left.end, left.displayed) == (right.start, right.end, right.displayed)
                && left
                    .midi
                    .clone()
                    .unwrap_or_else(|| defaults(left.start == left.end))
                    == right
                        .midi
                        .clone()
                        .unwrap_or_else(|| defaults(right.start == right.end))
        })
}
fn same_phrases(left: &[Phrase], right: &[Phrase]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.range == right.range
                && left.midi.clone().unwrap_or_else(|| defaults(false))
                    == right.midi.clone().unwrap_or_else(|| defaults(false))
        })
}
fn same_track(left: Option<&Track>, right: Option<&Track>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            same_hits(&left.hits, &right.hits) && same_phrases(&left.phrases, &right.phrases)
        }
        _ => false,
    }
}
fn same_chart(left: Option<&PercussionChart>, right: Option<&PercussionChart>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.resolution == right.resolution
                && left.tracks.len() == right.tracks.len()
                && left
                    .tracks
                    .iter()
                    .all(|(part, track)| same_track(Some(track), right.tracks.get(part)))
        }
        _ => false,
    }
}
type PhraseEdge = (u64, u8, u8, bool);
fn phrase_edge(tick: u64, kind: K<'_>) -> Option<PhraseEdge> {
    let K::Midi { channel, message } = kind else {
        return None;
    };
    let (key, on) = match message {
        N::NoteOn { key, vel } => (key.as_int(), vel.as_int() > 0),
        N::NoteOff { key, .. } => (key.as_int(), false),
        _ => return None,
    };
    matches!(key, 105 | 106).then_some((tick, channel.as_int(), key, on))
}
fn rewrite_track(
    header: Header,
    source: &[TrackEvent<'_>],
    before: Option<&Track>,
    after: Option<&Track>,
) -> Result<Vec<u8>> {
    complete_owner(source)?;
    let phrases = !same_phrases(
        before.map_or(&[], |track| &track.phrases),
        after.map_or(&[], |track| &track.phrases),
    );
    // Re-emit managed hits with changed markers so scoring starts precede
    // same-tick hit starts, while retaining every hit's timing and attributes.
    let hits = phrases
        || !same_hits(
            before.map_or(&[], |track| &track.hits),
            after.map_or(&[], |track| &track.hits),
        );
    let mut managed = BTreeSet::new();
    if phrases && let Some(before) = before {
        for phrase in &before.phrases {
            let channel = phrase
                .midi
                .as_ref()
                .map_or(0, |attributes| attributes.channel);
            let key = if phrase.range.player == 2 { 106 } else { 105 };
            managed.insert((phrase.range.start, channel, key, true));
            managed.insert((phrase.range.end, channel, key, false));
        }
    }
    let mut tick = 0u64;
    let mut events = Vec::new();
    for event in source {
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .context("Vocal tick overflow")?;
        let kind = event.kind;
        let managed_hit = hits
            && before.is_some()
            && matches!(kind,
            K::Midi { message: N::NoteOn { key, .. } | N::NoteOff { key, .. }, .. }
            if matches!(key.as_int(), 96 | 97));
        if matches!(kind, K::Meta(M::EndOfTrack))
            || managed_hit
            || phrase_edge(tick, kind).is_some_and(|edge| managed.contains(&edge))
        {
            continue;
        }
        // Keep all retained source events in their original same-tick order.
        // New endings precede them; new beginnings follow them at boundaries.
        events.push((tick, 2, kind));
    }
    if let Some(after) = after {
        events.extend(authored_events(after, hits, phrases));
    }
    encode_events(header, events, tick)
}

/// Selectively edit percussion against a previously verified source binding.
/// The expected hash must come from that binding, not be recomputed to accept
/// changed input. A stale view, shared control conflict or invalid candidate
/// refuses without modifying any input or returning a partially rewritten song.
pub fn rewrite_midi(
    original: &[u8],
    expected_midi_sha256: &str,
    before: Option<&PercussionChart>,
    after: Option<&PercussionChart>,
) -> Result<Vec<u8>> {
    ensure!(
        format!("{:x}", Sha256::digest(original)) == expected_midi_sha256,
        "Percussion source MIDI changed; recover the verified source before editing"
    );
    let actual = import_midi(original)?;
    ensure!(
        actual.as_ref() == before,
        "Percussion baseline changed; select the current source again"
    );
    if let Some(after) = after {
        after.validate()?;
    }
    let source = Smf::parse(original)?;
    let resolution = crate::midi::timing(original)?.resolution;
    ensure!(
        after.is_none_or(|chart| chart.resolution == resolution),
        "Percussion/source MIDI resolutions differ"
    );
    if same_chart(before, after) {
        return Ok(original.to_vec());
    }
    let (header, raw) = midi_chunks(original, source.tracks.len())?;
    let mut output_tracks = Vec::new();
    let mut existing = BTreeSet::new();
    let mut source_harmonies = false;
    for (events, chunk) in source.tracks.iter().zip(raw) {
        let label = name(events)?;
        source_harmonies |= label.and_then(HarmonyPart::from_midi).is_some();
        let Some(part) = label.and_then(Part::from_midi) else {
            output_tracks.push(chunk.to_vec());
            continue;
        };
        ensure!(
            existing.insert(part),
            "Duplicate vocal percussion owner; source retained"
        );
        let left = before.and_then(|chart| chart.tracks.get(&part));
        let right = after.and_then(|chart| chart.tracks.get(&part));
        if same_track(left, right) {
            output_tracks.push(chunk.to_vec());
        } else {
            let bytes = rewrite_track(source.header, events, left, right)?;
            output_tracks.push(midi_chunks(&bytes, 1)?.1[0].to_vec());
        }
    }
    if let Some(after) = after {
        for (&part, track) in &after.tracks {
            if existing.contains(&part) {
                continue;
            }
            ensure!(
                part != Part::Harm1 || !source_harmonies,
                "Existing harmony tracks need their original shared scoring owner; source retained"
            );
            let bytes = encode_track(source.header, part, track)?;
            output_tracks.push(midi_chunks(&bytes, 1)?.1[0].to_vec());
        }
    }
    let mut output = original[..header].to_vec();
    output[8..10].copy_from_slice(&1u16.to_be_bytes());
    output[10..12].copy_from_slice(&u16::try_from(output_tracks.len())?.to_be_bytes());
    for track in output_tracks {
        output.extend(track);
    }
    let candidate = import_midi(&output)?;
    ensure!(
        same_chart(candidate.as_ref(), after),
        "Percussion rewrite did not retain the requested document"
    );
    Ok(output)
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
