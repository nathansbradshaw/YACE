use crate::{
    chart::Tracks,
    mapping::{DrumHit, PitchNote, Transcription, Voice},
    timing::{Tempo, TempoMap},
};
use anyhow::{Context, Result, ensure};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
    num::{u4, u7, u15, u24, u28},
};
use std::collections::{BTreeMap, BTreeSet};

fn name<'a>(track: &'a [TrackEvent<'a>]) -> Option<&'a str> {
    track.iter().find_map(|e| match e.kind {
        TrackEventKind::Meta(MetaMessage::TrackName(n)) => std::str::from_utf8(n).ok(),
        _ => None,
    })
}

pub fn timing(bytes: &[u8]) -> Result<TempoMap> {
    let smf = Smf::parse(bytes).context("Invalid MIDI")?;
    ensure!(
        smf.header.format != Format::Sequential,
        "Asynchronous MIDI format 2 is unsupported"
    );
    let Timing::Metrical(resolution) = smf.header.timing else {
        anyhow::bail!("SMPTE MIDI timing is unsupported");
    };
    let mut tempos = BTreeMap::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for event in track {
            tick += event.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = event.kind {
                ensure!(us.as_int() > 0, "Zero MIDI tempo");
                if let Some(previous) = tempos.insert(tick, us.as_int()) {
                    ensure!(previous == us.as_int(), "Conflicting MIDI tempos");
                }
            }
        }
    }
    TempoMap::new(
        resolution.as_int() as u32,
        0.0,
        tempos
            .into_iter()
            .map(|(tick, us)| Tempo {
                tick,
                bpm: 60_000_000.0 / us as f64,
            })
            .collect(),
    )
}

pub fn import(bytes: &[u8]) -> Result<Transcription> {
    let smf = Smf::parse(bytes)?;
    let map = timing(bytes)?;
    let mut out = Transcription::default();
    for track in &smf.tracks {
        let label = name(track).unwrap_or("").to_ascii_lowercase();
        ensure!(
            !label.starts_with("part "),
            "This appears to be game-chart MIDI, not a General MIDI performance; use enhance for game charts"
        );
        let bass = label.contains("bass");
        let piano = label.contains("piano") || label.contains("keys") || label.contains("keyboard");
        let mut programs = [None; 16];
        let mut active = BTreeMap::new();
        let mut tick = 0u64;
        for event in track {
            tick += event.delta.as_int() as u64;
            if let TrackEventKind::Midi { channel, message } = event.kind {
                match message {
                    MidiMessage::ProgramChange { program } => {
                        programs[channel.as_int() as usize] = Some(program.as_int());
                    }
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        if channel.as_int() == 9 {
                            if let Some(voice) = Voice::from_gm(key.as_int()) {
                                out.drums.push(DrumHit {
                                    time: map.seconds(tick),
                                    voice,
                                    confidence: vel.as_int() as f32 / 127.0,
                                });
                            }
                        } else {
                            let piano = piano
                                || programs[channel.as_int() as usize].is_some_and(|p| p <= 7);
                            active.insert(
                                (channel.as_int(), key.as_int()),
                                (tick, vel.as_int(), piano),
                            );
                        }
                    }
                    MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, vel: _ } => {
                        if let Some((start, velocity, piano)) =
                            active.remove(&(channel.as_int(), key.as_int()))
                            && tick > start
                        {
                            let note = PitchNote {
                                start: map.seconds(start),
                                end: map.seconds(tick),
                                pitch: key.as_int(),
                                confidence: velocity as f32 / 127.0,
                            };
                            if bass {
                                out.bass.push(note);
                            } else if piano {
                                out.pro_piano.push(note.clone());
                                out.piano.push(note);
                            } else {
                                out.guitar.push(note);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        ensure!(
            active.is_empty(),
            "Unterminated notes in performance MIDI track {label}"
        );
    }
    out.validate()?;
    Ok(out)
}

fn events_from_absolute(
    mut events: Vec<(u64, TrackEventKind<'static>)>,
) -> Result<Vec<TrackEvent<'static>>> {
    events.sort_by_key(|(tick, kind)| {
        (
            *tick,
            match kind {
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOff { .. },
                    ..
                } => 0,
                _ => 1,
            },
        )
    });
    let mut previous = 0;
    let mut out = Vec::new();
    for (tick, kind) in events {
        let delta = tick - previous;
        ensure!(delta <= 0x0fff_ffff, "MIDI delta exceeds supported range");
        out.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    out.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(out)
}

fn instrument_tracks(
    tracks: &Tracks,
    resolution: u32,
) -> Result<Vec<(&'static str, Vec<TrackEvent<'static>>)>> {
    let mut out = Vec::new();
    for (suffix, label) in [
        ("Single", "PART GUITAR"),
        ("DoubleBass", "PART BASS"),
        ("Drums", "PART DRUMS"),
        ("Keyboard", "PART KEYS"),
    ] {
        let mut notes = BTreeSet::new();
        for (difficulty, base) in [("Easy", 60u8), ("Medium", 72), ("Hard", 84), ("Expert", 96)] {
            let Some(group) = tracks.get(&format!("{difficulty}{suffix}")) else {
                continue;
            };
            let cymbals: BTreeSet<_> = group
                .iter()
                .filter(|n| (66..=68).contains(&n.lane))
                .map(|n| (n.tick, n.lane - 64))
                .collect();
            for n in group.iter().filter(|n| n.lane <= 4) {
                let duration = n.sustain.max(1);
                notes.insert((n.tick, base + n.lane, duration));
                if suffix == "Drums" && n.lane >= 2 && !cymbals.contains(&(n.tick, n.lane)) {
                    notes.insert((n.tick, 108 + n.lane, (resolution as u64 / 8).max(1)));
                }
            }
        }
        if notes.is_empty() {
            continue;
        }
        let mut events = vec![(
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(label.as_bytes())),
        )];
        for (tick, key, duration) in notes {
            events.push((
                tick,
                TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOn {
                        key: u7::new(key),
                        vel: u7::new(100),
                    },
                },
            ));
            events.push((
                tick.checked_add(duration).context("MIDI tick overflow")?,
                TrackEventKind::Midi {
                    channel: u4::new(0),
                    message: MidiMessage::NoteOff {
                        key: u7::new(key),
                        vel: u7::new(0),
                    },
                },
            ));
        }
        out.push((label, events_from_absolute(events)?));
    }
    for difficulty in ["Easy", "Medium", "Hard", "Expert"] {
        let track_name = format!("{difficulty}ProKeys");
        if let Some(notes) = tracks.get(&track_name) {
            if notes.is_empty() {
                continue;
            }
            // Reuse the strict chart-to-MIDI encoder for actual pitches and
            // range markers; never run Pro Keys through the five-fret encoder.
            let map = TempoMap::new(resolution, 0., vec![])?;
            let chart = crate::chart::Chart::parse(crate::chart::new_chart(
                "",
                "",
                &map,
                &BTreeMap::from([(track_name.clone(), notes.clone())]),
            ))?;
            let bytes = crate::game_export::chart_to_midi(&chart)?;
            let smf = Smf::parse(&bytes)?;
            let label = crate::keys::midi_track(&track_name).unwrap();
            let track = smf
                .tracks
                .into_iter()
                .find(|t| name(t) == Some(label))
                .context("Missing Pro Keys MIDI track")?;
            out.push((
                label,
                track
                    .into_iter()
                    .map(|e| {
                        let mut owned = e.to_static();
                        // midly deliberately blanks borrowed meta text in to_static.
                        // Restore the known static game track label after detaching.
                        if matches!(e.kind, TrackEventKind::Meta(MetaMessage::TrackName(_))) {
                            owned.kind =
                                TrackEventKind::Meta(MetaMessage::TrackName(label.as_bytes()));
                        }
                        owned
                    })
                    .collect(),
            ));
        }
    }
    Ok(out)
}

pub fn new_midi(map: &TempoMap, tracks: &Tracks) -> Result<Vec<u8>> {
    ensure!(map.resolution <= 32767, "MIDI resolution too large");
    ensure!(
        map.offset == 0.0,
        "MIDI export cannot represent .chart Offset"
    );
    let mut tempo = vec![(
        0,
        TrackEventKind::Meta(MetaMessage::TimeSignature(4, 2, 24, 8)),
    )];
    for t in &map.tempos {
        let us = (60_000_000.0 / t.bpm).round() as u32;
        ensure!(us <= 0xff_ffff, "Tempo cannot be represented in MIDI");
        tempo.push((
            t.tick,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us))),
        ));
    }
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(map.resolution as u16)),
    ));
    smf.tracks.push(events_from_absolute(tempo)?);
    for (_, track) in instrument_tracks(tracks, map.resolution)? {
        smf.tracks.push(track);
    }
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)?;
    Ok(bytes)
}

