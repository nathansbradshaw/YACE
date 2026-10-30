//! Editable tempo and meter events; the source resolution and offset stay fixed.
use crate::{
    chart::Chart,
    editor::Project,
    timing::{Tempo, TempoMap},
    vocal_review::VocalReviewMap,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub tick: u64,
    pub numerator: u8,
    pub denominator: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncTrack {
    pub tempos: Vec<Tempo>,
    pub meters: Vec<Meter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_vocal_review: Option<VocalReviewMap>,
}
fn event(line: &str) -> Option<(&str, &str)> {
    let (_, body) = line.split_once('=')?;
    let body = body.trim();
    let (kind, _) = body.split_once(char::is_whitespace)?;
    Some((kind, body))
}
pub fn imported(chart: &Chart) -> Result<SyncTrack> {
    let mut meters = Vec::new();
    if let Some(section) = chart.sections.iter().find(|s| s.name == "SyncTrack") {
        for line in chart.raw[section.body_start..section.body_end].lines() {
            if let Some(("TS", body)) = event(line) {
                let fields: Vec<_> = body.split_whitespace().collect();
                ensure!((2..=3).contains(&fields.len()), "Malformed time signature");
                let exponent: u32 = if fields.len() == 3 {
                    fields[2].parse()?
                } else {
                    2
                };
                let denominator = 1u16
                    .checked_shl(exponent)
                    .context("Unsupported time-signature denominator")?;
                meters.push(Meter {
                    tick: line.split_once('=').unwrap().0.trim().parse()?,
                    numerator: fields[1].parse()?,
                    denominator,
                });
            }
        }
    }
    meters.sort_by_key(|m| m.tick);
    if meters.first().is_none_or(|m| m.tick != 0) {
        meters.insert(
            0,
            Meter {
                tick: 0,
                numerator: 4,
                denominator: 4,
            },
        );
    }
    let result = SyncTrack {
        tempos: chart.timing.tempos.clone(),
        meters,
        archived_vocal_review: None,
    };
    result.validate(&chart.timing)?;
    Ok(result)
}
impl SyncTrack {
    pub fn validate(&self, source: &TempoMap) -> Result<TempoMap> {
        ensure!(
            !self.tempos.is_empty()
                && self.tempos.len() <= 2048
                && self.tempos[0].tick == 0
                && self.tempos.windows(2).all(|p| p[0].tick < p[1].tick),
            "Keep one tempo at tick 0 and unique tempos in time order"
        );
        let timing = TempoMap::new(source.resolution, source.offset, self.tempos.clone())?;
        ensure!(
            self.tempos
                .iter()
                .all(|t| (t.bpm * 1000. - (t.bpm * 1000.).round()).abs() < 1e-7),
            "Use BPM with at most three decimal places"
        );
        ensure!(
            !self.meters.is_empty()
                && self.meters.len() <= 2048
                && self.meters[0].tick == 0
                && self.meters.windows(2).all(|p| p[0].tick < p[1].tick),
            "Keep one time signature at tick 0 and unique signatures in time order"
        );
        ensure!(
            self.meters.iter().all(|m| m.numerator > 0
                && m.denominator.is_power_of_two()
                && m.denominator <= 128),
            "Use a positive numerator and a power-of-two denominator from 1 to 128"
        );
        Ok(timing)
    }
}
pub fn effective(project: &Project, source: &TempoMap) -> Result<TempoMap> {
    project
        .content
        .sync
        .as_ref()
        .map_or_else(|| Ok(source.clone()), |sync| sync.validate(source))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    PutTempo {
        previous: Option<Tempo>,
        tempo: Tempo,
    },
    RemoveTempo {
        tempo: Tempo,
    },
    PutMeter {
        previous: Option<Meter>,
        meter: Meter,
    },
    RemoveMeter {
        meter: Meter,
    },
}
pub fn apply(project: &mut Project, source: &Chart, edit: &Edit) -> Result<()> {
    let before = effective(project, &source.timing)?;
    let mut sync = project
        .content
        .sync
        .clone()
        .map_or_else(|| imported(source), Ok)?;
    let tick = match edit {
        Edit::PutTempo { previous, tempo } => {
            if let Some(previous) = previous {
                ensure!(
                    previous.tick != 0 || tempo.tick == 0,
                    "Keep the first tempo at tick 0"
                );
                let index = sync
                    .tempos
                    .iter()
                    .position(|t| t == previous)
                    .context("This tempo changed. Open it again.")?;
                sync.tempos.remove(index);
            }
            sync.tempos.push(tempo.clone());
            tempo.tick
        }
        Edit::RemoveTempo { tempo } => {
            ensure!(tempo.tick != 0, "The tempo at tick 0 cannot be removed");
            let index = sync
                .tempos
                .iter()
                .position(|t| t == tempo)
                .context("This tempo changed. Open it again.")?;
            sync.tempos.remove(index);
            0
        }
        Edit::PutMeter { previous, meter } => {
            if let Some(previous) = previous {
                ensure!(
                    previous.tick != 0 || meter.tick == 0,
                    "Keep the first time signature at tick 0"
                );
                let index = sync
                    .meters
                    .iter()
                    .position(|m| m == previous)
                    .context("This time signature changed. Open it again.")?;
                sync.meters.remove(index);
            }
            sync.meters.push(meter.clone());
            meter.tick
        }
        Edit::RemoveMeter { meter } => {
            ensure!(
                meter.tick != 0,
                "The time signature at tick 0 cannot be removed"
            );
            let index = sync
                .meters
                .iter()
                .position(|m| m == meter)
                .context("This time signature changed. Open it again.")?;
            sync.meters.remove(index);
            0
        }
    };
    sync.tempos.sort_by_key(|t| t.tick);
    sync.meters.sort_by_key(|m| m.tick);
    let timing = sync.validate(&source.timing)?;
    ensure!(
        timing.seconds(tick).is_finite() && timing.seconds(tick) <= project.duration,
        "Timing marker starts beyond the recording"
    );
    if before != timing
        && let Some(review) = project.content.vocal_review.take()
    {
        sync.archived_vocal_review = Some(review);
    }
    project.content.sync = Some(sync);
    Ok(())
}
pub fn render(text: String, sync: &Option<SyncTrack>) -> Result<String> {
    let Some(sync) = sync else { return Ok(text) };
    let chart = Chart::parse(text)?;
    sync.validate(&chart.timing)?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<_> = sync
        .tempos
        .iter()
        .map(|t| {
            (
                t.tick,
                1,
                format!("  {} = B {:.0}{newline}", t.tick, t.bpm * 1000.),
            )
        })
        .collect();
    lines.extend(sync.meters.iter().map(|m| {
        (
            m.tick,
            0,
            format!(
                "  {} = TS {} {}{newline}",
                m.tick,
                m.numerator,
                m.denominator.trailing_zeros()
            ),
        )
    }));
    lines.sort_by_key(|(tick, kind, _)| (*tick, *kind));
    let lines: String = lines.into_iter().map(|(_, _, line)| line).collect();
    let mut output = chart.raw.clone();
    if let Some(section) = chart.sections.iter().find(|s| s.name == "SyncTrack") {
        let mut body = String::new();
        let mut inserted = false;
        for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if matches!(event(line), Some(("B" | "TS", _))) {
                if !inserted {
                    body.push_str(&lines);
                    inserted = true;
                }
            } else {
                body.push_str(line);
            }
        }
        if !inserted {
            body.push_str(&lines);
        }
        output.replace_range(section.body_start..section.body_end, &body);
    } else {
        if !output.ends_with('\n') {
            output.push_str(newline);
        }
        output.push_str(&format!(
            "{newline}[SyncTrack]{newline}{{{newline}{lines}}}{newline}"
        ));
    }
    Ok(output)
}

