//! Convert supported .chart events to MIDI for game exports with vocals.
//! Keep the editable .chart in the project. Reject unsupported events before
//! export so conversion cannot silently lose them.
use crate::{chart::Chart, vocals::VocalChart};
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

type Events = Vec<(u64, u8, Vec<u8>)>;

fn variable(mut n: u64, out: &mut Vec<u8>) -> Result<()> {
    ensure!(n <= 0x0fff_ffff, "MIDI delta exceeds supported range");
    let mut bytes = [0u8; 4];
    let mut i = 3;
    bytes[i] = (n & 127) as u8;
    while {
        n >>= 7;
        n != 0
    } {
        i -= 1;
        bytes[i] = ((n & 127) as u8) | 128;
    }
    out.extend_from_slice(&bytes[i..]);
    Ok(())
}
fn meta(kind: u8, text: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![0xff, kind];
    variable(text.len() as u64, &mut out)?;
    out.extend_from_slice(text);
    Ok(out)
}
fn track(mut events: Events) -> Result<Vec<u8>> {
    events.sort_by_key(|(tick, priority, _)| (*tick, *priority));
    let mut data = Vec::new();
    let mut last = 0;
    for (tick, _, bytes) in events {
        variable(tick - last, &mut data)?;
        data.extend(bytes);
        last = tick;
    }
    data.extend([0, 0xff, 0x2f, 0]);
    let mut out = b"MTrk".to_vec();
    out.extend(u32::try_from(data.len())?.to_be_bytes());
    out.extend(data);
    Ok(out)
}
fn note(events: &mut Events, start: u64, length: u64, key: u8, velocity: u8) -> Result<()> {
    events.push((start, 2, vec![0x90, key, velocity]));
    events.push((
        start.checked_add(length.max(1)).context("Note overflow")?,
        0,
        vec![0x80, key, 0],
    ));
    Ok(())
}
fn rows(body: &str) -> impl Iterator<Item = &str> {
    body.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("//"))
}
fn event(line: &str) -> Result<(u64, &str)> {
    let (tick, body) = line.split_once('=').context("Unsupported chart row")?;
    Ok((tick.trim().parse()?, body.trim()))
}
fn quoted(text: &str) -> Result<String> {
    // JSON quoting is the same subset written by chart-engine; refuse unfamiliar
    // escapes instead of altering author text.
    serde_json::from_str(text).context("Unsupported chart text escaping")
}

