//! Drum activation/fill and roll intervals. S64 does not distinguish BRE.
use crate::{chart::Chart, editor::Project, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const PREFIX: &str = "YACE drum ranges v1\n";
const MAX: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    FillActivation,
    SingleRoll,
    DoubleRoll,
}
impl Kind {
    fn code(self) -> u8 {
        match self {
            Self::FillActivation => 64,
            Self::SingleRoll => 65,
            Self::DoubleRoll => 66,
        }
    }
    fn from_code(code: &str) -> Option<Self> {
        match code {
            "64" => Some(Self::FillActivation),
            "65" => Some(Self::SingleRoll),
            "66" => Some(Self::DoubleRoll),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: u64,
    pub end: u64,
    pub kind: Kind,
}
pub type Tracks = BTreeMap<String, Vec<Range>>;
pub const TRACKS: [&str; 4] = ["EasyDrums", "MediumDrums", "HardDrums", "ExpertDrums"];
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub tracks: BTreeMap<String, Option<Vec<Range>>>,
    pub shared: bool,
}
fn order(range: &Range) -> (u64, Kind, u64) {
    (range.start, range.kind, range.end)
}
fn is_track(track: &str) -> bool {
    track.ends_with("Drums") && crate::parts::editable_track(track)
}
fn managed(line: &str) -> bool {
    line.split_once('=').is_some_and(|(_, body)| {
        let mut fields = body.split_whitespace();
        fields.next() == Some("S") && fields.next().and_then(Kind::from_code).is_some()
    })
}
fn validate_shape(ranges: &[Range]) -> Result<()> {
    ensure!(
        ranges.len() <= MAX,
        "A drum track can have up to 2,048 fill/roll ranges"
    );
    ensure!(
        ranges.iter().all(|r| r.start < r.end),
        "A drum range must end after it starts"
    );
    ensure!(
        ranges.windows(2).all(|r| order(&r[0]) <= order(&r[1])),
        "Drum ranges must be ordered"
    );
    // Different marker kinds are independent; retain source overlaps between them.
    for kind in [Kind::FillActivation, Kind::SingleRoll, Kind::DoubleRoll] {
        let mut end = 0;
        for range in ranges.iter().filter(|r| r.kind == kind) {
            ensure!(
                range.start >= end,
                "Drum ranges of the same kind cannot overlap"
            );
            end = range.end;
        }
    }
    Ok(())
}
pub fn imported(chart: &Chart, track: &str) -> Result<Vec<Range>> {
    ensure!(is_track(track), "Choose an editable drum track");
    let sections: Vec<_> = chart.sections.iter().filter(|s| s.name == track).collect();
    ensure!(sections.len() <= 1, "Duplicate drum sections are ambiguous");
    let Some(section) = sections.first() else {
        return Ok(Vec::new());
    };
    let mut ranges = Vec::new();
    for line in chart.raw[section.body_start..section.body_end]
        .lines()
        .filter(|line| managed(line))
    {
        let (tick, body) = line.split_once('=').unwrap();
        let fields: Vec<_> = body.split_whitespace().collect();
        ensure!(fields.len() == 3, "Malformed drum range; source retained");
        let start: u64 = tick.trim().parse()?;
        let length: u64 = fields[2].parse()?;
        ranges.push(Range {
            start,
            end: start
                .checked_add(length)
                .context("Drum range tick overflow")?,
            kind: Kind::from_code(fields[1]).unwrap(),
        });
    }
    ranges.sort_by_key(order);
    validate_shape(&ranges)?;
    Ok(ranges)
}
pub fn current(project: &Project, source: &Chart, track: &str) -> Result<Vec<Range>> {
    project
        .content
        .drum_ranges
        .get(track)
        .cloned()
        .map_or_else(|| imported(source, track), Ok)
}
fn authored_shared(project: &Project) -> Result<Option<&Vec<Range>>> {
    let mut authored = project
        .content
        .drum_ranges
        .iter()
        .filter(|(track, _)| is_track(track) && project.content.tracks.contains_key(*track))
        .map(|(_, ranges)| ranges);
    let Some(first) = authored.next() else {
        return Ok(None);
    };
    validate_shape(first)?;
    ensure!(
        authored.all(|ranges| ranges == first),
        "Shared MIDI drum ranges disagree; source retained read-only"
    );
    Ok(Some(first))
}
fn shared_source(chart: &Chart) -> Result<Vec<Range>> {
    let mut ranges = Vec::new();
    for track in TRACKS {
        ranges.extend(imported(chart, track)?);
    }
    ranges.sort_by_key(order);
    ranges.dedup();
    validate_shape(&ranges)?;
    Ok(ranges)
}
pub fn shared_current(project: &Project, chart: &Chart) -> Result<Vec<Range>> {
    authored_shared(project)?
        .cloned()
        .map_or_else(|| shared_source(chart), Ok)
}
pub fn view(project: &Project, source: &Source, track: &str) -> Option<Vec<Range>> {
    if source.shared {
        if let Some(ranges) = authored_shared(project).ok()? {
            return Some(ranges.clone());
        }
    } else if let Some(ranges) = project.content.drum_ranges.get(track) {
        return Some(ranges.clone());
    }
    source
        .tracks
        .get(track)
        .cloned()
        .unwrap_or_else(|| Some(Vec::new()))
}
pub fn source(project: &Project, chart: &Chart, shared: bool) -> Source {
    let tracks = if shared {
        let ranges = shared_source(chart).ok();
        TRACKS
            .into_iter()
            .map(|track| (track.to_owned(), ranges.clone()))
            .collect()
    } else {
        project
            .content
            .tracks
            .keys()
            .filter(|track| is_track(track))
            .map(|track| (track.clone(), imported(chart, track).ok()))
            .collect()
    };
    Source { tracks, shared }
}
pub fn validate(ranges: &[Range], timing: &TempoMap, duration: f64) -> Result<()> {
    validate_shape(ranges)?;
    ensure!(
        ranges.iter().all(|r| {
            let end = timing.seconds(r.end);
            end.is_finite() && end <= duration + 0.1
        }),
        "Drum range extends beyond the recording or timeline"
    );
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clipboard {
    pub version: u32,
    pub resolution: u32,
    pub ranges: Vec<Range>,
}
impl Clipboard {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.resolution > 0,
            "Unsupported drum range clipboard"
        );
        ensure!(
            !self.ranges.is_empty() && self.ranges[0].start == 0,
            "Drum clipboard needs ranges relative to tick zero"
        );
        validate_shape(&self.ranges)
    }
}
pub fn encode_clipboard(ranges: &[Range], resolution: u32) -> Result<String> {
    let mut ranges = ranges.to_vec();
    ranges.sort_by_key(order);
    validate_shape(&ranges)?;
    let start = ranges.first().context("Select a drum range to copy")?.start;
    for range in &mut ranges {
        range.start -= start;
        range.end -= start;
    }
    let clipboard = Clipboard {
        version: 1,
        resolution,
        ranges,
    };
    clipboard.validate()?;
    Ok(format!("{PREFIX}{}", serde_json::to_string(&clipboard)?))
}
pub fn decode_clipboard(text: &str) -> Result<Clipboard> {
    ensure!(
        text.len() <= 256 * 1024,
        "Drum range clipboard is too large"
    );
    let payload = text
        .strip_prefix(PREFIX)
        .context("Clipboard does not contain YACE drum ranges")?;
    let clipboard: Clipboard = serde_json::from_str(payload)?;
    clipboard.validate()?;
    Ok(clipboard)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        track: String,
        previous: Option<Range>,
        range: Range,
    },
    Remove {
        track: String,
        range: Range,
    },
    Paste {
        track: String,
        at: u64,
        clipboard: Clipboard,
    },
}
impl Edit {
    pub fn track(&self) -> &str {
        match self {
            Self::Put { track, .. } | Self::Remove { track, .. } | Self::Paste { track, .. } => {
                track
            }
        }
    }
    pub(crate) fn touches_activation(&self) -> bool {
        match self {
            Self::Put {
                previous, range, ..
            } => {
                range.kind == Kind::FillActivation
                    || previous
                        .as_ref()
                        .is_some_and(|r| r.kind == Kind::FillActivation)
            }
            Self::Remove { range, .. } => range.kind == Kind::FillActivation,
            Self::Paste { clipboard, .. } => clipboard
                .ranges
                .iter()
                .any(|r| r.kind == Kind::FillActivation),
        }
    }
}
pub fn apply(project: &mut Project, source: &Chart, edit: &Edit) -> Result<()> {
    let track = edit.track();
    ensure!(
        is_track(track) && project.content.tracks.contains_key(track),
        "Choose an editable drum track"
    );
    imported(source, track)?;
    let mut ranges = current(project, source, track)?;
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { range, .. } => Some(range),
        Edit::Paste { .. } => None,
    };
    if let Some(previous) = previous {
        let index = ranges
            .iter()
            .position(|range| range == previous)
            .context("This drum range changed. Select it again.")?;
        ranges.remove(index);
    }
    let timing = crate::sync_track::effective(project, &source.timing)?;
    match edit {
        Edit::Put { range, .. } => ranges.push(range.clone()),
        Edit::Remove { .. } => (),
        Edit::Paste { at, clipboard, .. } => {
            clipboard.validate()?;
            ensure!(
                clipboard.resolution == timing.resolution,
                "Clipboard resolution differs; paste into a matching chart clock"
            );
            for range in &clipboard.ranges {
                ranges.push(Range {
                    start: at
                        .checked_add(range.start)
                        .context("Drum paste tick overflow")?,
                    end: at
                        .checked_add(range.end)
                        .context("Drum paste tick overflow")?,
                    kind: range.kind,
                });
            }
        }
    }
    ranges.sort_by_key(order);
    validate(&ranges, &timing, project.duration)?;
    project.content.drum_ranges.insert(track.into(), ranges);
    Ok(())
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
    for (track, ranges) in tracks {
        imported(&chart, track)?;
        validate_shape(ranges)?;
        ensure!(
            ranges.is_empty() || chart.sections.iter().any(|s| s.name == *track),
            "Missing drum range track {track}"
        );
    }
    let mut out = chart.raw.clone();
    for section in chart.sections.iter().rev() {
        let Some(ranges) = tracks.get(&section.name) else {
            continue;
        };
        let mut body: String = chart.raw[section.body_start..section.body_end]
            .split_inclusive('\n')
            .filter(|line| !managed(line))
            .collect();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(newline);
        }
        for range in ranges {
            body.push_str(&format!(
                "  {} = S {} {}{newline}",
                range.start,
                range.kind.code(),
                range.end - range.start
            ));
        }
        out.replace_range(section.body_start..section.body_end, &body);
    }
    Ok(out)
}
#[cfg(test)]
#[path = "drum_ranges/tests.rs"]
mod tests;
