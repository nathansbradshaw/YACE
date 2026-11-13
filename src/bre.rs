//! Big rock endings require global coda context. A drum S64 alone remains a fill.
use crate::{chart::Chart, song_files::game_midi::NoteAttributes, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[path = "bre_project.rs"]
mod project;
#[cfg(test)]
#[path = "bre_project_tests.rs"]
mod project_tests;
#[cfg(test)]
#[path = "bre_tests.rs"]
mod tests;
pub use project::{
    apply, current, midi_required, owner, render_project, selected, source, validate,
};

pub const OWNERS: [(&str, &str); 5] = [
    ("PART GUITAR", "Single"),
    ("PART BASS", "DoubleBass"),
    ("PART GUITAR COOP", "DoubleGuitar"),
    ("PART RHYTHM", "DoubleRhythm"),
    ("PART KEYS", "Keyboard"),
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coda {
    pub tick: u64,
    pub text: String,
}
pub fn is_coda(text: &str) -> bool {
    matches!(text.trim(), "coda" | "[coda]")
}
pub fn supported_context(text: &str) -> Result<()> {
    ensure!(
        !matches!(
            text.trim().trim_matches(['[', ']']),
            "midcoda" | "midcoda_end"
        ),
        "Mid-song coda contexts remain read-only; retain their original events"
    );
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub range: Range,
    pub lanes: [NoteAttributes; 5],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u32,
    pub resolution: u32,
    pub source_sha256: Option<String>,
    pub coda: Option<Coda>,
    pub owners: BTreeMap<String, Vec<Phrase>>,
}
impl State {
    pub fn empty(resolution: u32) -> Self {
        Self {
            version: 1,
            resolution,
            source_sha256: None,
            coda: None,
            owners: BTreeMap::new(),
        }
    }
    pub fn validate_shape(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported BRE state"
        );
        ensure!(
            self.source_sha256
                .as_ref()
                .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid BRE source binding"
        );
        if let Some(coda) = &self.coda {
            ensure!(
                coda.tick <= u64::from(u32::MAX) && is_coda(&coda.text),
                "Invalid coda witness"
            );
        }
        for (owner, phrases) in &self.owners {
            ensure!(
                OWNERS.iter().any(|(name, _)| name == owner) && phrases.len() <= 2048,
                "Unsupported BRE owner or count"
            );
            let mut last = 0;
            for p in phrases {
                ensure!(
                    p.range.start >= last
                        && p.range.start < p.range.end
                        && p.range.end <= u64::from(u32::MAX),
                    "Overlapping or empty BRE controls"
                );
                last = p.range.end;
                for attrs in &p.lanes {
                    attrs.validate()?;
                    ensure!(
                        !attrs.zero_length && attrs.talkie_pitch.is_none(),
                        "Invalid BRE lane attributes"
                    );
                }
            }
        }
        Ok(())
    }
    pub fn validate(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.validate_shape()?;
        ensure!(
            self.resolution == timing.resolution,
            "BRE/source resolution changed"
        );
        for tick in self
            .coda
            .iter()
            .map(|c| c.tick)
            .chain(self.owners.values().flatten().map(|p| p.range.end))
        {
            let seconds = timing.seconds(tick);
            ensure!(
                seconds.is_finite() && seconds <= duration + 0.1,
                "BRE extends beyond the recording or timeline"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    DrumPut {
        track: String,
        previous: Option<crate::drum_ranges::Range>,
        range: Range,
    },
    DrumRemove {
        track: String,
        range: crate::drum_ranges::Range,
    },
    SetCoda {
        previous: Option<Coda>,
        tick: Option<u64>,
    },
    Put {
        owner: String,
        previous: Option<Phrase>,
        range: Range,
    },
    Remove {
        owner: String,
        phrase: Phrase,
    },
}
pub fn edited(state: &State, edit: &Edit) -> Result<State> {
    state.validate_shape()?;
    let mut next = state.clone();
    match edit {
        Edit::DrumPut { .. } | Edit::DrumRemove { .. } => {
            anyhow::bail!("Drum BRE uses the project's existing activation ranges")
        }
        Edit::SetCoda { previous, tick } => {
            ensure!(*previous == state.coda, "Coda changed. Select it again.");
            next.coda = tick.map(|tick| Coda {
                tick,
                text: previous
                    .as_ref()
                    .map_or_else(|| "[coda]".into(), |p| p.text.clone()),
            });
        }
        Edit::Put {
            owner,
            previous,
            range,
        } => {
            ensure!(
                OWNERS.iter().any(|(name, _)| name == owner),
                "Choose a supported physical MIDI BRE owner"
            );
            ensure!(
                state.coda.as_ref().is_some_and(|c| c.tick <= range.start),
                "Set the global coda at or before this BRE first"
            );
            let phrases = next.owners.entry(owner.clone()).or_default();
            if let Some(old) = previous {
                let at = phrases
                    .iter()
                    .position(|p| p == old)
                    .ok_or_else(|| anyhow::anyhow!("BRE changed. Select it again."))?;
                phrases.remove(at);
            }
            let lanes = previous.as_ref().map_or_else(
                || {
                    std::array::from_fn(|_| NoteAttributes {
                        channel: 0,
                        velocity: 100,
                        off_velocity: 0,
                        off_as_note_on: false,
                        zero_length: false,
                        talkie_pitch: None,
                    })
                },
                |p| p.lanes.clone(),
            );
            phrases.push(Phrase {
                range: range.clone(),
                lanes,
            });
            phrases.sort_by_key(|p| (p.range.start, p.range.end));
        }
        Edit::Remove { owner, phrase } => {
            let phrases = next
                .owners
                .get_mut(owner)
                .ok_or_else(|| anyhow::anyhow!("BRE owner changed"))?;
            let at = phrases
                .iter()
                .position(|p| p == phrase)
                .ok_or_else(|| anyhow::anyhow!("BRE changed. Select it again."))?;
            phrases.remove(at);
        }
    }
    next.owners.retain(|_, p| !p.is_empty());
    next.validate_shape()?;
    Ok(next)
}
fn row(line: &str) -> Result<Option<(u64, String)>> {
    let Some((tick, body)) = line.split_once('=') else {
        return Ok(None);
    };
    let body = body.trim();
    let Some(text) = body.strip_prefix("E ").or_else(|| body.strip_prefix("E\t")) else {
        return Ok(None);
    };
    let text = text.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(text);
    supported_context(text)?;
    if !is_coda(text) {
        return Ok(None);
    }
    Ok(Some((
        tick.trim()
            .parse()
            .context("Invalid global coda tick; retain source")?,
        text.into(),
    )))
}
pub fn chart_coda(chart: &Chart) -> Result<Option<Coda>> {
    let mut markers = vec![];
    for section in chart.sections.iter().filter(|s| s.name == "Events") {
        for line in chart.raw[section.body_start..section.body_end].lines() {
            if let Some((tick, text)) = row(line)? {
                markers.push(Coda { tick, text });
            }
        }
    }
    ensure!(
        markers.len() <= 1,
        "Multiple global coda markers are read-only; retain source"
    );
    Ok(markers.pop())
}
pub fn render_chart(text: String, state: Option<&State>) -> Result<String> {
    let Some(state) = state else { return Ok(text) };
    state.validate_shape()?;
    let chart = Chart::parse(text)?;
    ensure!(
        state.owners.is_empty(),
        "Guitar/keys BRE requires MIDI; .chart has no representation"
    );
    if chart_coda(&chart)? == state.coda {
        return Ok(chart.raw);
    }
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = chart.raw.clone();
    let sections: Vec<_> = chart
        .sections
        .iter()
        .filter(|s| s.name == "Events")
        .collect();
    ensure!(
        sections.len() <= 1,
        "Duplicate global event sections are read-only"
    );
    if let Some(section) = sections.first() {
        let mut body = String::new();
        for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if row(line)?.is_none() {
                body.push_str(line);
            }
        }
        if !body.ends_with('\n') && !body.is_empty() {
            body.push_str(newline);
        }
        if let Some(coda) = &state.coda {
            body.push_str(&format!(
                "  {} = E \"{}\"{newline}",
                coda.tick,
                crate::chart::quoted(&coda.text)
            ));
        }
        output.replace_range(section.body_start..section.body_end, &body);
    } else if let Some(coda) = &state.coda {
        output.push_str(&format!(
            "{newline}[Events]{newline}{{{newline}  {} = E \"{}\"{newline}}}{newline}",
            coda.tick,
            crate::chart::quoted(&coda.text)
        ));
    }
    Ok(output)
}