pub fn chart_to_midi(chart: &Chart) -> Result<Vec<u8>> {
    ensure!(
        chart.timing.offset == 0.0,
        "Vocal MIDI export requires zero chart Offset; the source project is retained"
    );
    chart_ticks_to_midi(chart, &BTreeMap::new())
}
fn chart_ticks_to_midi(chart: &Chart, ini: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let resolution = u16::try_from(chart.timing.resolution)?;
    ensure!(
        (1..=32767).contains(&resolution),
        "Unsupported MIDI resolution"
    );
    let mut tempo: Events = vec![(0, 1, meta(3, b"TEMPO")?)];
    let mut global: Events = vec![(0, 1, meta(3, b"EVENTS")?)];
    let mut instruments: std::collections::BTreeMap<&str, Events> = Default::default();
    for section in &chart.sections {
        let body = &chart.raw[section.body_start..section.body_end];
        match section.name.as_str() {
            "Song" => continue,
            "SyncTrack" => {
                for line in rows(body) {
                    let (tick, body) = event(line)?;
                    let fields: Vec<_> = body.split_whitespace().collect();
                    match fields.as_slice() {
                        ["B", value] => {
                            let bpm = value.parse::<f64>()? / 1000.;
                            let us = (60_000_000. / bpm).round();
                            ensure!(
                                us.is_finite() && (1. ..=16_777_215.).contains(&us),
                                "Unsupported MIDI tempo"
                            );
                            let us = us as u32;
                            tempo.push((
                                tick,
                                1,
                                vec![0xff, 0x51, 3, (us >> 16) as u8, (us >> 8) as u8, us as u8],
                            ));
                        }
                        ["TS", numerator] | ["TS", numerator, _] => {
                            let denominator = if fields.len() == 3 {
                                fields[2].parse::<u8>()?
                            } else {
                                2
                            };
                            let numerator = numerator.parse::<u8>()?;
                            ensure!(
                                numerator > 0 && denominator <= 7,
                                "Unsupported time signature"
                            );
                            tempo.push((
                                tick,
                                1,
                                vec![0xff, 0x58, 4, numerator, denominator, 24, 8],
                            ));
                        }
                        _ => bail!("Unsupported SyncTrack event: {body}"),
                    }
                }
                continue;
            }
            "Events" => {
                for line in rows(body) {
                    let (tick, body) = event(line)?;
                    let text = quoted(
                        body.strip_prefix("E ")
                            .context("Unsupported global event")?
                            .trim(),
                    )?;
                    let text = if text.starts_with('[') {
                        text
                    } else {
                        format!("[{text}]")
                    };
                    global.push((tick, 1, meta(1, text.as_bytes())?));
                }
                continue;
            }
            _ => {}
        }
        let (difficulty, suffix) = ["Easy", "Medium", "Hard", "Expert"]
            .iter()
            .enumerate()
            .find_map(|(d, prefix)| section.name.strip_prefix(prefix).map(|s| (d, s)))
            .context(format!(
                "Unsupported section [{}]; export the editable chart separately",
                section.name
            ))?;
        let label = match suffix {
            "Single" => "PART GUITAR",
            "DoubleBass" => "PART BASS",
            "Drums" => "PART DRUMS",
            "Keyboard" => "PART KEYS",
            "ProKeys" => {
                crate::keys::midi_track(&section.name).context("Invalid Pro Keys difficulty")?
            }
            _ => bail!("Unsupported chart instrument [{}]", section.name),
        };
        let events = instruments
            .entry(label)
            .or_insert(vec![(0, 1, meta(3, label.as_bytes())?)]);
        let notes = chart.notes(&section.name)?;
        let frets = matches!(suffix, "Single" | "DoubleBass");
        if frets {
            use crate::gameplay_view::{self, Technique};
            let policy = gameplay_view::yarg_chart_policy(chart.timing.resolution, ini)?;
            let attacks: BTreeMap<_, _> =
                gameplay_view::classify(&section.name, &notes, &chart.timing, &policy)?
                    .into_iter()
                    .map(|gem| (gem.tick, gem.technique))
                    .collect();
            ensure!(
                notes
                    .iter()
                    .filter(|n| matches!(n.lane, 5 | 6))
                    .all(|n| attacks.contains_key(&n.tick)),
                "Orphan guitar type marker in {}",
                section.name
            );
            if notes.iter().any(|n| n.lane == 7) {
                events.push((0, 1, meta(1, b"[ENHANCED_OPENS]")?));
            }
            for (tick, kind) in attacks {
                match kind {
                    // Resolve the chart's policy explicitly; MIDI has different defaults.
                    Some(Technique::Hopo) => {
                        note(events, tick, 1, 65 + difficulty as u8 * 12, 100)?
                    }
                    Some(Technique::Strum) => {
                        note(events, tick, 1, 66 + difficulty as u8 * 12, 100)?
                    }
                    Some(Technique::Tap) => {
                        // Phase Shift tap ranges include their last tick. A point range
                        // isolates this attack, even beside a note one tick later.
                        for (priority, value) in [(1, 1), (3, 0)] {
                            events.push((
                                tick,
                                priority,
                                vec![0xf0, 8, b'P', b'S', 0, 0, difficulty as u8, 4, value, 0xf7],
                            ));
                        }
                    }
                    _ => bail!("Cannot resolve guitar note type in {}", section.name),
                }
            }
        }
        if suffix == "ProKeys" {
            for marker in crate::key_ranges::plan(&notes)?.markers {
                note(events, marker.tick, marker.sustain, marker.lane, 100)?;
            }
        }
        let cymbals: BTreeSet<_> = notes
            .iter()
            .filter(|n| (66..=68).contains(&n.lane))
            .map(|n| (n.tick, n.lane - 64))
            .collect();
        for line in rows(body) {
            let (tick, body) = event(line)?;
            let fields: Vec<_> = body.split_whitespace().collect();
            match fields.as_slice() {
                ["N", lane, duration] => {
                    let lane = lane.parse::<u8>()?;
                    let duration = duration.parse::<u64>()?;
                    let base = 60 + difficulty as u8 * 12;
                    if frets && matches!(lane, 5 | 6) {
                        continue;
                    }
                    if frets && lane == 7 {
                        note(events, tick, duration, base - 1, 100)?;
                        continue;
                    }
                    if suffix == "ProKeys" {
                        ensure!(
                            (crate::keys::MIN_PITCH..=crate::keys::MAX_PITCH).contains(&lane),
                            "Pro Keys pitch {lane} is outside MIDI48–72"
                        );
                        note(events, tick, duration, lane, 100)?;
                        continue;
                    }
                    if suffix == "Drums" && (66..=68).contains(&lane) {
                        continue;
                    }
                    ensure!(
                        lane <= 4,
                        "Chart lane {lane} in {} needs an explicit MIDI conversion; export refused",
                        section.name
                    );
                    note(events, tick, duration, base + lane, 100)?;
                    if suffix == "Drums" && lane >= 2 && !cymbals.contains(&(tick, lane)) {
                        note(
                            events,
                            tick,
                            duration.max(resolution as u64 / 8),
                            108 + lane,
                            100,
                        )?;
                    }
                }
                ["S", "2", duration] => note(events, tick, duration.parse()?, 116, 100)?,
                _ => bail!(
                    "Unsupported event in {}: {body}; source project retained",
                    section.name
                ),
            }
        }
    }
    // Shared drum markers/starpower from multiple difficulties must not become
    // duplicate simultaneous note-ons in the MIDI track.
    let mut chunks = vec![track(tempo)?, track(global)?];
    for (_, mut events) in instruments {
        events.sort();
        // Drum tom/star-power markers are shared across difficulty tracks.
        // Union their intervals so overlapping profiles cannot turn one marker
        // off while another profile is still holding it.
        for key in [110, 111, 112, 116] {
            let mut spans = Vec::new();
            // Derive spans from all onset/end pairs with an overlap counter,
            // rather than pairing one onset with the first off event.
            let mut count = 0usize;
            let mut start = 0;
            for (tick, _, data) in &events {
                if data.len() != 3 || data[1] != key {
                    continue;
                }
                if data[0] == 0x90 {
                    if count == 0 {
                        start = *tick;
                    }
                    count += 1;
                }
                if data[0] == 0x80 {
                    ensure!(count > 0, "Unpaired MIDI marker");
                    count -= 1;
                    if count == 0 {
                        spans.push((start, *tick));
                    }
                }
            }
            ensure!(count == 0, "Unclosed MIDI marker");
            events.retain(|(_, _, data)| {
                !(data.len() == 3 && data[1] == key && matches!(data[0], 0x80 | 0x90))
            });
            for (start, end) in spans {
                note(&mut events, start, end - start, key, 100)?;
            }
        }
        events.sort();
        events.dedup();
        chunks.push(track(events)?);
    }
    let mut out = b"MThd\0\0\0\x06\0\x01".to_vec();
    out.extend(u16::try_from(chunks.len())?.to_be_bytes());
    out.extend(resolution.to_be_bytes());
    for chunk in chunks {
        out.extend(chunk);
    }
    Ok(out)
}

