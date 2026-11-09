use super::decode;
use super::{AttributeRecord, NoteAttributes};
use crate::{chart::Chart, sync_track, vocals::VocalChart};
use anyhow::{Context, Result, ensure};
use midly::{
    Format, MetaMessage, MidiMessage, Smf, TrackEvent, TrackEventKind,
    num::{u4, u7, u28},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn chunks(bytes: &[u8], expected: usize) -> Result<(usize, Vec<&[u8]>)> {
    ensure!(
        bytes.len() >= 14 && &bytes[..4] == b"MThd",
        "Invalid MIDI header"
    );
    let length = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(length >= 6, "Truncated MIDI header");
    let header = 8usize.checked_add(length).context("MIDI header overflow")?;
    ensure!(header <= bytes.len(), "Truncated MIDI header");
    let mut position = header;
    let mut tracks = Vec::new();
    while position < bytes.len() {
        ensure!(
            bytes.len() - position >= 8 && &bytes[position..position + 4] == b"MTrk",
            "Unsupported MIDI chunk or trailing bytes; source retained"
        );
        let length = u32::from_be_bytes(bytes[position + 4..position + 8].try_into()?) as usize;
        let end = position
            .checked_add(8)
            .and_then(|p| p.checked_add(length))
            .context("MIDI chunk overflow")?;
        ensure!(end <= bytes.len(), "Truncated MIDI track");
        tracks.push(&bytes[position..end]);
        position = end;
    }
    ensure!(
        tracks.len() == expected
            && expected == u16::from_be_bytes(bytes[10..12].try_into()?) as usize,
        "MIDI track count mismatch"
    );
    Ok((header, tracks))
}

/// midly's permissive collector can stop silently at a malformed event. Do not
/// build an editable view whose later rewrite would discard that unread tail.
pub(crate) fn validate_complete(bytes: &[u8]) -> Result<()> {
    let mapped = midly::SmfBytemap::parse(bytes)?;
    let (_, chunks) = chunks(bytes, mapped.tracks.len())?;
    for (events, chunk) in mapped.tracks.iter().zip(chunks) {
        ensure!(
            events
                .last()
                .is_some_and(|(raw, _)| raw.as_ptr().wrapping_add(raw.len())
                    == chunk.as_ptr().wrapping_add(chunk.len())),
            "Malformed or unread MIDI track data; source retained"
        );
        for (raw, event) in events {
            if let TrackEventKind::Meta(message) = event.kind {
                let expected = match message {
                    MetaMessage::TrackNumber(None) | MetaMessage::EndOfTrack => Some(0),
                    MetaMessage::MidiChannel(_) | MetaMessage::MidiPort(_) => Some(1),
                    MetaMessage::TrackNumber(Some(_)) | MetaMessage::KeySignature(..) => Some(2),
                    MetaMessage::Tempo(_) => Some(3),
                    MetaMessage::TimeSignature(..) => Some(4),
                    MetaMessage::SmpteOffset(_) => Some(5),
                    _ => None,
                };
                if let Some(expected) = expected {
                    let mut prefix = 2;
                    while prefix < raw.len() {
                        let byte = raw[prefix];
                        prefix += 1;
                        if byte & 128 == 0 {
                            break;
                        }
                    }
                    ensure!(
                        raw.len() - prefix == expected,
                        "Extended fixed-size MIDI metadata is not editable; source retained"
                    );
                }
            }
        }
    }
    Ok(())
}

fn absolute<'a>(track: &[TrackEvent<'a>]) -> Vec<(u64, TrackEventKind<'a>)> {
    let mut tick = 0;
    track
        .iter()
        .map(|e| {
            tick += u64::from(e.delta.as_int());
            (tick, e.kind)
        })
        .collect()
}

