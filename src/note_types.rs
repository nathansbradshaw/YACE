//! Five-fret chart flags belong to an attack, including every note in its chord.
use crate::{
    chart::Note,
    editor::{EditorNote, Project},
    note_groups::Snapshot,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Auto,
    Forced,
    Tap,
}
impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Forced => "forced",
            Self::Tap => "tap",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "forced" => Some(Self::Forced),
            "tap" => Some(Self::Tap),
            _ => None,
        }
    }
    fn lane(self) -> Option<u8> {
        match self {
            Self::Auto => None,
            Self::Forced => Some(5),
            Self::Tap => Some(6),
        }
    }
}
pub fn supported(track: &str) -> bool {
    ["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
        ["Single", "DoubleBass"]
            .iter()
            .any(|part| track == format!("{tier}{part}"))
    })
}
pub fn markers(notes: &[EditorNote]) -> BTreeMap<u64, Kind> {
    let mut result = BTreeMap::new();
    for item in notes {
        if item.note.lane == 6 {
            result.insert(item.note.tick, Kind::Tap);
        } else if item.note.lane == 5 {
            result.entry(item.note.tick).or_insert(Kind::Forced);
        }
    }
    result
}
pub fn apply(project: &mut Project, selection: &Snapshot, kind: Kind) -> Result<()> {
    ensure!(
        supported(&selection.track),
        "Note types are for five-fret guitar and bass"
    );
    selection.validate(project)?;
    let original = &project.content.tracks[&selection.track];
    let ticks: BTreeSet<_> = original
        .iter()
        .filter(|n| selection.ids.contains(&n.id))
        .map(|n| n.note.tick)
        .collect();
    let mut notes = original.clone();
    notes.retain(|n| {
        !ticks.contains(&n.note.tick)
            || !matches!(n.note.lane, 5 | 6)
            || Some(n.note.lane) == kind.lane()
    });
    let mut next = project.next_id;
    let mut ids: BTreeSet<_> = project
        .content
        .tracks
        .values()
        .flatten()
        .map(|n| n.id.clone())
        .collect();
    if let Some(lane) = kind.lane() {
        let existing: BTreeSet<_> = notes
            .iter()
            .filter(|n| n.note.lane == lane)
            .map(|n| n.note.tick)
            .collect();
        for tick in ticks.difference(&existing) {
            let id = loop {
                let candidate = format!("edit-note-{next}");
                next = next.checked_add(1).context("Note ID overflow")?;
                if ids.insert(candidate.clone()) {
                    break candidate;
                }
            };
            notes.push(EditorNote {
                id,
                note: Note {
                    tick: *tick,
                    lane,
                    sustain: 0,
                },
            });
        }
    }
    project
        .content
        .tracks
        .insert(selection.track.clone(), notes);
    project.next_id = next;
    Ok(())
}
