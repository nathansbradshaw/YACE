//! Hash-guarded local-text rewriting; notes and unrelated owners stay opaque.
use super::{Event, parse, validate};
use crate::song_files::game_midi;
use anyhow::{Context, Result, ensure};
use midly::{Format, MetaMessage as Meta, Smf, TrackEventKind as Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub resolution: u32,
    pub source_sha256: Option<String>,
    pub events: Vec<Event>,
}
impl State {
    pub fn validate_shape(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported drum mix state"
        );
        ensure!(
            self.source_sha256
                .as_ref()
                .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid drum mix source fingerprint"
        );
        validate(&self.events)
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn label(track: &[midly::TrackEvent<'_>]) -> Result<bool> {
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
    if !names.contains(&b"PART DRUMS".as_slice()) {
        return Ok(false);
    }
    ensure!(
        names.len() == 1,
        "Ambiguous drum owner labels; retain original"
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
        "Incomplete drum owner ending; retain original"
    );
    Ok(true)
}
fn mix_event(tick: u64, kind: Kind<'_>) -> Result<Option<Event>> {
    let Kind::Meta(Meta::Text(bytes)) = kind else {
        return Ok(None);
    };
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            let prefix = bytes
                .iter()
                .copied()
                .skip_while(u8::is_ascii_whitespace)
                .take(4)
                .collect::<Vec<_>>();
            ensure!(
                !prefix.starts_with(b"mix") && !prefix.starts_with(b"[mix"),
                "Drum mix text encoding is unsupported; retain original"
            );
            return Ok(None);
        }
    };
    let Some(mix) = parse(text)? else {
        return Ok(None);
    };
    ensure!(
        text.len() <= 4096,
        "Oversized drum mix text; retain original"
    );
    Ok(Some(Event {
        tick,
        mix,
        text: text.into(),
    }))
}
pub fn import_midi(bytes: &[u8]) -> Result<State> {
    game_midi::validate_midi_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    ensure!(
        midi.header.format != Format::Sequential,
        "Sequential MIDI has no shared drum mix clock"
    );
    let midly::Timing::Metrical(ppq) = midi.header.timing else {
        anyhow::bail!("Drum mix editing requires metrical MIDI")
    };
    let mut state = State {
        version: 1,
        resolution: u32::from(ppq.as_int()),
        source_sha256: Some(hash(bytes)),
        events: vec![],
    };
    let mut found = false;
    let mut alternate = false;
    for track in &midi.tracks {
        alternate |= track.iter().any(|e| {
            matches!(
                e.kind,
                Kind::Meta(Meta::TrackName(b"PART DRUM" | b"PART DRUMS_2X"))
            )
        });
        if !label(track)? {
            continue;
        }
        ensure!(!found, "Duplicate drum owners; retain original");
        found = true;
        for (t, k) in game_midi::midi_events(track) {
            if let Some(e) = mix_event(t, k)? {
                state.events.push(e)
            }
        }
    }
    ensure!(
        found || !alternate,
        "Alternate drum owners are read-only; retain original"
    );
    state.events.sort_by_key(|e| (e.tick, e.mix.difficulty));
    state.validate_shape()?;
    Ok(state)
}

/// Input bytes may already contain ordinary note edits. Their mix transitions
/// must still match the bound original view before any managed text is changed.
pub fn rewrite_midi(
    bytes: &[u8],
    expected_sha256: &str,
    before: &State,
    after: &State,
) -> Result<Vec<u8>> {
    ensure!(
        hash(bytes) == expected_sha256,
        "Drum mix MIDI input changed; export refused"
    );
    before.validate_shape()?;
    after.validate_shape()?;
    ensure!(
        before.resolution == after.resolution && before.source_sha256 == after.source_sha256,
        "Drum mix source binding changed"
    );
    let actual = import_midi(bytes)?;
    ensure!(
        actual.resolution == before.resolution && actual.events == before.events,
        "Original drum mix events changed; export refused"
    );
    if before.events == after.events {
        return Ok(bytes.to_vec());
    }
    let key = |e: &Event| (e.tick, e.text.clone());
    let old: BTreeSet<_> = before.events.iter().map(key).collect();
    let new: BTreeSet<_> = after.events.iter().map(key).collect();
    let removed: BTreeSet<_> = old.difference(&new).cloned().collect();
    let added: Vec<_> = after
        .events
        .iter()
        .filter(|e| !old.contains(&key(e)))
        .collect();
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = game_midi::midi_chunks(bytes, midi.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut seen = false;
    for (track, chunk) in midi.tracks.iter().zip(chunks) {
        if !label(track)? {
            output.extend(chunk);
            continue;
        }
        seen = true;
        let mut events = Vec::new();
        for (i, (tick, kind)) in game_midi::midi_events(track).into_iter().enumerate() {
            if mix_event(tick, kind)?.is_some_and(|e| removed.contains(&key(&e))) {
                continue;
            }
            events.push((tick, 1, i, kind));
        }
        let base = track.len();
        events.extend(added.iter().enumerate().map(|(i, e)| {
            (
                e.tick,
                1,
                base + i,
                Kind::Meta(Meta::Text(e.text.as_bytes())),
            )
        }));
        output.extend(game_midi::encode_midi_owner(events, midi.header)?);
    }
    if !seen {
        ensure!(!after.events.is_empty(), "No drum mix owner to remove");
        let mut events = vec![(0, 1, 0, Kind::Meta(Meta::TrackName(b"PART DRUMS")))];
        events.extend(
            added
                .iter()
                .enumerate()
                .map(|(i, e)| (e.tick, 1, i + 1, Kind::Meta(Meta::Text(e.text.as_bytes())))),
        );
        output.extend(game_midi::encode_midi_owner(events, midi.header)?);
        output[10..12].copy_from_slice(&u16::try_from(midi.tracks.len() + 1)?.to_be_bytes());
        if midi.header.format == Format::SingleTrack {
            output[8..10].copy_from_slice(&1u16.to_be_bytes());
        }
    }
    let candidate = import_midi(&output).context("Drum mix candidate is not valid MIDI")?;
    ensure!(
        candidate.events == after.events,
        "Drum mix export verification failed; retain original"
    );
    Ok(output)
}