fn encode<'a>(
    mut events: Vec<(u64, u8, usize, TrackEventKind<'a>)>,
    header: midly::Header,
) -> Result<Vec<u8>> {
    let end = events.iter().map(|e| e.0).max().unwrap_or(0);
    events.sort_by_key(|e| (e.0, e.1, e.2));
    let mut previous = 0;
    let mut track = Vec::new();
    for (tick, _, _, kind) in events {
        if matches!(kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
            continue;
        }
        let delta = tick - previous;
        ensure!(
            delta <= 0x0fff_ffff,
            "Edited MIDI delta exceeds supported range"
        );
        track.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    ensure!(
        end - previous <= 0x0fff_ffff,
        "MIDI end-of-track delta overflow"
    );
    track.push(TrackEvent {
        delta: u28::new((end - previous) as u32),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let smf = Smf {
        header,
        tracks: vec![track],
    };
    let mut encoded = Vec::new();
    smf.write_std(&mut encoded)?;
    Ok(encoded[14..].to_vec())
}

fn vocal_event(kind: TrackEventKind<'_>) -> bool {
    match kind {
        TrackEventKind::Midi {
            message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. },
            ..
        } => matches!(key.as_int(), 36..=84 | 105 | 106),
        TrackEventKind::Meta(MetaMessage::Lyric(_)) => true,
        TrackEventKind::Meta(MetaMessage::Text(text)) => !text.starts_with(b"["),
        TrackEventKind::Meta(MetaMessage::SequencerSpecific(data)) => {
            data == b"YACE\0continue-word\0v1" || data == b"YACE\0missing-lyric\0v1"
        }
        _ => false,
    }
}

pub(super) fn vocal_view(smf: &Smf<'_>) -> Result<Option<Vec<u8>>> {
    let tracks: Vec<_> = smf
        .tracks
        .iter()
        .filter(|t| decode::label(t).as_deref() == Some("PART VOCALS"))
        .collect();
    ensure!(
        tracks.len() <= 1,
        "Duplicate lead vocal tracks are ambiguous"
    );
    let Some(track) = tracks.first() else {
        return Ok(None);
    };
    let mut events = vec![(
        0,
        1,
        0,
        TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS")),
    )];
    for (index, (tick, kind)) in absolute(track).into_iter().enumerate() {
        if vocal_event(kind) {
            events.push((tick, 1, index + 1, kind));
        }
    }
    let chunk = encode(events, smf.header)?;
    let mut output = b"MThd\0\0\0\x06\0\x01\0\x01".to_vec();
    match smf.header.timing {
        midly::Timing::Metrical(ppq) => output.extend(ppq.as_int().to_be_bytes()),
        _ => anyhow::bail!("SMPTE game MIDI unsupported"),
    }
    output.extend(chunk);
    Ok(Some(output))
}

fn body<'a>(chart: &'a Chart, name: &str) -> &'a str {
    chart
        .sections
        .iter()
        .find(|s| s.name == name)
        .map_or("", |s| &chart.raw[s.body_start..s.body_end])
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Component {
    Notes,
    StarPower,
    Solo,
    Fill,
    Roll,
    SpecialRoll,
    VocalPhrases,
}

fn row_component(line: &str) -> Option<Component> {
    let (_, row) = line.split_once('=')?;
    let mut fields = row.split_whitespace();
    Some(match fields.next()? {
        "N" => Component::Notes,
        "S" => match fields.next()? {
            "2" => Component::StarPower,
            "64" => Component::Fill,
            "65" => Component::Roll,
            "66" => Component::SpecialRoll,
            _ => return None,
        },
        "E" => Component::Solo,
        _ => return None,
    })
}

fn changes(name: &str, before: &State<'_>, after: &State<'_>) -> BTreeSet<Component> {
    let mut result = BTreeSet::new();
    if name == "PART VOCALS" {
        if before.vocals.map(|v| &v.notes) != after.vocals.map(|v| &v.notes) {
            result.insert(Component::Notes);
        }
        if before.vocals.map(|v| &v.phrases) != after.vocals.map(|v| &v.phrases) {
            result.insert(Component::VocalPhrases);
        }
        return result;
    }
    let Some((suffix, difficulty)) = decode::part(name) else {
        return result;
    };
    for (d, tier) in ["Easy", "Medium", "Hard", "Expert"].iter().enumerate() {
        if difficulty.is_some_and(|target| target != d) {
            continue;
        }
        let track = format!("{tier}{suffix}");
        for component in [
            Component::Notes,
            Component::StarPower,
            Component::Solo,
            Component::Fill,
            Component::Roll,
            Component::SpecialRoll,
        ] {
            let signature = |chart: &Chart| {
                body(chart, &track)
                    .lines()
                    .filter(|line| row_component(line) == Some(component))
                    .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                    .collect::<BTreeSet<_>>()
            };
            if signature(before.chart) != signature(after.chart) {
                result.insert(component);
            }
        }
    }
    if gem_attributes(name, before.attributes) != gem_attributes(name, after.attributes) {
        result.insert(Component::Notes);
    }
    result
}

pub(super) fn gem_key(name: &str, key: u8, enhanced: bool) -> bool {
    let Some((suffix, difficulty)) = decode::part(name) else {
        return name == "PART VOCALS" && (36..=84).contains(&key);
    };
    if difficulty.is_some() {
        return (48..=72).contains(&key);
    }
    if crate::pro_guitar::is_track(suffix) {
        return (0..4u8).any(|d| (24 + d * 24..=29 + d * 24).contains(&key));
    }
    (0..4u8).any(|d| {
        (60 + d * 12..=64 + d * 12).contains(&key)
            || (crate::six_fret::is_track(suffix)
                && crate::six_fret::chart_lane(key, 60 + d * 12).is_some())
            || (enhanced && suffix != "Drums" && key == 59 + d * 12)
    }) || (suffix == "Drums" && key == 95)
}

fn managed(name: &str, kind: TrackEventKind<'_>, enhanced: bool, fills: bool) -> bool {
    if name == "PART VOCALS" {
        return vocal_event(kind);
    }
    let Some((suffix, pro)) = decode::part(name) else {
        return false;
    };
    match kind {
        TrackEventKind::Midi {
            message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. },
            ..
        } => {
            let key = key.as_int();
            if gem_key(name, key, enhanced) {
                return true;
            }
            if crate::pro_guitar::is_track(suffix) {
                return matches!(key, 115 | 116) || (0..4u8).any(|d| key == 30 + d * 24);
            }
            if pro.is_some() {
                // Original Pro Keys range markers are presentation data, kept
                // intact until range authoring is supported by the editor.
                return matches!(key, 115 | 116);
            }
            if suffix == "Drums" {
                return matches!(key, 103 | 110..=112 | 116 | 126 | 127)
                    || (fills && (120..=124).contains(&key));
            }
            (0..4u8).any(|d| matches!(key.checked_sub(60 + d * 12), Some(5 | 6)))
                || matches!(key, 103 | 104 | 116)
        }
        TrackEventKind::SysEx(data) => {
            pro.is_none()
                && suffix != "Drums"
                && !crate::pro_guitar::is_track(suffix)
                && decode::phase_event(data)
        }
        TrackEventKind::Meta(MetaMessage::Text(text)) => {
            std::str::from_utf8(text).is_ok_and(|text| {
                let text = text.trim().trim_matches(['[', ']']);
                if suffix == "Drums" {
                    text.eq_ignore_ascii_case("ENABLE_CHART_DYNAMICS")
                } else {
                    pro.is_none()
                        && !crate::pro_guitar::is_track(suffix)
                        && text.eq_ignore_ascii_case("ENHANCED_OPENS")
                }
            })
        }
        _ => false,
    }
}

