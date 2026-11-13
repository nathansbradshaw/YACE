//! Immutable source and existing hash-bound working MIDI for drum mix text.
use super::*;
use crate::drum_mix::{self as mix, AuthoredMidi, Edit, State};
use std::collections::BTreeSet;

pub fn source_drum_mix(song: &Path, resolution: u32) -> Result<State> {
    let (_, directory) =
        source(song)?.context("Drum mix MIDI needs its verified imported source")?;
    let state = mix::import_midi(&fs::read(directory.join("notes.mid"))?)?;
    ensure!(
        state.resolution == resolution,
        "Drum mix/source resolution mismatch"
    );
    Ok(state)
}
fn has_owner(bytes: &[u8]) -> Result<bool> {
    Ok(midly::Smf::parse(bytes)?.tracks.iter().any(|track| {
        track.iter().any(|e| {
            matches!(
                e.kind,
                midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"PART DRUMS"))
            )
        })
    }))
}
pub fn read_drum_mix_state(song: &Path, resolution: u32) -> Result<Option<AuthoredMidi>> {
    let working = song_layout::resolve(song, "chart-engine-working");
    let path = working.join("game-notes.mid");
    if !fs::is_file(&path) || !exists(song) {
        return Ok(None);
    }
    let original = match source_drum_mix(song, resolution) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        version: u32,
        master_sha256: String,
        midi_sha256: String,
    }
    let binding: Binding = serde_json::from_slice(&fs::read(working.join("binding.json"))?)?;
    let bytes = fs::read(path)?;
    ensure!(
        binding.version == 1
            && binding.midi_sha256 == hash(&bytes)
            && binding.master_sha256 == hash(&fs::read(working.join("notes.chart"))?),
        "Working drum mix MIDI binding changed; retain source"
    );
    let mut current = if has_owner(&bytes)? {
        mix::import_midi(&bytes)?
    } else {
        State {
            version: 1,
            resolution,
            source_sha256: None,
            events: vec![],
        }
    };
    ensure!(
        current.resolution == resolution,
        "Working drum mix resolution changed"
    );
    current.source_sha256 = original.source_sha256;
    let difficulties = (0..4)
        .filter(|d| {
            original
                .events
                .iter()
                .filter(|e| e.mix.difficulty == *d)
                .collect::<Vec<_>>()
                != current
                    .events
                    .iter()
                    .filter(|e| e.mix.difficulty == *d)
                    .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    if difficulties.is_empty() {
        return Ok(None);
    }
    Ok(Some(AuthoredMidi {
        state: current,
        difficulties,
    }))
}
pub fn edit_drum_mix(
    project: &mut crate::editor::Project,
    chart: &Chart,
    song: &Path,
    difficulty: u8,
    edit: &Edit,
) -> Result<()> {
    let before = source_drum_mix(song, chart.timing.resolution)?;
    let mut next = project
        .content
        .midi_drum_mix
        .clone()
        .unwrap_or_else(|| AuthoredMidi {
            state: before.clone(),
            difficulties: BTreeSet::new(),
        });
    ensure!(
        next.state.source_sha256 == before.source_sha256,
        "Drum mix belongs to another source"
    );
    next.state.events = mix::edited(&next.state.events, difficulty, edit)?;
    next.difficulties.insert(difficulty);
    next.validate(
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    let (_, directory) = source(song)?.context("Missing drum mix source")?;
    let bytes = fs::read(directory.join("notes.mid"))?;
    mix::rewrite_midi(&bytes, &hash(&bytes), &before, &next.state)?;
    project.content.midi_drum_mix = Some(next);
    Ok(())
}
/// Filter independently owned mix settings to the selected difficulties. An
/// unsupported inherited source stays opaque when no edit was requested.
pub fn export_drum_mix(
    song: &Path,
    state: Option<&AuthoredMidi>,
    bytes: &[u8],
    selected: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let Some((_, directory)) = source(song)? else {
        ensure!(
            state.is_none(),
            "Physical drum mix edits need their MIDI source"
        );
        return Ok(bytes.to_vec());
    };
    let original = fs::read(directory.join("notes.mid"))?;
    let before = match mix::import_midi(&original) {
        Ok(state) => state,
        Err(_) if state.is_none() => return Ok(bytes.to_vec()),
        Err(e) => return Err(e),
    };
    let mut after = state.map_or_else(|| before.clone(), |s| s.state.clone());
    ensure!(
        after.source_sha256 == before.source_sha256,
        "Drum mix source binding changed; export refused"
    );
    let retained: BTreeSet<_> = crate::drum_ranges::TRACKS
        .iter()
        .enumerate()
        .filter(|(_, t)| selected.contains(**t))
        .map(|(d, _)| d as u8)
        .collect();
    if retained.is_empty() {
        return Ok(bytes.to_vec());
    }
    after
        .events
        .retain(|e| retained.contains(&e.mix.difficulty));
    mix::rewrite_midi(bytes, &hash(bytes), &before, &after)
}
