//! Non-destructive section replacement and source routing, shared by both clients.
use crate::{
    editor::{Content, Project, TimeRange},
    mapping::GuitarSource,
    timing::TempoMap,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInterval {
    pub range: TimeRange,
    pub source: GuitarSource,
}
pub type Sources = BTreeMap<String, Vec<SourceInterval>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Setup,
    Analyzing,
    Applying,
    Review,
    Stopped,
    Failed,
    Empty,
}
impl Phase {
    pub fn with_problem(self, problem: Option<&str>) -> Self {
        if self == Self::Failed
            && problem.is_some_and(|problem| {
                problem.contains("No notes were detected")
                    || problem.contains("No vocal notes were detected")
            })
        {
            Self::Empty
        } else {
            self
        }
    }
    pub fn for_job(busy: bool, state: Option<&crate::generation::State>, error: bool) -> Self {
        use crate::generation::State;
        if busy {
            return if state == Some(&State::Complete) {
                Self::Applying
            } else {
                Self::Analyzing
            };
        }
        match state {
            Some(State::Cancelled) => Self::Stopped,
            _ if error => Self::Failed,
            Some(State::Complete) => Self::Review,
            Some(State::Failed) => Self::Failed,
            _ => Self::Setup,
        }
    }
}
#[derive(Clone, Debug)]
pub struct AuditionRoute {
    pub intervals: Vec<SourceInterval>,
    pub fallback: GuitarSource,
    pub playback_rate: f64,
}
impl AuditionRoute {
    fn at(&self, seconds: f64) -> GuitarSource {
        self.intervals
            .iter()
            .find(|interval| seconds >= interval.range.start && seconds < interval.range.end)
            .map_or(self.fallback, |interval| interval.source)
    }
    /// A 10ms crossfade in source time. Seeking uses the same deterministic weights.
    pub fn weights(&self, seconds: f64) -> [(crate::stem_review::StemId, f32); 2] {
        let current = self.at(seconds);
        let boundary = self
            .intervals
            .iter()
            .flat_map(|interval| [interval.range.start, interval.range.end])
            .filter(|boundary| *boundary <= seconds && seconds - *boundary < 0.010)
            .max_by(f64::total_cmp);
        if let Some(boundary) = boundary {
            let previous = self.at(boundary - 1e-7);
            let fraction = ((seconds - boundary) / 0.010).clamp(0., 1.) as f32;
            [(previous.stem(), 1. - fraction), (current.stem(), fraction)]
        } else {
            [(current.stem(), 1.), (current.stem(), 0.)]
        }
    }
}
pub fn audition_route(
    project: &Project,
    fallback: GuitarSource,
    enabled: bool,
) -> Option<AuditionRoute> {
    (enabled && project.selected_track.ends_with("Single") && project.kit_audition.is_none())
        .then(|| {
            project
                .content
                .analysis_sources
                .get(&project.selected_track)
                .filter(|intervals| !intervals.is_empty())
                .map(|intervals| AuditionRoute {
                    intervals: intervals.clone(),
                    fallback,
                    playback_rate: project.transport.playback_rate,
                })
        })
        .flatten()
}
pub fn preview_transport(action: &crate::editor::Action) -> bool {
    use crate::editor::Action;
    matches!(
        action,
        Action::Seek { .. }
            | Action::Playback { .. }
            | Action::Pause { .. }
            | Action::SetLoop { .. }
            | Action::SetPlaybackRate { .. }
    )
}