#[derive(Clone, Debug)]
pub struct Bar {
    pub tick: u64,
    pub number: u64,
}
// A meter change starts a new bar, including when it cuts a previous bar short.
pub fn bars(timing: &TempoMap, meters: &[Meter], first: u64, last: u64, limit: usize) -> Vec<Bar> {
    let mut result = Vec::new();
    let mut bar_number = 1u128;
    for (index, meter) in meters.iter().enumerate() {
        let next = meters.get(index + 1).map(|m| m.tick);
        let length = timing.resolution as u128 * 4 * meter.numerator as u128;
        let denominator = meter.denominator as u128;
        if length == 0 || denominator == 0 {
            return result;
        }
        let end = next.map_or(last, |n| n.saturating_sub(1).min(last));
        let begin = first.max(meter.tick);
        // Bar ticks round to whole chart ticks, even at unusual resolutions.
        let index_at = |offset: u128| {
            (offset * denominator)
                .saturating_sub(denominator / 2)
                .div_ceil(length)
        };
        if begin <= end {
            let a = index_at((begin - meter.tick) as u128);
            let after = index_at((end - meter.tick) as u128 + 1);
            if after == 0 {
                continue;
            }
            let b = after - 1;
            let count = b.saturating_sub(a) + 1;
            let stride = count.div_ceil(limit.max(1) as u128).max(1);
            let mut i = a;
            while i <= b && result.len() < limit {
                let tick = meter.tick as u128 + (i * length + denominator / 2) / denominator;
                if let (Ok(tick), Ok(number)) = (u64::try_from(tick), u64::try_from(bar_number + i))
                {
                    result.push(Bar { tick, number });
                }
                i += stride;
            }
        }
        if let Some(next) = next {
            bar_number += index_at((next - meter.tick) as u128);
        } else {
            break;
        }
    }
    result
}