fn component(name: &str, kind: TrackEventKind<'_>) -> Component {
    if let TrackEventKind::Midi {
        message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. },
        ..
    } = kind
    {
        return match key.as_int() {
            105 | 106 if name == "PART VOCALS" => Component::VocalPhrases,
            116 => Component::StarPower,
            103 | 115 => Component::Solo,
            120..=124 => Component::Fill,
            126 => Component::Roll,
            127 => Component::SpecialRoll,
            _ => Component::Notes,
        };
    }
    Component::Notes
}

fn priority(kind: TrackEventKind<'_>) -> u8 {
    match kind {
        TrackEventKind::Midi {
            message: MidiMessage::NoteOff { .. },
            ..
        } => 0,
        TrackEventKind::Midi {
            message: MidiMessage::NoteOn { vel, .. },
            ..
        } if vel.as_int() == 0 => 0,
        TrackEventKind::SysEx(data) if decode::phase_event(data) && data[6] == 0 => 3,
        TrackEventKind::Midi {
            message: MidiMessage::NoteOn { .. },
            ..
        } => 2,
        _ => 1,
    }
}

pub(super) struct State<'a> {
    pub chart: &'a Chart,
    pub vocals: Option<&'a VocalChart>,
    pub attributes: &'a [AttributeRecord],
}

type GemAttributes = BTreeMap<(u64, u8), (NoteAttributes, u64)>;

