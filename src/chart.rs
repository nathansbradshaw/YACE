use crate::timing::{Tempo, TempoMap};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Note {
    pub tick: u64,
    pub lane: u8,
    pub sustain: u64,
}

pub type Tracks = BTreeMap<String, Vec<Note>>;

#[derive(Debug)]
pub struct Section {
    pub name: String,
    pub body_start: usize,
    pub body_end: usize,
}

pub struct Chart {
    pub raw: String,
    pub sections: Vec<Section>,
    pub timing: TempoMap,
}

impl Chart {
    pub fn parse(raw: String) -> Result<Self> {
        let mut sections = Vec::new();
        let mut pending: Option<String> = None;
        let mut open: Option<(String, usize)> = None;
        let mut names = BTreeSet::new();
        let mut offset = 0;
        for line in raw.split_inclusive('\n') {
            let s = line.trim().trim_start_matches('\u{feff}');
            if s.starts_with('[') && s.ends_with(']') {
                ensure!(
                    open.is_none() && pending.is_none(),
                    "Unclosed chart section"
                );
                let name = s[1..s.len() - 1].to_owned();
                ensure!(names.insert(name.clone()), "Duplicate section {name}");
                pending = Some(name);
            } else if s == "{" {
                let name = pending.take().context("Unexpected opening brace")?;
                open = Some((name, offset + line.len()));
            } else if s == "}" {
                let (name, body_start) = open.take().context("Unexpected closing brace")?;
                sections.push(Section {
                    name,
                    body_start,
                    body_end: offset,
                });
            }
            offset += line.len();
        }
        ensure!(
            open.is_none() && pending.is_none(),
            "Unclosed chart section"
        );
        let body = |name: &str| -> &str {
            sections
                .iter()
                .find(|s| s.name == name)
                .map_or("", |s| &raw[s.body_start..s.body_end])
        };
        ensure!(
            sections.iter().any(|s| s.name == "Song"),
            "Missing [Song] section"
        );
        let mut resolution = 192;
        let mut chart_offset = 0.0;
        for line in body("Song").lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "Resolution" => {
                        resolution = value.trim().parse().context("Invalid Resolution")?
                    }
                    "Offset" => chart_offset = value.trim().parse().context("Invalid Offset")?,
                    _ => {}
                }
            }
        }
        let mut tempos = Vec::new();
        for line in body("SyncTrack").lines() {
            if let Some((tick, event)) = line.split_once('=') {
                let words: Vec<_> = event.split_whitespace().collect();
                if words.first() == Some(&"A") {
                    bail!(
                        "Anchored .chart timing is not supported yet; refusing to misalign new notes"
                    );
                }
                if words.first() == Some(&"B") {
                    ensure!(words.len() == 2, "Malformed tempo event");
                    tempos.push(Tempo {
                        tick: tick.trim().parse()?,
                        bpm: words[1].parse::<f64>()? / 1000.0,
                    });
                }
            }
        }
        let timing = TempoMap::new(resolution, chart_offset, tempos)?;
        Ok(Self {
            raw,
            sections,
            timing,
        })
    }

    pub fn notes(&self, name: &str) -> Result<Vec<Note>> {
        let Some(s) = self.sections.iter().find(|s| s.name == name) else {
            return Ok(Vec::new());
        };
        let mut notes = Vec::new();
        for line in self.raw[s.body_start..s.body_end].lines() {
            if let Some((tick, event)) = line.split_once('=') {
                let words: Vec<_> = event.split_whitespace().collect();
                if words.first() == Some(&"N") {
                    ensure!(words.len() == 3, "Malformed note in {name}");
                    notes.push(Note {
                        tick: tick.trim().parse()?,
                        lane: words[1].parse()?,
                        sustain: words[2].parse()?,
                    });
                }
            }
        }
        Ok(notes)
    }

    /// Inserts only missing/empty difficulty tracks. Existing bytes, including
    /// events and tempo data, are retained exactly, even with CRLF line endings.
    pub fn enhance(&self, generated: &Tracks) -> Result<(String, Vec<String>)> {
        let newline = if self.raw.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut insertions = Vec::new();
        let mut appended = String::new();
        let mut added = Vec::new();
        for (name, notes) in generated {
            if notes.is_empty() || !self.notes(name)?.is_empty() {
                continue;
            }
            let content = note_lines(notes, newline);
            if let Some(s) = self.sections.iter().find(|s| &s.name == name) {
                insertions.push((s.body_end, content));
            } else {
                appended.push_str(&format!(
                    "{newline}[{name}]{newline}{{{newline}{content}}}{newline}"
                ));
            }
            added.push(name.clone());
        }
        let mut output = self.raw.clone();
        insertions.sort_by_key(|i| std::cmp::Reverse(i.0));
        for (index, content) in insertions {
            output.insert_str(index, &content);
        }
        output.push_str(&appended);
        Ok((output, added))
    }
}

pub fn note_lines(notes: &[Note], newline: &str) -> String {
    let mut ordered = notes.to_vec();
    ordered.sort();
    ordered.dedup();
    ordered
        .iter()
        .map(|n| format!("  {} = N {} {}{newline}", n.tick, n.lane, n.sustain))
        .collect()
}

pub fn new_chart(title: &str, artist: &str, timing: &TempoMap, tracks: &Tracks) -> String {
    let mut out = format!(
        "[Song]\n{{\n  Name = \"{}\"\n  Artist = \"{}\"\n  Charter = \"YACE (automatic draft)\"\n  Resolution = {}\n  Offset = {}\n}}\n[SyncTrack]\n{{\n  0 = TS 4\n",
        quoted(title),
        quoted(artist),
        timing.resolution,
        timing.offset
    );
    for t in &timing.tempos {
        out.push_str(&format!(
            "  {} = B {}\n",
            t.tick,
            (t.bpm * 1000.0).round() as u64
        ));
    }
    out.push_str("}\n[Events]\n{\n}\n");
    for (name, notes) in tracks {
        out.push_str(&format!("[{name}]\n{{\n{}}}\n", note_lines(notes, "\n")));
    }
    out
}

pub fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}
