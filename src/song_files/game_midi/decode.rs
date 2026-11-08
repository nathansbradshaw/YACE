use super::{AttributeRecord, NoteAttributes};
use crate::{
    chart::{self, Chart, Note},
    gameplay_view::{self, Technique, ThresholdSource},
    midi,
    song_files::timing,
    song_metadata::ini_fields,
    vocals::VocalChart,
};
use anyhow::{Context, Result, ensure};
use midly::{MetaMessage, MidiMessage, Smf, TrackEvent, TrackEventKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Decoded {
    pub chart: Chart,
    pub vocals: Option<VocalChart>,
    pub warnings: Vec<String>,
    pub attributes: Vec<AttributeRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct Span {
    pub start: u64,
    pub end: u64,
    pub key: u8,
    pub velocity: u8,
    pub channel: u8,
    pub off_velocity: u8,
    pub off_as_note_on: bool,
}

pub(super) fn label(track: &[TrackEvent<'_>]) -> Option<String> {
    track
        .iter()
        .find_map(|e| match e.kind {
            TrackEventKind::Meta(MetaMessage::TrackName(name)) => std::str::from_utf8(name)
                .ok()
                .map(|s| s.to_ascii_uppercase()),
            _ => None,
        })
        .map(|s| {
            if s == "PART DRUM" {
                "PART DRUMS".into()
            } else {
                s
            }
        })
}

pub(super) fn part(name: &str) -> Option<(&'static str, Option<usize>)> {
    Some(match name {
        "PART GUITAR" => ("Single", None),
        "PART BASS" => ("DoubleBass", None),
        "PART RHYTHM" => ("DoubleRhythm", None),
        "PART GUITAR COOP" => ("DoubleGuitar", None),
        "PART GUITAR GHL" => ("GHLGuitar", None),
        "PART BASS GHL" => ("GHLBass", None),
        "PART RHYTHM GHL" => ("GHLRhythm", None),
        "PART GUITAR COOP GHL" => ("GHLCoop", None),
        "PART KEYS GHL" => ("GHLKeys", None),
        "PART DRUMS" => ("Drums", None),
        "PART KEYS" => ("Keyboard", None),
        "PART REAL_KEYS_E" => ("ProKeys", Some(0)),
        "PART REAL_KEYS_M" => ("ProKeys", Some(1)),
        "PART REAL_KEYS_H" => ("ProKeys", Some(2)),
        "PART REAL_KEYS_X" => ("ProKeys", Some(3)),
        _ => return None,
    })
}

pub(super) fn spans(track: &[TrackEvent<'_>], relevant: impl Fn(u8) -> bool) -> Result<Vec<Span>> {
    let mut active = BTreeMap::new();
    let mut out = Vec::new();
    let mut tick = 0u64;
    for event in track {
        tick = tick
            .checked_add(u64::from(event.delta.as_int()))
            .context("MIDI tick overflow")?;
        let TrackEventKind::Midi { channel, message } = event.kind else {
            continue;
        };
        match message {
            MidiMessage::NoteOn { key, vel } if relevant(key.as_int()) && vel.as_int() > 0 => {
                ensure!(
                    active
                        .insert((channel.as_int(), key.as_int()), (tick, vel.as_int()))
                        .is_none(),
                    "Overlapping same-key game MIDI notes"
                );
            }
            MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. }
                if relevant(key.as_int()) =>
            {
                let (start, velocity) = active
                    .remove(&(channel.as_int(), key.as_int()))
                    .context("Unmatched game MIDI note-off")?;
                let (off_velocity, off_as_note_on) = match message {
                    MidiMessage::NoteOff { vel, .. } => (vel.as_int(), false),
                    _ => (0, true),
                };
                out.push(Span {
                    start,
                    end: tick,
                    key: key.as_int(),
                    velocity,
                    channel: channel.as_int(),
                    off_velocity,
                    off_as_note_on,
                });
            }
            _ => {}
        }
    }
    ensure!(active.is_empty(), "Unterminated game MIDI notes");
    out.sort_by_key(|s| (s.start, s.key));
    Ok(out)
}

pub(super) fn text_enabled(track: &[TrackEvent<'_>], name: &str) -> bool {
    track.iter().any(|e| matches!(e.kind, TrackEventKind::Meta(MetaMessage::Text(text))
        if std::str::from_utf8(text).is_ok_and(|s| s.trim().trim_matches(['[', ']']).eq_ignore_ascii_case(name))))
}

pub(super) type Ranges = BTreeMap<u8, Vec<(u64, u64)>>;
pub(super) fn ranges(spans: &[Span]) -> Ranges {
    let mut result = Ranges::new();
    for span in spans {
        result
            .entry(span.key)
            .or_default()
            .push((span.start, span.end));
    }
    for (key, intervals) in &mut result {
        // Tom markers toggle cymbal flags. Overlapping markers on separate
        // channels cancel, unlike ordinary phrase/range unions.
        if (110..=112).contains(key) {
            let mut boundaries = BTreeMap::<u64, bool>::new();
            for &(start, end) in intervals.iter() {
                *boundaries.entry(start).or_default() ^= true;
                *boundaries
                    .entry(end.max(start.saturating_add(1)))
                    .or_default() ^= true;
            }
            let mut start = None;
            intervals.clear();
            for (tick, toggles) in boundaries {
                if toggles {
                    if let Some(begin) = start.take() {
                        intervals.push((begin, tick));
                    } else {
                        start = Some(tick);
                    }
                }
            }
            continue;
        }
        intervals.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for &(start, end) in intervals.iter() {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        *intervals = merged;
    }
    result
}
fn active(ranges: &Ranges, key: u8, tick: u64) -> bool {
    ranges.get(&key).is_some_and(|spans| {
        let index = spans.partition_point(|s| s.0 <= tick);
        index > 0
            && (tick < spans[index - 1].1
                || (spans[index - 1].0 == spans[index - 1].1 && tick == spans[index - 1].1))
    })
}

pub(super) fn phase_event(data: &[u8]) -> bool {
    data.len() == 8
        && &data[..4] == b"PS\0\0"
        && matches!(data[5], 1 | 4)
        && (data[4] <= 3 || data[4] == 255)
        && data[6] <= 1
        && data[7] == 0xf7
}

type PhaseRanges = BTreeMap<(u8, u8), Vec<(u64, u64)>>;
fn phase_ranges(track: &[TrackEvent<'_>]) -> Result<PhaseRanges> {
    let mut position = 0u64;
    let mut active = BTreeMap::new();
    let mut result = BTreeMap::<_, Vec<_>>::new();
    for event in track {
        position += u64::from(event.delta.as_int());
        if let TrackEventKind::SysEx(data) = event.kind
            && phase_event(data)
        {
            let key = (data[4], data[5]);
            if data[6] == 1 {
                ensure!(
                    active.insert(key, position).is_none(),
                    "Overlapping Phase Shift range"
                );
            } else {
                let start = active
                    .remove(&key)
                    .context("Unmatched Phase Shift range end")?;
                result.entry(key).or_default().push((start, position));
            }
        }
    }
    ensure!(active.is_empty(), "Unclosed Phase Shift range");
    Ok(result)
}

fn phase_active(ranges: &PhaseRanges, difficulty: usize, code: u8, tick: u64) -> bool {
    [difficulty as u8, 255].into_iter().any(|d| {
        ranges.get(&(d, code)).is_some_and(|spans| {
            let index = spans.partition_point(|s| s.0 <= tick);
            index > 0
                && (tick < spans[index - 1].1
                    || (code == 4 && tick == spans[index - 1].1)
                    || (spans[index - 1].0 == spans[index - 1].1 && tick == spans[index - 1].1))
        })
    })
}

pub(super) fn fills_representable(ranges: &Ranges) -> bool {
    let first = ranges.get(&120).map(Vec::as_slice).unwrap_or_default();
    (121..=124).all(|key| ranges.get(&key).map(Vec::as_slice).unwrap_or_default() == first)
}

pub(super) fn section(text: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(text)
        .ok()?
        .trim()
        .trim_matches(['[', ']']);
    let name = text
        .strip_prefix("section ")
        .or_else(|| text.strip_prefix("prc_"))?;
    Some(format!("section {name}"))
}

pub(super) fn import(bytes: &[u8], ini: &str) -> Result<Decoded> {
    let smf = Smf::parse(bytes).context("Invalid game MIDI")?;
    super::merge::chunks(bytes, smf.tracks.len())?;
    super::merge::validate_complete(bytes)?;
    let mut clock = midi::timing(bytes)?;
    clock.offset = timing::legacy_delay(ini)?.unwrap_or(0.);
    // Existing game export represents the external recording clock in whole ms.
    ensure!(
        (clock.offset * 1000. - (clock.offset * 1000.).round()).abs() < 1e-6,
        "Sub-millisecond MIDI recording delay is not supported; source retained"
    );
    let fields = ini_fields(ini);
    ensure!(
        fields
            .get("multiplier_note")
            .is_none_or(|v| v.trim().parse::<u8>() != Ok(103)),
        "Legacy multiplier_note=103 MIDI needs a dedicated phrase profile; source retained"
    );
    ensure!(
        !fields.get("five_lane_drums").is_some_and(|v| {
            let v = v.trim();
            v.starts_with('1')
                || v.get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("true"))
        }),
        "Five-lane MIDI drums are not editable yet; source retained"
    );
    let mut meter = BTreeMap::new();
    let mut globals = Vec::new();
    let mut parts = BTreeMap::<String, Vec<String>>::new();
    let mut names = BTreeSet::new();
    let mut attributes = Vec::new();
    let mut warnings = vec!["Original MIDI retained for export. Unsupported tracks and events are preserved; their controls are not editable here.".into(),
        "MIDI Star Power is shared across an instrument's difficulties. Pro Keys range markers, harmonies, venue events and other unsupported controls are retained in the source; they are not editable here.".into()];
    let mut has_game_track = false;
    for track in &smf.tracks {
        let name = label(track).unwrap_or_default();
        if name.starts_with("PART ") || name.starts_with("HARM") {
            has_game_track = true;
        }
        let mut tick = 0u64;
        for event in track {
            tick += u64::from(event.delta.as_int());
            if let TrackEventKind::Meta(MetaMessage::TimeSignature(n, d, _, _)) = event.kind {
                ensure!(
                    n > 0 && d <= 7,
                    "Unsupported MIDI time signature; source retained"
                );
                if let Some(previous) = meter.insert(tick, (n, d)) {
                    ensure!(previous == (n, d), "Conflicting MIDI time signatures");
                }
            }
            if name == "EVENTS"
                && let TrackEventKind::Meta(MetaMessage::Text(text)) = event.kind
                && let Some(text) = section(text)
            {
                globals.push(format!("  {tick} = E \"{}\"\n", chart::quoted(&text)));
            }
        }
        let Some((suffix, pro_difficulty)) = part(&name) else {
            if name != "PART VOCALS"
                && (name.starts_with("PART ") || name.starts_with("HARM") || name == "VENUE")
            {
                warnings.push(format!(
                    "{name} is preserved in MIDI but is not editable in this project."
                ));
            }
            continue;
        };
        ensure!(
            suffix != "Drums"
                || !track.iter().any(|event| matches!(event.kind,
            TrackEventKind::Midi { message: MidiMessage::NoteOn { key, vel }, .. }
            if vel.as_int() > 0 && (0..4u8).any(|d| key.as_int() == 65 + 12 * d))),
            "Five-lane MIDI drums are not editable yet; source retained"
        );
        ensure!(
            names.insert(name.clone()),
            "Duplicate supported MIDI track {name}; choose an unambiguous source"
        );
        let enhanced = text_enabled(track, "ENHANCED_OPENS");
        let six_fret = crate::six_fret::is_track(suffix);
        let dynamics = text_enabled(track, "ENABLE_CHART_DYNAMICS");
        let events = spans(track, |key| {
            if pro_difficulty.is_some() {
                (48..=72).contains(&key) || matches!(key, 115 | 116)
            } else if suffix == "Drums" {
                (0..4u8).any(|d| (60 + d * 12..=64 + d * 12).contains(&key))
                    || matches!(key, 95 | 103 | 110..=112 | 116 | 120..=124 | 126 | 127)
            } else {
                (0..4u8).any(|d| {
                    (60 + d * 12..=66 + d * 12).contains(&key)
                        || (six_fret && crate::six_fret::chart_lane(key, 60 + d * 12).is_some())
                        || (enhanced && key == 59 + d * 12)
                }) || matches!(key, 103 | 104 | 116)
            }
        })?;
        ensure!(
            events
                .iter()
                .all(|s| s.end > s.start
                    || !matches!(s.key, 103 | 115 | 116 | 120..=124 | 126 | 127)),
            "Zero-length MIDI phrases cannot be represented faithfully; source retained"
        );
        let controls = ranges(&events);
        let phases = if suffix != "Drums" && pro_difficulty.is_none() {
            phase_ranges(track)?
        } else {
            BTreeMap::new()
        };
        let fills = fills_representable(&controls);
        let any_gems = events
            .iter()
            .any(|event| super::merge::gem_key(&name, event.key, enhanced));
        for difficulty in 0..4usize {
            if pro_difficulty.is_some_and(|d| d != difficulty) {
                continue;
            }
            let base = 60 + difficulty as u8 * 12;
            let mut notes = Vec::new();
            for event in &events {
                let lane = if pro_difficulty.is_some() {
                    (48..=72).contains(&event.key).then_some(event.key)
                } else if six_fret {
                    crate::six_fret::chart_lane(event.key, base)
                } else if suffix == "Drums" && difficulty == 3 && event.key == 95 {
                    Some(32)
                } else if (base..=base + 4).contains(&event.key) {
                    Some(event.key - base)
                } else if suffix != "Drums" && enhanced && event.key == base - 1 {
                    Some(7)
                } else {
                    None
                };
                let Some(mut lane) = lane else {
                    continue;
                };
                if suffix != "Drums"
                    && !six_fret
                    && pro_difficulty.is_none()
                    && lane == 0
                    && phase_active(&phases, difficulty, 1, event.start)
                {
                    lane = 7;
                }
                let note = Note {
                    tick: event.start,
                    lane,
                    sustain: event.end - event.start,
                };
                let metadata = NoteAttributes {
                    channel: event.channel,
                    velocity: event.velocity,
                    off_velocity: event.off_velocity,
                    off_as_note_on: event.off_as_note_on,
                    zero_length: event.end == event.start,
                    talkie_pitch: None,
                };
                if !metadata.is_default() {
                    attributes.push(AttributeRecord {
                        track: format!(
                            "{}{suffix}",
                            ["Easy", "Medium", "Hard", "Expert"][difficulty]
                        ),
                        note: note.clone(),
                        attributes: metadata,
                    });
                }
                notes.push(note);
                if suffix == "Drums" {
                    if (2..=4).contains(&lane) && !active(&controls, 108 + lane, event.start) {
                        notes.push(Note {
                            tick: event.start,
                            lane: lane + 64,
                            sustain: 0,
                        });
                    }
                    if dynamics && (1..=4).contains(&lane) && matches!(event.velocity, 1 | 127) {
                        notes.push(Note {
                            tick: event.start,
                            lane: lane + if event.velocity == 127 { 33 } else { 39 },
                            sustain: 0,
                        });
                    }
                }
            }
            notes.sort();
            ensure!(
                notes
                    .windows(2)
                    .all(|n| (n[0].tick, n[0].lane) != (n[1].tick, n[1].lane)),
                "Overlapping channels create duplicate game gems"
            );
            if notes.is_empty() && (any_gems || (pro_difficulty.is_none() && difficulty != 3)) {
                continue;
            }
            let tier = ["Easy", "Medium", "Hard", "Expert"][difficulty];
            let name = format!("{tier}{suffix}");
            if suffix != "Drums" && pro_difficulty.is_none() {
                let chart_policy = gameplay_view::yarg_chart_policy(clock.resolution, &fields)?;
                let mut midi_policy = chart_policy.clone();
                let chart_threshold = chart_policy.hopo_threshold_ticks.unwrap();
                midi_policy.hopo_threshold_ticks = Some(
                    if chart_policy.threshold_source == ThresholdSource::SongIniFrequency {
                        chart_threshold + 1
                    } else {
                        chart_threshold - u64::from(clock.resolution) / 192 + 1
                    },
                );
                midi_policy.chord_hopo_cancellation = true;
                let natural_chart: BTreeMap<_, _> =
                    gameplay_view::classify_for_midi(&name, &notes, &clock, &chart_policy)?
                        .into_iter()
                        .map(|n| (n.tick, n.technique))
                        .collect();
                let natural_midi =
                    gameplay_view::classify_for_midi(&name, &notes, &clock, &midi_policy)?;
                let mut done = BTreeSet::new();
                for gem in natural_midi {
                    if !done.insert(gem.tick) {
                        continue;
                    }
                    ensure!(
                        !(active(&controls, base + 5, gem.tick)
                            && active(&controls, base + 6, gem.tick)),
                        "Conflicting forced HOPO/strum markers; source retained"
                    );
                    let desired = if active(&controls, 104, gem.tick)
                        || phase_active(&phases, difficulty, 4, gem.tick)
                    {
                        Technique::Tap
                    } else if active(&controls, base + 5, gem.tick) {
                        Technique::Hopo
                    } else if active(&controls, base + 6, gem.tick) {
                        Technique::Strum
                    } else {
                        gem.technique.context("Missing MIDI attack technique")?
                    };
                    if desired == Technique::Tap {
                        notes.push(Note {
                            tick: gem.tick,
                            lane: 6,
                            sustain: 0,
                        });
                    } else if natural_chart[&gem.tick] != Some(desired) {
                        notes.push(Note {
                            tick: gem.tick,
                            lane: 5,
                            sustain: 0,
                        });
                    }
                }
            }
            let rows = parts.entry(name).or_default();
            rows.push(chart::note_lines(&notes, "\n"));
            for event in &events {
                if event.key == 116 && event.end > event.start {
                    rows.push(format!(
                        "  {} = S 2 {}\n",
                        event.start,
                        event.end - event.start
                    ));
                }
                if event.key == if pro_difficulty.is_some() { 115 } else { 103 } {
                    rows.push(format!(
                        "  {} = E solo\n  {} = E soloend\n",
                        event.start, event.end
                    ));
                }
                if suffix == "Drums" {
                    let kind = match event.key {
                        120 if fills => Some(64),
                        126 => Some(65),
                        127 => Some(66),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        rows.push(format!(
                            "  {} = S {kind} {}\n",
                            event.start,
                            event.end - event.start
                        ));
                    }
                }
            }
        }
    }
    ensure!(
        has_game_track,
        "No named game MIDI parts found; use the performance MIDI import for General MIDI"
    );
    if parts.is_empty() {
        parts.insert("ExpertSingle".into(), Vec::new());
    }
    let mut raw = format!(
        "[Song]\n{{\n  Resolution = {}\n  Offset = {}\n}}\n[SyncTrack]\n{{\n",
        clock.resolution, clock.offset
    );
    for tempo in &clock.tempos {
        raw.push_str(&format!("  {} = B {}\n", tempo.tick, tempo.bpm * 1000.));
    }
    for (tick, (n, d)) in meter {
        raw.push_str(&format!("  {tick} = TS {n} {d}\n"));
    }
    raw.push_str("}\n[Events]\n{\n");
    raw.extend(globals);
    raw.push_str("}\n");
    for (name, rows) in parts {
        raw.push_str(&format!("[{name}]\n{{\n{}\n}}\n", rows.concat()));
    }
    let chart = Chart::parse(raw)?;
    // Filter only a supported lead-vocal view; its other original events remain
    // in the immutable source for export rather than failing the entire song.
    let vocals = super::merge::vocal_view(&smf)?
        .map(|bytes| crate::vocals::import_midi(&bytes))
        .transpose();
    let vocals = match vocals {
        Ok(v) => v,
        Err(error) => {
            warnings.push(format!(
                "Lead vocals retained in source MIDI but not editable: {error:#}"
            ));
            None
        }
    };
    Ok(Decoded {
        chart,
        vocals,
        warnings,
        attributes,
    })
}
