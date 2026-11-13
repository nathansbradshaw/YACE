//! Convert supported .chart events to MIDI for game exports with vocals.
//! Keep the editable .chart in the project. Reject unsupported events before
//! export so conversion cannot silently lose them.
use crate::{chart::Chart, vocals::VocalChart};
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

type Events = Vec<(u64, u8, Vec<u8>)>;

#[cfg(test)]
#[path = "game_export/tests.rs"]
mod tests;

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
                        // Anchors constrain editing, while B events define the
                        // playable clock. The bound chart retains A metadata.
                        ["A", _] => {}
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
            "DoubleRhythm" => "PART RHYTHM",
            "DoubleGuitar" => "PART GUITAR COOP",
            "GHLGuitar" => "PART GUITAR GHL",
            "GHLBass" => "PART BASS GHL",
            "GHLRhythm" => "PART RHYTHM GHL",
            "GHLCoop" => "PART GUITAR COOP GHL",
            "GHLKeys" => "PART KEYS GHL",
            "Drums" => "PART DRUMS",
            "Keyboard" => "PART KEYS",
            "ProKeys" => {
                crate::keys::midi_track(&section.name).context("Invalid Pro Keys difficulty")?
            }
            "ProGuitar17" | "ProGuitar22" | "ProBass17" | "ProBass22" => {
                crate::pro_guitar::midi_name(suffix).unwrap()
            }
            _ => bail!("Unsupported chart instrument [{}]", section.name),
        };
        let events = instruments
            .entry(label)
            .or_insert(vec![(0, 1, meta(3, label.as_bytes())?)]);
        if crate::lane_ranges::supports(&section.name, crate::lane_ranges::Kind::Trill) {
            crate::lane_ranges::imported(chart, &section.name)?;
        }
        let notes = chart.notes(&section.name)?;
        let pro_guitar = crate::pro_guitar::is_track(suffix);
        let six_fret = matches!(
            suffix,
            "GHLGuitar" | "GHLBass" | "GHLRhythm" | "GHLCoop" | "GHLKeys"
        );
        let frets = six_fret
            || matches!(
                suffix,
                "Single" | "DoubleBass" | "DoubleRhythm" | "DoubleGuitar" | "Keyboard"
            );
        if frets {
            use crate::gameplay_view::{self, Technique};
            let policy = gameplay_view::yarg_chart_policy(chart.timing.resolution, ini)?;
            let attacks: BTreeMap<_, _> =
                gameplay_view::classify_for_midi(&section.name, &notes, &chart.timing, &policy)?
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
            if !six_fret && notes.iter().any(|n| n.lane == 7) {
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
        let positions: BTreeSet<_> = notes.iter().map(|n| (n.tick, n.lane)).collect();
        let mut dynamics = BTreeMap::new();
        if suffix == "Drums" {
            for marker in &notes {
                let (lane, velocity) = match marker.lane {
                    34..=37 => (marker.lane - 33, 127),
                    40..=43 => (marker.lane - 39, 1),
                    _ => continue,
                };
                ensure!(
                    positions.contains(&(marker.tick, lane)),
                    "Orphan drum dynamics marker in {}",
                    section.name
                );
                if let Some(previous) = dynamics.insert((marker.tick, lane), velocity) {
                    ensure!(
                        previous == velocity,
                        "Conflicting drum dynamics in {}",
                        section.name
                    );
                }
            }
            if !dynamics.is_empty() {
                events.push((0, 1, meta(1, b"[ENABLE_CHART_DYNAMICS]")?));
            }
        }
        let mut solo_boundaries = Vec::new();
        for line in rows(body) {
            let (tick, body) = event(line)?;
            let fields: Vec<_> = body.split_whitespace().collect();
            match fields.as_slice() {
                ["N", lane, duration] => {
                    let lane = lane.parse::<u8>()?;
                    let duration = duration.parse::<u64>()?;
                    if pro_guitar {
                        ensure!(
                            lane < 6 || lane == crate::pro_guitar::HOPO,
                            "Unsupported Pro Guitar string or control in {}",
                            section.name
                        );
                        let key = 24
                            + difficulty as u8 * 24
                            + if lane == crate::pro_guitar::HOPO {
                                6
                            } else {
                                lane
                            };
                        note(events, tick, duration, key, 100)?;
                        continue;
                    }
                    let base = 60 + difficulty as u8 * 12;
                    if frets && matches!(lane, 5 | 6) {
                        continue;
                    }
                    if six_fret {
                        let key = match lane {
                            0..=4 => base + lane - 1,
                            8 => base + 4,
                            7 => base - 2,
                            _ => bail!("Unsupported six-fret lane {lane} in {}", section.name),
                        };
                        note(events, tick, duration, key, 100)?;
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
                    if suffix == "Drums" && matches!(lane, 34..=37 | 40..=43) {
                        continue;
                    }
                    if suffix == "Drums" && lane == 32 {
                        ensure!(
                            difficulty == 3,
                            "Double kick MIDI export requires Expert drums"
                        );
                        note(events, tick, duration, 95, 100)?;
                        continue;
                    }
                    ensure!(
                        lane <= 4,
                        "Chart lane {lane} in {} needs an explicit MIDI conversion; export refused",
                        section.name
                    );
                    let velocity = dynamics.get(&(tick, lane)).copied().unwrap_or(100);
                    note(events, tick, duration, base + lane, velocity)?;
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
                ["S", kind @ ("64" | "65" | "66"), duration] if suffix == "Drums" => {
                    let duration = duration.parse::<u64>()?;
                    ensure!(duration > 0, "Empty drum phrase in {}", section.name);
                    match *kind {
                        "64" => {
                            for key in 120..=124 {
                                note(events, tick, duration, key, 100)?;
                            }
                        }
                        "65" => note(events, tick, duration, 126, 100)?,
                        "66" => note(events, tick, duration, 127, 100)?,
                        _ => unreachable!(),
                    }
                }
                ["S", code @ ("65" | "66"), duration]
                    if crate::lane_ranges::supports(
                        &section.name,
                        crate::lane_ranges::Kind::Trill,
                    ) =>
                {
                    use crate::lane_ranges::{Kind, Range};
                    let duration = duration.parse::<u64>()?;
                    let range = Range {
                        start: tick,
                        end: tick
                            .checked_add(duration)
                            .context("Lane marker tick overflow")?,
                        kind: if *code == "65" {
                            Kind::Tremolo
                        } else {
                            Kind::Trill
                        },
                    };
                    if suffix == "ProKeys" {
                        ensure!(
                            range.kind == Kind::Trill,
                            "Pro Keys tremolo is not a supported MIDI marker"
                        );
                        note(events, tick, duration, 127, 100)?;
                    } else {
                        let expert =
                            crate::lane_ranges::imported(chart, &format!("Expert{suffix}"))?;
                        let hard = crate::lane_ranges::imported(chart, &format!("Hard{suffix}"))?;
                        if difficulty == 3 {
                            note(
                                events,
                                tick,
                                duration,
                                if range.kind == Kind::Trill { 127 } else { 126 },
                                if hard.contains(&range) { 45 } else { 100 },
                            )?;
                        } else {
                            ensure!(
                                difficulty == 2 && expert.contains(&range),
                                "This independent chart-tier lane marker cannot be represented by standard MIDI scope; export .chart instead"
                            );
                        }
                    }
                }
                ["E", ..] => {
                    let text = body[1..].trim();
                    let text = if text.starts_with('"') {
                        quoted(text)?
                    } else {
                        text.to_owned()
                    };
                    match text.as_str() {
                        "solo" => solo_boundaries.push((tick, true)),
                        "soloend" => solo_boundaries.push((tick, false)),
                        _ => bail!(
                            "Unsupported event in {}: {body}; source project retained",
                            section.name
                        ),
                    }
                }
                _ => bail!(
                    "Unsupported event in {}: {body}; source project retained",
                    section.name
                ),
            }
        }
        // Local text events need not be ordered in the source. At an adjacent
        // boundary, close the previous solo before opening the next one.
        solo_boundaries.sort_unstable();
        let mut solo_start = None;
        for (tick, starts) in solo_boundaries {
            if starts {
                ensure!(
                    solo_start.replace(tick).is_none(),
                    "Overlapping solo starts in {}",
                    section.name
                );
            } else {
                let start = solo_start
                    .take()
                    .with_context(|| format!("Solo end without a start in {}", section.name))?;
                ensure!(tick > start, "Empty solo in {}", section.name);
                // Pro Keys uses MIDI 115; other supported instruments use 103.
                let key = if suffix == "ProKeys" || pro_guitar {
                    115
                } else {
                    103
                };
                note(events, start, tick - start, key, 100)?;
            }
        }
        ensure!(solo_start.is_none(), "Unclosed solo in {}", section.name);
    }
    // Shared phrases and drum markers from multiple difficulties must not become
    // duplicate simultaneous note-ons in the MIDI track.
    let mut chunks = vec![track(tempo)?, track(global)?];
    for (owner, mut events) in instruments {
        events.sort();
        // Phrase and tom markers are shared across difficulty tracks.
        // Union their intervals so overlapping profiles cannot turn one marker
        // off while another profile is still holding it.
        for key in [
            103, 110, 111, 112, 115, 116, 120, 121, 122, 123, 124, 126, 127,
        ] {
            // Instrument lane markers were already checked and scoped above.
            // Unioning them at velocity 100 would erase Expert+Hard scope.
            if matches!(key, 126 | 127) && owner != "PART DRUMS" {
                continue;
            }
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
    let delay = chart_delay(chart)?;
    let midi = chart_ticks_to_midi(chart, ini)?;
    let midi = if let Some(vocals) = vocals {
        crate::vocals::append_midi(&midi, vocals)?
    } else {
        midi
    };
    Ok((midi, delay))
}

/// The bound working master and game MIDI must use the same exact audio clock.
pub(crate) fn chart_delay(chart: &Chart) -> Result<i64> {
    let delay = chart.timing.offset * 1000.;
    ensure!(
        delay.abs() <= i32::MAX as f64 && (delay - delay.round()).abs() < 0.000001,
        "Chart Offset cannot be preserved exactly in MIDI song.ini delay"
    );
    Ok(delay.round() as i64)
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
