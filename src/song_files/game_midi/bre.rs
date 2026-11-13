//! Physical five-lane BRE controls and global coda, preserving unrelated bytes.
use super::{NoteAttributes, decode, hash, merge};
use crate::bre::{self, Coda, Phrase, Range, State};
use anyhow::{Context, Result, ensure};
use midly::{
    MetaMessage as Meta, MidiMessage as Note, Smf, Timing, TrackEventKind as Kind,
    num::{u4, u7},
};
use std::collections::{BTreeMap, BTreeSet};

fn owner(track: &[midly::TrackEvent<'_>]) -> Result<Option<String>> {
    let names: Vec<_> = track
        .iter()
        .filter_map(|e| {
            if let Kind::Meta(Meta::TrackName(n)) = e.kind {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    let label = decode::label(track);
    let relevant = label
        .as_ref()
        .is_some_and(|s| bre::OWNERS.iter().any(|(n, _)| n == s));
    if relevant || label.as_deref() == Some("EVENTS") {
        ensure!(
            names.len() == 1 && Some(names[0]) == label.as_deref().map(str::as_bytes),
            "Ambiguous BRE owner labels; retain source"
        );
        ensure!(
            track
                .last()
                .is_some_and(|e| matches!(e.kind, Kind::Meta(Meta::EndOfTrack)))
                && track
                    .iter()
                    .filter(|e| matches!(e.kind, Kind::Meta(Meta::EndOfTrack)))
                    .count()
                    == 1,
            "Incomplete BRE owner ending"
        );
    }
    Ok(label)
}
fn coda(tick: u64, kind: Kind<'_>) -> Option<Coda> {
    if let Kind::Meta(Meta::Text(bytes)) = kind {
        let text = std::str::from_utf8(bytes).ok()?;
        if bre::is_coda(text) {
            return Some(Coda {
                tick,
                text: text.into(),
            });
        }
    }
    None
}
pub fn import_bre(bytes: &[u8]) -> Result<State> {
    merge::validate_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    ensure!(
        midi.header.format != midly::Format::Sequential,
        "Sequential MIDI BRE is read-only"
    );
    let Timing::Metrical(resolution) = midi.header.timing else {
        anyhow::bail!("SMPTE BRE is read-only")
    };
    let mut state = State::empty(u32::from(resolution.as_int()));
    state.source_sha256 = Some(hash(bytes));
    let mut seen = BTreeSet::new();
    for track in &midi.tracks {
        let Some(name) = owner(track)? else { continue };
        if name == "EVENTS" {
            for (tick, kind) in merge::absolute(track) {
                if let Some(marker) = coda(tick, kind) {
                    ensure!(
                        state.coda.replace(marker).is_none(),
                        "Multiple global coda markers are read-only"
                    );
                }
            }
        }
        if !bre::OWNERS.iter().any(|(n, _)| *n == name) {
            continue;
        }
        ensure!(seen.insert(name.clone()), "Duplicate physical BRE owner");
        let spans = decode::spans(track, |k| (120..=124).contains(&k))?;
        let mut grouped = BTreeMap::<(u64, u64), BTreeMap<u8, NoteAttributes>>::new();
        for s in spans {
            ensure!(
                grouped
                    .entry((s.start, s.end))
                    .or_default()
                    .insert(
                        s.key,
                        NoteAttributes {
                            channel: s.channel,
                            velocity: s.velocity,
                            off_velocity: s.off_velocity,
                            off_as_note_on: s.off_as_note_on,
                            zero_length: false,
                            talkie_pitch: None
                        }
                    )
                    .is_none(),
                "Overlapping BRE lane channels are read-only"
            );
        }
        let mut phrases = vec![];
        for ((start, end), lanes) in grouped {
            ensure!(
                lanes.len() == 5 && (120..=124).all(|key| lanes.contains_key(&key)),
                "Independent or incomplete BRE lanes are read-only"
            );
            phrases.push(Phrase {
                range: Range { start, end },
                lanes: std::array::from_fn(|i| lanes[&(120 + i as u8)].clone()),
            });
        }
        if !phrases.is_empty() {
            state.owners.insert(name, phrases);
        }
    }
    state.validate_shape()?;
    Ok(state)
}
fn edge(tick: u64, kind: Kind<'_>) -> Option<(u64, u8, u8, bool)> {
    match kind {
        Kind::Midi {
            channel,
            message: Note::NoteOn { key, vel },
        } if (120..=124).contains(&key.as_int()) => {
            Some((tick, channel.as_int(), key.as_int(), vel.as_int() > 0))
        }
        Kind::Midi {
            channel,
            message: Note::NoteOff { key, .. },
        } if (120..=124).contains(&key.as_int()) => {
            Some((tick, channel.as_int(), key.as_int(), false))
        }
        _ => None,
    }
}
pub fn rewrite_bre(
    bytes: &[u8],
    expected_sha: &str,
    before: &State,
    after: &State,
) -> Result<Vec<u8>> {
    ensure!(hash(bytes) == expected_sha, "BRE input MIDI changed");
    before.validate_shape()?;
    after.validate_shape()?;
    ensure!(
        before.resolution == after.resolution && before.source_sha256 == after.source_sha256,
        "BRE source binding changed"
    );
    let actual = import_bre(bytes)?;
    ensure!(
        actual.resolution == before.resolution
            && actual.coda == before.coda
            && actual.owners == before.owners,
        "Physical BRE or coda witness changed"
    );
    if before.coda == after.coda && before.owners == after.owners {
        return Ok(bytes.to_vec());
    }
    let changed: BTreeSet<_> = before
        .owners
        .keys()
        .chain(after.owners.keys())
        .filter(|o| before.owners.get(*o) != after.owners.get(*o))
        .cloned()
        .collect();
    let coda_changed = before.coda != after.coda;
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = merge::chunks(bytes, midi.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut count = midi.tracks.len();
    let mut seen = BTreeSet::new();
    let mut added_coda = false;
    for (track, chunk) in midi.tracks.iter().zip(chunks) {
        let name = owner(track)?.unwrap_or_default();
        let instrument = changed.contains(&name);
        let global = coda_changed && name == "EVENTS";
        if !instrument && !global {
            output.extend(chunk);
            continue;
        }
        seen.insert(name.clone());
        let mut events = vec![];
        let mut removed = BTreeSet::new();
        if instrument {
            for p in before.owners.get(&name).into_iter().flatten() {
                if after.owners.get(&name).is_some_and(|v| v.contains(p)) {
                    continue;
                }
                for (i, a) in p.lanes.iter().enumerate() {
                    removed.insert((p.range.start, a.channel, 120 + i as u8, true));
                    removed.insert((p.range.end, a.channel, 120 + i as u8, false));
                }
            }
        }
        for (i, (tick, kind)) in merge::absolute(track).into_iter().enumerate() {
            if global && coda(tick, kind).is_some()
                || instrument && edge(tick, kind).is_some_and(|e| removed.contains(&e))
            {
                continue;
            }
            events.push((tick, 1, i, kind));
        }
        let mut serial = track.len();
        if global && !added_coda {
            if let Some(marker) = &after.coda {
                events.push((
                    marker.tick,
                    1,
                    serial,
                    Kind::Meta(Meta::Text(marker.text.as_bytes())),
                ));
                serial += 1;
            }
            added_coda = true;
        }
        if instrument {
            append(
                &mut events,
                &mut serial,
                before.owners.get(&name),
                after.owners.get(&name),
            );
        }
        output.extend(merge::encode(events, midi.header)?);
    }
    for name in changed.iter().filter(|name| !seen.contains(*name)) {
        let mut events = vec![(0, 1, 0, Kind::Meta(Meta::TrackName(name.as_bytes())))];
        let mut serial = 1;
        append(&mut events, &mut serial, None, after.owners.get(name));
        output.extend(merge::encode(events, midi.header)?);
        count += 1;
    }
    if coda_changed
        && !added_coda
        && let Some(marker) = &after.coda
    {
        output.extend(merge::encode(
            vec![
                (0, 1, 0, Kind::Meta(Meta::TrackName(b"EVENTS"))),
                (
                    marker.tick,
                    1,
                    1,
                    Kind::Meta(Meta::Text(marker.text.as_bytes())),
                ),
            ],
            midi.header,
        )?);
        count += 1;
    }
    output[10..12].copy_from_slice(&u16::try_from(count)?.to_be_bytes());
    if count > 1 && midi.header.format == midly::Format::SingleTrack {
        output[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    let result = import_bre(&output).context("BRE candidate failed verification")?;
    ensure!(
        result.coda == after.coda && result.owners == after.owners,
        "BRE candidate did not preserve its requested controls"
    );
    Ok(output)
}
fn append<'a>(
    events: &mut Vec<(u64, u8, usize, Kind<'a>)>,
    serial: &mut usize,
    before: Option<&Vec<Phrase>>,
    after: Option<&Vec<Phrase>>,
) {
    for p in after.into_iter().flatten() {
        if before.is_some_and(|v| v.contains(p)) {
            continue;
        }
        for (i, a) in p.lanes.iter().enumerate() {
            let key = u7::new(120 + i as u8);
            let channel = u4::new(a.channel);
            events.push((
                p.range.start,
                1,
                *serial,
                Kind::Midi {
                    channel,
                    message: Note::NoteOn {
                        key,
                        vel: u7::new(a.velocity),
                    },
                },
            ));
            *serial += 1;
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
            events.push((p.range.end, 0, *serial, Kind::Midi { channel, message }));
            *serial += 1;
        }
    }
}
