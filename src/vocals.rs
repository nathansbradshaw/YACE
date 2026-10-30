//! Editable, model-free lead-vocal charts. MIDI support is a deliberately strict
//! subset of YARG's PART VOCALS format, not a lossless general MIDI editor.
use anyhow::{Context, Result, bail, ensure};
use midly::{
    Format, MetaMessage, MidiMessage, Smf, TrackEvent, TrackEventKind,
    num::{u4, u7, u28},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// Standard MIDI sequencer-specific metadata; YARG ignores this non-text event.
// Its paired explicit empty lyric distinguishes our known-word continuation
// from external MIDI with missing/unknown syllables. Never infer an owner.
const CONTINUE_WORD_MARKER: &[u8] = b"YACE\0continue-word\0v1";
const MISSING_LYRIC_MARKER: &[u8] = b"YACE\0missing-lyric\0v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalChart {
    pub version: u32,
    pub resolution: u32,
    /// Human-readable origin; never a claim of listening acceptance.
    pub source: String,
    pub phrases: Vec<VocalPhrase>,
    pub notes: Vec<VocalNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalPhrase {
    pub start: u64,
    pub end: u64,
    /// MIDI phrase marker 105 (player 1) or 106 (player 2).
    pub player: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalNote {
    pub start: u64,
    pub end: u64,
    pub tone: VocalTone,
    pub lyric: Syllable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VocalTone {
    Pitched { midi: u8 },
    Talkie { lenient: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Syllable {
    /// Measured pitch with no established word; never a slide or a talkie.
    Missing,
    Text {
        text: String,
        join_next: bool,
    },
    /// A second pitch on the preceding syllable (YARG's '+' slide marker).
    Continue,
    /// Another measured note belonging to the preceding known word, without
    /// asserting a connecting pitch or a new attack across their time gap.
    ContinueWord,
}

impl VocalChart {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported vocal document version");
        ensure!(
            (1..=32767).contains(&self.resolution),
            "Vocal MIDI resolution must be 1..32767"
        );
        ensure!(
            !self.source.trim().is_empty(),
            "Vocal source/provenance is required"
        );
        ensure!(
            !self.phrases.is_empty() && !self.notes.is_empty(),
            "Vocal chart needs notes and scoring phrases; lyrics alone are not vocals"
        );
        let span = |start: u64, end: u64| -> Result<()> {
            ensure!(
                start < end && end <= u32::MAX as u64,
                "Invalid/overflowing vocal span {start}..{end}"
            );
            Ok(())
        };
        for phrase in &self.phrases {
            span(phrase.start, phrase.end)?;
            ensure!(
                matches!(phrase.player, 1 | 2),
                "Vocal phrase player must be 1 or 2"
            );
        }
        ensure!(
            self.phrases.windows(2).all(|p| p[0].end <= p[1].start),
            "Vocal phrases must be ordered and non-overlapping"
        );
        ensure!(
            self.notes.windows(2).all(|n| n[0].end <= n[1].start),
            "Lead vocal notes must be ordered and monophonic; put harmonies in separate parts"
        );
        for note in &self.notes {
            span(note.start, note.end)?;
            if let VocalTone::Pitched { midi } = note.tone {
                ensure!(
                    (36..=84).contains(&midi),
                    "YARG vocal pitch must be MIDI 36..84; do not silently transpose"
                );
            }
            if let Syllable::Text { text, .. } = &note.lyric {
                ensure!(
                    !text.is_empty()
                        && !text.chars().any(|c| c.is_control()
                            || c.is_whitespace()
                            || "-#^*+%=/$§_\"<>[]".contains(c)),
                    "Syllable must be nonempty plain text without whitespace or game-control symbols; use explicit join/talkie/continue fields"
                );
            }
            if note.lyric == Syllable::Missing {
                ensure!(
                    matches!(note.tone, VocalTone::Pitched { .. }),
                    "A missing lyric needs a pitched note"
                );
            }
            ensure!(
                self.phrases
                    .iter()
                    .filter(|p| p.start <= note.start && note.end <= p.end)
                    .count()
                    == 1,
                "Every vocal note must fit wholly inside one scoring phrase"
            );
        }
        for phrase in &self.phrases {
            let notes: Vec<_> = self
                .notes
                .iter()
                .filter(|n| phrase.start <= n.start && n.end <= phrase.end)
                .collect();
            ensure!(!notes.is_empty(), "Empty scoring phrase");
            for (index, note) in notes.iter().enumerate() {
                if matches!(note.lyric, Syllable::Continue | Syllable::ContinueWord) {
                    ensure!(
                        index > 0
                            && matches!(note.tone, VocalTone::Pitched { .. })
                            && matches!(notes[index - 1].tone, VocalTone::Pitched { .. }),
                        "Continuation needs a preceding pitched syllable in the same phrase"
                    );
                    ensure!(
                        notes[..index]
                            .iter()
                            .rev()
                            .find(|n| !matches!(
                                n.lyric,
                                Syllable::Continue | Syllable::ContinueWord
                            ))
                            .is_some_and(|n| matches!(n.lyric, Syllable::Text { .. })),
                        "Continuation needs a known preceding syllable"
                    );
                }
                if matches!(
                    &note.lyric,
                    Syllable::Text {
                        join_next: true,
                        ..
                    }
                ) {
                    ensure!(
                        notes[index + 1..]
                            .iter()
                            .any(|n| matches!(n.lyric, Syllable::Text { .. })),
                        "Joined syllable needs a following text syllable in the same phrase"
                    );
                }
            }
        }
        Ok(())
    }
}

fn track_name<'a>(track: &'a [TrackEvent<'a>]) -> Option<&'a str> {
    track.iter().find_map(|event| match event.kind {
        TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => std::str::from_utf8(bytes).ok(),
        _ => None,
    })
}

fn vocal_label(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "PART VOCALS" | "HARM1" | "HARM2" | "HARM3" | "PART HARM1" | "PART HARM2" | "PART HARM3"
    )
}

fn marker(note: &VocalNote) -> String {
    match &note.lyric {
        Syllable::Missing => String::new(),
        Syllable::Continue => "+".into(),
        Syllable::ContinueWord => String::new(),
        Syllable::Text { text, join_next } => {
            let mut text = text.clone();
            if *join_next {
                text.push('-');
            }
            if let VocalTone::Talkie { lenient } = note.tone {
                text.push(if lenient { '^' } else { '#' });
            }
            text
        }
    }
}

/// Append a new lead vocal track. Every original track chunk is preserved.
/// Existing vocal/harmony tracks are refused, including empty tracks, rather
/// than overwritten or accidentally changing authored vocal interpretation.
pub fn append_midi(original: &[u8], vocals: &VocalChart) -> Result<Vec<u8>> {
    vocals.validate()?;
    let parsed = Smf::parse(original).context("Invalid source MIDI")?;
    validate_chunks(original, parsed.tracks.len())?;
    let map = crate::midi::timing(original)?;
    ensure!(
        map.resolution == vocals.resolution,
        "Vocal/source MIDI resolutions differ"
    );
    ensure!(
        !parsed
            .tracks
            .iter()
            .flatten()
            .any(|event| match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(name)) =>
                    std::str::from_utf8(name).is_ok_and(vocal_label),
                _ => false,
            }),
        "Source contains authored vocals/harmonies; preserve it and edit explicitly"
    );
    ensure!(
        parsed.tracks.len() < u16::MAX as usize,
        "Too many MIDI tracks"
    );
    // Buffer text before building borrowed midly events.
    let lyrics: Vec<_> = vocals.notes.iter().map(marker).collect();
    let mut events = vec![(
        0,
        1,
        TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS")),
    )];
    let mut push_note = |start: u64, end: u64, key: u8| {
        events.push((
            start,
            3,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(key),
                    vel: u7::new(100),
                },
            },
        ));
        events.push((
            end,
            0,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key: u7::new(key),
                    vel: u7::new(0),
                },
            },
        ));
    };
    for phrase in &vocals.phrases {
        push_note(phrase.start, phrase.end, 104 + phrase.player);
    }
    for note in &vocals.notes {
        let key = match note.tone {
            VocalTone::Pitched { midi } => midi,
            VocalTone::Talkie { .. } => 60,
        };
        push_note(note.start, note.end, key);
    }
    for (note, lyric) in vocals.notes.iter().zip(&lyrics) {
        let metadata = match note.lyric {
            Syllable::ContinueWord => Some(CONTINUE_WORD_MARKER),
            Syllable::Missing => Some(MISSING_LYRIC_MARKER),
            _ => None,
        };
        if let Some(metadata) = metadata {
            events.push((
                note.start,
                2,
                TrackEventKind::Meta(MetaMessage::SequencerSpecific(metadata)),
            ));
        }
        events.push((
            note.start,
            2,
            TrackEventKind::Meta(MetaMessage::Lyric(lyric.as_bytes())),
        ));
    }
    events.sort_by_key(|(tick, priority, _)| (*tick, *priority));
    let mut previous = 0;
    let mut track = Vec::new();
    for (tick, _, kind) in events {
        let delta = tick - previous;
        ensure!(
            delta <= 0x0fff_ffff,
            "Vocal MIDI event gap exceeds MIDI delta range"
        );
        track.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let one = Smf {
        header: parsed.header,
        tracks: vec![track],
    };
    let mut encoded = Vec::new();
    one.write_std(&mut encoded)?;
    let mut output = original.to_vec();
    output[8..10].copy_from_slice(&1u16.to_be_bytes());
    output[10..12].copy_from_slice(&((parsed.tracks.len() + 1) as u16).to_be_bytes());
    output.extend_from_slice(&encoded[14..]);
    Ok(output)
}

