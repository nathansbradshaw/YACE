//! Instrument tremolo/trill ranges. Physical MIDI scope is handled separately.
use crate::{chart::Chart, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_RANGES: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Tremolo,
    Trill,
}
impl Kind {
    pub fn code(self) -> u8 {
        match self {
            Self::Tremolo => 65,
            Self::Trill => 66,
        }
    }
    fn from_code(code: &str) -> Option<Self> {
        match code.parse::<u8>().ok()? {
            65 => Some(Self::Tremolo),
            66 => Some(Self::Trill),
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

pub fn current(
    project: &crate::editor::Project,
    source: &Chart,
    track: &str,
) -> Result<Vec<Range>> {
    project
        .content
        .lane_ranges
        .get(track)
        .cloned()
        .map_or_else(|| imported(source, track), Ok)
}

pub fn apply(
    project: &mut crate::editor::Project,
    source: &Chart,
    track: &str,
    edit: &Edit,
) -> Result<()> {
    ensure!(
        project.content.tracks.contains_key(track),
        "Choose an existing instrument track"
    );
    imported(source, track)?;
    let next = edited(track, &current(project, source, track)?, edit)?;
    validate(
        track,
        &next,
        &crate::sync_track::effective(project, &source.timing)?,
        project.duration,
    )?;
    project.content.lane_ranges.insert(track.to_owned(), next);
    Ok(())
}

pub fn supports(track: &str, kind: Kind) -> bool {
    let Some(suffix) = ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
    else {
        return false;
    };
    match suffix {
        "ProKeys" => kind == Kind::Trill,
        "Single" | "DoubleBass" | "DoubleRhythm" | "DoubleGuitar" | "Keyboard" | "ProGuitar17"
        | "ProGuitar22" | "ProBass17" | "ProBass22" => true,
        _ => false,
    }
}
fn order(range: &Range) -> (u64, Kind, u64) {
    (range.start, range.kind, range.end)
}
fn managed(track: &str, line: &str) -> bool {
    line.split_once('=').is_some_and(|(_, body)| {
        let mut fields = body.split_whitespace();
        fields.next() == Some("S")
            && fields
                .next()
                .and_then(Kind::from_code)
                .is_some_and(|kind| supports(track, kind))
    })
}
pub fn validate_shape(track: &str, ranges: &[Range]) -> Result<()> {
    ensure!(
        ranges.len() <= MAX_RANGES,
        "A track can have up to 2,048 lane ranges"
    );
    ensure!(
        supports(track, Kind::Trill),
        "This instrument does not support lane ranges"
    );
    ensure!(
        ranges
            .iter()
            .all(|r| supports(track, r.kind) && r.start < r.end),
        "Unsupported or empty lane range"
    );
    ensure!(
        ranges
            .windows(2)
            .all(|pair| order(&pair[0]) <= order(&pair[1])),
        "Lane ranges must be ordered"
    );
    for kind in [Kind::Tremolo, Kind::Trill] {
        let mut last_end = 0;
        for range in ranges.iter().filter(|r| r.kind == kind) {
            ensure!(
                range.start >= last_end,
                "Lane ranges of the same kind cannot overlap"
            );
            last_end = range.end;
        }
    }
    Ok(())
}
pub fn validate(track: &str, ranges: &[Range], timing: &TempoMap, duration: f64) -> Result<()> {
    validate_shape(track, ranges)?;
    ensure!(
        ranges.iter().all(|r| {
            let end = timing.seconds(r.end);
            end.is_finite() && end <= duration + 0.1
        }),
        "Lane range extends beyond the recording or timeline"
    );
    Ok(())
}
pub fn imported(chart: &Chart, track: &str) -> Result<Vec<Range>> {
    ensure!(
        supports(track, Kind::Trill),
        "Choose an applicable instrument track"
    );
    let sections: Vec<_> = chart.sections.iter().filter(|s| s.name == track).collect();
    ensure!(
        sections.len() <= 1,
        "Duplicate lane-range sections are ambiguous"
    );
    let Some(section) = sections.first() else {
        return Ok(Vec::new());
    };
    let mut ranges = Vec::new();
    for line in chart.raw[section.body_start..section.body_end]
        .lines()
        .filter(|line| managed(track, line))
    {
        let (tick, body) = line.split_once('=').unwrap();
        let fields: Vec<_> = body.split_whitespace().collect();
        ensure!(fields.len() == 3, "Malformed lane range; source retained");
        let start: u64 = tick.trim().parse()?;
        let length: u64 = fields[2].parse()?;
        ranges.push(Range {
            start,
            end: start
                .checked_add(length)
                .context("Lane range tick overflow")?,
            kind: Kind::from_code(fields[1]).unwrap(),
        });
    }
    ranges.sort_by_key(order);
    validate_shape(track, &ranges)?;
    Ok(ranges)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Range>,
        range: Range,
    },
    Remove {
        range: Range,
    },
}
/// Build an atomic replacement, retaining the caller's draft/source on refusal.
pub fn edited(track: &str, current: &[Range], edit: &Edit) -> Result<Vec<Range>> {
    validate_shape(track, current)?;
    let mut next = current.to_vec();
    let old = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { range } => Some(range),
    };
    if let Some(old) = old {
        let index = next
            .iter()
            .position(|r| r == old)
            .context("The lane range changed. Select it again.")?;
        next.remove(index);
    }
    if let Edit::Put { range, .. } = edit {
        next.push(range.clone());
    }
    next.sort_by_key(order);
    validate_shape(track, &next)?;
    Ok(next)
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
        validate_shape(track, ranges)?;
        ensure!(
            ranges.is_empty() || chart.sections.iter().any(|s| s.name == *track),
            "Missing lane-range track {track}"
        );
    }
    let mut out = chart.raw.clone();
    for section in chart.sections.iter().rev() {
        let Some(ranges) = tracks.get(&section.name) else {
            continue;
        };
        let mut body: String = chart.raw[section.body_start..section.body_end]
            .split_inclusive('\n')
            .filter(|line| !managed(&section.name, line))
            .collect();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(newline)
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
mod tests {
    use super::*;
    use crate::editor::{Action, Actor, ActorKind, Request, Store};
    fn command(store: &Store, action: Action) -> Result<()> {
        store.apply(Request {
            expected_revision: store.load()?.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Lane range regression".into(),
            },
            action,
        })?;
        Ok(())
    }
    fn project(midi: bool) -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let song = temp.path().join("song");
        std::fs::create_dir(&song).unwrap();
        let text = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n0 = N 0 120\n240 = S 65 480\n}\n";
        if midi {
            std::fs::write(
                song.join("notes.mid"),
                crate::game_export::song(
                    &Chart::parse(text.replace("240 = S 65 480\n", "")).unwrap(),
                    None,
                )
                .unwrap()
                .0,
            )
            .unwrap();
        } else {
            std::fs::write(song.join("notes.chart"), text).unwrap();
        }
        std::fs::write(
            song.join("song.ini"),
            "[song]\nname = Lane range regression\nsong_length = 10000\n",
        )
        .unwrap();
        let store = Store::create(&song, None, &temp.path().join("project")).unwrap();
        (temp, store)
    }
    #[test]
    fn lane_range_project_commands_keep_history_witnesses_and_explicit_clear() {
        let (temp, store) = project(false);
        let old = range(240, 720, Kind::Tremolo);
        let next = range(480, 960, Kind::Tremolo);
        command(
            &store,
            Action::EditLaneRange {
                track: "ExpertSingle".into(),
                edit: Edit::Put {
                    previous: Some(old.clone()),
                    range: next.clone(),
                },
            },
        )
        .unwrap();
        let before = std::fs::read(store.root.join("project.json")).unwrap();
        assert!(
            command(
                &store,
                Action::EditLaneRange {
                    track: "ExpertSingle".into(),
                    edit: Edit::Remove { range: old }
                }
            )
            .is_err()
        );
        assert_eq!(
            before,
            std::fs::read(store.root.join("project.json")).unwrap()
        );
        command(&store, Action::Undo).unwrap();
        assert!(store.load().unwrap().content.lane_ranges.is_empty());
        command(&store, Action::Redo).unwrap();
        assert_eq!(
            store.load().unwrap().content.lane_ranges["ExpertSingle"],
            vec![next.clone()]
        );
        command(
            &store,
            Action::EditLaneRange {
                track: "ExpertSingle".into(),
                edit: Edit::Remove { range: next },
            },
        )
        .unwrap();
        let output = temp.path().join("export");
        command(
            &store,
            Action::Export {
                output: output.clone(),
            },
        )
        .unwrap();
        let reopened = Store::create(&output, None, &temp.path().join("reopened")).unwrap();
        let doc = reopened.load().unwrap();
        let chart = Chart::parse(doc.chart_text(&reopened).unwrap()).unwrap();
        assert!(current(&doc, &chart, "ExpertSingle").unwrap().is_empty());
    }
    #[test]
    fn lane_range_midi_commands_refuse_until_scope_writer_is_available() {
        let (_temp, store) = project(true);
        let before = std::fs::read(store.root.join("project.json")).unwrap();
        assert!(
            command(
                &store,
                Action::EditLaneRange {
                    track: "ExpertSingle".into(),
                    edit: Edit::Put {
                        previous: None,
                        range: range(960, 1440, Kind::Trill)
                    }
                }
            )
            .unwrap_err()
            .to_string()
            .contains("MIDI lane-range authoring")
        );
        assert_eq!(
            before,
            std::fs::read(store.root.join("project.json")).unwrap()
        );
    }
    #[test]
    fn lane_range_note_proposals_bind_ranges_without_dropping_them() {
        let (_temp, store) = project(false);
        let base = store.load().unwrap();
        let mut edited = base.clone();
        edited.content.tracks.get_mut("ExpertSingle").unwrap()[0]
            .note
            .tick += 120;
        let author = Actor {
            kind: ActorKind::Human,
            name: "Lane proposal regression".into(),
        };
        let legacy = crate::collaboration::Proposal::between(
            &base,
            &edited,
            "legacy".into(),
            "Notes".into(),
            "Independent notes".into(),
            author.clone(),
        )
        .unwrap();
        edited
            .content
            .lane_ranges
            .insert("ExpertSingle".into(), vec![range(480, 960, Kind::Trill)]);
        assert!(
            crate::collaboration::Proposal::between(
                &base,
                &edited,
                "refuse".into(),
                "Notes".into(),
                "Independent notes".into(),
                author.clone()
            )
            .is_err()
        );
        let mut same = base;
        same.content.lane_ranges = edited.content.lane_ranges.clone();
        let bound = crate::collaboration::Proposal::between(
            &same,
            &edited,
            "bound".into(),
            "Notes".into(),
            "Independent notes".into(),
            author,
        )
        .unwrap();
        assert!(legacy.star_power_sha256.is_none());
        assert!(bound.star_power_sha256.is_some());
        assert_ne!(legacy.fingerprint().unwrap(), bound.fingerprint().unwrap());
    }
    fn range(start: u64, end: u64, kind: Kind) -> Range {
        Range { start, end, kind }
    }
    fn source() -> Chart {
        Chart::parse("[Song]\r\n{\r\n Resolution = 480\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n}\r\n[ExpertSingle]\r\n{\r\n 0 = N 0 120\r\n 240 = S 65 480\r\n 360 = S 66 480\r\n 400 = E opaque\r\n}\r\n[HardSingle]\r\n{\r\n 120 = S 66 240\r\n}\r\n[ExpertProKeys]\r\n{\r\n 100 = S 65 100\r\n 240 = S 66 480\r\n}\r\n".into()).unwrap()
    }
    #[test]
    fn lane_ranges_keep_independent_kinds_tiers_and_opaque_rows() {
        let src = source();
        let original = imported(&src, "ExpertSingle").unwrap();
        assert_eq!(original.len(), 2);
        let next = edited(
            "ExpertSingle",
            &original,
            &Edit::Put {
                previous: Some(original[0].clone()),
                range: range(480, 960, Kind::Tremolo),
            },
        )
        .unwrap();
        let text = render(
            src.raw.clone(),
            &Tracks::from([("ExpertSingle".into(), next.clone())]),
        )
        .unwrap();
        let parsed = Chart::parse(text.clone()).unwrap();
        assert_eq!(imported(&parsed, "ExpertSingle").unwrap(), next);
        assert_eq!(
            imported(&parsed, "HardSingle").unwrap(),
            imported(&src, "HardSingle").unwrap()
        );
        assert!(text.contains(" 0 = N 0 120\r\n"));
        assert!(text.contains(" 400 = E opaque\r\n"));
        assert!(text.contains("[HardSingle]\r\n{\r\n 120 = S 66 240\r\n}"));
        assert!(!text.replace("\r\n", "").contains('\n'));
    }
    #[test]
    fn lane_ranges_reject_stale_overlap_and_unsupported_edits_atomically() {
        let ranges = imported(&source(), "ExpertSingle").unwrap();
        let before = ranges.clone();
        assert!(
            edited(
                "ExpertSingle",
                &ranges,
                &Edit::Put {
                    previous: Some(range(241, 720, Kind::Tremolo)),
                    range: range(900, 1000, Kind::Tremolo)
                }
            )
            .is_err()
        );
        assert!(
            edited(
                "ExpertSingle",
                &ranges,
                &Edit::Put {
                    previous: None,
                    range: range(300, 500, Kind::Tremolo)
                }
            )
            .is_err()
        );
        assert_eq!(ranges, before);
        for track in ["ExpertDrums", "ExpertGHLGuitar", "Vocals", "UnknownSingle"] {
            assert!(!supports(track, Kind::Trill));
            assert!(
                edited(
                    track,
                    &[],
                    &Edit::Put {
                        previous: None,
                        range: range(0, 10, Kind::Trill)
                    }
                )
                .is_err()
            );
        }
        assert!(
            edited(
                "ExpertProKeys",
                &[],
                &Edit::Put {
                    previous: None,
                    range: range(0, 10, Kind::Tremolo)
                }
            )
            .is_err()
        );
    }
    #[test]
    fn lane_ranges_pro_keys_trill_clear_preserves_unsupported_marker() {
        let src = source();
        assert_eq!(
            imported(&src, "ExpertProKeys").unwrap(),
            vec![range(240, 720, Kind::Trill)]
        );
        let text = render(
            src.raw.clone(),
            &Tracks::from([("ExpertProKeys".into(), vec![])]),
        )
        .unwrap();
        let parsed = Chart::parse(text.clone()).unwrap();
        assert!(imported(&parsed, "ExpertProKeys").unwrap().is_empty());
        assert!(text.contains(" 100 = S 65 100\r\n"));
        assert_eq!(
            imported(&src, "ExpertSingle").unwrap(),
            imported(&parsed, "ExpertSingle").unwrap()
        );
    }
    #[test]
    fn lane_ranges_refuse_malformed_original_bounds_and_duplicate_sections() {
        let src = source();
        for bad in [
            "240 = S 65 0",
            "240 = S 65 480 extra",
            "18446744073709551615 = S 65 1",
        ] {
            let text = src.raw.replace("240 = S 65 480", bad);
            let c = Chart::parse(text.clone()).unwrap();
            assert!(imported(&c, "ExpertSingle").is_err());
            assert!(render(text, &Tracks::from([("ExpertSingle".into(), vec![])])).is_err());
        }
        assert!(Chart::parse(format!("{}[ExpertSingle]\r\n{{\r\n}}\r\n", src.raw)).is_err());
    }
}
