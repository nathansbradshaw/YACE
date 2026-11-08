//! Per-pad dynamics use the existing portable chart markers, never cymbal flags.
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
    Normal,
    Accent,
    Ghost,
}
impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Accent => "accent",
            Self::Ghost => "ghost",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "accent" => Some(Self::Accent),
            "ghost" => Some(Self::Ghost),
            _ => None,
        }
    }
    fn offset(self) -> Option<u8> {
        match self {
            Self::Normal => None,
            Self::Accent => Some(33),
            Self::Ghost => Some(39),
        }
    }
}
pub fn supported(track: &str) -> bool {
    ["Easy", "Medium", "Hard", "Expert"]
        .iter()
        .any(|tier| track == format!("{tier}Drums"))
}
pub fn pad(lane: u8) -> bool {
    (1..=4).contains(&lane)
}
fn marker(lane: u8) -> Option<(u8, u8)> {
    match lane {
        34..=38 => Some((lane - 33, 1)),
        40..=44 => Some((lane - 39, 2)),
        _ => None,
    }
}
/// Missing entries are normal. None retains an imported accent+ghost conflict
/// for display until the user explicitly chooses a new dynamic.
pub fn markers(notes: &[EditorNote]) -> BTreeMap<(u64, u8), Option<Kind>> {
    let mut flags = BTreeMap::<_, u8>::new();
    for item in notes {
        if let Some((lane, flag)) = marker(item.note.lane) {
            *flags.entry((item.note.tick, lane)).or_default() |= flag;
        }
    }
    flags
        .into_iter()
        .map(|(key, flags)| {
            (
                key,
                match flags {
                    1 => Some(Kind::Accent),
                    2 => Some(Kind::Ghost),
                    _ => None,
                },
            )
        })
        .collect()
}
pub fn apply(project: &mut Project, selection: &Snapshot, kind: Kind) -> Result<()> {
    ensure!(supported(&selection.track), "Dynamics require a drum track");
    selection.validate(project)?;
    let selected: BTreeSet<_> = project.content.tracks[&selection.track]
        .iter()
        .filter(|n| selection.ids.contains(&n.id) && project.drum_layout().pad(n.note.lane))
        .map(|n| (n.note.tick, n.note.lane))
        .collect();
    ensure!(
        !selected.is_empty(),
        "Select at least one drum pad; kick notes have no accent or ghost dynamic"
    );
    let mut notes = project.content.tracks[&selection.track].clone();
    let mut retained = BTreeSet::new();
    notes.retain(|n| {
        let Some((lane, _)) = marker(n.note.lane) else {
            return true;
        };
        let key = (n.note.tick, lane);
        !selected.contains(&key)
            || (kind
                .offset()
                .is_some_and(|offset| n.note.lane == lane + offset)
                && retained.insert(key))
    });
    let mut next = project.next_id;
    let mut ids: BTreeSet<_> = project
        .content
        .tracks
        .values()
        .flatten()
        .map(|n| n.id.clone())
        .collect();
    if let Some(offset) = kind.offset() {
        for (tick, lane) in selected.difference(&retained) {
            let id = loop {
                let candidate = format!("edit-note-{next}");
                next = next.checked_add(1).context("Note ID overflow")?;
                if ids.insert(candidate.clone()) {
                    break candidate;
                }
            };
            notes.push(EditorNote {
                id,
                midi: None,
                note: Note {
                    tick: *tick,
                    lane: lane + offset,
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