// Appending after an ignored/truncated chunk or trailing bytes would produce a
// file that different parsers interpret differently. Refuse such a source.
fn validate_chunks(bytes: &[u8], expected: usize) -> Result<()> {
    ensure!(
        bytes.len() >= 14 && &bytes[..4] == b"MThd",
        "Invalid MIDI header"
    );
    let header = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(header >= 6, "Truncated MIDI header");
    let mut position = 8usize.checked_add(header).context("MIDI size overflow")?;
    ensure!(position <= bytes.len(), "Truncated MIDI header");
    let mut count = 0;
    while position < bytes.len() {
        ensure!(
            bytes.len() - position >= 8 && &bytes[position..position + 4] == b"MTrk",
            "Unsupported MIDI chunk/trailing bytes; retain source MIDI"
        );
        let size = u32::from_be_bytes(bytes[position + 4..position + 8].try_into()?) as usize;
        position = position
            .checked_add(8)
            .and_then(|n| n.checked_add(size))
            .context("MIDI size overflow")?;
        ensure!(position <= bytes.len(), "Truncated MIDI track chunk");
        count += 1;
    }
    ensure!(
        count == expected && count == u16::from_be_bytes(bytes[10..12].try_into()?) as usize,
        "MIDI track count mismatch"
    );
    Ok(())
}

