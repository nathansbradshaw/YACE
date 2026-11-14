//! Ordinary MIDI text retains physical track scope; lyrics and musical controls
//! keep their dedicated owners. Unchanged track chunks remain byte-identical.
use crate::{
    song_files::game_midi,
    text_events::{Document, Edit, Event, MAX_EVENTS, Scope},
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use midly::{Format, MetaMessage, Smf, TrackEvent, TrackEventKind};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub document: Document,
    pub resolution: u32,
    pub owners: Vec<String>,
    pub global: bool,
}
fn plain(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= 1024 && !text.chars().any(char::is_control)
}
fn label(track: &[TrackEvent<'_>]) -> Option<String> {
    track
        .iter()
        .find_map(|e| match e.kind {
            TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                std::str::from_utf8(name).ok().map(str::to_ascii_uppercase)
            }
            _ => None,
        })
        .map(|name| {
            if name == "PART DRUM" {
                "PART DRUMS".into()
            } else {
                name
            }
        })
}
fn scope(owner: &str) -> Scope {
    if owner == "EVENTS" {
        Scope::Global
    } else {
        Scope::Physical {
            owner: owner.into(),
        }
    }
}
pub fn dedicated(scope: &Scope, text: &str) -> bool {
    let value = text.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    match scope {
        Scope::Global => {
            value.starts_with("section ")
                || value.starts_with("prc_")
                || matches!(value.as_str(), "coda" | "midcoda" | "midcoda_end")
        }
        Scope::Physical { owner } => {
            matches!(value.as_str(), "solo" | "soloend" | "enhanced_opens")
                || (matches!(owner.as_str(), "PART VOCALS" | "HARM1" | "HARM2" | "HARM3")
                    && !text.starts_with('['))
                || (owner == "PART DRUMS"
                    && (value.starts_with("mix ") || value == "enable_chart_dynamics"))
        }
        Scope::Local { .. } => true,
    }
}
fn event(kind: TrackEventKind<'_>, scope: &Scope, tick: u64, ordinal: usize) -> Option<Event> {
    let TrackEventKind::Meta(MetaMessage::Text(bytes)) = kind else {
        return None;
    };
    let text = std::str::from_utf8(bytes).ok()?;
    if !plain(text) || dedicated(scope, text) {
        return None;
    }
    Some(Event {
        id: format!("source:{}:{ordinal}", scope.section()),
        scope: scope.clone(),
        tick,
        text: text.into(),
    })
}
pub fn imported(bytes: &[u8]) -> Result<Source> {
    game_midi::validate_midi_complete(bytes)?;
    let midi = Smf::parse(bytes)?;
    ensure!(
        midi.header.format != Format::Sequential,
        "Sequential MIDI text remains read-only"
    );
    let midly::Timing::Metrical(resolution) = midi.header.timing else {
        anyhow::bail!("SMPTE MIDI text remains read-only")
    };
    ensure!(
        resolution.as_int() > 0,
        "MIDI text resolution must be positive"
    );
    let mut owners = BTreeSet::new();
    let mut events = Vec::new();
    let mut global = false;
    for track in &midi.tracks {
        let Some(owner) = label(track).filter(|name| game_midi::text_owner(name)) else {
            continue;
        };
        ensure!(
            owners.insert(owner.clone()),
            "Ambiguous duplicate physical MIDI text owner; original retained"
        );
        ensure!(
            track
                .iter()
                .filter(|e| matches!(e.kind, TrackEventKind::Meta(MetaMessage::TrackName(_))))
                .count()
                == 1,
            "Ambiguous physical MIDI track names; original retained"
        );
        ensure!(
            track
                .iter()
                .filter(|e| matches!(e.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)))
                .count()
                == 1
                && track.last().is_some_and(|e| matches!(
                    e.kind,
                    TrackEventKind::Meta(MetaMessage::EndOfTrack)
                )),
            "Unsupported MIDI end-of-track boundaries; original retained"
        );
        global |= owner == "EVENTS";
        let scope = scope(&owner);
        let mut ordinal = 0;
        for (tick, kind) in game_midi::midi_events(track) {
            if let Some(value) = event(kind, &scope, tick, ordinal) {
                events.push(value);
                ordinal += 1;
                ensure!(
                    events.len() <= MAX_EVENTS,
                    "Edit at most 16,384 text events in one project"
                );
            }
        }
    }
    owners.remove("EVENTS");
    Ok(Source {
        document: Document {
            version: 1,
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            events,
        },
        resolution: u32::from(resolution.as_int()),
        owners: owners.into_iter().collect(),
        global,
    })
}
pub fn validate_shape(document: &Document) -> Result<()> {
    ensure!(
        document.version == 1
            && document.source_sha256.len() == 64
            && document
                .source_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Invalid MIDI text source binding"
    );
    ensure!(
        document.events.len() <= MAX_EVENTS,
        "Edit at most 16,384 text events in one project"
    );
    let mut ids = BTreeSet::new();
    for e in &document.events {
        ensure!(
            ids.insert(&e.id) && !e.id.is_empty() && e.id.len() <= 160,
            "Invalid or duplicate MIDI text event identity"
        );
        ensure!(
            plain(&e.text) && !dedicated(&e.scope, &e.text),
            "Use plain text; lyrics, sections, solos, BRE and drum controls have dedicated editors"
        );
        ensure!(
            matches!(&e.scope, Scope::Global)
                || matches!(&e.scope,Scope::Physical {owner} if owner!="EVENTS"&&game_midi::text_owner(owner)),
            "Choose a physical MIDI owner, shared by its difficulties"
        );
    }
    Ok(())
}
fn bound(source: &Source, document: &Document) -> Result<()> {
    validate_shape(document)?;
    ensure!(
        document.source_sha256 == source.document.source_sha256,
        "MIDI text belongs to another source"
    );
    let originals: BTreeSet<_> = source
        .document
        .events
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    for e in &document.events {
        ensure!(
            originals.contains(e.id.as_str())
                || e.id.strip_prefix("new:").is_some_and(|n| n
                    .parse::<u64>()
                    .is_ok_and(|id| id > 0 && id.to_string() == n)),
            "Unknown MIDI text identity"
        );
        if let Scope::Physical { owner } = &e.scope {
            ensure!(
                source.owners.contains(owner),
                "Choose an existing physical MIDI track"
            );
        }
    }
    Ok(())
}
pub fn validate(
    source: &Source,
    document: &Document,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    bound(source, document)?;
    ensure!(
        source.resolution == timing.resolution,
        "MIDI text/source resolution changed"
    );
    let originals: BTreeMap<_, _> = source.document.events.iter().map(|e| (&e.id, e)).collect();
    for e in &document.events {
        if !originals
            .get(&e.id)
            .is_some_and(|old| old.scope == e.scope && old.tick == e.tick)
        {
            let seconds = timing.seconds(e.tick);
            ensure!(
                seconds.is_finite() && seconds <= duration + 0.1,
                "Text event is beyond the recording or timeline"
            );
        }
    }
    Ok(())
}
pub fn edited(
    source: &Source,
    current: &Document,
    edit: &Edit,
    next_id: u64,
    revision: u64,
    timing: &TempoMap,
    duration: f64,
) -> Result<(Document, u64)> {
    bound(source, current)?;
    let mut after = current.clone();
    let mut next_id = next_id;
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { event } => Some(event),
    };
    let id = if let Some(previous) = previous {
        let index = after
            .events
            .iter()
            .position(|e| e == previous)
            .context("This MIDI text event changed. Select it again")?;
        after.events.remove(index).id
    } else {
        next_id = next_id.max(
            revision
                .checked_add(1)
                .context("Text event identity exhausted")?,
        );
        let occupied: BTreeSet<_> = after.events.iter().map(|e| e.id.as_str()).collect();
        loop {
            let id = format!("new:{next_id}");
            next_id = next_id
                .checked_add(1)
                .context("Text event identity exhausted")?;
            if !occupied.contains(id.as_str()) {
                break id;
            }
        }
    };
    if let Edit::Put {
        scope, tick, text, ..
    } = edit
    {
        after.events.push(Event {
            id,
            scope: scope.clone(),
            tick: *tick,
            text: text.clone(),
        });
    }
    validate(source, &after, timing, duration)?;
    after
        .events
        .sort_by(|a, b| (&a.scope, a.tick, &a.id).cmp(&(&b.scope, b.tick, &b.id)));
    Ok((after, next_id))
}
fn groups(document: &Document) -> BTreeMap<Scope, BTreeMap<String, Event>> {
    let mut out = BTreeMap::<_, BTreeMap<_, _>>::new();
    for event in &document.events {
        out.entry(event.scope.clone())
            .or_default()
            .insert(event.id.clone(), event.clone());
    }
    out
}
/// Compose after other MIDI writers. Missing local owners were omitted by the
/// export selection and are never recreated by their saved text edits.
pub fn rewrite(bytes: &[u8], source: &Source, after: &Document) -> Result<Vec<u8>> {
    rewrite_from(bytes, source, &source.document, after)
}
fn values(events: Option<&BTreeMap<String, Event>>) -> Vec<(u64, &str)> {
    let mut values: Vec<_> = events
        .into_iter()
        .flat_map(|events| events.values())
        .map(|e| (e.tick, e.text.as_str()))
        .collect();
    values.sort();
    values
}
/// A reopened export has a verified working text baseline with retained event
/// IDs. Match its raw text multiplicities rather than inventing IDs from the
/// reordered game events after a move.
pub fn rewrite_from(
    bytes: &[u8],
    source: &Source,
    expected: &Document,
    after: &Document,
) -> Result<Vec<u8>> {
    bound(source, expected)?;
    bound(source, after)?;
    let before = groups(expected);
    let after = groups(after);
    let changed: BTreeSet<_> = before
        .keys()
        .chain(after.keys())
        .filter(|s| before.get(*s) != after.get(*s))
        .cloned()
        .collect();
    if changed.is_empty() {
        return Ok(bytes.to_vec());
    }
    let actual = imported(bytes)?;
    ensure!(
        actual.resolution == source.resolution,
        "MIDI text/export resolution changed"
    );
    let actual_events = groups(&actual.document);
    for owner in &changed {
        if let Scope::Physical { owner } = owner
            && !actual.owners.contains(owner)
        {
            continue;
        }
        ensure!(
            values(before.get(owner)) == values(actual_events.get(owner)),
            "Text events in {owner:?} differ from their editable baseline; export refused"
        );
    }
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = game_midi::midi_chunks(bytes, midi.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut count = 0usize;
    for (track, raw) in midi.tracks.iter().zip(chunks) {
        count += 1;
        let owner = label(track).map(|name| scope(&name));
        let Some(owner) = owner.filter(|owner| changed.contains(owner)) else {
            output.extend(raw);
            continue;
        };
        let mut rewritten = Vec::new();
        let mut used = BTreeSet::new();
        let mut originals = BTreeMap::<(u64, String), VecDeque<&Event>>::new();
        for event in before
            .get(&owner)
            .into_iter()
            .flat_map(|events| events.values())
        {
            originals
                .entry((event.tick, event.text.clone()))
                .or_default()
                .push_back(event);
        }
        for (index, (tick, kind)) in game_midi::midi_events(track).into_iter().enumerate() {
            if let Some(raw) = event(kind, &owner, tick, 0) {
                let original = originals
                    .get_mut(&(raw.tick, raw.text))
                    .and_then(VecDeque::pop_front)
                    .context("Working MIDI text no longer matches its editable baseline")?;
                if let Some(event) = after
                    .get(&owner)
                    .and_then(|events| events.get(&original.id))
                {
                    used.insert(event.id.as_str());
                    let kind = if event == original {
                        kind
                    } else {
                        TrackEventKind::Meta(MetaMessage::Text(event.text.as_bytes()))
                    };
                    rewritten.push((event.tick, 1, index, kind));
                }
            } else {
                rewritten.push((tick, 1, index, kind));
            }
        }
        for (index, event) in after
            .get(&owner)
            .into_iter()
            .flat_map(|events| events.values())
            .enumerate()
        {
            if used.insert(event.id.as_str()) {
                rewritten.push((
                    event.tick,
                    1,
                    track.len() + index,
                    TrackEventKind::Meta(MetaMessage::Text(event.text.as_bytes())),
                ));
            }
        }
        output.extend(game_midi::encode_midi_owner(rewritten, midi.header)?);
    }
    if !actual.global && changed.contains(&Scope::Global) && after.contains_key(&Scope::Global) {
        ensure!(
            !source.global,
            "The original global MIDI text track disappeared; export refused"
        );
        let mut events = vec![(
            0,
            0,
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(b"EVENTS")),
        )];
        for (index, event) in after[&Scope::Global].values().enumerate() {
            events.push((
                event.tick,
                1,
                index,
                TrackEventKind::Meta(MetaMessage::Text(event.text.as_bytes())),
            ));
        }
        output.extend(game_midi::encode_midi_owner(events, midi.header)?);
        count += 1;
    }
    ensure!(count <= usize::from(u16::MAX), "MIDI track count overflow");
    if count > 1 {
        output[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    output[10..12].copy_from_slice(&(count as u16).to_be_bytes());
    Ok(output)
}