fn gem_attributes(name: &str, records: &[AttributeRecord]) -> GemAttributes {
    let mut attributes = GemAttributes::new();
    let Some((suffix, pro)) = decode::part(name) else {
        return attributes;
    };
    for record in records {
        let Some(difficulty) = ["Easy", "Medium", "Hard", "Expert"]
            .iter()
            .position(|tier| record.track == format!("{tier}{suffix}"))
        else {
            continue;
        };
        if pro.is_some_and(|d| d != difficulty) {
            continue;
        }
        let base = 60 + difficulty as u8 * 12;
        let key = if crate::pro_guitar::is_track(suffix) && record.note.lane < 6 {
            24 + difficulty as u8 * 24 + record.note.lane
        } else if pro.is_some() && (48..=72).contains(&record.note.lane) {
            record.note.lane
        } else if crate::six_fret::is_track(suffix) {
            let Some(key) = crate::six_fret::midi_key(record.note.lane, base) else {
                continue;
            };
            key
        } else if suffix == "Drums" && difficulty == 3 && record.note.lane == 32 {
            95
        } else if record.note.lane <= 4 {
            base + record.note.lane
        } else if suffix != "Drums" && record.note.lane == 7 {
            base - 1
        } else {
            continue;
        };
        attributes.insert(
            (record.note.tick, key),
            (record.attributes.clone(), record.note.sustain),
        );
    }
    attributes
}

type AbsoluteEvent<'a> = (u64, u8, TrackEventKind<'a>);

/// New manually authored Pro Guitar projects also need velocity frets and
/// channel articulations; they have no original-MIDI descriptor to merge.
pub(super) fn authored_attributes(bytes: &[u8], records: &[AttributeRecord]) -> Result<Vec<u8>> {
    let parsed = Smf::parse(bytes)?;
    let (header, chunks) = chunks(bytes, parsed.tracks.len())?;
    let mut output = bytes[..header].to_vec();
    for (track, chunk) in parsed.tracks.iter().zip(chunks) {
        let name = decode::label(track).unwrap_or_default();
        let attributes = gem_attributes(&name, records);
        if attributes.is_empty() {
            output.extend(chunk);
        } else {
            let events = apply_gem_attributes(&name, absolute(track), &attributes)?
                .into_iter()
                .enumerate()
                .map(|(serial, (tick, priority, kind))| (tick, priority, serial, kind))
                .collect();
            output.extend(encode(events, parsed.header)?);
        }
    }
    Ok(output)
}

fn apply_gem_attributes<'a>(
    name: &str,
    generated: Vec<(u64, TrackEventKind<'a>)>,
    attributes: &GemAttributes,
) -> Result<Vec<AbsoluteEvent<'a>>> {
    if name == "PART VOCALS" {
        return Ok(generated
            .into_iter()
            .map(|(tick, kind)| (tick, priority(kind), kind))
            .collect());
    }
    let new_dynamics = generated.iter().any(|(_, kind)| {
        matches!(
            kind,
            TrackEventKind::Meta(MetaMessage::Text(b"[ENABLE_CHART_DYNAMICS]"))
        )
    });
    let mut active = BTreeMap::new();
    let mut result = Vec::with_capacity(generated.len());
    for (mut tick, mut kind) in generated {
        let mut order = priority(kind);
        if let TrackEventKind::Midi { channel, message } = kind {
            match message {
                MidiMessage::NoteOn { key, vel }
                    if vel.as_int() > 0 && gem_key(name, key.as_int(), true) =>
                {
                    let metadata = attributes.get(&(tick, key.as_int()));
                    if let Some((value, _)) = metadata {
                        value.validate()?;
                    }
                    let actual_channel = metadata.map_or(channel, |(a, _)| u4::new(a.channel));
                    let mut actual_velocity = metadata.map_or(vel, |(a, _)| u7::new(a.velocity));
                    let dynamics = |velocity: u7| {
                        if new_dynamics && matches!(velocity.as_int(), 1 | 127) {
                            velocity.as_int()
                        } else {
                            100
                        }
                    };
                    if name == "PART DRUMS" && dynamics(actual_velocity) != dynamics(vel) {
                        actual_velocity = vel;
                    }
                    let zero = metadata.is_some_and(|(a, length)| a.zero_length && *length == 0);
                    ensure!(
                        active
                            .insert(
                                (channel, key),
                                (
                                    actual_channel,
                                    metadata.map(|(a, _)| a.clone()),
                                    zero.then_some(tick)
                                )
                            )
                            .is_none(),
                        "Edited overlapping same-key MIDI sustains are unsupported; project retained"
                    );
                    kind = TrackEventKind::Midi {
                        channel: actual_channel,
                        message: MidiMessage::NoteOn {
                            key,
                            vel: actual_velocity,
                        },
                    };
                }
                MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. }
                    if gem_key(name, key.as_int(), true) =>
                {
                    let (actual_channel, metadata, zero) = active
                        .remove(&(channel, key))
                        .context("Unmatched generated MIDI note-off")?;
                    if let Some(start) = zero {
                        tick = start;
                        order = 3;
                    }
                    let message = if metadata.as_ref().is_some_and(|a| a.off_as_note_on) {
                        MidiMessage::NoteOn {
                            key,
                            vel: u7::new(0),
                        }
                    } else {
                        MidiMessage::NoteOff {
                            key,
                            vel: u7::new(metadata.as_ref().map_or(0, |a| a.off_velocity)),
                        }
                    };
                    kind = TrackEventKind::Midi {
                        channel: actual_channel,
                        message,
                    };
                }
                _ => {}
            }
        }
        result.push((tick, order, kind));
    }
    ensure!(active.is_empty(), "Unclosed generated MIDI gems");
    Ok(result)
}

