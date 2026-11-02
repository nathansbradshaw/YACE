use crate::{
    MAX_ROW,
    timing::{Timing, TimingKind},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Sm,
    Ssc,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Sm => "sm",
            Self::Ssc => "ssc",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Difficulty {
    Beginner,
    Easy,
    Medium,
    Hard,
    Challenge,
    Edit,
}
impl Difficulty {
    pub const GENERATED: [Self; 5] = [
        Self::Beginner,
        Self::Easy,
        Self::Medium,
        Self::Hard,
        Self::Challenge,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
            Self::Challenge => "Challenge",
            Self::Edit => "Edit",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::GENERATED
            .into_iter()
            .chain([Self::Edit])
            .find(|d| d.name().eq_ignore_ascii_case(s))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    Tap,
    Hold,
    Roll,
    Mine,
    Lift,
    Fake,
}
impl NoteKind {
    pub const ALL: [Self; 6] = [
        Self::Tap,
        Self::Hold,
        Self::Roll,
        Self::Mine,
        Self::Lift,
        Self::Fake,
    ];
    pub fn symbol(self) -> char {
        match self {
            Self::Tap => '1',
            Self::Hold => '2',
            Self::Roll => '4',
            Self::Mine => 'M',
            Self::Lift => 'L',
            Self::Fake => 'F',
        }
    }
    pub fn sustained(self) -> bool {
        matches!(self, Self::Hold | Self::Roll)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub row: i64,
    pub lane: u8,
    pub kind: NoteKind,
    pub end_row: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    #[serde(default)]
    pub profile: Option<crate::Profile>,
    pub id: String,
    pub stepstype: String,
    pub difficulty: String,
    pub meter: u32,
    pub description: String,
    pub notes: Vec<Note>,
    pub timing: Option<Timing>,
    pub metadata: BTreeMap<String, String>,
    pub read_only_reason: Option<String>,
    pub original_notes: String,
}
impl Chart {
    pub fn validate(&self) -> Result<()> {
        if self.read_only_reason.is_some() {
            return Ok(());
        }
        ensure!(
            self.stepstype == "dance-single",
            "Only dance-single is editable"
        );
        ensure!(
            Difficulty::parse(&self.difficulty).is_some(),
            "Unknown difficulty"
        );
        ensure!((1..=999).contains(&self.meter), "Meter must be 1–999");
        ensure!(self.notes.len() <= 2_000_000, "Too many notes");
        let mut ids = BTreeSet::new();
        let mut occupied = BTreeSet::new();
        let mut ends = [-1; 4];
        let mut sorted: Vec<_> = self.notes.iter().collect();
        sorted.sort_by_key(|n| (n.row, n.lane));
        for n in sorted {
            ensure!(
                n.lane < 4 && (0..=MAX_ROW).contains(&n.row),
                "Note outside chart"
            );
            ensure!(
                !n.id.is_empty() && ids.insert(&n.id),
                "Duplicate note identity"
            );
            ensure!(
                occupied.insert((n.row, n.lane)),
                "Two notes occupy the same lane and row"
            );
            ensure!(
                n.row > ends[n.lane as usize],
                "A note overlaps a hold or its tail"
            );
            ensure!(
                n.kind.sustained() == n.end_row.is_some(),
                "Only holds and rolls need tails"
            );
            if let Some(end) = n.end_row {
                ensure!(
                    end > n.row && end <= MAX_ROW,
                    "Hold tail must follow its head"
                );
                ends[n.lane as usize] = end;
            }
        }
        if let Some(timing) = &self.timing {
            timing.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: Format,
    pub raw: String,
    pub metadata: BTreeMap<String, String>,
    pub timing: Timing,
    pub charts: Vec<Chart>,
}
#[derive(Clone, Debug)]
struct Tag {
    key: String,
    value: String,
    start: usize,
    end: usize,
}
fn tags(raw: &str) -> Result<Vec<Tag>> {
    ensure!(raw.len() <= 32 * 1024 * 1024, "Simfile exceeds 32 MiB");
    let bytes = raw.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] != b'#' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let key_start = i;
        while i < bytes.len() && bytes[i] != b':' {
            ensure!(bytes[i] != b';', "Malformed simfile tag");
            i += 1;
        }
        ensure!(i < bytes.len(), "Unterminated simfile tag");
        let key = raw[key_start..i].trim().to_ascii_uppercase();
        i += 1;
        let mut value = String::new();
        let mut begin = i;
        loop {
            ensure!(i < bytes.len(), "Unterminated #{key} tag");
            if bytes[i] == b'\\' {
                i += 2;
                ensure!(i <= bytes.len(), "Unterminated escape");
                continue;
            }
            if bytes[i..].starts_with(b"//") {
                value.push_str(&raw[begin..i]);
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                begin = i;
                continue;
            }
            if bytes[i] == b';' {
                value.push_str(&raw[begin..i]);
                i += 1;
                break;
            }
            i += 1;
        }
        out.push(Tag {
            key,
            value,
            start,
            end: i,
        });
        ensure!(out.len() < 100_000, "Too many simfile tags");
    }
    Ok(out)
}
pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(c) = chars.next() {
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }
    out
}
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace(';', "\\;")
        .replace("//", "\\/\\/")
}
fn map(tags: &[Tag]) -> BTreeMap<String, String> {
    tags.iter()
        .map(|t| (t.key.clone(), unescape(t.value.trim())))
        .collect()
}
fn split_fields(raw: &str, count: usize) -> Vec<String> {
    let mut fields = Vec::new();
    let mut begin = 0;
    let mut escaped = false;
    for (i, c) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if c == ':' && fields.len() < count - 1 {
            fields.push(unescape(raw[begin..i].trim()));
            begin = i + 1;
        }
    }
    fields.push(unescape(raw[begin..].trim()));
    fields
}
fn notes(raw: &str, id: &str) -> Result<Vec<Note>> {
    let mut out: Vec<Note> = Vec::new();
    let mut holds: [Option<usize>; 4] = [None; 4];
    let mut seen = BTreeSet::new();
    for (measure, body) in raw.split(',').enumerate() {
        let lines: Vec<_> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        ensure!(lines.len() <= 192, "Measure uses unsupported subdivisions");
        for (i, line) in lines.iter().enumerate() {
            ensure!(
                line.len() == 4 && line.is_ascii(),
                "Note row must have four plain columns; keysounds and attack notes are preserved read-only"
            );
            let row = measure as i64 * 192 + ((i * 192) as f64 / lines.len() as f64).round() as i64;
            ensure!(row <= MAX_ROW, "Chart exceeds supported length");
            for (lane, symbol) in line.chars().enumerate() {
                if symbol == '0' {
                    continue;
                }
                ensure!(
                    seen.insert((row, lane)),
                    "Subdivision produces colliding notes"
                );
                if symbol == '3' {
                    let index = holds[lane].take().context("Hold tail without a head")?;
                    out[index].end_row = Some(row);
                    continue;
                }
                ensure!(holds[lane].is_none(), "Note inside an unclosed hold");
                let kind = match symbol {
                    '1' => NoteKind::Tap,
                    '2' => NoteKind::Hold,
                    '4' => NoteKind::Roll,
                    'M' => NoteKind::Mine,
                    'L' => NoteKind::Lift,
                    'F' => NoteKind::Fake,
                    _ => bail!("Unsupported note symbol {symbol}"),
                };
                let index = out.len();
                if kind.sustained() {
                    holds[lane] = Some(index);
                }
                out.push(Note {
                    id: format!("{id}-n{}", index + 1),
                    row,
                    lane: lane as u8,
                    kind,
                    end_row: None,
                });
            }
        }
    }
    ensure!(
        holds.iter().all(Option::is_none),
        "Hold head without a tail"
    );
    Ok(out)
}
fn render_notes(chart: &Chart) -> Result<String> {
    chart.validate()?;
    let max = chart
        .notes
        .iter()
        .map(|n| n.end_row.unwrap_or(n.row))
        .max()
        .unwrap_or(0);
    let mut grid: BTreeMap<i64, [char; 4]> = BTreeMap::new();
    for n in &chart.notes {
        grid.entry(n.row).or_insert(['0'; 4])[n.lane as usize] = n.kind.symbol();
        if let Some(end) = n.end_row {
            grid.entry(end).or_insert(['0'; 4])[n.lane as usize] = '3';
        }
    }
    let mut measures = Vec::new();
    for measure in 0..=max / 192 {
        let start = measure * 192;
        let rows: Vec<_> = grid.range(start..start + 192).collect();
        let step = rows
            .iter()
            .fold(48, |step, (row, _)| gcd(step, **row - start));
        let step = if step == 0 { 48 } else { step };
        let mut text = String::new();
        for row in (start..start + 192).step_by(step as usize) {
            text.extend(grid.get(&row).unwrap_or(&['0'; 4]));
            text.push('\n');
        }
        measures.push(text);
    }
    Ok(measures.join(",\n"))
}
fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Document {
    pub fn parse(raw: String, format: Format) -> Result<Self> {
        let tokens = tags(&raw)?;
        ensure!(!tokens.is_empty(), "No simfile tags");
        let first = tokens
            .iter()
            .position(|t| {
                t.key
                    == if format == Format::Sm {
                        "NOTES"
                    } else {
                        "NOTEDATA"
                    }
            })
            .unwrap_or(tokens.len());
        let mut metadata = map(&tokens[..first]);
        let timing = Timing::from_tags(&metadata, None)?;
        metadata.retain(|k, _| k != "OFFSET" && TimingKind::parse(k).is_none());
        let mut charts = Vec::new();
        let groups: Vec<Vec<Tag>> = if format == Format::Sm {
            tokens[first..]
                .iter()
                .filter(|t| t.key == "NOTES")
                .map(|t| vec![t.clone()])
                .collect()
        } else {
            let mut groups: Vec<Vec<Tag>> = Vec::new();
            for token in &tokens[first..] {
                if token.key == "NOTEDATA" {
                    groups.push(Vec::new());
                }
                if let Some(group) = groups.last_mut() {
                    group.push(token.clone());
                }
            }
            groups
        };
        for (index, group) in groups.iter().enumerate() {
            let id = format!("chart-{}", index + 1);
            let (fields, note_text, override_timing) = if format == Format::Sm {
                let fields = split_fields(&group[0].value, 6);
                ensure!(fields.len() == 6, "#NOTES needs six fields");
                (
                    BTreeMap::from([
                        ("STEPSTYPE".into(), fields[0].clone()),
                        ("DESCRIPTION".into(), fields[1].clone()),
                        ("DIFFICULTY".into(), fields[2].clone()),
                        ("METER".into(), fields[3].clone()),
                        ("RADARVALUES".into(), fields[4].clone()),
                    ]),
                    fields[5].clone(),
                    None,
                )
            } else {
                let fields = map(group);
                let note_text = fields.get("NOTES").context("Chart has no #NOTES")?.clone();
                let local = fields
                    .keys()
                    .any(|k| k == "OFFSET" || TimingKind::parse(k).is_some());
                let local = if local {
                    Some(Timing::from_tags(&fields, Some(&timing))?)
                } else {
                    None
                };
                (fields, note_text, local)
            };
            let stepstype = fields.get("STEPSTYPE").cloned().unwrap_or_default();
            let difficulty = fields
                .get("DIFFICULTY")
                .cloned()
                .unwrap_or_else(|| "Edit".into());
            let mut chart = Chart {
                profile: None,
                id: id.clone(),
                stepstype: stepstype.clone(),
                difficulty,
                meter: fields
                    .get("METER")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                description: fields.get("DESCRIPTION").cloned().unwrap_or_default(),
                notes: Vec::new(),
                timing: override_timing,
                metadata: fields,
                read_only_reason: None,
                original_notes: note_text.clone(),
            };
            if stepstype != "dance-single" {
                chart.read_only_reason = Some(format!(
                    "{stepstype} is preserved; only dance-single is editable"
                ));
            } else {
                match notes(&note_text, &id) {
                    Ok(notes) => chart.notes = notes,
                    Err(error) => {
                        let message = error.to_string();
                        if message.starts_with("Unsupported note")
                            || message.contains("plain columns")
                            || message.contains("unsupported subdivisions")
                        {
                            chart.read_only_reason = Some(message);
                        } else {
                            return Err(error.context(format!("Chart {}", index + 1)));
                        }
                    }
                }
            }
            if Difficulty::parse(&chart.difficulty).is_none() {
                chart.read_only_reason = Some("Unknown difficulty is preserved".into());
            }
            chart.validate()?;
            charts.push(chart);
        }
        Ok(Self {
            format,
            raw,
            metadata,
            timing,
            charts,
        })
    }
    pub fn validate(&self) -> Result<()> {
        self.timing.validate()?;
        let mut ids = BTreeSet::new();
        for chart in &self.charts {
            ensure!(ids.insert(&chart.id), "Duplicate chart identity");
            chart.validate()?;
        }
        Ok(())
    }
    pub fn effective_timing(&self, id: &str) -> Result<&Timing> {
        Ok(self
            .charts
            .iter()
            .find(|c| c.id == id)
            .context("Chart is missing")?
            .timing
            .as_ref()
            .unwrap_or(&self.timing))
    }
    pub fn export(&self, format: Format) -> Result<String> {
        self.validate()?;
        if format == Format::Sm {
            ensure!(
                self.charts.iter().all(|c| c.timing.is_none()),
                "SM cannot represent chart-specific timing; export SSC"
            );
            ensure!(
                self.timing
                    .events
                    .iter()
                    .all(|e| matches!(e.kind, TimingKind::Bpms | TimingKind::Stops)),
                "SM export would lose advanced timing; export SSC"
            );
            if self.format == Format::Ssc {
                ensure!(
                    self.charts
                        .iter()
                        .all(|c| c.metadata.keys().all(|k| matches!(
                            k.as_str(),
                            "NOTEDATA"
                                | "STEPSTYPE"
                                | "DESCRIPTION"
                                | "DIFFICULTY"
                                | "METER"
                                | "RADARVALUES"
                                | "NOTES"
                        ))),
                    "SM export would lose chart metadata; export SSC"
                );
            }
        }
        if self.raw.is_empty() || format != self.format {
            let mut out = String::new();
            let mut song = self.metadata.clone();
            song.remove("VERSION");
            if format == Format::Ssc {
                song.insert("VERSION".into(), "0.83".into());
            }
            song.extend(self.timing.tags());
            for (k, v) in song {
                out.push_str(&format!("#{k}:{};\n", escape(&v)));
            }
            for chart in &self.charts {
                out.push_str(&chart_text(chart, format)?);
            }
            return Ok(out);
        }
        let baseline = Self::parse(self.raw.clone(), self.format)?;
        let tokens = tags(&self.raw)?;
        let first = tokens
            .iter()
            .position(|t| {
                t.key
                    == if format == Format::Sm {
                        "NOTES"
                    } else {
                        "NOTEDATA"
                    }
            })
            .unwrap_or(tokens.len());
        let mut edits = Vec::new();
        let mut desired = self.metadata.clone();
        desired.extend(self.timing.tags());
        let mut old = baseline.metadata.clone();
        old.extend(baseline.timing.tags());
        patch_scope(
            &tokens[..first],
            &old,
            &desired,
            if first < tokens.len() {
                tokens[first].start
            } else {
                self.raw.len()
            },
            &mut edits,
        );
        let starts: Vec<_> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.key
                    == if format == Format::Sm {
                        "NOTES"
                    } else {
                        "NOTEDATA"
                    }
            })
            .map(|(i, _)| i)
            .collect();
        for (index, old_chart) in baseline.charts.iter().enumerate() {
            let start = starts[index];
            let end = starts.get(index + 1).copied().unwrap_or(tokens.len());
            let Some(chart) = self.charts.iter().find(|c| c.id == old_chart.id) else {
                edits.push((
                    tokens[start].start,
                    if end < tokens.len() {
                        tokens[end].start
                    } else {
                        self.raw.len()
                    },
                    String::new(),
                ));
                continue;
            };
            if chart == old_chart {
                continue;
            }
            ensure!(
                old_chart.read_only_reason.is_none(),
                "Unsupported chart cannot be edited"
            );
            if format == Format::Sm {
                edits.push((
                    tokens[start].start,
                    tokens[start].end,
                    chart_text(chart, format)?,
                ));
            } else {
                let old = chart_tags(old_chart)?;
                let desired = chart_tags(chart)?;
                patch_scope(
                    &tokens[start..end],
                    &old,
                    &desired,
                    if end < tokens.len() {
                        tokens[end].start
                    } else {
                        self.raw.len()
                    },
                    &mut edits,
                );
            }
        }
        for chart in &self.charts {
            if !baseline.charts.iter().any(|c| c.id == chart.id) {
                edits.push((self.raw.len(), self.raw.len(), chart_text(chart, format)?));
            }
        }
        edits.sort_by_key(|(start, end, _)| (*start, *end));
        let mut result = self.raw.clone();
        for (start, end, value) in edits.into_iter().rev() {
            result.replace_range(start..end, &value);
        }
        Ok(result)
    }
}
fn chart_tags(chart: &Chart) -> Result<BTreeMap<String, String>> {
    let mut tags = chart.metadata.clone();
    tags.insert("STEPSTYPE".into(), chart.stepstype.clone());
    tags.insert("DESCRIPTION".into(), chart.description.clone());
    tags.insert("DIFFICULTY".into(), chart.difficulty.clone());
    tags.insert("METER".into(), chart.meter.to_string());
    if let Some(timing) = &chart.timing {
        for kind in TimingKind::ALL {
            tags.remove(kind.tag());
        }
        tags.remove("OFFSET");
        tags.extend(timing.tags());
    } else {
        for kind in TimingKind::ALL {
            tags.remove(kind.tag());
        }
        tags.remove("OFFSET");
    }
    tags.insert(
        "NOTES".into(),
        if chart.read_only_reason.is_some() {
            chart.original_notes.clone()
        } else {
            render_notes(chart)?
        },
    );
    Ok(tags)
}
fn chart_text(chart: &Chart, format: Format) -> Result<String> {
    let tags = chart_tags(chart)?;
    if format == Format::Sm {
        return Ok(format!(
            "\n#NOTES:\n{}:\n{}:\n{}:\n{}:\n{}:\n{};\n",
            escape(&chart.stepstype),
            escape(&chart.description),
            escape(&chart.difficulty),
            chart.meter,
            tags.get("RADARVALUES").map_or("0,0,0,0,0", String::as_str),
            tags["NOTES"]
        ));
    }
    let mut out = "\n#NOTEDATA:;\n".to_owned();
    for (k, v) in tags
        .iter()
        .filter(|(k, _)| k.as_str() != "NOTEDATA" && k.as_str() != "NOTES")
    {
        out.push_str(&format!("#{k}:{};\n", escape(v)));
    }
    out.push_str(&format!("#NOTES:\n{};\n", tags["NOTES"]));
    Ok(out)
}
fn patch_scope(
    tokens: &[Tag],
    old: &BTreeMap<String, String>,
    desired: &BTreeMap<String, String>,
    end: usize,
    edits: &mut Vec<(usize, usize, String)>,
) {
    for key in old.keys().chain(desired.keys()).collect::<BTreeSet<_>>() {
        if old.get(key) == desired.get(key) {
            continue;
        }
        let matching: Vec<_> = tokens.iter().filter(|t| &t.key == key).collect();
        let text = desired
            .get(key)
            .map(|v| {
                format!(
                    "#{key}:{};",
                    if key == "NOTES" { v.clone() } else { escape(v) }
                )
            })
            .unwrap_or_default();
        if let Some(last) = matching.last() {
            for t in &matching[..matching.len() - 1] {
                edits.push((t.start, t.end, String::new()));
            }
            edits.push((last.start, last.end, text));
        } else if !text.is_empty() {
            edits.push((end, end, format!("\n{text}\n")));
        }
    }
}
