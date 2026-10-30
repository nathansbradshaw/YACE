//! Per-track Star Power ranges. Other chart events stay in the source copy.
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
// A malformed source phrase disables authoring for that track, not note editing.
pub type Source = BTreeMap<String, Option<Vec<Phrase>>>;

fn is_power(line: &str) -> bool {
    line.split_once('=').is_some_and(|(_, body)| {
        let mut words = body.split_whitespace();
        words.next() == Some("S") && words.next() == Some("2")
    })
}
pub fn imported(chart: &Chart, track: &str) -> Result<Vec<Phrase>> {
    let Some(section) = chart.sections.iter().find(|s| s.name == track) else {
        return Ok(Vec::new());
    };
    let mut phrases = Vec::new();
    for line in chart.raw[section.body_start..section.body_end]
        .lines()
        .filter(|line| is_power(line))
    {
        let (tick, body) = line.split_once('=').unwrap();
        let words: Vec<_> = body.split_whitespace().collect();
        ensure!(words.len() == 3, "Malformed Star Power event in {track}");
        let start: u64 = tick.trim().parse()?;
        let length: u64 = words[2].parse()?;
        phrases.push(Phrase {
            start,
            end: start
                .checked_add(length)
                .context("Star Power tick overflow")?,
        });
    }
    phrases.sort_by_key(|phrase| phrase.start);
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
        .star_power
        .get(track)
        .cloned()
        .map_or_else(|| imported(source, track), Ok)
}
fn validate_shape(phrases: &[Phrase]) -> Result<()> {
    ensure!(
        phrases.len() <= 2048,
        "A track can have up to 2,048 Star Power phrases"
    );
    ensure!(
        phrases.iter().all(|p| p.start < p.end),
        "Star Power must end after it starts"
    );
    ensure!(
        phrases.windows(2).all(|p| p[0].end <= p[1].start),
        "Star Power phrases cannot overlap or share a start tick"
    );
    Ok(())
}
pub fn validate(phrases: &[Phrase], timing: &TempoMap, duration: f64) -> Result<()> {
    validate_shape(phrases)?;
    ensure!(
        phrases
            .iter()
            .all(|p| timing.seconds(p.end).is_finite() && timing.seconds(p.end) <= duration + 0.1),
        "Star Power extends beyond the recording"
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
        project.content.tracks.contains_key(track),
        "Choose an instrument track for Star Power"
    );
    let mut phrases = current(project, source, track)?;
    if let Some(previous) = previous {
        let index = phrases
            .iter()
            .position(|p| p == previous)
            .context("This Star Power phrase changed. Open it again.")?;
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
    project.content.star_power.insert(track.clone(), phrases);
    Ok(())
}

pub fn selected(
    project: &Project,
    selection: &crate::note_groups::Selection,
    step: u64,
) -> Result<Phrase> {
    let notes = project
        .content
        .tracks
        .get(&selection.track)
        .context("Select instrument notes first")?;
    let selected: Vec<_> = notes
        .iter()
        .filter(|n| {
            selection.ids.contains(&n.id) && crate::editor::playable(&selection.track, n.note.lane)
        })
        .collect();
    ensure!(
        !selected.is_empty() && selected.len() == selection.ids.len(),
        "Select notes to cover with Star Power; refresh a changed selection"
    );
    let mut phrase = Phrase {
        start: u64::MAX,
        end: 0,
    };
    for item in selected {
        phrase.start = phrase.start.min(item.note.tick);
        phrase.end = phrase.end.max(
            item.note
                .tick
                .checked_add(item.note.sustain.max(step.max(1)))
                .context("Star Power tick overflow")?,
        );
    }
    Ok(phrase)
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
    let mut output = chart.raw.clone();
    for (track, phrases) in tracks {
        validate_shape(phrases)?;
        ensure!(
            phrases.is_empty() || chart.sections.iter().any(|s| s.name == *track),
            "Missing Star Power track {track}"
        );
    }
    for section in chart.sections.iter().rev() {
        let Some(phrases) = tracks.get(&section.name) else {
            continue;
        };
        let lines: String = phrases
            .iter()
            .map(|p| format!("  {} = S 2 {}{newline}", p.start, p.end - p.start))
            .collect();
        let mut body = String::new();
        let mut inserted = false;
        for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if is_power(line) {
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
    }
    Ok(output)
}
