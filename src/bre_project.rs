//! Project binding for global coda and supported physical BRE controls.
use super::{Edit, State};
use crate::{
    chart::Chart,
    editor::{Project, Store},
    song_files::game_midi,
    timing::TempoMap,
};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

pub fn source(store: &Store, chart: &Chart) -> Result<State> {
    game_midi::source_bre(&store.root.join("source"), chart)
}
pub fn current(project: &Project, source: &State) -> Result<State> {
    let state = project.content.bre.as_ref().unwrap_or(source);
    ensure!(
        state.source_sha256 == source.source_sha256 && state.resolution == source.resolution,
        "BRE belongs to a different source"
    );
    state.validate_shape()?;
    Ok(state.clone())
}
pub fn owner(track: &str) -> Option<&'static str> {
    super::OWNERS.iter().find_map(|(owner, suffix)| {
        ["Easy", "Medium", "Hard", "Expert"]
            .into_iter()
            .any(|tier| track == format!("{tier}{suffix}"))
            .then_some(*owner)
    })
}
pub fn selected(owner: &str, selected: &BTreeSet<String>) -> bool {
    selected.contains(&format!("MIDI: {owner}"))
        || super::OWNERS
            .iter()
            .find(|(name, _)| *name == owner)
            .is_some_and(|(_, suffix)| {
                ["Easy", "Medium", "Hard", "Expert"]
                    .into_iter()
                    .any(|tier| selected.contains(&format!("{tier}{suffix}")))
            })
}
pub fn midi_required(project: &Project, selected: &BTreeSet<String>) -> bool {
    project.content.bre.as_ref().is_some_and(|state| {
        state
            .owners
            .iter()
            .any(|(owner, phrases)| !phrases.is_empty() && self::selected(owner, selected))
    })
}
pub fn render_project(
    text: String,
    state: Option<&State>,
    physical_source: bool,
) -> Result<String> {
    // Physical controls keep their authority in the retained MIDI. Inserting
    // them into the normalized chart would rewrite untouched global tracks.
    if physical_source {
        return Ok(text);
    }
    let Some(state) = state else { return Ok(text) };
    let mut coda = state.clone();
    coda.owners.clear();
    super::render_chart(text, Some(&coda))
}
pub fn apply(store: &Store, project: &mut Project, chart: &Chart, edit: &Edit) -> Result<()> {
    let before = source(store, chart)?;
    let old = current(project, &before)?;
    let drum = match edit {
        Edit::DrumPut {
            track,
            previous,
            range,
        } => {
            ensure!(
                old.coda.as_ref().is_some_and(|c| c.tick <= range.start),
                "Set the global coda at or before this drum BRE first"
            );
            ensure!(
                previous
                    .as_ref()
                    .is_none_or(|r| r.kind == crate::drum_ranges::Kind::FillActivation),
                "Choose a drum activation range"
            );
            Some(crate::drum_ranges::Edit::Put {
                track: track.clone(),
                previous: previous.clone(),
                range: crate::drum_ranges::Range {
                    start: range.start,
                    end: range.end,
                    kind: crate::drum_ranges::Kind::FillActivation,
                },
            })
        }
        Edit::DrumRemove { track, range } => {
            ensure!(
                range.kind == crate::drum_ranges::Kind::FillActivation
                    && old.coda.as_ref().is_some_and(|c| c.tick <= range.start),
                "This range is no longer a drum BRE; select it again"
            );
            Some(crate::drum_ranges::Edit::Remove {
                track: track.clone(),
                range: range.clone(),
            })
        }
        _ => None,
    };
    if let Some(edit) = drum {
        return if game_midi::exists(&store.root.join("source")) {
            game_midi::edit_drum_ranges(project, chart, &store.root.join("source"), &edit)
        } else {
            crate::drum_ranges::apply(project, chart, &edit)
        };
    }
    if let Edit::Put { owner, .. } | Edit::Remove { owner, .. } = edit {
        ensure!(
            project
                .content
                .tracks
                .keys()
                .any(|track| self::owner(track) == Some(owner.as_str())),
            "Create or select an instrument part before editing its BRE"
        );
    }
    let next = super::edited(&old, edit)?;
    next.validate(
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    project.content.bre = Some(next);
    Ok(())
}
pub fn validate(state: &State, timing: &TempoMap, duration: f64) -> Result<()> {
    ensure!(
        state.source_sha256.is_some(),
        "BRE project has no source binding"
    );
    state.validate(timing, duration)
}
