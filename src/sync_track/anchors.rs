//! Moonscraper anchors constrain tempo edits; B events define playback time.
use super::Edit;
use crate::{
    chart::Chart,
    timing::{Tempo, TempoMap},
};
use anyhow::{Context, Result, ensure};

#[cfg(test)]
#[path = "anchors/tests.rs"]
mod tests;

const TOLERANCE: f64 = 0.001;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub tick: u64,
    pub microseconds: u64,
}

pub fn parse(body: &str) -> Result<Vec<Anchor>> {
    let mut anchors = Vec::new();
    for line in body.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let Some((tick, body)) = line.split_once('=') else {
            continue;
        };
        let words: Vec<_> = body.split_whitespace().collect();
        if words.first() != Some(&"A") {
            continue;
        }
        ensure!(words.len() == 2, "Malformed timing anchor");
        let microseconds = words[1]
            .parse::<u64>()
            .context("Invalid anchor microseconds")?;
        ensure!(
            microseconds <= (1u64 << 53),
            "Anchor exceeds exact microsecond range"
        );
        anchors.push(Anchor {
            tick: tick.trim().parse().context("Invalid anchor tick")?,
            microseconds,
        });
    }
    anchors.sort_by_key(|a| a.tick);
    ensure!(anchors.len() <= 2048, "Too many timing anchors");
    ensure!(
        anchors
            .windows(2)
            .all(|a| a[0].tick < a[1].tick && a[0].microseconds < a[1].microseconds),
        "Timing anchors must have unique ticks and increasing times"
    );
    Ok(anchors)
}

pub fn from_chart(chart: &Chart) -> Result<Vec<Anchor>> {
    chart
        .sections
        .iter()
        .find(|s| s.name == "SyncTrack")
        .map_or_else(
            || Ok(Vec::new()),
            |s| parse(&chart.raw[s.body_start..s.body_end]),
        )
}

pub fn validate(anchors: &[Anchor], clock: &TempoMap) -> Result<()> {
    for anchor in anchors {
        let target = anchor.microseconds as f64 / 1_000_000.;
        ensure!(
            (clock.seconds(anchor.tick) - clock.offset - target).abs() <= TOLERANCE,
            "Anchor at tick {} disagrees with the tempo map by more than 1 ms; correct the source timing before import",
            anchor.tick
        );
    }
    Ok(())
}

pub fn constrain(tempos: &mut Vec<Tempo>, source: &Chart, edit: &Edit) -> Result<()> {
    let anchors = from_chart(source)?;
    if anchors.is_empty() {
        return Ok(());
    }
    match edit {
        Edit::PutTempo {
            previous: Some(previous),
            tempo,
        } => ensure!(
            previous.tick == tempo.tick || !anchors.iter().any(|a| a.tick == previous.tick),
            "An anchored tempo cannot be moved; its anchor is fixed at the source tick"
        ),
        Edit::RemoveTempo { tempo } => ensure!(
            !anchors.iter().any(|a| a.tick == tempo.tick),
            "An anchored tempo cannot be removed"
        ),
        Edit::PutMeter { .. } | Edit::RemoveMeter { .. } => return Ok(()),
        _ => {}
    }
    TempoMap::new(
        source.timing.resolution,
        source.timing.offset,
        tempos.clone(),
    )?;
    // An A row without a B at the same tick still defines a boundary for edits.
    // Splitting the current tempo there preserves the original playable clock.
    for anchor in &anchors {
        if let Err(index) = tempos.binary_search_by_key(&anchor.tick, |t| t.tick) {
            let bpm = index.checked_sub(1).map_or(120., |i| tempos[i].bpm);
            tempos.insert(
                index,
                Tempo {
                    tick: anchor.tick,
                    bpm,
                },
            );
        }
    }
    for anchor in &anchors {
        let index = tempos
            .binary_search_by_key(&anchor.tick, |t| t.tick)
            .unwrap();
        if index == 0 {
            continue;
        }
        let clock = TempoMap::new(
            source.timing.resolution,
            source.timing.offset,
            tempos.clone(),
        )?;
        let previous = &tempos[index - 1];
        let interval =
            anchor.microseconds as f64 / 1_000_000. - (clock.seconds(previous.tick) - clock.offset);
        ensure!(
            interval.is_finite() && interval > 0.,
            "Tempo edit would cross a timing anchor"
        );
        let bpm = (60_000. * (anchor.tick - previous.tick) as f64
            / (source.timing.resolution as f64 * interval))
            .round()
            / 1000.;
        ensure!(
            (1. ..=1000.).contains(&bpm),
            "Anchor requires a tempo outside 1–1000 BPM"
        );
        if let Edit::PutTempo { tempo, .. } = edit {
            ensure!(
                tempo.tick != previous.tick || (tempo.bpm - bpm).abs() < 1e-7,
                "That tempo is fixed by the next anchor; add another tempo before adjusting this interval"
            );
        }
        tempos[index - 1].bpm = bpm;
    }
    let clock = TempoMap::new(
        source.timing.resolution,
        source.timing.offset,
        tempos.clone(),
    )?;
    validate(&anchors, &clock)
}