pub fn with_vocals(chart: &Chart, vocals: &VocalChart) -> Result<Vec<u8>> {
    crate::vocals::append_midi(&chart_to_midi(chart)?, vocals)
}

/// MIDI has no .chart audio offset field. Preserve exact millisecond offsets
/// through song.ini's `delay`, as YARG's SongRunner expects. Callers must write
/// both outputs together. Sub-millisecond offsets are refused, not rounded.
pub fn song_with_vocals(chart: &Chart, vocals: &VocalChart) -> Result<(Vec<u8>, i64)> {
    song(chart, Some(vocals))
}

/// Convert the editable master to one playable MIDI, optionally with vocals.
/// Pro Keys custom working sections must always pass through this conversion.
pub fn song(chart: &Chart, vocals: Option<&VocalChart>) -> Result<(Vec<u8>, i64)> {
    song_with_ini(chart, vocals, &BTreeMap::new())
}
pub fn song_with_ini(
    chart: &Chart,
    vocals: Option<&VocalChart>,
    ini: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, i64)> {
    let delay = chart.timing.offset * 1000.;
    ensure!(
        delay.abs() <= i32::MAX as f64 && (delay - delay.round()).abs() < 0.000001,
        "Chart Offset cannot be preserved exactly in MIDI song.ini delay"
    );
    let midi = chart_ticks_to_midi(chart, ini)?;
    let midi = if let Some(vocals) = vocals {
        crate::vocals::append_midi(&midi, vocals)?
    } else {
        midi
    };
    Ok((midi, delay.round() as i64))
}

pub fn ini_with_delay(raw: &str, delay: i64) -> String {
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut output = String::new();
    let mut song = false;
    let mut found = false;
    for line in raw.lines() {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.starts_with('[') {
            if song && !found {
                output.push_str(&format!("delay = {delay}{newline}"));
                found = true;
            }
            song = trimmed.eq_ignore_ascii_case("[song]");
        }
        if song
            && trimmed.split_once('=').is_some_and(|(key, _)| {
                key.trim().eq_ignore_ascii_case("delay")
                    || key.trim().eq_ignore_ascii_case("delay_seconds")
            })
        {
            if !found {
                output.push_str(&format!("delay = {delay}{newline}"));
                found = true;
            }
        } else {
            output.push_str(line);
            output.push_str(newline);
        }
    }
    if !found {
        if !song {
            output.push_str(&format!("[song]{newline}"));
        }
        output.push_str(&format!("delay = {delay}{newline}"));
    }
    output
}
