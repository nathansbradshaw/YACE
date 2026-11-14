//! Source-bound chart text events. Dedicated musical controls keep their owners.
use crate::{chart::Chart, editor::Project, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_EVENTS: usize = 16_384;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    Global,
    Local { track: String },
}
impl Scope {
    pub fn section(&self) -> &str {
        match self {
            Self::Global => "Events",
            Self::Local { track } => track,
        }
    }
    fn from_section(section: &str) -> Option<Self> {
        if section == "Events" {
            Some(Self::Global)
        } else if crate::parts::editable_track(section) {
            Some(Self::Local {
                track: section.into(),
            })
        } else {
            None
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub scope: Scope,
    pub tick: u64,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub source_sha256: String,
    pub events: Vec<Event>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Event>,
        scope: Scope,
        tick: u64,
        text: String,
    },
    Remove {
        event: Event,
    },
}
fn source_hash(source: &Chart) -> String {
    format!("{:x}", Sha256::digest(source.raw.as_bytes()))
}
fn plain(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= 1024
        && !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '\\'))
}
/// Match the existing section/BRE/solo/disco owners, rather than creating a
/// second editable copy that their renderers could silently overwrite.
pub fn dedicated(scope: &Scope, text: &str) -> bool {
    match scope {
        Scope::Global => {
            text.strip_prefix("section")
                .is_some_and(|s| s.starts_with(char::is_whitespace))
                || crate::bre::is_coda(text)
                || crate::bre::supported_context(text).is_err()
        }
        Scope::Local { track } => {
            matches!(text, "solo" | "soloend")
                || (track.ends_with("Drums")
                    && text
                        .trim_matches(['[', ']'])
                        .strip_prefix("mix")
                        .is_some_and(|s| s.starts_with(char::is_whitespace)))
        }
    }
}
fn parsed(line: &str, scope: &Scope, ordinal: usize) -> Option<Event> {
    let (tick, value) = line.split_once('=')?;
    let value = value.trim().strip_prefix('E')?;
    if !value.starts_with(char::is_whitespace) {
        return None;
    }
    let value = value.trim();
    if !value.starts_with('"') && value.contains("//") {
        return None;
    }
    let text = if value.starts_with('"') {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        value
    };
    // Unrecognized quoting, comments and other foreign syntax remain raw.
    if !plain(text) || dedicated(scope, text) {
        return None;
    }
    Some(Event {
        id: format!("source:{}:{ordinal}", scope.section()),
        scope: scope.clone(),
        tick: tick.trim().parse().ok()?,
        text: text.into(),
    })
}
pub fn imported(source: &Chart) -> Result<Document> {
    let mut events = Vec::new();
    for section in &source.sections {
        let Some(scope) = Scope::from_section(&section.name) else {
            continue;
        };
        let mut ordinal = 0;
        for line in source.raw[section.body_start..section.body_end].lines() {
            if let Some(event) = parsed(line, &scope, ordinal) {
                events.push(event);
                ordinal += 1;
                ensure!(
                    events.len() <= MAX_EVENTS,
                    "Edit at most 16,384 text events in one project"
                );
            }
        }
    }
    Ok(Document {
        version: 1,
        source_sha256: source_hash(source),
        events,
    })
}
pub fn current(project: &Project, source: &Chart) -> Result<Document> {
    if let Some(document) = &project.content.text_events {
        ensure!(
            document.version == 1 && document.source_sha256 == source_hash(source),
            "Text event source changed; reopen the current project"
        );
        Ok(document.clone())
    } else {
        imported(source)
    }
}
pub fn validate(
    document: &Document,
    project: &Project,
    source: &Chart,
    timing: &TempoMap,
) -> Result<()> {
    ensure!(
        document.version == 1 && document.source_sha256 == source_hash(source),
        "Text events belong to another source chart"
    );
    ensure!(
        document.events.len() <= MAX_EVENTS,
        "Edit at most 16,384 text events in one project"
    );
    let originals: BTreeMap<_, _> = imported(source)?
        .events
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let mut ids = BTreeSet::new();
    for event in &document.events {
        ensure!(ids.insert(&event.id), "Duplicate text event identity");
        let original = originals.get(&event.id);
        ensure!(
            original.is_some()
                || event.id.strip_prefix("new:").is_some_and(|n| !n.is_empty()
                    && n.len() <= 10
                    && n.bytes().all(|c| c.is_ascii_digit())),
            "Unknown text event identity"
        );
        ensure!(
            plain(&event.text),
            "Use 1–1024 plain characters without quotes, backslashes or control characters"
        );
        ensure!(
            !dedicated(&event.scope, &event.text),
            "Use the dedicated section, solo, BRE or drum-mix controls for this event"
        );
        if let Scope::Local { track } = &event.scope {
            ensure!(
                crate::parts::editable_track(track) && project.content.tracks.contains_key(track),
                "Choose an existing editable instrument difficulty for a local event"
            );
        }
        // Preserve pre-existing timing outside a recording's declared span.
        // New events and deliberate scope/tick moves must fit the timeline.
        if !original.is_some_and(|old| old.scope == event.scope && old.tick == event.tick) {
            let seconds = timing.seconds(event.tick);
            ensure!(
                seconds.is_finite() && seconds <= project.duration + 0.1,
                "Text event is beyond the recording or timeline"
            );
        }
    }
    Ok(())
}
pub fn apply(project: &mut Project, source: &Chart, edit: &Edit) -> Result<()> {
    let mut document = current(project, source)?;
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { event } => Some(event),
    };
    let id = if let Some(previous) = previous {
        let index = document
            .events
            .iter()
            .position(|e| e == previous)
            .context("This text event changed. Select it again")?;
        document.events.remove(index).id
    } else {
        (1..=MAX_EVENTS + 1)
            .map(|n| format!("new:{n}"))
            .find(|id| document.events.iter().all(|e| &e.id != id))
            .context("No text event identity available")?
    };
    if let Edit::Put {
        scope, tick, text, ..
    } = edit
    {
        document.events.push(Event {
            id,
            scope: scope.clone(),
            tick: *tick,
            text: text.clone(),
        });
    }
    validate(
        &document,
        project,
        source,
        &crate::sync_track::effective(project, &source.timing)?,
    )?;
    document
        .events
        .sort_by(|a, b| (&a.scope, a.tick, &a.id).cmp(&(&b.scope, b.tick, &b.id)));
    project.content.text_events = Some(document);
    Ok(())
}
fn line(event: &Event, newline: &str) -> String {
    format!("  {} = E \"{}\"{newline}", event.tick, event.text)
}

