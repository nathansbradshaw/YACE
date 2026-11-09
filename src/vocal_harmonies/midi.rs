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
    encode_events(header, events)
}
fn encode_events(header: Header, mut events: Vec<(u64, K<'_>)>) -> Result<Vec<u8>> {
    let end = events.iter().map(|e| e.0).max().unwrap_or(0);
    events.retain(|(_, kind)| !matches!(kind, K::Meta(M::EndOfTrack)));
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
    ensure!(
        end - previous <= 0x0fff_ffff,
        "Harmony MIDI end delta exceeds format range"
    );
    track.push(TrackEvent {
        delta: u28::new((end - previous) as u32),
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

fn notes(chart: &HarmonyChart, part: Part) -> &[crate::vocals::VocalNote] {
    chart
        .voices
        .get(&part)
        .map_or(&[], |voice| voice.notes.as_slice())
}
fn owned_phrases(chart: &HarmonyChart, part: Part) -> &[VocalPhrase] {
    match part {
        Part::Harm1 => &chart.phrases,
        Part::Harm2 => &chart.lyric_phrases,
        Part::Harm3 => &[],
    }
}
fn same_content(a: &HarmonyChart, b: &HarmonyChart) -> bool {
    a.resolution == b.resolution
        && a.phrases == b.phrases
        && a.lyric_phrases == b.lyric_phrases
        && Part::ALL
            .into_iter()
            .all(|part| notes(a, part) == notes(b, part))
}

/// Remove editable singer notes and lyrics while retaining source percussion,
/// scoring/lyric ranges, overdrive and author controls. An empty editable
/// document must never restore the original pitched singers on later export.
pub fn clear_midi(original: &[u8], before: &HarmonyChart) -> Result<Vec<u8>> {
    before.validate()?;
    let baseline = import_midi(original)?.context("No editable source harmonies")?;
    ensure!(
        same_content(&baseline, before),
        "Harmony baseline differs from the retained source; reopen before exporting"
    );
    let source = Smf::parse(original)?;
    let (header, chunks) = midi_chunks(original, source.tracks.len())?;
    let mut result = original[..header].to_vec();
    for (track, raw) in source.tracks.iter().zip(chunks) {
        if part(track)?.is_none() {
            result.extend(raw);
            continue;
        }
        let body = absolute(track)?
            .into_iter()
            .filter(|(_, kind)| !note_or_lyric(*kind))
            .collect();
        let encoded = encode_events(source.header, body)?;
        result.extend(midi_chunks(&encoded, 1)?.1[0]);
    }
    Ok(result)
}

/// Rewrite only explicitly edited harmony components. Lead, unknown tracks,
/// original controls and unchanged singer chunks stay in their original form.
/// The baseline must describe this exact source view, never another revision.
pub fn rewrite_midi(
    original: &[u8],
    before: &HarmonyChart,
    after: &HarmonyChart,
) -> Result<Vec<u8>> {
    before.validate()?;
    after.validate()?;
    let baseline = import_midi(original)?.context("No editable source harmonies")?;
    ensure!(
        same_content(&baseline, before),
        "Harmony baseline differs from the retained source; reopen before exporting"
    );
    ensure!(
        before.resolution == after.resolution,
        "Imported harmony resolution must remain unchanged"
    );
    if same_content(before, after) {
        return Ok(original.to_vec());
    }
    let source = Smf::parse(original)?;
    let (header, original_chunks) = midi_chunks(original, source.tracks.len())?;
    validate_percussion_scoring(&source, before, after)?;
    let generated_bytes = append_midi(&empty_midi(source.header)?, after)?;
    let generated = Smf::parse(&generated_bytes)?;
    let mut generated_tracks = BTreeMap::new();
    for track in &generated.tracks {
        if let Some(part) = part(track)? {
            generated_tracks.insert(part, track);
        }
    }
    let mut chunks = Vec::new();
    for (track, raw) in source.tracks.iter().zip(original_chunks) {
        let Some(part) = part(track)? else {
            chunks.push(raw.to_vec());
            continue;
        };
        let notes_changed = notes(before, part) != notes(after, part);
        let phrases_changed = owned_phrases(before, part) != owned_phrases(after, part);
        let new_track = generated_tracks.remove(&part);
        if !notes_changed && !phrases_changed {
            chunks.push(raw.to_vec());
            continue;
        }
        let replace =
            |kind| (notes_changed && note_or_lyric(kind)) || (phrases_changed && phrase_key(kind));
        let mut body: Vec<_> = absolute(track)?
            .into_iter()
            .filter(|(_, kind)| !replace(*kind))
            .collect();
        if let Some(new_track) = new_track {
            body.extend(
                absolute(new_track)?
                    .into_iter()
                    .filter(|(_, kind)| replace(*kind)),
            );
        }
        let encoded = encode_events(source.header, body)?;
        chunks.push(midi_chunks(&encoded, 1)?.1[0].to_vec());
    }
    for (_, track) in generated_tracks {
        let encoded = encode_events(source.header, absolute(track)?)?;
        chunks.push(midi_chunks(&encoded, 1)?.1[0].to_vec());
    }
    let mut result = original[..header].to_vec();
    if chunks.len() > 1 {
        result[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    result[10..12].copy_from_slice(&u16::try_from(chunks.len())?.to_be_bytes());
    for chunk in chunks {
        result.extend(chunk);
    }
    validate_midi_complete(&result)?;
    Ok(result)
}

pub(crate) fn validate_percussion_scoring(
    source: &Smf<'_>,
    before: &HarmonyChart,
    after: &HarmonyChart,
) -> Result<()> {
    if before.phrases == after.phrases {
        return Ok(());
    }
    for track in &source.tracks {
        if part(track)?.is_none() {
            continue;
        }
        for (start, end) in crate::song_files::game_midi::harmony_percussion_ranges(track)? {
            if before
                .phrases
                .iter()
                .any(|p| p.start <= start && start < p.end && end <= p.end)
            {
                ensure!(
                    after
                        .phrases
                        .iter()
                        .any(|p| p.start <= start && start < p.end && end <= p.end),
                    "Edited harmony scoring phrases would displace retained percussion; source retained"
                );
            }
        }
    }
    Ok(())
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
    fn controlled_source() -> Vec<u8> {
        let chart = fixture();
        let bytes = append_midi(&original(&chart), &chart).unwrap();
        let midi = Smf::parse(&bytes).unwrap();
        let (header, chunks) = midi_chunks(&bytes, midi.tracks.len()).unwrap();
        let mut result = bytes[..header].to_vec();
        for (track, chunk) in midi.tracks.iter().zip(chunks) {
            if part(track).unwrap() != Some(Part::Harm2) {
                result.extend(chunk);
                continue;
            }
            let mut events = absolute(track).unwrap();
            for (_, kind) in &mut events {
                if matches!(kind, K::Meta(M::TrackName(_))) {
                    *kind = K::Meta(M::TrackName(b"PART HARM2"));
                }
                if let K::Midi { channel, message } = kind {
                    if matches!(message,N::NoteOn {key,..}|N::NoteOff {key,..} if key.as_int()==105)
                    {
                        *channel = u4::new(7);
                    }
                }
            }
            events.extend([
                (123, K::Meta(M::Text(b"[mood_play]"))),
                (
                    130,
                    K::Midi {
                        channel: u4::new(2),
                        message: N::Controller {
                            controller: u7::new(11),
                            value: u7::new(71),
                        },
                    },
                ),
                (
                    480,
                    K::Midi {
                        channel: u4::new(3),
                        message: N::NoteOn {
                            key: u7::new(96),
                            vel: u7::new(88),
                        },
                    },
                ),
                (
                    500,
                    K::Midi {
                        channel: u4::new(3),
                        message: N::NoteOff {
                            key: u7::new(96),
                            vel: u7::new(12),
                        },
                    },
                ),
                (
                    0,
                    K::Midi {
                        channel: u4::new(4),
                        message: N::NoteOn {
                            key: u7::new(116),
                            vel: u7::new(90),
                        },
                    },
                ),
                (
                    960,
                    K::Midi {
                        channel: u4::new(4),
                        message: N::NoteOff {
                            key: u7::new(116),
                            vel: u7::new(17),
                        },
                    },
                ),
            ]);
            let encoded = encode_events(midi.header, events).unwrap();
            result.extend(midi_chunks(&encoded, 1).unwrap().1[0]);
        }
        result
    }
    fn assert_other_chunks_identical(before: &[u8], after: &[u8], changed: Part) {
        let old = Smf::parse(before).unwrap();
        let new = Smf::parse(after).unwrap();
        let a = midi_chunks(before, old.tracks.len()).unwrap().1;
        let b = midi_chunks(after, new.tracks.len()).unwrap().1;
        assert_eq!(a.len(), b.len());
        for ((track, a), b) in old.tracks.iter().zip(a).zip(b) {
            if part(track).unwrap() != Some(changed) {
                assert_eq!(a, b);
            }
        }
    }
    fn project_command(store: &crate::editor::Store, action: crate::editor::Action) {
        store
            .apply(crate::editor::Request {
                expected_revision: store.load().unwrap().revision,
                actor: crate::editor::Actor {
                    kind: crate::editor::ActorKind::Human,
                    name: "Harmony interchange fixture".into(),
                },
                action,
            })
            .unwrap();
    }
    fn imported_project() -> (tempfile::TempDir, crate::editor::Store, Vec<u8>) {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song");
        std::fs::create_dir(&song).unwrap();
        let bytes = controlled_source();
        std::fs::write(song.join("notes.mid"), &bytes).unwrap();
        std::fs::write(
            song.join("song.ini"),
            "[song]\nname = Independent harmony interchange\nsong_length = 4000\n",
        )
        .unwrap();
        let store =
            crate::editor::Store::create(&song, None, &root.path().join("project")).unwrap();
        (root, store, bytes)
    }
    fn export_project(store: &crate::editor::Store, path: &std::path::Path) -> Vec<u8> {
        project_command(
            store,
            crate::editor::Action::Export {
                output: path.into(),
            },
        );
        std::fs::read(path.join("notes.mid")).unwrap()
    }
    fn raw_track(bytes: &[u8], name: &[u8]) -> Vec<u8> {
        let midi = Smf::parse(bytes).unwrap();
        let chunks = midi_chunks(bytes, midi.tracks.len()).unwrap().1;
        midi.tracks
            .iter()
            .zip(chunks)
            .find(|(events, _)| {
                events
                    .iter()
                    .any(|e| matches!(e.kind,K::Meta(M::TrackName(n)) if n==name))
            })
            .unwrap()
            .1
            .to_vec()
    }
    #[test]
    fn ordinary_midi_project_import_edit_export_and_plain_reimport_keep_independent_singers() {
        use crate::editor::{Action, Store};
        let (root, store, original) = imported_project();
        let doc = store.load().unwrap();
        assert!(
            !doc.review
                .coverage_limits
                .iter()
                .any(|warning| { warning.contains("HARM") && warning.contains("not editable") })
        );
        let parts = crate::export_options::available(&store, &doc).unwrap();
        assert!(parts.contains("Harmonies"));
        assert!(
            !parts
                .iter()
                .any(|p| p.starts_with("MIDI: HARM") || p.starts_with("MIDI: PART HARM"))
        );
        let lead = doc.content.vocals.clone();
        let mut harmonies = doc.content.harmonies.unwrap();
        let note = &mut harmonies.voices.get_mut(&Part::Harm2).unwrap().notes[0];
        note.start += 1;
        note.tone = VocalTone::Pitched { midi: 65 };
        note.lyric = crate::vocals::Syllable::Missing;
        project_command(
            &store,
            Action::SetHarmonies {
                document: harmonies.clone(),
            },
        );
        let output = root.path().join("export");
        let bytes = export_project(&store, &output);
        assert!(
            std::fs::read_to_string(output.join("song.ini"))
                .unwrap()
                .contains("diff_vocals_harm = 0")
        );
        assert_other_chunks_identical(&original, &bytes, Part::Harm2);
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(
            reopened.load().unwrap().content.harmonies,
            Some(harmonies.clone())
        );
        assert_eq!(reopened.load().unwrap().content.vocals, lead);
        let plain = root.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        for name in ["notes.mid", "song.ini"] {
            std::fs::copy(output.join(name), plain.join(name)).unwrap();
        }
        let imported = Store::create(&plain, None, &root.path().join("plain-project")).unwrap();
        let actual = imported.load().unwrap().content.harmonies.unwrap();
        assert!(same_content(&actual, &harmonies));
        assert_eq!(
            raw_track(&bytes, b"PART VOCALS"),
            raw_track(&original, b"PART VOCALS")
        );
    }
    #[test]
    fn clearing_all_harmony_notes_keeps_controls_without_restoring_singers_after_reopen() {
        use crate::editor::{Action, Store};
        let (root, store, original) = imported_project();
        project_command(&store, Action::ClearHarmonies);
        assert!(store.load().unwrap().content.harmonies.is_none());
        let retained =
            crate::song_files::game_midi::retained_harmony_phrases(&store.root.join("source"))
                .unwrap()
                .unwrap();
        let baseline = import_midi(&original).unwrap().unwrap();
        assert_eq!(retained.scoring, baseline.phrases);
        assert_eq!(retained.lyrics, baseline.lyric_phrases);
        assert!(
            crate::export_options::available(&store, &store.load().unwrap())
                .unwrap()
                .contains("Harmonies")
        );
        let output = root.path().join("cleared");
        let bytes = export_project(&store, &output);
        assert!(import_midi(&bytes).unwrap().is_none());
        let midi = Smf::parse(&bytes).unwrap();
        for track in &midi.tracks {
            if let Some(p) = part(track).unwrap() {
                assert!(track.iter().all(|e| !note_or_lyric(e.kind)));
                if p == Part::Harm2 {
                    assert!(
                        track
                            .iter()
                            .any(|e| matches!(e.kind, K::Meta(M::Text(b"[mood_play]"))))
                    );
                    for key in [96, 116] {
                        assert!(track.iter().any(|e|matches!(e.kind,K::Midi{message:N::NoteOn {key:k,vel},..} if k.as_int()==key&&vel.as_int()>0)));
                    }
                }
            }
        }
        assert_eq!(
            raw_track(&original, b"PART VOCALS"),
            raw_track(&bytes, b"PART VOCALS")
        );
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert!(reopened.load().unwrap().content.harmonies.is_none());
        assert_eq!(export_project(&reopened, &root.path().join("again")), bytes);
        project_command(&store, Action::Undo);
        assert!(store.load().unwrap().content.harmonies.is_some());
    }
    #[test]
    fn explicit_harmony_omission_is_durable_when_new_notes_are_authored_later() {
        use crate::{
            editor::{Action, Actor, ActorKind, Store},
            export_options::{self, Options},
        };
        let (root, store, original) = imported_project();
        let doc = store.load().unwrap();
        let mut authored = doc.content.harmonies.clone().unwrap();
        authored.voices.get_mut(&Part::Harm2).unwrap().notes[0].tone =
            VocalTone::Pitched { midi: 66 };
        let selected = export_options::available(&store, &doc)
            .unwrap()
            .into_iter()
            .filter(|part| part != "Harmonies")
            .collect();
        let output = root.path().join("omitted");
        store
            .export_with_options(
                &doc,
                &output,
                &Actor {
                    kind: ActorKind::Human,
                    name: "Explicit harmony omission".into(),
                },
                &Options {
                    tracks: Some(selected),
                    ..Default::default()
                },
            )
            .unwrap();
        let bytes = std::fs::read(output.join("notes.mid")).unwrap();
        assert!(
            std::fs::read_to_string(output.join("song.ini"))
                .unwrap()
                .contains("diff_vocals_harm = -1")
        );
        assert!(
            Smf::parse(&bytes)
                .unwrap()
                .tracks
                .iter()
                .all(|t| part(t).unwrap().is_none())
        );
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert!(reopened.load().unwrap().content.harmonies.is_none());
        assert!(
            crate::song_files::game_midi::retained_harmony_phrases(&reopened.root.join("source"))
                .unwrap()
                .is_none()
        );
        assert!(
            !export_options::available(&reopened, &reopened.load().unwrap())
                .unwrap()
                .iter()
                .any(|p| p == "Harmonies" || p.contains("HARM"))
        );
        project_command(
            &reopened,
            Action::SetHarmonies {
                document: authored.clone(),
            },
        );
        let new_output = root.path().join("new-notes");
        let bytes = export_project(&reopened, &new_output);
        assert!(same_content(
            &import_midi(&bytes).unwrap().unwrap(),
            &authored
        ));
        for track in &Smf::parse(&bytes).unwrap().tracks {
            if part(track).unwrap().is_some() {
                assert!(
                    !track
                        .iter()
                        .any(|e| matches!(e.kind, K::Meta(M::Text(b"[mood_play]"))))
                );
                assert!(!track.iter().any(|e|matches!(e.kind,K::Midi{message:N::NoteOn{key,vel},..} if matches!(key.as_int(),96|116)&&vel.as_int()>0)));
            }
        }
        assert_eq!(
            raw_track(&original, b"PART VOCALS"),
            raw_track(&bytes, b"PART VOCALS")
        );
        let again = Store::create(&new_output, None, &root.path().join("again-project")).unwrap();
        assert_eq!(
            export_project(&again, &root.path().join("again-export")),
            bytes
        );
    }
    #[test]
    fn altered_imported_harmony_baseline_refuses_export_without_mutating_project() {
        let (root, store, _) = imported_project();
        let doc = store.load().unwrap();
        let project = std::fs::read(store.root.join("project.json")).unwrap();
        let baseline = store
            .root
            .join("source/.yace/originals/game-midi/harmonies.json");
        let mut bytes = std::fs::read(&baseline).unwrap();
        bytes.push(b' ');
        std::fs::write(&baseline, bytes).unwrap();
        let output = root.path().join("refused");
        let error = store
            .export_with_options(
                &doc,
                &output,
                &crate::editor::Actor {
                    kind: crate::editor::ActorKind::Human,
                    name: "Altered baseline refusal fixture".into(),
                },
                &Default::default(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("harmony baseline changed"));
        assert!(!output.exists());
        assert_eq!(
            std::fs::read(store.root.join("project.json")).unwrap(),
            project
        );
    }
    #[test]
    fn phrase_split_cannot_cut_a_retained_percussion_span_and_refused_command_is_atomic() {
        use crate::{
            editor::{Action, Actor, ActorKind, Request},
            vocal_harmonies::{Edit, PhraseKind},
        };
        let (_root, store, _) = imported_project();
        let mut chart = store.load().unwrap().content.harmonies.unwrap();
        for voice in chart.voices.values_mut() {
            for note in &mut voice.notes {
                note.start = 100;
                note.end = 200;
            }
        }
        let mut later = chart.voices[&Part::Harm1].notes[0].clone();
        later.start = 600;
        later.end = 800;
        chart
            .voices
            .get_mut(&Part::Harm1)
            .unwrap()
            .notes
            .push(later);
        project_command(
            &store,
            Action::SetHarmonies {
                document: chart.clone(),
            },
        );
        let path = store.root.join("project.json");
        let original = std::fs::read(&path).unwrap();
        let request = |tick| Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Retained percussion split guard".into(),
            },
            action: Action::EditHarmonies {
                edit: Edit::SplitPhrase {
                    kind: PhraseKind::Scoring,
                    index: 0,
                    expected: chart.phrases[0].clone(),
                    tick,
                },
            },
        };
        let error = store.apply(request(490)).unwrap_err();
        assert!(error.to_string().contains("retained percussion"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        store.apply(request(500)).unwrap();
        let after = store.load().unwrap().content.harmonies.unwrap();
        assert_eq!(after.voices, chart.voices);
        assert_eq!(after.phrases.len(), 2);
    }
    #[test]
    fn source_aware_edits_keep_other_singers_lead_alias_and_unedited_controls() {
        let original = controlled_source();
        let before = import_midi(&original).unwrap().unwrap();
        assert_eq!(rewrite_midi(&original, &before, &before).unwrap(), original);
        let mut after = before.clone();
        let note = &mut after.voices.get_mut(&Part::Harm2).unwrap().notes[0];
        note.start += 1;
        note.tone = VocalTone::Pitched { midi: 65 };
        note.lyric = crate::vocals::Syllable::Missing;
        let output = rewrite_midi(&original, &before, &after).unwrap();
        assert_other_chunks_identical(&original, &output, Part::Harm2);
        let midi = Smf::parse(&original).unwrap();
        let edited = Smf::parse(&output).unwrap();
        let controls = |track| {
            absolute(track)
                .unwrap()
                .into_iter()
                .filter(|(_, kind)| !note_or_lyric(*kind))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            controls(
                midi.tracks
                    .iter()
                    .find(|t| part(t).unwrap() == Some(Part::Harm2))
                    .unwrap()
            ),
            controls(
                edited
                    .tracks
                    .iter()
                    .find(|t| part(t).unwrap() == Some(Part::Harm2))
                    .unwrap()
            )
        );
        assert_eq!(
            import_midi(&output).unwrap().unwrap().voices[&Part::Harm2].notes,
            after.voices[&Part::Harm2].notes
        );
    }
    #[test]
    fn shared_lyric_edits_rewrite_only_the_harm2_owner_and_empty_singer_retains_controls() {
        let original = controlled_source();
        let before = import_midi(&original).unwrap().unwrap();
        let mut after = before.clone();
        after.lyric_phrases[0].end = 1000;
        after.voices.remove(&Part::Harm2);
        let output = rewrite_midi(&original, &before, &after).unwrap();
        assert_other_chunks_identical(&original, &output, Part::Harm2);
        let reopened = import_midi(&output).unwrap().unwrap();
        assert!(reopened.voices[&Part::Harm2].notes.is_empty());
        assert_eq!(reopened.lyric_phrases, after.lyric_phrases);
        let midi = Smf::parse(&output).unwrap();
        assert!(midi.tracks.iter().filter(|t|part(t).unwrap()==Some(Part::Harm2)).flatten().any(|e|
            matches!(e.kind,K::Midi{message:N::NoteOn{key,vel},..} if key.as_int()==96&&vel.as_int()==88)));
    }
    #[test]
    fn stale_harmony_baseline_and_displaced_percussion_refuse_without_a_rewrite() {
        let original = controlled_source();
        let before = import_midi(&original).unwrap().unwrap();
        let mut stale = before.clone();
        stale.voices.get_mut(&Part::Harm1).unwrap().notes[0].start += 1;
        assert!(
            rewrite_midi(&original, &stale, &before)
                .unwrap_err()
                .to_string()
                .contains("baseline differs")
        );
        let mut after = before.clone();
        after.phrases[0].start = 600;
        for voice in after.voices.values_mut() {
            for note in &mut voice.notes {
                note.start = 700;
                note.end = 800;
            }
        }
        assert!(
            rewrite_midi(&original, &before, &after)
                .unwrap_err()
                .to_string()
                .contains("retained percussion")
        );
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
