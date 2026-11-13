//! Project/source binding and export composition for vocal power markers.
use super::*;
use crate::vocal_overdrive::{self as power, Edit, Part, State};
use std::collections::BTreeSet;

pub fn source_overdrive(song: &Path, resolution: u32) -> Result<State> {
    let state = match source(song)? {
        Some((_, directory)) => power::import_midi(&fs::read(directory.join("notes.mid"))?)?,
        None => State::empty(resolution, None),
    };
    ensure!(
        state.resolution == resolution,
        "Vocal overdrive/source resolution mismatch"
    );
    Ok(state)
}

pub fn read_overdrive_state(song: &Path, resolution: u32) -> Result<Option<State>> {
    let working = song_layout::resolve(song, "chart-engine-working");
    let path = working.join("game-notes.mid");
    if !fs::is_file(&path) {
        return Ok(None);
    }
    // Validate the immutable receipt even for unsupported power originals.
    let original = match source(song)? {
        Some((_, directory)) => match power::import_midi(&fs::read(directory.join("notes.mid"))?) {
            Ok(state) => state,
            Err(_) => return Ok(None), // Its original control bytes remain opaque.
        },
        None => State::empty(resolution, None),
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
        "Working vocal overdrive MIDI binding changed; source retained"
    );
    let mut current = power::import_midi(&bytes)?;
    ensure!(
        current.resolution == resolution,
        "Working vocal overdrive resolution changed"
    );
    current.source_sha256 = original.source_sha256.clone();
    let has_ranges = |state: &State| state.tracks.values().any(|t| !t.phrases.is_empty());
    if current.tracks == original.tracks || !has_ranges(&current) && !has_ranges(&original) {
        return Ok(None);
    }
    Ok(Some(current))
}

fn applicable(project: &crate::editor::Project, current: &State, part: Part) -> bool {
    // A retained physical vocal owner can receive its first power range even
    // when it contains percussion alone. Explicit omissions use the current
    // empty working state, so this cannot revive an archived owner.
    if current.tracks.contains_key(&part) {
        return true;
    }
    let singing = match part {
        Part::Lead => project.content.vocals.is_some(),
        Part::Harm1 => project.content.harmonies.is_some(),
    };
    singing
        || project
            .content
            .vocal_percussion
            .as_ref()
            .and_then(|s| s.document())
            .is_some_and(|p| p.tracks.contains_key(&part))
}

fn empty_midi(resolution: u32) -> Result<Vec<u8>> {
    use midly::{
        Format, Header, MetaMessage, Smf, Timing, TrackEvent, TrackEventKind,
        num::{u15, u28},
    };
    let mut bytes = Vec::new();
    Smf {
        header: Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(u16::try_from(resolution)?)),
        ),
        tracks: vec![vec![
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::TrackName(b"YACE")),
            },
            TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            },
        ]],
    }
    .write_std(&mut bytes)?;
    Ok(bytes)
}

pub fn edit_overdrive(
    project: &mut crate::editor::Project,
    chart: &Chart,
    song: &Path,
    part: Part,
    edit: &Edit,
) -> Result<()> {
    let before = source_overdrive(song, chart.timing.resolution)?;
    let current = project.content.vocal_overdrive.as_ref().unwrap_or(&before);
    ensure!(
        current.source_sha256 == before.source_sha256,
        "Vocal overdrive belongs to another source"
    );
    ensure!(
        applicable(project, current, part),
        "Create a vocal part or select its existing overdrive before editing"
    );
    let next = power::edited(current, part, edit)?;
    next.validate(
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    let bytes = match source(song)? {
        Some((_, directory)) => fs::read(directory.join("notes.mid"))?,
        None => empty_midi(before.resolution)?,
    };
    power::rewrite_midi(&bytes, &hash(&bytes), &before, &next)?;
    project.content.vocal_overdrive = Some(next);
    Ok(())
}

/// Apply only after singer, harmony, percussion and instrument composition.
/// Omitted parts stay omitted; a selected marker-only owner can be appended.
pub fn export_overdrive(
    song: &Path,
    state: &State,
    bytes: &[u8],
    selected: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let mut before = source_overdrive(song, state.resolution)?;
    ensure!(
        state.source_sha256 == before.source_sha256,
        "Vocal overdrive source binding changed; export refused"
    );
    let actual = power::import_midi(bytes)?;
    if let Some((imported, _)) = source(song)? {
        let active = extra_parts(song)?;
        let extras = original_extra_parts(song, false)?;
        for (part, track) in &mut before.tracks {
            let retained = match part {
                Part::Lead if imported.baseline_vocals_sha256.is_some() => {
                    retained_vocal_controls(song)?
                }
                Part::Harm1 if imported.baseline_harmonies_sha256.is_some() => {
                    retained_harmony_controls(song)?
                }
                _ => {
                    let key = format!("MIDI: {}", track.owner);
                    !extras.contains(&key) || active.contains(&key)
                }
            };
            if !retained {
                track.phrases.clear();
            }
        }
    }
    before
        .tracks
        .retain(|part, _| actual.tracks.contains_key(part));
    let mut after = state.clone();
    after.tracks.retain(|part, _| {
        selected.contains(match part {
            Part::Lead => "Vocals",
            Part::Harm1 => "Harmonies",
        })
    });
    power::rewrite_midi(bytes, &hash(bytes), &before, &after)
}
