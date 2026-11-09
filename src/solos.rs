//! Per-track solo boundaries, with untouched source events retained verbatim.
use crate::{chart::Chart, editor::Project, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub start: u64,
    pub end: u64,
}
pub type Tracks = BTreeMap<String, Vec<Phrase>>;
pub type Source = BTreeMap<String, Option<Vec<Phrase>>>;

fn boundary(line: &str) -> Option<(&str, bool)> {
    let (tick, body) = line.split_once('=')?;
    let value = body.trim().strip_prefix('E')?;
    if !value.starts_with(char::is_whitespace) {
        return None;
    }
    let value = value.trim();
    let value = if value.starts_with('"') {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        value
    };
    match value {
        "solo" => Some((tick.trim(), true)),
        "soloend" => Some((tick.trim(), false)),
        _ => None,
    }
}
fn validate_shape(phrases: &[Phrase]) -> Result<()> {
    ensure!(phrases.len() <= 2048, "A track can have up to 2,048 solos");
    ensure!(
        phrases.iter().all(|p| p.start < p.end),
        "A solo must end after it starts"
    );
    ensure!(
        phrases.windows(2).all(|p| p[0].end <= p[1].start),
        "Solos cannot overlap or share a start tick"
    );
    Ok(())
}
pub fn imported(chart: &Chart, track: &str) -> Result<Vec<Phrase>> {
    let sections: Vec<_> = chart.sections.iter().filter(|s| s.name == track).collect();
    ensure!(
        sections.len() <= 1,
        "Duplicate solo track sections are ambiguous"
    );
    let Some(section) = sections.first() else {
        return Ok(Vec::new());
    };
    let mut edges = chart.raw[section.body_start..section.body_end]
        .lines()
        .filter_map(boundary)
        .map(|(tick, start)| Ok((tick.parse::<u64>()?, start)))
        .collect::<Result<Vec<_>>>()?;
    // Match the game exporter: an end precedes a new start at a shared boundary.
    edges.sort_unstable();
    let mut active = None;
    let mut phrases = Vec::new();
    for (tick, start) in edges {
        if start {
            ensure!(
                active.replace(tick).is_none(),
                "Overlapping solo starts; source retained"
            );
        } else {
            phrases.push(Phrase {
                start: active
                    .take()
                    .context("Solo end has no start; source retained")?,
                end: tick,
            });
        }
    }
    ensure!(active.is_none(), "Unclosed solo; source retained");
    validate_shape(&phrases)?;
    Ok(phrases)
}
pub fn source(project: &Project, chart: &Chart) -> Source {
    project
        .content
        .tracks
        .keys()
        .map(|track| (track.clone(), imported(chart, track).ok()))
        .collect()
}
pub fn current(project: &Project, source: &Chart, track: &str) -> Result<Vec<Phrase>> {
    project
        .content
        .solos
        .get(track)
        .cloned()
        .map_or_else(|| imported(source, track), Ok)
}
pub fn validate(phrases: &[Phrase], timing: &TempoMap, duration: f64) -> Result<()> {
    validate_shape(phrases)?;
    ensure!(
        phrases.iter().all(|p| {
            let end = timing.seconds(p.end);
            end.is_finite() && end <= duration + 0.1
        }),
        "Solo extends beyond the recording or timeline"
    );
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        track: String,
        previous: Option<Phrase>,
        phrase: Phrase,
    },
    Remove {
        track: String,
        phrase: Phrase,
    },
}
pub fn apply(project: &mut Project, source: &Chart, edit: &Edit) -> Result<()> {
    let (track, previous) = match edit {
        Edit::Put {
            track, previous, ..
        } => (track, previous.as_ref()),
        Edit::Remove { track, phrase } => (track, Some(phrase)),
    };
    ensure!(
        crate::parts::editable_track(track) && project.content.tracks.contains_key(track),
        "Choose an editable instrument track for solos"
    );
    // A corrupt source disables solo authoring without disabling note editing.
    imported(source, track)?;
    let mut phrases = current(project, source, track)?;
    if let Some(previous) = previous {
        let index = phrases
            .iter()
            .position(|p| p == previous)
            .context("This solo changed. Select it again.")?;
        phrases.remove(index);
    }
    if let Edit::Put { phrase, .. } = edit {
        phrases.push(phrase.clone());
    }
    phrases.sort_by_key(|p| p.start);
    validate(
        &phrases,
        &crate::sync_track::effective(project, &source.timing)?,
        project.duration,
    )?;
    project.content.solos.insert(track.clone(), phrases);
    Ok(())
}
pub fn selected(
    project: &Project,
    selection: &crate::note_groups::Selection,
    step: u64,
) -> Result<Phrase> {
    let phrase = crate::star_power::selected(project, selection, step)?;
    Ok(Phrase {
        start: phrase.start,
        end: phrase.end,
    })
}
pub fn render(text: String, tracks: &Tracks) -> Result<String> {
    if tracks.is_empty() {
        return Ok(text);
    }
    let chart = Chart::parse(text)?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for (track, phrases) in tracks {
        imported(&chart, track)?;
        validate_shape(phrases)?;
        ensure!(
            phrases.is_empty() || chart.sections.iter().any(|s| s.name == *track),
            "Missing solo track {track}"
        );
    }
    let mut output = chart.raw.clone();
    for section in chart.sections.iter().rev() {
        let Some(phrases) = tracks.get(&section.name) else {
            continue;
        };
        let mut body: String = chart.raw[section.body_start..section.body_end]
            .split_inclusive('\n')
            .filter(|line| boundary(line).is_none())
            .collect();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(newline);
        }
        for phrase in phrases {
            body.push_str(&format!(
                "  {} = E solo{newline}  {} = E soloend{newline}",
                phrase.start, phrase.end
            ));
        }
        output.replace_range(section.body_start..section.body_end, &body);
    }
    Ok(output)
}

#[cfg(test)]
#[path = "solos/tests.rs"]
mod tests;
