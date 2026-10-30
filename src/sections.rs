//! Song section names share the chart clock. Other global events stay untouched.
use crate::{chart::Chart, editor::Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub tick: u64,
    pub name: String,
}

fn parse(line: &str) -> Option<Marker> {
    let (tick, event) = line.split_once('=')?;
    let event = event.trim().strip_prefix('E')?;
    if !event.starts_with(char::is_whitespace) {
        return None;
    }
    let value = event.trim();
    let value = if value.starts_with('"') {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        value
    };
    let name = value.strip_prefix("section")?;
    if !name.starts_with(char::is_whitespace) {
        return None;
    }
    Some(Marker {
        tick: tick.trim().parse().ok()?,
        name: name.trim().into(),
    })
}
pub fn imported(chart: &Chart) -> Vec<Marker> {
    let mut markers: Vec<_> = chart
        .sections
        .iter()
        .filter(|s| s.name == "Events")
        .flat_map(|s| {
            chart.raw[s.body_start..s.body_end]
                .lines()
                .filter_map(parse)
        })
        .collect();
    markers.sort_by_key(|marker| marker.tick);
    markers
}
pub fn current(project: &Project, source: &Chart) -> Vec<Marker> {
    project
        .content
        .sections
        .clone()
        .unwrap_or_else(|| imported(source))
}
pub fn validate(markers: &[Marker], timing: &crate::timing::TempoMap, duration: f64) -> Result<()> {
    ensure!(
        markers.len() <= 2048,
        "A song can have up to 2,048 sections"
    );
    let mut positions = BTreeSet::new();
    for marker in markers {
        ensure!(
            !marker.name.trim().is_empty()
                && marker.name == marker.name.trim()
                && marker.name.chars().count() <= 128,
            "Use a section name between 1 and 128 characters"
        );
        ensure!(
            !marker
                .name
                .chars()
                .any(|c| c.is_control() || "\"\\[]{}".contains(c)),
            "Section names cannot contain quotes, brackets or control characters"
        );
        ensure!(
            positions.insert(marker.tick),
            "Two sections cannot start at the same tick"
        );
        let seconds = timing.seconds(marker.tick);
        ensure!(
            seconds.is_finite() && seconds <= duration,
            "Section starts beyond the recording"
        );
    }
    ensure!(
        markers.windows(2).all(|pair| pair[0].tick < pair[1].tick),
        "Sections must be in time order"
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Marker>,
        marker: Marker,
    },
    Remove {
        marker: Marker,
    },
}
pub fn apply(project: &mut Project, source: &Chart, edit: &Edit) -> Result<()> {
    let mut markers = current(project, source);
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { marker } => Some(marker),
    };
    if let Some(previous) = previous {
        let index = markers
            .iter()
            .position(|m| m == previous)
            .context("This section changed. Open it again before editing.")?;
        markers.remove(index);
    }
    if let Edit::Put { marker, .. } = edit {
        markers.push(marker.clone());
    }
    markers.sort_by_key(|m| m.tick);
    validate(&markers, &source.timing, project.duration)?;
    project.content.sections = Some(markers);
    Ok(())
}

pub fn render(text: String, markers: &Option<Vec<Marker>>) -> Result<String> {
    let Some(markers) = markers else {
        return Ok(text);
    };
    let chart = Chart::parse(text)?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: String = markers
        .iter()
        .map(|m| format!("  {} = E \"section {}\"{newline}", m.tick, m.name))
        .collect();
    let mut output = chart.raw.clone();
    if let Some(section) = chart.sections.iter().find(|s| s.name == "Events") {
        let mut body = String::new();
        let mut inserted = false;
        for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if parse(line).is_some() {
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
    } else if !markers.is_empty() {
        if !output.ends_with('\n') {
            output.push_str(newline);
        }
        output.push_str(&format!(
            "{newline}[Events]{newline}{{{newline}{lines}}}{newline}"
        ));
    }
    Ok(output)
}
