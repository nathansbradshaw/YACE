//! A bounded, source-bound VENUE authoring subset. Unknown commands remain opaque.
//! Wire mappings are grounded in YARG.Core's MidIOHelper/VenueLookup at
//! 3beb94e526558134145bcd3e409428f759001a40; this is a cue schematic, not a game renderer.
use crate::{song_files::game_midi as midi, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use midly::{Format, MetaMessage as M, Smf, TrackEventKind as K};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_EVENTS: usize = 16_384;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Text(&'static str),
    Note(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cue {
    pub code: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    pub encoding: Encoding,
}
macro_rules! cues {($(($code:literal,$category:literal,$label:literal,$encoding:expr)),*$(,)?)=>{pub const CUES:&[Cue]=&[$(Cue{code:$code,category:$category,label:$label,encoding:$encoding}),*];};}
cues! {
    ("light_intro","Lighting","Intro",Encoding::Text("[lighting (intro)]")),
    ("light_verse","Lighting","Verse",Encoding::Text("[lighting (verse)]")),
    ("light_chorus","Lighting","Chorus",Encoding::Text("[lighting (chorus)]")),
    ("light_cool","Lighting","Cool automatic",Encoding::Text("[lighting (loop_cool)]")),
    ("light_warm","Lighting","Warm automatic",Encoding::Text("[lighting (loop_warm)]")),
    ("light_blackout","Lighting","Blackout",Encoding::Text("[lighting (blackout_fast)]")),
    ("post_default","Post-processing","Default",Encoding::Text("[ProFilm_a.pp]")),
    ("post_bloom","Post-processing","Bloom",Encoding::Text("[bloom.pp]")),
    ("post_sepia","Post-processing","Sepia",Encoding::Text("[film_sepia_ink.pp]")),
    ("post_negative","Post-processing","Negative",Encoding::Text("[photo_negative.pp]")),
    ("camera_all","Camera","Full band · far",Encoding::Text("[coop_all_far]")),
    ("camera_guitar","Camera","Directed guitar",Encoding::Text("[directed_guitar]")),
    ("camera_drums","Camera","Directed drums",Encoding::Text("[directed_drums]")),
    ("camera_vocals","Camera","Directed vocals",Encoding::Text("[directed_vocals]")),
    ("fog_on","Stage","Fog on",Encoding::Text("[FogOn]")),
    ("fog_off","Stage","Fog off",Encoding::Text("[FogOff]")),
    ("bonus","Stage","Bonus effect",Encoding::Text("[bonusfx]")),
    ("spot_bass","Performer","Bass spotlight",Encoding::Note(37)),
    ("spot_drums","Performer","Drum spotlight",Encoding::Note(38)),
    ("spot_guitar","Performer","Guitar spotlight",Encoding::Note(39)),
    ("spot_vocals","Performer","Vocal spotlight",Encoding::Note(40)),
    ("spot_keys","Performer","Keys spotlight",Encoding::Note(41)),
    ("sing_bass","Performer","Bass singalong",Encoding::Note(85)),
    ("sing_drums","Performer","Drum singalong",Encoding::Note(86)),
    ("sing_guitar","Performer","Guitar singalong",Encoding::Note(87)),
}
pub fn cue(code: &str) -> Option<&'static Cue> {
    CUES.iter().find(|c| c.code == code)
}
fn text_cue(bytes: &[u8]) -> Option<&'static Cue> {
    let s = std::str::from_utf8(bytes)
        .ok()?
        .trim()
        .trim_matches(['[', ']']);
    CUES.iter()
        .find(|c| matches!(c.encoding,Encoding::Text(t) if t.trim_matches(['[',']'])==s))
}
fn note_cue(note: u8) -> Option<&'static Cue> {
    CUES.iter().find(|c| c.encoding == Encoding::Note(note))
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attributes {
    pub channel: u8,
    pub velocity: u8,
    pub off_velocity: u8,
    pub zero_note_on: bool,
}
impl Default for Attributes {
    fn default() -> Self {
        Self {
            channel: 0,
            velocity: 100,
            off_velocity: 0,
            zero_note_on: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub cue: String,
    pub tick: u64,
    pub end: Option<u64>,
    pub attributes: Option<Attributes>,
}
fn order(event: &Event) -> (bool, u64) {
    let (prefix, value) = event.id.split_once(':').unwrap_or(("", ""));
    (prefix != "source", value.parse().unwrap_or(u64::MAX))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub source_sha256: String,
    pub events: Vec<Event>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub document: Document,
    pub resolution: u32,
    pub present: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Event>,
        cue: String,
        tick: u64,
        end: Option<u64>,
    },
    Remove {
        event: Event,
    },
}
struct Indexed {
    source: Source,
    indices: Vec<(usize, Option<usize>)>,
    track: Option<usize>,
}
fn index(bytes: &[u8]) -> Result<Indexed> {
    midi::validate_midi_complete(bytes)?;
    let smf = Smf::parse(bytes)?;
    ensure!(
        smf.header.format != Format::Sequential,
        "Sequential MIDI venue editing is read-only"
    );
    let midly::Timing::Metrical(resolution) = smf.header.timing else {
        anyhow::bail!("SMPTE venue editing is read-only")
    };
    ensure!(resolution.as_int() > 0, "Invalid venue resolution");
    let mut found = None;
    let mut events = Vec::new();
    let mut indices = Vec::new();
    for (ti, track) in smf.tracks.iter().enumerate() {
        let names: Vec<_> = track
            .iter()
            .filter_map(|e| {
                if let K::Meta(M::TrackName(n)) = e.kind {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        if !names.iter().any(|n| n.eq_ignore_ascii_case(b"VENUE")) {
            continue;
        }
        ensure!(
            found.replace(ti).is_none() && names.len() == 1,
            "Ambiguous VENUE owners remain read-only"
        );
        ensure!(
            track
                .last()
                .is_some_and(|e| matches!(e.kind, K::Meta(M::EndOfTrack)))
                && track
                    .iter()
                    .filter(|e| matches!(e.kind, K::Meta(M::EndOfTrack)))
                    .count()
                    == 1,
            "Unsupported VENUE boundary"
        );
        let mut active = BTreeMap::new();
        for (i, (tick, kind)) in midi::midi_events(track).into_iter().enumerate() {
            match kind {
                K::Meta(M::Text(bytes)) => {
                    if let Some(c) = text_cue(bytes) {
                        events.push(Event {
                            id: String::new(),
                            cue: c.code.into(),
                            tick,
                            end: None,
                            attributes: None,
                        });
                        indices.push((i, None));
                    }
                }
                K::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        midly::MidiMessage::NoteOn { key, vel }
                            if vel.as_int() > 0 && note_cue(key.as_int()).is_some() =>
                        {
                            ensure!(
                                active
                                    .insert((ch, key.as_int()), (i, tick, vel.as_int()))
                                    .is_none(),
                                "Overlapping VENUE note ownership remains read-only"
                            );
                        }
                        midly::MidiMessage::NoteOff { key, vel }
                        | midly::MidiMessage::NoteOn { key, vel }
                            if note_cue(key.as_int()).is_some() =>
                        {
                            let (start, at, velocity) = active
                                .remove(&(ch, key.as_int()))
                                .context("Unpaired VENUE note remains read-only")?;
                            ensure!(tick > at, "Zero-length VENUE note remains read-only");
                            events.push(Event {
                                id: String::new(),
                                cue: note_cue(key.as_int()).unwrap().code.into(),
                                tick: at,
                                end: Some(tick),
                                attributes: Some(Attributes {
                                    channel: ch,
                                    velocity,
                                    off_velocity: vel.as_int(),
                                    zero_note_on: matches!(
                                        message,
                                        midly::MidiMessage::NoteOn { .. }
                                    ),
                                }),
                            });
                            indices.push((start, Some(i)));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ensure!(active.is_empty(), "Unclosed VENUE notes remain read-only");
    }
    let mut ordered: Vec<_> = events.into_iter().zip(indices).collect();
    ordered.sort_by_key(|(_, indices)| indices.0);
    let (mut events, mut indices) = (Vec::new(), Vec::new());
    for (i, (mut e, ix)) in ordered.into_iter().enumerate() {
        e.id = format!("source:{i}");
        events.push(e);
        indices.push(ix);
    }
    let source = Source {
        document: Document {
            version: 1,
            source_sha256: format!("{:x}", Sha256::digest(bytes)),
            events,
        },
        resolution: resolution.as_int().into(),
        present: found.is_some(),
    };
    validate_shape(&source.document)?;
    Ok(Indexed {
        source,
        indices,
        track: found,
    })
}
pub fn imported(bytes: &[u8]) -> Result<Source> {
    Ok(index(bytes)?.source)
}
pub fn validate_shape(document: &Document) -> Result<()> {
    ensure!(
        document.version == 1
            && document.source_sha256.len() == 64
            && document
                .source_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit()),
        "Invalid venue source binding"
    );
    ensure!(
        document.events.len() <= MAX_EVENTS,
        "Edit at most 16,384 venue events"
    );
    let mut ids = BTreeSet::new();
    let mut ranges = BTreeMap::<(u8, u8), Vec<(u64, u64)>>::new();
    for e in &document.events {
        ensure!(
            !e.id.is_empty() && e.id.len() <= 160 && ids.insert(&e.id),
            "Invalid venue event identity"
        );
        let c = cue(&e.cue).context("Unsupported venue cue")?;
        match c.encoding {
            Encoding::Text(_) => ensure!(
                e.end.is_none() && e.attributes.is_none(),
                "Point venue cue cannot have a note range"
            ),
            Encoding::Note(note) => {
                let end = e.end.context("Performer cue needs an end tick")?;
                let a = e
                    .attributes
                    .as_ref()
                    .context("Missing venue note attributes")?;
                ensure!(
                    end > e.tick
                        && a.channel < 16
                        && (1..=127).contains(&a.velocity)
                        && a.off_velocity < 128
                        && (!a.zero_note_on || a.off_velocity == 0),
                    "Invalid venue note range or attributes"
                );
                ranges
                    .entry((a.channel, note))
                    .or_default()
                    .push((e.tick, end));
            }
        }
    }
    for values in ranges.values_mut() {
        values.sort();
        ensure!(
            values.windows(2).all(|w| w[0].1 <= w[1].0),
            "Overlapping performer cues on the same MIDI channel"
        );
    }
    Ok(())
}
fn bound(source: &Source, document: &Document) -> Result<()> {
    validate_shape(document)?;
    ensure!(
        document.source_sha256 == source.document.source_sha256,
        "Venue belongs to another source"
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
                || e.id
                    .strip_prefix("new:")
                    .is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == s)),
            "Unknown venue identity"
        );
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
        "Venue source resolution changed"
    );
    let originals: BTreeMap<_, _> = source.document.events.iter().map(|e| (&e.id, e)).collect();
    for e in &document.events {
        let old = originals.get(&e.id);
        if old.is_none_or(|o| o.tick != e.tick || o.end != e.end) {
            let t = timing.seconds(e.end.unwrap_or(e.tick));
            ensure!(
                t.is_finite() && t <= duration + 0.1,
                "Venue cue is beyond the recording or timeline"
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
    let mut next = next_id.max(
        revision
            .checked_add(1)
            .context("Venue identity exhausted")?,
    );
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { event } => Some(event),
    };
    let id = if let Some(p) = previous {
        let i = after
            .events
            .iter()
            .position(|e| e == p)
            .context("This venue event changed. Select it again")?;
        after.events.remove(i).id
    } else {
        loop {
            let id = format!("new:{next}");
            next = next.checked_add(1).context("Venue identity exhausted")?;
            if !after.events.iter().any(|e| e.id == id) {
                break id;
            }
        }
    };
    if let Edit::Put {
        cue: code,
        tick,
        end,
        ..
    } = edit
    {
        let c = cue(code).context("Choose a supported venue cue")?;
        let attributes = match c.encoding {
            Encoding::Text(_) => None,
            Encoding::Note(_) => Some(
                previous
                    .and_then(|p| p.attributes.clone())
                    .unwrap_or_default(),
            ),
        };
        after.events.push(Event {
            id,
            cue: code.clone(),
            tick: *tick,
            end: *end,
            attributes,
        });
    }
    validate(source, &after, timing, duration)?;
    after.events.sort_by_key(|e| (e.tick, order(e)));
    Ok((after, next))
}
pub fn values(document: &Document) -> Vec<(String, u64, Option<u64>, Option<Attributes>)> {
    let mut v: Vec<_> = document
        .events
        .iter()
        .map(|e| (e.cue.clone(), e.tick, e.end, e.attributes.clone()))
        .collect();
    v.sort();
    v
}
/// Compose with the immutable source VENUE after other instrument writers.
/// All unknown commands and untouched track chunks survive.
pub fn rewrite(bytes: &[u8], source: &Source, after: &Document) -> Result<Vec<u8>> {
    bound(source, after)?;
    if after == &source.document {
        return Ok(bytes.to_vec());
    }
    let actual = index(bytes)?;
    ensure!(
        actual.source.resolution == source.resolution
            && actual.source.present == source.present
            && values(&actual.source.document) == values(&source.document),
        "VENUE differs from its editable baseline; export refused"
    );
    let smf = Smf::parse(bytes)?;
    let (header, chunks) = midi::midi_chunks(bytes, smf.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    let mut count = smf.tracks.len();
    let mut events = Vec::new();
    let mut skipped = BTreeSet::new();
    let originals: BTreeMap<_, _> = source
        .document
        .events
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();
    let mut raw_by_id = BTreeMap::new();
    if let Some(ti) = actual.track {
        for (e, (start, end)) in actual.source.document.events.iter().zip(&actual.indices) {
            skipped.insert(*start);
            if let Some(end) = end {
                skipped.insert(*end);
            }
            raw_by_id.insert(e.id.as_str(), (*start, *end));
        }
        for (i, (tick, kind)) in midi::midi_events(&smf.tracks[ti]).into_iter().enumerate() {
            if !skipped.contains(&i) {
                events.push((tick, 1, i, kind));
            }
        }
    } else {
        events.push((0, 0, 0, K::Meta(M::TrackName(b"VENUE"))));
    }
    let raw = actual
        .track
        .map(|i| midi::midi_events(&smf.tracks[i]))
        .unwrap_or_default();
    let base = raw.len() + 1;
    for (i, e) in after.events.iter().enumerate() {
        let order = raw_by_id.get(e.id.as_str()).map_or(base + i * 2, |v| v.0);
        let original = originals.get(e.id.as_str());
        let c = cue(&e.cue).unwrap();
        match c.encoding {
            Encoding::Text(text) => {
                let kind = if original.is_some_and(|o| o.cue == e.cue) {
                    raw_by_id
                        .get(e.id.as_str())
                        .map(|v| raw[v.0].1)
                        .unwrap_or(K::Meta(M::Text(text.as_bytes())))
                } else {
                    K::Meta(M::Text(text.as_bytes()))
                };
                events.push((e.tick, 1, order, kind));
            }
            Encoding::Note(note) => {
                let a = e.attributes.as_ref().unwrap();
                // Moving a range across an earlier source ordinal must not put
                // its new note-on before a touching range's note-off.
                let unchanged = original.is_some_and(|old| *old == e);
                events.push((
                    e.tick,
                    if unchanged { 1 } else { 2 },
                    order,
                    K::Midi {
                        channel: a.channel.into(),
                        message: midly::MidiMessage::NoteOn {
                            key: note.into(),
                            vel: a.velocity.into(),
                        },
                    },
                ));
                let message = if a.zero_note_on {
                    midly::MidiMessage::NoteOn {
                        key: note.into(),
                        vel: 0.into(),
                    }
                } else {
                    midly::MidiMessage::NoteOff {
                        key: note.into(),
                        vel: a.off_velocity.into(),
                    }
                };
                let offorder = raw_by_id
                    .get(e.id.as_str())
                    .and_then(|v| v.1)
                    .unwrap_or(base + i * 2 + 1);
                events.push((
                    e.end.unwrap(),
                    if unchanged { 1 } else { 0 },
                    offorder,
                    K::Midi {
                        channel: a.channel.into(),
                        message,
                    },
                ));
            }
        }
    }
    let rewritten = midi::encode_midi_owner(events, smf.header)?;
    for (i, chunk) in chunks.into_iter().enumerate() {
        if actual.track == Some(i) {
            output.extend(&rewritten);
        } else {
            output.extend(chunk);
        }
    }
    if actual.track.is_none() {
        output.extend(rewritten);
        count += 1;
    }
    ensure!(count <= usize::from(u16::MAX), "MIDI track count overflow");
    if count > 1 {
        output[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    output[10..12].copy_from_slice(&(count as u16).to_be_bytes());
    let verified = imported(&output)?;
    ensure!(
        values(&verified.document) == values(after),
        "Rewritten VENUE differs from its editable state"
    );
    Ok(output)
}

/// Cue schematic state at a tick: point cues latch; performer ranges are active
/// only for their authored duration. Unknown cues never acquire guessed effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub lighting: String,
    pub post: String,
    pub camera: String,
    pub fog: bool,
    pub bonus: bool,
    pub performers: Vec<String>,
}
pub fn preview(document: &Document, tick: u64, resolution: u32) -> Preview {
    let mut p = Preview {
        lighting: "Default".into(),
        post: "Default".into(),
        camera: "Full band".into(),
        fog: false,
        bonus: false,
        performers: Vec::new(),
    };
    let mut events: Vec<_> = document.events.iter().collect();
    events.sort_by_key(|e| (e.tick, order(e)));
    for e in events {
        if e.tick > tick {
            continue;
        }
        let Some(c) = cue(&e.cue) else { continue };
        match c.category {
            "Lighting" => p.lighting = c.label.into(),
            "Post-processing" => p.post = c.label.into(),
            "Camera" => p.camera = c.label.into(),
            "Stage" => match c.code {
                "fog_on" => p.fog = true,
                "fog_off" => p.fog = false,
                "bonus" => p.bonus = tick.saturating_sub(e.tick) < u64::from(resolution),
                _ => {}
            },
            "Performer" if e.end.is_some_and(|end| tick < end) => p.performers.push(c.label.into()),
            _ => {}
        }
    }
    p
}