/// Import only the supported lead-vocal subset. Unsupported markers, orphan
/// lyrics and extra events cause an error, never silent destructive conversion.
/// Keep the source MIDI alongside the editable document for recovery.
pub fn import_midi(bytes: &[u8]) -> Result<VocalChart> {
    let parsed = Smf::parse(bytes)?;
    validate_chunks(bytes, parsed.tracks.len())?;
    let map = crate::midi::timing(bytes)?;
    ensure!(
        parsed.header.format != Format::Sequential,
        "Sequential MIDI unsupported"
    );
    let tracks: Vec<_> = parsed
        .tracks
        .iter()
        .filter(|t| track_name(t) == Some("PART VOCALS"))
        .collect();
    ensure!(tracks.len() == 1, "Expected exactly one PART VOCALS track");
    let mut out = VocalChart {
        version: 1,
        resolution: map.resolution,
        source: "Imported PART VOCALS MIDI; source MIDI retained separately".into(),
        phrases: Vec::new(),
        notes: Vec::new(),
    };
    let mut tick = 0u64;
    let mut active = BTreeMap::new();
    let mut lyrics = BTreeMap::new();
    let mut continue_words = BTreeSet::new();
    let mut missing_lyrics = BTreeSet::new();
    let mut notes = Vec::new();
    for event in tracks[0] {
        tick = tick
            .checked_add(event.delta.as_int() as u64)
            .context("MIDI tick overflow")?;
        match event.kind {
            TrackEventKind::Meta(MetaMessage::SequencerSpecific(data))
                if data == MISSING_LYRIC_MARKER =>
            {
                ensure!(
                    missing_lyrics.insert(tick),
                    "Duplicate missing-lyric marker"
                );
            }
            TrackEventKind::Meta(MetaMessage::TrackName(_) | MetaMessage::EndOfTrack) => {}
            TrackEventKind::Meta(MetaMessage::SequencerSpecific(data))
                if data == CONTINUE_WORD_MARKER =>
            {
                ensure!(
                    continue_words.insert(tick),
                    "Duplicate known-word continuation marker"
                );
            }
            TrackEventKind::Meta(MetaMessage::Lyric(text) | MetaMessage::Text(text)) => {
                let text = std::str::from_utf8(text)
                    .context("Vocal lyric must be UTF-8")?
                    .to_owned();
                ensure!(
                    lyrics.insert(tick, text).is_none(),
                    "Multiple vocal lyrics at tick {tick}"
                );
            }
            TrackEventKind::Midi { channel, message } => match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    let key = key.as_int();
                    ensure!(
                        matches!(key, 36..=84 | 105 | 106),
                        "Unsupported vocal MIDI note {key}; retain source MIDI"
                    );
                    ensure!(
                        active.insert((channel.as_int(), key), tick).is_none(),
                        "Overlapping same-key vocal MIDI notes"
                    );
                }
                MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                    let key = key.as_int();
                    let start = active
                        .remove(&(channel.as_int(), key))
                        .context("Unmatched vocal note-off")?;
                    ensure!(start < tick, "Zero-length vocal event");
                    if key == 105 || key == 106 {
                        out.phrases.push(VocalPhrase {
                            start,
                            end: tick,
                            player: key - 104,
                        });
                    } else {
                        notes.push((start, tick, key));
                    }
                }
                _ => bail!("Unsupported vocal MIDI message; retain source MIDI"),
            },
            _ => bail!("Unsupported vocal MIDI event; retain source MIDI"),
        }
    }
    ensure!(active.is_empty(), "Unterminated vocal MIDI notes");
    notes.sort_unstable();
    out.phrases.sort_by_key(|p| p.start);
    for (start, end, midi) in notes {
        let mut text = lyrics
            .remove(&start)
            .context("Vocal note lacks an exactly aligned syllable/continuation")?;
        let (lyric, tone) = if missing_lyrics.remove(&start) {
            ensure!(
                text.is_empty() && !continue_words.contains(&start),
                "Missing lyric marker requires an empty lyric and no word owner"
            );
            (Syllable::Missing, VocalTone::Pitched { midi })
        } else if continue_words.remove(&start) {
            ensure!(
                text.is_empty(),
                "Known-word continuation marker requires its explicit empty lyric"
            );
            (Syllable::ContinueWord, VocalTone::Pitched { midi })
        } else if text == "+" {
            (Syllable::Continue, VocalTone::Pitched { midi })
        } else {
            let tone = if text.ends_with('#') || text.ends_with('^') {
                let lenient = text.pop() == Some('^');
                VocalTone::Talkie { lenient }
            } else {
                VocalTone::Pitched { midi }
            };
            let join_next = text.ends_with('-');
            if join_next {
                text.pop();
            }
            (Syllable::Text { text, join_next }, tone)
        };
        out.notes.push(VocalNote {
            start,
            end,
            tone,
            lyric,
        });
    }
    ensure!(
        lyrics.is_empty(),
        "Orphan/unaligned lyrics cannot be imported as playable vocals"
    );
    ensure!(
        continue_words.is_empty(),
        "Orphan known-word continuation marker"
    );
    ensure!(missing_lyrics.is_empty(), "Orphan missing-lyric marker");
    out.validate()?;
    Ok(out)
}