/// Original MIDI track chunks stay byte-for-byte unchanged. Only missing
/// instruments are appended, avoiding global tom-marker changes to authored
/// difficulties in an existing PART DRUMS track.
pub fn enhance(bytes: &[u8], generated: &Tracks) -> Result<(Vec<u8>, Vec<String>)> {
    let smf = Smf::parse(bytes)?;
    let map = timing(bytes)?;
    let names: BTreeSet<_> = smf.tracks.iter().filter_map(|t| name(t)).collect();
    let additions: Vec<_> = instrument_tracks(generated, map.resolution)?
        .into_iter()
        .filter(|(n, _)| !names.contains(n))
        .collect();
    let count = smf.tracks.len() + additions.len();
    ensure!(count <= u16::MAX as usize, "Too many MIDI tracks");
    let mut out = bytes.to_vec();
    ensure!(
        out.len() >= 14 && &out[..4] == b"MThd",
        "Invalid MIDI header"
    );
    if !additions.is_empty() {
        out[8..10].copy_from_slice(&1u16.to_be_bytes());
        out[10..12].copy_from_slice(&(count as u16).to_be_bytes());
    }
    let mut added = Vec::new();
    for (label, track) in additions {
        let one = Smf {
            header: Header::new(
                Format::Parallel,
                Timing::Metrical(u15::new(map.resolution as u16)),
            ),
            tracks: vec![track],
        };
        let mut data = Vec::new();
        one.write_std(&mut data)?;
        out.extend_from_slice(&data[14..]);
        added.push(label.to_owned());
    }
    Ok((out, added))
}

pub fn inventory(bytes: &[u8]) -> Result<Vec<(String, usize)>> {
    let smf = Smf::parse(bytes)?;
    Ok(smf.tracks.iter().map(|track| {
        let label=name(track).unwrap_or("(unnamed)");
        let game=label.starts_with("PART ");
        let vocal=matches!(label,"PART VOCALS"|"HARM1"|"HARM2"|"HARM3");
        let pro_keys=label.starts_with("PART REAL_KEYS_");
        let notes=track.iter().filter(|event| matches!(event.kind, TrackEventKind::Midi { message:MidiMessage::NoteOn { key,vel },.. } if vel.as_int()>0 && (if pro_keys{matches!(key.as_int(),48..=72)}else if vocal{matches!(key.as_int(),36..=84)}else{!game || matches!(key.as_int(),60..=64|72..=76|84..=88|95..=100)}))).count();
        (label.to_owned(),notes)
    }).collect())
}
