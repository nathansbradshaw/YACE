//! Project/history view for local drum-mix authoring.
use super::{Edit, Event, State, Tracks, difficulty};
use crate::{
    chart::Chart,
    editor::{Project, Store},
    song_files::game_midi,
    timing::TempoMap,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredMidi {
    pub state: State,
    pub difficulties: BTreeSet<u8>,
}
impl AuthoredMidi {
    pub fn validate(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.state.validate_shape()?;
        ensure!(
            self.state.source_sha256.is_some() && self.state.resolution == timing.resolution,
            "Drum mix source/clock binding changed"
        );
        for &difficulty in &self.difficulties {
            ensure!(difficulty <= 3, "Unknown authored drum mix difficulty");
            validate_ranges(&self.state.events, difficulty, timing, duration)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub midi: bool,
    pub physical: Option<State>,
    pub chart: Tracks,
    pub error: Option<String>,
}
pub fn source(store: &Store, chart: &Chart) -> Source {
    let song = store.root.join("source");
    if game_midi::exists(&song) {
        return match game_midi::source_drum_mix(&song, chart.timing.resolution) {
            Ok(state) => Source {
                midi: true,
                physical: Some(state),
                ..Default::default()
            },
            Err(e) => Source {
                midi: true,
                error: Some(format!("{e:#}")),
                ..Default::default()
            },
        };
    }
    let mut result = Source::default();
    for track in crate::drum_ranges::TRACKS {
        match super::import_chart(chart, track) {
            Ok(events) => {
                result.chart.insert(track.into(), events);
            }
            Err(e) => {
                result.error = Some(format!("{e:#}"));
                return result;
            }
        }
    }
    result
}
pub fn view(project: &Project, source: &Source, track: &str) -> Result<Vec<Event>> {
    ensure!(
        source.error.is_none(),
        "{}",
        source
            .error
            .as_deref()
            .unwrap_or("Unsupported drum mix source")
    );
    let difficulty = difficulty(track)?;
    if source.midi {
        let state = project
            .content
            .midi_drum_mix
            .as_ref()
            .map(|s| &s.state)
            .or(source.physical.as_ref())
            .ok_or_else(|| anyhow::anyhow!("Drum mix source is unavailable"))?;
        return Ok(state
            .events
            .iter()
            .filter(|e| e.mix.difficulty == difficulty)
            .cloned()
            .collect());
    }
    Ok(project
        .content
        .disco_ranges
        .get(track)
        .or(source.chart.get(track))
        .cloned()
        .unwrap_or_default())
}
pub fn all(project: &Project, source: &Source) -> Result<Vec<Event>> {
    let mut events = vec![];
    for track in crate::drum_ranges::TRACKS {
        events.extend(view(project, source, track)?);
    }
    events.sort_by_key(|e| (e.tick, e.mix.difficulty));
    super::validate(&events)?;
    Ok(events)
}
pub fn validate_ranges(
    events: &[Event],
    difficulty: u8,
    timing: &TempoMap,
    duration: f64,
) -> Result<()> {
    for range in super::ranges(events, difficulty)? {
        let end = timing.seconds(range.end());
        ensure!(
            end.is_finite() && end <= duration + 0.1,
            "Disco range extends beyond the recording or timeline"
        );
    }
    Ok(())
}
pub(crate) fn validate_edit(project: &Project, chart: &Chart, edit: &Edit) -> Result<()> {
    if let Edit::Put { end, .. } = edit {
        let timing = crate::sync_track::effective(project, &chart.timing)?;
        let end = timing.seconds(*end);
        ensure!(
            end.is_finite() && end <= project.duration + 0.1,
            "Disco range extends beyond the recording or timeline"
        );
    }
    Ok(())
}
pub fn apply(
    store: &Store,
    project: &mut Project,
    chart: &Chart,
    track: &str,
    edit: &Edit,
) -> Result<()> {
    ensure!(
        project.content.tracks.contains_key(track),
        "Create or select a drum part before editing disco flip"
    );
    let difficulty = difficulty(track)?;
    validate_edit(project, chart, edit)?;
    if game_midi::exists(&store.root.join("source")) {
        return game_midi::edit_drum_mix(
            project,
            chart,
            &store.root.join("source"),
            difficulty,
            edit,
        );
    }
    let source = source(store, chart);
    let events = all(project, &source)?;
    let next = super::edited(&events, difficulty, edit)?;
    project.content.disco_ranges.insert(
        track.into(),
        next.into_iter()
            .filter(|e| e.mix.difficulty == difficulty)
            .collect(),
    );
    Ok(())
}

pub fn validate_chart(
    tracks: &BTreeMap<String, Vec<Event>>,
    project: &Project,
    timing: &TempoMap,
) -> Result<()> {
    for (track, events) in tracks {
        let difficulty = difficulty(track)?;
        ensure!(
            project.content.tracks.contains_key(track)
                && events.iter().all(|e| e.mix.difficulty == difficulty),
            "Disco events belong to an unknown drum part"
        );
        super::validate(events)?;
        validate_ranges(events, difficulty, timing, project.duration)?;
    }
    Ok(())
}