/// Run after the other chart renderers. Only the explicitly changed text rows
/// are replaced; untouched rows, foreign syntax and non-text data stay exact.
pub fn render(text: String, source: &Chart, document: &Option<Document>) -> Result<String> {
    let Some(document) = document else {
        return Ok(text);
    };
    ensure!(
        document.version == 1 && document.source_sha256 == source_hash(source),
        "Text event source changed"
    );
    let chart = Chart::parse(text)?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let before: BTreeMap<_, _> = imported(source)?
        .events
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();
    let after: BTreeMap<_, _> = document.events.iter().map(|e| (e.id.as_str(), e)).collect();
    ensure!(
        after.len() == document.events.len(),
        "Duplicate text event identity"
    );
    let mut used = BTreeSet::new();
    let mut output = chart.raw.clone();
    let mut global = false;
    for section in chart.sections.iter().rev() {
        let Some(scope) = Scope::from_section(&section.name) else {
            continue;
        };
        global |= scope == Scope::Global;
        let mut body = String::new();
        let mut ordinal = 0;
        for raw in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if let Some(original) = parsed(raw, &scope, ordinal) {
                ordinal += 1;
                ensure!(
                    before.get(&original.id) == Some(&original),
                    "Another editor changed this text event; reopen its current value"
                );
                if let Some(event) = after.get(original.id.as_str())
                    && event.scope == scope
                {
                    used.insert(event.id.as_str());
                    if **event == original {
                        body.push_str(raw);
                    } else {
                        body.push_str(&line(event, newline));
                    }
                }
            } else {
                body.push_str(raw);
            }
        }
        for event in document.events.iter().filter(|e| e.scope == scope) {
            if used.insert(event.id.as_str()) {
                body.push_str(&line(event, newline));
            }
        }
        output.replace_range(section.body_start..section.body_end, &body);
    }
    if !global {
        let events: Vec<_> = document
            .events
            .iter()
            .filter(|e| e.scope == Scope::Global)
            .collect();
        if !events.is_empty() {
            if !output.ends_with('\n') {
                output.push_str(newline);
            }
            output.push_str(&format!("{newline}[Events]{newline}{{{newline}"));
            for event in events {
                used.insert(event.id.as_str());
                output.push_str(&line(event, newline));
            }
            output.push_str(&format!("}}{newline}"));
        }
    }
    ensure!(
        document.events.iter().all(|e| used.contains(e.id.as_str())),
        "The text event's local part is absent from this chart"
    );
    Ok(output)
}
