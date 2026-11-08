//! Pro Guitar strings and frets keep their ordinary MIDI note attributes.
//! String rows are independent of five-fret colors and six-fret button pairs.
use crate::{
    editor::{EditorNote, Project},
    song_files::game_midi::NoteAttributes,
};
use anyhow::{Context, Result, ensure};

pub const PARTS: [(&str, &str); 2] = [
    ("Pro Guitar (17 fret)", "ProGuitar17"),
    ("Pro Guitar (22 fret)", "ProGuitar22"),
];
/// A dedicated attack marker; lane 5 is the sixth string, never a forced flag.
pub const HOPO: u8 = 32;
pub const ROWS: [u8; 6] = [5, 4, 3, 2, 1, 0];

pub fn max_fret(track: &str) -> Option<u8> {
    let suffix = ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
        .unwrap_or(track);
    match suffix {
        "ProGuitar17" => Some(17),
        "ProGuitar22" => Some(22),
        _ => None,
    }
}
pub fn is_track(track: &str) -> bool {
    max_fret(track).is_some()
}
pub fn midi_name(track: &str) -> Option<&'static str> {
    match max_fret(track)? {
        17 => Some("PART REAL_GUITAR"),
        _ => Some("PART REAL_GUITAR_22"),
    }
}
pub fn rank(track: &str) -> Option<&'static str> {
    Some(match max_fret(track)? {
        17 => "diff_guitar_real",
        _ => "diff_guitar_real_22",
    })
}
pub fn string_name(lane: u8) -> Option<&'static str> {
    [
        "String 6 (low)",
        "String 5",
        "String 4",
        "String 3",
        "String 2",
        "String 1 (high)",
    ]
    .get(lane as usize)
    .copied()
}
pub fn color(lane: u8) -> &'static str {
    [
        "#ec5360", "#45c878", "#f2a65a", "#4b9be9", "#ead452", "#c889ff",
    ]
    .get(lane as usize)
    .copied()
    .unwrap_or("#697789")
}
pub fn fret(note: &EditorNote) -> u8 {
    note.midi
        .as_ref()
        .map_or(0, |m| m.velocity.saturating_sub(100))
}
pub fn channel(note: &EditorNote) -> u8 {
    note.midi.as_ref().map_or(0, |m| m.channel)
}
pub fn articulation(channel: u8) -> &'static str {
    match channel {
        0 => "Normal",
        1 => "Ghost guide",
        2 => "Bend",
        3 => "Muted",
        4 => "Tap",
        5 => "Harmonic",
        6 => "Pinch harmonic",
        _ => "Preserved MIDI channel",
    }
}
pub fn validate(track: &str, note: &EditorNote) -> Result<()> {
    max_fret(track).context("Choose a Pro Guitar track")?;
    if note.note.lane == HOPO {
        ensure!(
            note.note.sustain == 0 && note.midi.is_none(),
            "Invalid Pro Guitar HOPO attack marker"
        );
    } else {
        ensure!(note.note.lane < 6, "Invalid Pro Guitar string");
        validate_attributes(track, note.midi.as_ref())?;
    }
    Ok(())
}
pub fn validate_attributes(track: &str, attributes: Option<&NoteAttributes>) -> Result<()> {
    let limit = max_fret(track).context("Choose a Pro Guitar track")?;
    if let Some(attributes) = attributes {
        attributes.validate()?;
        ensure!(
            (100..=100 + limit).contains(&attributes.velocity),
            "Pro Guitar fret exceeds this {limit}-fret part; existing notes have been kept"
        );
        ensure!(
            attributes.talkie_pitch.is_none(),
            "Vocal attributes cannot become Pro Guitar frets"
        );
    }
    Ok(())
}
pub fn forced(notes: &[EditorNote], tick: u64) -> bool {
    notes
        .iter()
        .any(|n| n.note.tick == tick && n.note.lane == HOPO)
}
/// The command engine supplies revision guarding and a complete Undo checkpoint.
pub fn edit(
    project: &mut Project,
    id: &str,
    fret: u8,
    channel: u8,
    forced_hopo: bool,
) -> Result<()> {
    let (track, original) = project.find_note(id).context("Select a Pro Guitar note")?;
    let limit = max_fret(track).context("Select a Pro Guitar note")?;
    ensure!(
        original.note.lane < 6 && fret <= limit && channel < 16,
        "Choose a string note, fret 0–{limit} and valid MIDI channel"
    );
    let track = track.to_owned();
    let tick = original.note.tick;
    let mut metadata = original.midi.clone().unwrap_or(NoteAttributes {
        channel: 0,
        velocity: 100,
        off_velocity: 0,
        off_as_note_on: false,
        zero_length: false,
        talkie_pitch: None,
    });
    metadata.velocity = 100 + fret;
    metadata.channel = channel;
    let marker = if forced_hopo && !forced(&project.content.tracks[&track], tick) {
        let next = project.next_id.checked_add(1).context("Note ID overflow")?;
        let marker = EditorNote {
            id: format!("edit-note-{}", project.next_id),
            midi: None,
            note: crate::chart::Note {
                tick,
                lane: HOPO,
                sustain: 0,
            },
        };
        ensure!(
            !project
                .content
                .tracks
                .values()
                .flatten()
                .any(|n| n.id == marker.id),
            "Note ID already exists"
        );
        Some((marker, next))
    } else {
        None
    };
    let notes = project.content.tracks.get_mut(&track).unwrap();
    notes.iter_mut().find(|n| n.id == id).unwrap().midi =
        (!metadata.is_default()).then_some(metadata);
    if !forced_hopo {
        notes.retain(|n| n.note.tick != tick || n.note.lane != HOPO);
    }
    if let Some((marker, next)) = marker {
        notes.push(marker);
        project.next_id = next;
    }
    notes.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
    Ok(())
}