pub(super) fn rewrite(
    original: &[u8],
    generated: &[u8],
    before: State<'_>,
    after: State<'_>,
) -> Result<Vec<u8>> {
    let source = Smf::parse(original)?;
    let new = Smf::parse(generated)?;
    ensure!(
        before.vocals.is_some()
            || after.vocals.is_none()
            || !source
                .tracks
                .iter()
                .any(
                    |track| decode::label(track).as_deref() == Some("PART VOCALS")
                        && track.iter().any(|event| vocal_event(event.kind))
                ),
        "Original lead vocals could not be represented; replacing them would lose authored content. Source retained"
    );
    let (header, old_chunks) = chunks(original, source.tracks.len())?;
    ensure!(
        source.header.format != Format::Sequential && source.header.timing == new.header.timing,
        "Incompatible MIDI timing"
    );
    // Recording offset lives in song.ini, never in SMF tick timing. Keep the
    // original MIDI byte-for-byte when it is the only timing change.
    let timing_changed = before.chart.timing.resolution != after.chart.timing.resolution
        || before.chart.timing.tempos != after.chart.timing.tempos
        || sync_track::imported(before.chart)?.meters != sync_track::imported(after.chart)?.meters;
    let sections_changed = body(before.chart, "Events") != body(after.chart, "Events");
    let labels: BTreeSet<_> = source
        .tracks
        .iter()
        .chain(&new.tracks)
        .filter_map(|t| decode::label(t))
        .collect();
    let changed: BTreeMap<_, _> = labels
        .into_iter()
        .filter_map(|name| {
            let components = changes(&name, &before, &after);
            (!components.is_empty()).then_some((name, components))
        })
        .collect();
    if !timing_changed && !sections_changed && changed.is_empty() {
        return Ok(original.to_vec());
    }
    let mut generated_tracks: BTreeMap<_, _> = new
        .tracks
        .iter()
        .filter_map(|t| decode::label(t).map(|name| (name, t)))
        .collect();
    let mut output_chunks = Vec::new();
    for (index, track) in source.tracks.iter().enumerate() {
        let name = decode::label(track).unwrap_or_default();
        let instrument_changed = changed.contains_key(&name);
        let changed_component = |kind| {
            changed
                .get(&name)
                .is_some_and(|set| set.contains(&component(&name, kind)))
        };
        let remove_global = |kind: TrackEventKind<'_>| {
            timing_changed
                && matches!(
                    kind,
                    TrackEventKind::Meta(MetaMessage::Tempo(_) | MetaMessage::TimeSignature(..))
                )
        };
        let remove_section = |kind: TrackEventKind<'_>| {
            sections_changed
                && name == "EVENTS"
                && matches!(kind, TrackEventKind::Meta(MetaMessage::Text(text)) if decode::section(text).is_some())
        };
        if !instrument_changed
            && !track
                .iter()
                .any(|e| remove_global(e.kind) || remove_section(e.kind))
        {
            output_chunks.push(old_chunks[index].to_vec());
            generated_tracks.remove(&name);
            continue;
        }
        let enhanced = decode::text_enabled(track, "ENHANCED_OPENS");
        let fills = if name == "PART DRUMS" {
            let fill_events = decode::spans(track, |key| (120..=124).contains(&key))?;
            decode::fills_representable(&decode::ranges(&fill_events))
        } else {
            true
        };
        ensure!(
            fills
                || !changed
                    .get(&name)
                    .is_some_and(|set| set.contains(&Component::Fill)),
            "Partial-lane MIDI drum fills cannot be replaced in this editor; source retained"
        );
        let mut events = Vec::new();
        for (serial, (tick, kind)) in absolute(track).into_iter().enumerate() {
            if remove_global(kind)
                || remove_section(kind)
                || (changed_component(kind) && managed(&name, kind, enhanced, fills))
            {
                continue;
            }
            events.push((tick, 1, serial, kind));
        }
        if instrument_changed && let Some(track_new) = generated_tracks.get(&name) {
            let additions = absolute(track_new);
            if !enhanced
                && decode::text_enabled(track_new, "ENHANCED_OPENS")
                && decode::part(&name).is_some_and(|(part, pro)| part != "Drums" && pro.is_none())
            {
                ensure!(!track.iter().any(|event| matches!(event.kind, TrackEventKind::Midi { message: MidiMessage::NoteOn { key, .. }, .. }
                    if (0..4u8).any(|d| key.as_int() == 59 + d * 12))), "Enhanced opens would reinterpret an unsupported original note; source and project retained");
            }
            let additions =
                apply_gem_attributes(&name, additions, &gem_attributes(&name, after.attributes))?;
            let serial_base = track.len();
            for (serial, (tick, order, kind)) in additions.into_iter().enumerate() {
                if matches!(
                    kind,
                    TrackEventKind::Meta(MetaMessage::TrackName(_) | MetaMessage::EndOfTrack)
                ) {
                    continue;
                }
                if changed_component(kind) && managed(&name, kind, true, true) {
                    events.push((tick, order, serial_base + serial, kind));
                }
            }
        }
        generated_tracks.remove(&name);
        output_chunks.push(encode(events, source.header)?);
    }
    // New supported parts are appended; unknown original track chunks are never
    // regenerated merely because another instrument changed.
    for (name, track) in &generated_tracks {
        if changed.contains_key(name) {
            let events = apply_gem_attributes(
                name,
                absolute(track),
                &gem_attributes(name, after.attributes),
            )?
            .into_iter()
            .enumerate()
            .map(|(i, (t, p, k))| (t, p, i, k))
            .collect();
            output_chunks.push(encode(events, source.header)?);
        }
    }
    if timing_changed {
        let mut events = vec![(
            0,
            1,
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(b"TEMPO")),
        )];
        let original_meters: BTreeMap<_, _> = source
            .tracks
            .iter()
            .flat_map(|track| absolute(track))
            .filter_map(|(tick, kind)| match kind {
                TrackEventKind::Meta(MetaMessage::TimeSignature(_, _, c, n)) => {
                    Some((tick, (c, n)))
                }
                _ => None,
            })
            .collect();
        for (i, (tick, mut kind)) in new
            .tracks
            .iter()
            .flat_map(|track| absolute(track))
            .enumerate()
        {
            if let TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, _, _)) = kind {
                let (c, b) = original_meters.get(&tick).copied().unwrap_or((24, 8));
                kind = TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, c, b));
            }
            if matches!(
                kind,
                TrackEventKind::Meta(MetaMessage::Tempo(_) | MetaMessage::TimeSignature(..))
            ) {
                events.push((tick, 1, i + 1, kind));
            }
        }
        output_chunks.push(encode(events, source.header)?);
    }
    if sections_changed {
        let mut events = vec![(
            0,
            1,
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(b"EVENTS")),
        )];
        for (i, (tick, kind)) in new
            .tracks
            .iter()
            .filter(|t| decode::label(t).as_deref() == Some("EVENTS"))
            .flat_map(|t| absolute(t))
            .enumerate()
        {
            if matches!(kind, TrackEventKind::Meta(MetaMessage::Text(text)) if decode::section(text).is_some())
            {
                events.push((tick, 1, i + 1, kind));
            }
        }
        output_chunks.push(encode(events, source.header)?);
    }
    let mut output = original[..header].to_vec();
    if output_chunks.len() > 1 {
        output[8..10].copy_from_slice(&1u16.to_be_bytes());
    }
    output[10..12].copy_from_slice(&u16::try_from(output_chunks.len())?.to_be_bytes());
    for chunk in output_chunks {
        output.extend(chunk);
    }
    Smf::parse(&output).context("Edited MIDI failed validation")?;
    Ok(output)
}
