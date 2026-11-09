//! Harmony MIDI views never combine simultaneous singers into one lead voice.
use super::{HarmonyChart, Part, Voice};
use crate::{
    song_files::game_midi::{midi_chunks, validate_midi_complete},
    vocals::VocalPhrase,
};
use anyhow::{Context, Result, ensure};
use midly::{
    Format, Header, MetaMessage as M, MidiMessage as N, Smf, TrackEvent, TrackEventKind as K,
    num::{u4, u7, u28},
};
use std::collections::BTreeMap;

fn part(events: &[TrackEvent<'_>]) -> Result<Option<Part>> {
    let names: Vec<_> = events
        .iter()
        .filter_map(|event| match event.kind {
            K::Meta(M::TrackName(name)) => Some(name),
            _ => None,
        })
        .collect();
    let parts: Vec<_> = names
        .iter()
        .filter_map(|name| std::str::from_utf8(name).ok().and_then(Part::from_midi))
        .collect();
    ensure!(
        parts.is_empty() || (parts.len() == 1 && names.len() == 1),
        "Ambiguous harmony track names; original retained"
    );
    Ok(parts.first().copied())
}
fn absolute<'a>(events: &[TrackEvent<'a>]) -> Result<Vec<(u64, K<'a>)>> {
    let mut tick = 0u64;
    events
        .iter()
        .map(|event| {
            tick = tick
                .checked_add(u64::from(event.delta.as_int()))
                .context("Harmony MIDI tick overflow")?;
            Ok((tick, event.kind))
        })
        .collect()
}
fn phrase_key(kind: K<'_>) -> bool {
    matches!(kind,K::Midi{message:N::NoteOn{key,..}|N::NoteOff{key,..},..} if matches!(key.as_int(),105|106))
}
fn note_or_lyric(kind: K<'_>) -> bool {
    match kind {
        K::Midi {
            message: N::NoteOn { key, .. } | N::NoteOff { key, .. },
            ..
        } => (36..=84).contains(&key.as_int()),
        K::Meta(M::Lyric(_)) => true,
        K::Meta(M::Text(text)) => !text.starts_with(b"["),
        K::Meta(M::SequencerSpecific(data)) => {
            data == b"YACE\0continue-word\0v1" || data == b"YACE\0missing-lyric\0v1"
        }
        _ => false,
    }
}
fn encode_track<'a>(
    header: Header,
    name: &'static str,
    body: Vec<(u64, K<'a>)>,
    phrases: &[VocalPhrase],
) -> Result<Vec<u8>> {
    let mut events = body;
    events.push((0, K::Meta(M::TrackName(name.as_bytes()))));
    for phrase in phrases {
        let key = u7::new(if phrase.player == 2 { 106 } else { 105 });
        events.push((
            phrase.start,
            K::Midi {
                channel: u4::new(0),
                message: N::NoteOn {
                    key,
                    vel: u7::new(100),
                },
            },
        ));
        events.push((
            phrase.end,
            K::Midi {
                channel: u4::new(0),
                message: N::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
    }
    events.sort_by_key(|(tick, kind)| {
        (
            *tick,
            match kind {
                K::Meta(M::TrackName(_)) => 0,
                K::Midi {
                    message: N::NoteOff { .. },
                    ..
                } => 1,
                K::Midi {
                    message: N::NoteOn { vel, .. },
                    ..
                } if vel.as_int() == 0 => 1,
                K::Midi {
                    message: N::NoteOn { .. },
                    ..
                } => 3,
                _ => 2,
            },
        )
    });
    let mut previous = 0;
    let mut track = Vec::new();
    for (tick, kind) in events {
        let delta = tick - previous;
        ensure!(
            delta <= 0x0fff_ffff,
            "Harmony MIDI delta exceeds format range"
        );
        track.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: K::Meta(M::EndOfTrack),
    });
    let mut bytes = Vec::new();
    Smf {
        header: Header {
            format: Format::Parallel,
            ..header
        },
        tracks: vec![track],
    }
    .write_std(&mut bytes)?;
    Ok(bytes)
}
fn read_phrases(events: &[(u64, K<'_>)]) -> Result<Vec<VocalPhrase>> {
    let mut active = BTreeMap::new();
    let mut phrases = Vec::new();
    for &(tick, kind) in events {
        let K::Midi { channel, message } = kind else {
            continue;
        };
        match message {
            N::NoteOn { key, vel } if matches!(key.as_int(), 105 | 106) && vel.as_int() > 0 => {
                ensure!(
                    active
                        .insert((channel.as_int(), key.as_int()), tick)
                        .is_none(),
                    "Overlapping same-key harmony phrases"
                );
            }
            N::NoteOn { key, .. } | N::NoteOff { key, .. } if matches!(key.as_int(), 105 | 106) => {
                let start = active
                    .remove(&(channel.as_int(), key.as_int()))
                    .context("Unmatched harmony phrase end")?;
                phrases.push(VocalPhrase {
                    start,
                    end: tick,
                    player: if key.as_int() == 106 { 2 } else { 1 },
                });
            }
            _ => {}
        }
    }
    ensure!(active.is_empty(), "Unterminated harmony phrase");
    phrases.sort_by_key(|phrase| (phrase.start, phrase.end, phrase.player));
    Ok(phrases)
}
fn empty_midi(header: Header) -> Result<Vec<u8>> {
    encode_track(header, "TEMPO", Vec::new(), &[])
}

/// Append newly authored harmony tracks, retaining every original chunk.
/// Existing harmony tracks require the explicit preservation-aware edit path.
pub fn append_midi(original: &[u8], chart: &HarmonyChart) -> Result<Vec<u8>> {
    chart.validate()?;
    validate_midi_complete(original)?;
    let source = Smf::parse(original)?;
    ensure!(
        source.header.format != Format::Sequential,
        "Sequential MIDI unsupported"
    );
    ensure!(
        crate::midi::timing(original)?.resolution == chart.resolution,
        "Harmony/source MIDI resolutions differ"
    );
    for events in &source.tracks {
        ensure!(
            part(events)?.is_none(),
            "Source contains authored harmonies; edit them explicitly instead of replacing them"
        );
    }
    let mut output = original.to_vec();
    let mut appended = 0;
    for voice_part in Part::ALL {
        let phrases = match voice_part {
            Part::Harm1 => chart.phrases.as_slice(),
            Part::Harm2 => chart.lyric_phrases.as_slice(),
            Part::Harm3 => &[],
        };
        let voice = chart.voice_chart(voice_part);
        if voice.is_none() && phrases.is_empty() {
            continue;
        }
        let generated = match voice.filter(|voice| !voice.notes.is_empty()) {
            Some(voice) => crate::vocals::append_midi(&empty_midi(source.header)?, &voice)?,
            None => empty_midi(source.header)?,
        };
        let midi = Smf::parse(&generated)?;
        let body = absolute(midi.tracks.last().unwrap())?
            .into_iter()
            .filter(|(_, kind)| note_or_lyric(*kind))
            .collect();
        let encoded = encode_track(source.header, voice_part.track(), body, phrases)?;
        let (_, chunks) = midi_chunks(&encoded, 1)?;
        output.extend_from_slice(chunks[0]);
        appended += 1;
    }
    let count = source
        .tracks
        .len()
        .checked_add(appended)
        .context("MIDI track count overflow")?;
    ensure!(count <= u16::MAX as usize, "Too many MIDI tracks");
    output[8..10].copy_from_slice(&1u16.to_be_bytes());
    output[10..12].copy_from_slice(&(count as u16).to_be_bytes());
    Ok(output)
}

/// Read a supported editable view while callers retain the immutable MIDI.
/// Unknown controls/percussion/Star Power stay in that original, never invented
/// as singer notes. Ambiguous or unsupported lyrics refuse the view.
pub fn import_midi(bytes: &[u8]) -> Result<Option<HarmonyChart>> {
    validate_midi_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    ensure!(
        midi.header.format != Format::Sequential,
        "Sequential MIDI unsupported"
    );
    let mut tracks = BTreeMap::new();
    for events in &midi.tracks {
        if let Some(part) = part(events)? {
            ensure!(
                tracks.insert(part, absolute(events)?).is_none(),
                "Duplicate harmony aliases are ambiguous"
            );
        }
    }
    if tracks.is_empty() {
        return Ok(None);
    }
    let phrases = tracks
        .get(&Part::Harm1)
        .map(|events| read_phrases(events))
        .transpose()?
        .unwrap_or_default();
    let lyric_phrases = tracks
        .get(&Part::Harm2)
        .map(|events| read_phrases(events))
        .transpose()?
        .unwrap_or_default();
    let mut chart = HarmonyChart {
        version: 1,
        resolution: crate::midi::timing(bytes)?.resolution,
        source: "Imported harmony MIDI; original retained separately".into(),
        phrases,
        lyric_phrases,
        voices: BTreeMap::new(),
    };
    for (part, events) in tracks {
        if part == Part::Harm3 {
            ensure!(
                !events.iter().any(|(_, kind)| phrase_key(*kind)),
                "HARM3 phrase markers are not independently editable; source retained"
            );
        }
        let starts: Vec<_> = events
            .iter()
            .filter_map(|(tick, kind)| match kind {
                K::Midi {
                    message: N::NoteOn { key, vel },
                    ..
                } if (36..=84).contains(&key.as_int()) && vel.as_int() > 0 => Some(*tick),
                _ => None,
            })
            .collect();
        let source = format!(
            "Imported {}; original MIDI retained separately",
            part.track()
        );
        let notes = if starts.is_empty() {
            ensure!(
                events.iter().all(|(_, kind)| !note_or_lyric(*kind)),
                "Harmony lyrics or note endings without playable notes cannot become an empty editable voice; source retained"
            );
            Vec::new()
        } else {
            let body = events
                .into_iter()
                .filter(|(_, kind)| note_or_lyric(*kind))
                .collect();
            let relevant: Vec<_> = chart
                .phrases
                .iter()
                .filter(|phrase| {
                    starts
                        .iter()
                        .any(|tick| phrase.start <= *tick && *tick < phrase.end)
                })
                .cloned()
                .collect();
            let encoded = encode_track(midi.header, "PART VOCALS", body, &relevant)?;
            crate::vocals::import_midi(&encoded)?.notes
        };
        chart.voices.insert(part, Voice { source, notes });
    }
    if chart.voices.values().all(|voice| voice.notes.is_empty()) {
        return Ok(None);
    }
    chart.validate()?;
    Ok(Some(chart))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chart::Chart,
        song_files::game_midi::NoteAttributes,
        vocals::{VocalChart, VocalTone},
    };
    fn original(chart: &HarmonyChart) -> Vec<u8> {
        let clock = Chart::parse("[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n".into()).unwrap();
        let lead = VocalChart {
            version: 1,
            resolution: 480,
            source: "Independent lead fixture".into(),
            phrases: chart.phrases.clone(),
            notes: chart.voices[&Part::Harm1].notes.clone(),
        };
        crate::game_export::song(&clock, Some(&lead)).unwrap().0
    }
    fn fixture() -> HarmonyChart {
        crate::vocal_harmonies::tests::fixture()
    }
    #[test]
    fn simultaneous_lead_and_three_voices_round_trip_with_target_phrase_ownership() {
        let chart = fixture();
        let original = original(&chart);
        let output = append_midi(&original, &chart).unwrap();
        let midi = Smf::parse(&output).unwrap();
        let (_, old_chunks) =
            midi_chunks(&original, Smf::parse(&original).unwrap().tracks.len()).unwrap();
        let (_, new_chunks) = midi_chunks(&output, midi.tracks.len()).unwrap();
        assert_eq!(old_chunks, new_chunks[..old_chunks.len()]);
        for events in &midi.tracks {
            if let Some(part) = part(events).unwrap() {
                let events = absolute(events).unwrap();
                assert!(events.iter().any(|(tick,kind)|*tick==240&&matches!(kind,K::Midi{message:N::NoteOn{key,vel},..} if key.as_int()==match part{Part::Harm1=>60,Part::Harm2=>64,Part::Harm3=>68}&&vel.as_int()>0)));
                let ranges = read_phrases(&events).unwrap();
                assert_eq!(
                    ranges,
                    if part == Part::Harm3 {
                        vec![]
                    } else {
                        chart.phrases.clone()
                    }
                );
            }
        }
        let imported = import_midi(&output).unwrap().unwrap();
        assert_eq!(imported.phrases, chart.phrases);
        assert_eq!(imported.lyric_phrases, chart.lyric_phrases);
        for part in Part::ALL {
            assert_eq!(imported.voices[&part].notes, chart.voices[&part].notes);
        }
        assert!(
            append_midi(&output, &chart)
                .unwrap_err()
                .to_string()
                .contains("authored harmonies")
        );
    }
    #[test]
    fn other_singers_can_score_while_harm1_is_silent_and_note_attributes_survive() {
        let mut chart = fixture();
        let original = original(&chart);
        chart.voices.remove(&Part::Harm1);
        let voice = chart.voices.get_mut(&Part::Harm2).unwrap();
        voice.notes[0].tone = VocalTone::Talkie { lenient: true };
        voice.notes[0].midi = Some(NoteAttributes {
            channel: 6,
            velocity: 88,
            off_velocity: 0,
            off_as_note_on: true,
            zero_length: false,
            talkie_pitch: Some(67),
        });
        let output = append_midi(&original, &chart).unwrap();
        let imported = import_midi(&output).unwrap().unwrap();
        assert!(imported.voices[&Part::Harm1].notes.is_empty());
        assert_eq!(
            imported.voices[&Part::Harm2].notes,
            chart.voices[&Part::Harm2].notes
        );
        assert_eq!(
            imported.voices[&Part::Harm3].notes,
            chart.voices[&Part::Harm3].notes
        );
        assert_eq!(imported.phrases, chart.phrases);
    }
    #[test]
    fn duplicate_aliases_and_harm3_owned_phrases_refuse_instead_of_merging_singers() {
        let chart = fixture();
        let output = append_midi(&original(&chart), &chart).unwrap();
        let mut midi = Smf::parse(&output).unwrap();
        let mut duplicate = midi
            .tracks
            .iter()
            .find(|events| part(events).unwrap() == Some(Part::Harm3))
            .unwrap()
            .clone();
        for event in &mut duplicate {
            if matches!(event.kind, K::Meta(M::TrackName(_))) {
                event.kind = K::Meta(M::TrackName(b"PART HARM3"));
            }
        }
        midi.tracks.push(duplicate);
        let mut ambiguous = Vec::new();
        midi.write_std(&mut ambiguous).unwrap();
        assert!(
            import_midi(&ambiguous)
                .unwrap_err()
                .to_string()
                .contains("Duplicate harmony aliases")
        );
        let mut midi = Smf::parse(&output).unwrap();
        let first = midi
            .tracks
            .iter()
            .find(|events| part(events).unwrap() == Some(Part::Harm1))
            .unwrap()
            .clone();
        let third = midi
            .tracks
            .iter_mut()
            .find(|events| part(events).unwrap() == Some(Part::Harm3))
            .unwrap();
        *third = first;
        for event in third {
            if matches!(event.kind, K::Meta(M::TrackName(_))) {
                event.kind = K::Meta(M::TrackName(b"HARM3"));
            }
        }
        let mut malformed = Vec::new();
        midi.write_std(&mut malformed).unwrap();
        assert!(
            import_midi(&malformed)
                .unwrap_err()
                .to_string()
                .contains("HARM3 phrase markers")
        );
    }

    #[test]
    fn silent_harm1_does_not_discard_orphan_lyrics_as_an_empty_voice() {
        let mut chart = fixture();
        let original = original(&chart);
        chart.voices.remove(&Part::Harm1);
        let output = append_midi(&original, &chart).unwrap();
        let mut midi = Smf::parse(&output).unwrap();
        let first = midi
            .tracks
            .iter_mut()
            .find(|events| part(events).unwrap() == Some(Part::Harm1))
            .unwrap();
        first.insert(
            1,
            TrackEvent {
                delta: u28::new(0),
                kind: K::Meta(M::Lyric(b"orphan")),
            },
        );
        let mut bytes = Vec::new();
        midi.write_std(&mut bytes).unwrap();
        assert!(
            import_midi(&bytes)
                .unwrap_err()
                .to_string()
                .contains("without playable notes")
        );
    }
}