pub fn validate_range(range: TimeRange, duration: f64) -> Result<()> {
    ensure!(
        range.start.is_finite()
            && range.end.is_finite()
            && range.start >= 0.
            && range.start < range.end
            && range.end <= duration + 1e-6,
        "Choose a non-empty range inside the recording"
    );
    Ok(())
}
pub fn validate_sources(content: &Content, duration: f64) -> Result<()> {
    for (track, intervals) in &content.analysis_sources {
        crate::parts::instrument(track)?;
        let mut end = 0.;
        for interval in intervals {
            validate_range(interval.range, duration)?;
            ensure!(
                interval.range.start >= end,
                "Analysis sources overlap or are unordered"
            );
            end = interval.range.end;
        }
    }
    Ok(())
}
pub fn source_at(
    content: &Content,
    track: &str,
    seconds: f64,
    fallback: GuitarSource,
) -> GuitarSource {
    content
        .analysis_sources
        .get(track)
        .into_iter()
        .flatten()
        .find(|interval| seconds >= interval.range.start && seconds < interval.range.end)
        .map_or(fallback, |interval| interval.source)
}
pub fn replace_source(content: &mut Content, track: &str, range: TimeRange, source: GuitarSource) {
    let intervals = content.analysis_sources.entry(track.into()).or_default();
    let mut replaced = Vec::new();
    for interval in intervals.drain(..) {
        if interval.range.start < range.start {
            let end = interval.range.end.min(range.start);
            if interval.range.start < end {
                replaced.push(SourceInterval {
                    range: TimeRange {
                        start: interval.range.start,
                        end,
                    },
                    source: interval.source,
                });
            }
        }
        if interval.range.end > range.end {
            let start = interval.range.start.max(range.end);
            if start < interval.range.end {
                replaced.push(SourceInterval {
                    range: TimeRange {
                        start,
                        end: interval.range.end,
                    },
                    source: interval.source,
                });
            }
        }
    }
    replaced.push(SourceInterval { range, source });
    replaced.sort_by(|a, b| a.range.start.total_cmp(&b.range.start));
    for interval in replaced {
        if let Some(last) = intervals.last_mut()
            && last.source == interval.source
            && (last.range.end - interval.range.start).abs() < 1e-9
        {
            last.range.end = interval.range.end;
        } else {
            intervals.push(interval);
        }
    }
}
pub fn affected(content: &Content, track: &str) -> Vec<String> {
    let mut tracks = vec![track.into()];
    tracks.extend(
        content
            .difficulty_links
            .links
            .iter()
            .filter(|(_, link)| link.enabled && link.expert_track == track)
            .map(|(track, _)| track.clone()),
    );
    tracks.sort();
    tracks.dedup();
    tracks
}
fn event_ranges(content: &Content, tracks: &[String], timing: &TempoMap) -> Vec<TimeRange> {
    let mut ranges = Vec::new();
    for track in tracks {
        if track == "Vocals" {
            if let Some(chart) = &content.vocals {
                ranges.extend(chart.notes.iter().map(|n| TimeRange {
                    start: timing.seconds(n.start),
                    end: timing.seconds(n.end),
                }));
                ranges.extend(chart.phrases.iter().map(|p| TimeRange {
                    start: timing.seconds(p.start),
                    end: timing.seconds(p.end),
                }));
            }
        } else if let Some(notes) = content.tracks.get(track) {
            ranges.extend(notes.iter().map(|n| TimeRange {
                start: timing.seconds(n.note.tick),
                end: timing.seconds(n.note.tick.saturating_add(n.note.sustain)),
            }));
        }
    }
    ranges
}
/// Half-open scope; a tap exactly at the right boundary belongs to the next section.
pub fn contains(range: TimeRange, seconds: f64) -> bool {
    seconds >= range.start - 1e-9 && seconds < range.end - 1e-9
}
pub fn expand(
    mut range: TimeRange,
    contents: &[&Content],
    tracks: &[String],
    timing: &TempoMap,
    duration: f64,
) -> Result<TimeRange> {
    validate_range(range, duration)?;
    let events: Vec<_> = contents
        .iter()
        .flat_map(|content| event_ranges(content, tracks, timing))
        .collect();
    loop {
        let old = range;
        for event in &events {
            if event.start < range.end - 1e-9 && event.end > range.start + 1e-9 {
                range.start = range.start.min(event.start).max(0.);
                range.end = range.end.max(event.end).min(duration);
            }
        }
        if old == range {
            return Ok(range);
        }
    }
}

