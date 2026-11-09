//! Explicit hit and dedicated-phrase edits with exact selected-value guards.
use super::{Hit, Part, PercussionChart, Phrase, Track};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    PutHit {
        part: Part,
        index: Option<usize>,
        expected: Option<Hit>,
        hit: Hit,
        #[serde(default)]
        phrase: Option<Phrase>,
    },
    RemoveHit {
        part: Part,
        index: usize,
        expected: Hit,
    },
    PutPhrase {
        part: Part,
        index: usize,
        expected: Phrase,
        phrase: Phrase,
    },
    SplitPhrase {
        part: Part,
        index: usize,
        expected: Phrase,
        tick: u64,
    },
    MergePhrase {
        part: Part,
        index: usize,
        expected: Phrase,
        next_expected: Phrase,
    },
}
impl Edit {
    /// Validate a copy; a refusal leaves the original document untouched.
    /// A first hit requires an explicit dedicated phrase, never a guessed one.
    pub fn apply(
        &self,
        current: Option<&PercussionChart>,
        resolution: u32,
    ) -> Result<Option<PercussionChart>> {
        if let Some(current) = current {
            current.validate()?;
        }
        ensure!(
            current.is_some() || matches!(self, Self::PutHit { index: None, .. }),
            "Create a first percussion hit with an explicit dedicated phrase"
        );
        let mut chart = current.cloned().unwrap_or_else(|| PercussionChart {
            version: 1,
            resolution,
            source: "Manual vocal percussion authoring".into(),
            tracks: BTreeMap::new(),
        });
        ensure!(
            chart.resolution == resolution,
            "Percussion/project clock mismatch"
        );
        match self {
            Self::PutHit {
                part,
                index,
                expected,
                hit,
                phrase,
            } => {
                let track = chart.tracks.entry(*part).or_insert_with(|| Track {
                    source: format!("Manual {} percussion", part.track()),
                    phrases: vec![],
                    hits: vec![],
                });
                let mut old_phrase = None;
                if let Some(index) = index {
                    let actual = track
                        .hits
                        .get_mut(*index)
                        .context("Unknown percussion hit")?;
                    ensure!(
                        expected.as_ref() == Some(&*actual),
                        "Selected percussion hit changed; select it again"
                    );
                    old_phrase = track
                        .phrases
                        .iter()
                        .find(|phrase| phrase.covers(actual))
                        .cloned();
                    *actual = hit.clone();
                } else {
                    ensure!(
                        expected.is_none(),
                        "A new percussion hit cannot replace an existing hit"
                    );
                    track.hits.push(hit.clone());
                }
                if let Some(phrase) = phrase
                    && !track.phrases.contains(phrase)
                {
                    track.phrases.push(phrase.clone());
                }
                // Moving the last hit out of its original dedicated phrase is
                // one atomic edit. Prune only that witnessed old range; newly
                // supplied empty or conflicting ranges still fail validation.
                if let Some(old) = old_phrase
                    && !track.hits.iter().any(|hit| old.covers(hit))
                {
                    track.phrases.retain(|phrase| phrase != &old);
                }
            }
            Self::RemoveHit {
                part,
                index,
                expected,
            } => {
                let track = chart
                    .tracks
                    .get_mut(part)
                    .context("Unknown percussion part")?;
                ensure!(
                    track.hits.get(*index) == Some(expected),
                    "Selected percussion hit changed; select it again"
                );
                track.hits.remove(*index);
                // Dedicated phrases with no remaining hits have no playable
                // meaning. Other owners and every still-used phrase survive.
                track
                    .phrases
                    .retain(|phrase| track.hits.iter().any(|hit| phrase.covers(hit)));
                if track.hits.is_empty() {
                    chart.tracks.remove(part);
                }
                if chart.tracks.is_empty() {
                    return Ok(None);
                }
            }
            Self::PutPhrase {
                part,
                index,
                expected,
                phrase,
            } => {
                let track = chart
                    .tracks
                    .get_mut(part)
                    .context("Unknown percussion part")?;
                ensure!(
                    track.phrases.get(*index) == Some(expected),
                    "Selected percussion phrase changed; select it again"
                );
                track.phrases[*index] = phrase.clone();
            }
            Self::SplitPhrase {
                part,
                index,
                expected,
                tick,
            } => {
                let track = chart
                    .tracks
                    .get_mut(part)
                    .context("Unknown percussion part")?;
                ensure!(
                    track.phrases.get(*index) == Some(expected),
                    "Selected percussion phrase changed; select it again"
                );
                ensure!(
                    expected.range.start < *tick && *tick < expected.range.end,
                    "Split tick must be inside the percussion phrase"
                );
                ensure!(
                    !track
                        .hits
                        .iter()
                        .any(|hit| hit.start < *tick && *tick < hit.end),
                    "A percussion hit crosses the split tick; choose a gap"
                );
                track.phrases[*index].range.end = *tick;
                let mut second = expected.clone();
                second.range.start = *tick;
                track.phrases.insert(index + 1, second);
            }
            Self::MergePhrase {
                part,
                index,
                expected,
                next_expected,
            } => {
                let track = chart
                    .tracks
                    .get_mut(part)
                    .context("Unknown percussion part")?;
                let next = index
                    .checked_add(1)
                    .context("Invalid percussion phrase index")?;
                ensure!(
                    track.phrases.get(*index) == Some(expected)
                        && track.phrases.get(next) == Some(next_expected),
                    "Selected percussion phrases changed; select them again"
                );
                ensure!(
                    expected.range.player == next_expected.range.player
                        && expected.midi == next_expected.midi,
                    "Percussion phrases with different player or MIDI attributes cannot be merged losslessly"
                );
                track.phrases[*index].range.end = next_expected.range.end;
                track.phrases.remove(next);
            }
        }
        for track in chart.tracks.values_mut() {
            track.hits.sort_by_key(|hit| (hit.start, hit.end));
            track
                .phrases
                .sort_by_key(|phrase| (phrase.range.start, phrase.range.end));
        }
        chart.validate()?;
        Ok(Some(chart))
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