/// Include any linked reduction changes that extend beyond the current proposal.
pub fn include_linked_changes(
    mut range: TimeRange,
    before: &Content,
    after: &Content,
    tracks: &[String],
    timing: &TempoMap,
    duration: f64,
) -> Result<TimeRange> {
    for track in tracks {
        let a = before.tracks.get(track).cloned().unwrap_or_default();
        let b = after.tracks.get(track).cloned().unwrap_or_default();
        for (left, right) in [(&a, &b), (&b, &a)] {
            let existing: std::collections::BTreeSet<_> =
                right.iter().map(|note| &note.note).collect();
            for note in left.iter().filter(|note| !existing.contains(&note.note)) {
                range.start = range.start.min(timing.seconds(note.note.tick).max(0.));
                range.end = range.end.max(
                    timing
                        .seconds(note.note.tick.saturating_add(note.note.sustain.max(1)))
                        .min(duration),
                );
            }
        }
    }
    expand(range, &[before, after], tracks, timing, duration)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub track: String,
    pub before: usize,
    pub after: usize,
    pub added: usize,
    pub removed: usize,
}
pub fn changes(before: &Project, after: &Project, tracks: &[String]) -> Vec<Change> {
    tracks
        .iter()
        .map(|track| {
            let values = |p: &Project| -> Vec<String> {
                if track == "Vocals" {
                    p.content
                        .vocals
                        .as_ref()
                        .map(|v| {
                            v.notes
                                .iter()
                                .map(|n| serde_json::to_string(n).unwrap())
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    p.content
                        .tracks
                        .get(track)
                        .into_iter()
                        .flatten()
                        .map(|n| serde_json::to_string(&n.note).unwrap())
                        .collect()
                }
            };
            let a = values(before);
            let b = values(after);
            // Count duplicate events correctly without treating regenerated IDs as musical edits.
            let counts = |items: &[String]| {
                let mut counts = BTreeMap::new();
                for item in items {
                    *counts.entry(item.clone()).or_insert(0usize) += 1;
                }
                counts
            };
            let aa = counts(&a);
            let bb = counts(&b);
            Change {
                track: track.clone(),
                before: a.len(),
                after: b.len(),
                added: bb
                    .iter()
                    .map(|(k, n)| n.saturating_sub(*aa.get(k).unwrap_or(&0)))
                    .sum(),
                removed: aa
                    .iter()
                    .map(|(k, n)| n.saturating_sub(*bb.get(k).unwrap_or(&0)))
                    .sum(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_has_one_phase_and_stopped_jobs_never_look_running() {
        use crate::generation::State;
        assert_eq!(Phase::for_job(false, None, false), Phase::Setup);
        assert_eq!(Phase::for_job(true, None, false), Phase::Analyzing);
        assert_eq!(
            Phase::for_job(false, Some(&State::Cancelled), true),
            Phase::Stopped
        );
        assert_eq!(
            Phase::for_job(false, Some(&State::Failed), true),
            Phase::Failed
        );
        assert_eq!(
            Phase::for_job(false, Some(&State::Complete), false),
            Phase::Review
        );
        assert_eq!(
            Phase::for_job(true, Some(&State::Complete), false),
            Phase::Applying
        );
        assert_eq!(
            Phase::for_job(false, Some(&State::Complete), true),
            Phase::Failed
        );
        assert_eq!(
            Phase::for_job(true, Some(&State::Failed), false),
            Phase::Analyzing
        );
        assert_eq!(
            Phase::for_job(false, Some(&State::Complete), true)
                .with_problem(Some("No notes were detected for this difficulty")),
            Phase::Empty
        );
    }
    #[test]
    fn source_replacement_splits_and_coalesces() {
        let mut sources = Content::default();
        replace_source(
            &mut sources,
            "ExpertSingle",
            TimeRange {
                start: 0.,
                end: 10.,
            },
            GuitarSource::Guitar,
        );
        replace_source(
            &mut sources,
            "ExpertSingle",
            TimeRange { start: 3., end: 5. },
            GuitarSource::Other,
        );
        assert_eq!(sources.analysis_sources["ExpertSingle"].len(), 3);
        assert_eq!(
            source_at(&sources, "ExpertSingle", 4., GuitarSource::Guitar),
            GuitarSource::Other
        );
        replace_source(
            &mut sources,
            "ExpertSingle",
            TimeRange { start: 3., end: 5. },
            GuitarSource::Guitar,
        );
        assert_eq!(sources.analysis_sources["ExpertSingle"].len(), 1);
    }
    #[test]
    fn range_expansion_reaches_a_fixed_point_across_old_and_proposed_sustains() {
        let mut old = Content::default();
        let mut new = Content::default();
        let note = |tick, sustain| crate::editor::EditorNote {
            id: format!("{tick}"),
            midi: None,
            note: crate::chart::Note {
                tick,
                lane: 0,
                sustain,
            },
        };
        old.tracks
            .insert("ExpertSingle".into(), vec![note(960, 1920)]);
        new.tracks
            .insert("ExpertSingle".into(), vec![note(2880, 1920)]);
        let timing = TempoMap::constant(120.).unwrap();
        let range = expand(
            TimeRange {
                start: timing.seconds(2000),
                end: timing.seconds(3000),
            },
            &[&old, &new],
            &["ExpertSingle".into()],
            &timing,
            10.,
        )
        .unwrap();
        assert_eq!(
            range,
            TimeRange {
                start: timing.seconds(960),
                end: timing.seconds(4800)
            }
        );
        assert!(!contains(range, range.end));
        assert!(
            validate_range(
                TimeRange {
                    start: f64::NAN,
                    end: 2.
                },
                10.
            )
            .is_err()
        );
    }
    #[test]
    fn source_crossfades_and_unavailable_stems_use_original_pcm() {
        use crate::stem_review::{LoadedStems, StemId, StemMix};
        use std::sync::Arc;
        let original = Arc::new(vec![0.2; 400]);
        let loaded = LoadedStems::from_pcm(
            100,
            original,
            BTreeMap::from([(StemId::Guitar, Arc::new(vec![0.8; 400]))]),
        )
        .unwrap();
        let mut mix = StemMix::default().prepare([StemId::Guitar]).unwrap();
        mix.route = Some(AuditionRoute {
            intervals: vec![SourceInterval {
                range: TimeRange { start: 1., end: 2. },
                source: GuitarSource::Other,
            }],
            fallback: GuitarSource::Guitar,
            playback_rate: 1.,
        });
        assert_eq!(loaded.frame_with_mix(50, &mix), [0.8, 0.8]);
        assert_eq!(loaded.frame_with_mix(100, &mix), [0.8, 0.8]);
        assert_eq!(loaded.frame_with_mix(102, &mix), [0.2, 0.2]);
        let weights = mix.route.as_ref().unwrap().weights(1.005);
        assert!((weights[0].1 - 0.5).abs() < 1e-5 && (weights[1].1 - 0.5).abs() < 1e-5);
    }
}
