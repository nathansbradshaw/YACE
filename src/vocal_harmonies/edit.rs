//! Target one singer or the actual shared phrase owner, with stale-value guards.
use super::{HarmonyChart, Part, Voice};
use crate::vocals::{VocalNote, VocalPhrase};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhraseKind {
    Scoring,
    Lyrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    PutNote {
        part: Part,
        index: Option<usize>,
        expected: Option<VocalNote>,
        note: VocalNote,
        #[serde(default)]
        scoring_phrase: Option<VocalPhrase>,
        #[serde(default)]
        lyric_phrase: Option<VocalPhrase>,
    },
    RemoveNote {
        part: Part,
        index: usize,
        expected: VocalNote,
    },
    PutPhrase {
        kind: PhraseKind,
        index: Option<usize>,
        expected: Option<VocalPhrase>,
        phrase: VocalPhrase,
    },
    RemovePhrase {
        kind: PhraseKind,
        index: usize,
        expected: VocalPhrase,
    },
}
fn phrases(chart: &mut HarmonyChart, kind: PhraseKind) -> &mut Vec<VocalPhrase> {
    match kind {
        PhraseKind::Scoring => &mut chart.phrases,
        PhraseKind::Lyrics => &mut chart.lyric_phrases,
    }
}
fn add_phrase(phrases: &mut Vec<VocalPhrase>, phrase: &Option<VocalPhrase>) {
    if let Some(phrase) = phrase
        && !phrases.contains(phrase)
    {
        phrases.push(phrase.clone());
    }
}
fn covers(phrase: &VocalPhrase, note: &VocalNote) -> bool {
    phrase.start <= note.start && note.end <= phrase.end
}
impl Edit {
    /// Produce a validated copy. Failure leaves the caller's document intact.
    /// A first note needs explicit shared ranges; no musical content is guessed.
    pub fn apply(
        &self,
        current: Option<&HarmonyChart>,
        resolution: u32,
    ) -> Result<Option<HarmonyChart>> {
        if let Some(current) = current {
            current.validate()?;
        }
        ensure!(
            current.is_some() || matches!(self, Self::PutNote { index: None, .. }),
            "Create a first harmony note with explicit shared phrases"
        );
        let mut chart = current.cloned().unwrap_or_else(|| HarmonyChart {
            version: 1,
            resolution,
            source: "Manual independent harmony authoring".into(),
            phrases: vec![],
            lyric_phrases: vec![],
            voices: BTreeMap::new(),
        });
        ensure!(
            chart.resolution == resolution,
            "Harmony/project timing mismatch"
        );
        match self {
            Self::PutNote {
                part,
                index,
                expected,
                note,
                scoring_phrase,
                lyric_phrase,
            } => {
                let voice = chart.voices.entry(*part).or_insert_with(|| Voice {
                    source: format!("Manual {} authoring", part.track()),
                    notes: vec![],
                });
                if let Some(index) = index {
                    let actual = voice
                        .notes
                        .get_mut(*index)
                        .context("Unknown harmony note")?;
                    ensure!(
                        expected.as_ref() == Some(&*actual),
                        "Selected harmony note changed; select it again"
                    );
                    *actual = note.clone();
                } else {
                    ensure!(
                        expected.is_none(),
                        "A new harmony note cannot replace an existing note"
                    );
                    voice.notes.push(note.clone());
                }
                voice.notes.sort_by_key(|note| (note.start, note.end));
                add_phrase(&mut chart.phrases, scoring_phrase);
                add_phrase(&mut chart.lyric_phrases, lyric_phrase);
            }
            Self::RemoveNote {
                part,
                index,
                expected,
            } => {
                let voice = chart
                    .voices
                    .get_mut(part)
                    .context("Unknown harmony singer")?;
                ensure!(
                    voice.notes.get(*index) == Some(expected),
                    "Selected harmony note changed; select it again"
                );
                voice.notes.remove(*index);
                if chart.voices.values().all(|voice| voice.notes.is_empty()) {
                    return Ok(None);
                }
                chart.phrases.retain(|phrase| {
                    chart
                        .voices
                        .values()
                        .any(|voice| voice.notes.iter().any(|note| covers(phrase, note)))
                });
                chart.lyric_phrases.retain(|phrase| {
                    [Part::Harm2, Part::Harm3]
                        .into_iter()
                        .filter_map(|part| chart.voices.get(&part))
                        .any(|voice| voice.notes.iter().any(|note| covers(phrase, note)))
                });
            }
            Self::PutPhrase {
                kind,
                index,
                expected,
                phrase,
            } => {
                let target = phrases(&mut chart, *kind);
                if let Some(index) = index {
                    let actual = target
                        .get_mut(*index)
                        .context("Unknown shared harmony phrase")?;
                    ensure!(
                        expected.as_ref() == Some(&*actual),
                        "Shared harmony phrase changed; select it again"
                    );
                    *actual = phrase.clone();
                } else {
                    ensure!(
                        expected.is_none(),
                        "A new shared phrase cannot replace an existing phrase"
                    );
                    target.push(phrase.clone());
                }
            }
            Self::RemovePhrase {
                kind,
                index,
                expected,
            } => {
                let target = phrases(&mut chart, *kind);
                ensure!(
                    target.get(*index) == Some(expected),
                    "Shared harmony phrase changed; select it again"
                );
                target.remove(*index);
            }
        }
        chart
            .phrases
            .sort_by_key(|phrase| (phrase.start, phrase.end));
        chart
            .lyric_phrases
            .sort_by_key(|phrase| (phrase.start, phrase.end));
        chart.validate()?;
        Ok(Some(chart))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocals::VocalTone;
    fn fixture() -> HarmonyChart {
        crate::vocal_harmonies::tests::fixture()
    }
    fn put(part: Part, expected: VocalNote, note: VocalNote) -> Edit {
        Edit::PutNote {
            part,
            index: Some(0),
            expected: Some(expected),
            note,
            scoring_phrase: None,
            lyric_phrase: None,
        }
    }
    #[test]
    fn note_edit_targets_one_singer_and_rejects_a_stale_value_at_the_same_index() {
        let before = fixture();
        let expected = before.voices[&Part::Harm2].notes[0].clone();
        let mut changed = expected.clone();
        changed.start += 1;
        changed.tone = VocalTone::Pitched { midi: 65 };
        let edit = put(Part::Harm2, expected, changed.clone());
        let after = edit.apply(Some(&before), 480).unwrap().unwrap();
        assert_eq!(after.voices[&Part::Harm2].notes, [changed]);
        assert_eq!(after.voices[&Part::Harm1], before.voices[&Part::Harm1]);
        assert_eq!(after.voices[&Part::Harm3], before.voices[&Part::Harm3]);
        assert_eq!(after.phrases, before.phrases);
        assert_eq!(after.lyric_phrases, before.lyric_phrases);
        assert!(
            edit.apply(Some(&after), 480)
                .unwrap_err()
                .to_string()
                .contains("note changed")
        );
    }
    #[test]
    fn removing_singers_prunes_only_their_shared_ranges_and_last_note_clears_document() {
        let mut chart = fixture();
        for part in [Part::Harm2, Part::Harm3, Part::Harm1] {
            let expected = chart.voices[&part].notes[0].clone();
            let next = Edit::RemoveNote {
                part,
                index: 0,
                expected,
            }
            .apply(Some(&chart), 480)
            .unwrap();
            if part == Part::Harm1 {
                assert!(next.is_none());
                return;
            }
            chart = next.unwrap();
            assert!(!chart.phrases.is_empty());
            assert_eq!(chart.lyric_phrases.is_empty(), part == Part::Harm3);
        }
    }
    #[test]
    fn first_harm3_note_needs_explicit_scoring_and_shared_lyric_ranges() {
        let source = fixture();
        let note = source.voices[&Part::Harm3].notes[0].clone();
        let missing = Edit::PutNote {
            part: Part::Harm3,
            index: None,
            expected: None,
            note: note.clone(),
            scoring_phrase: None,
            lyric_phrase: None,
        };
        assert!(missing.apply(None, 480).is_err());
        let chart = Edit::PutNote {
            part: Part::Harm3,
            index: None,
            expected: None,
            note,
            scoring_phrase: Some(source.phrases[0].clone()),
            lyric_phrase: Some(source.lyric_phrases[0].clone()),
        }
        .apply(None, 480)
        .unwrap()
        .unwrap();
        assert_eq!(chart.voices.len(), 1);
        assert!(chart.voices.contains_key(&Part::Harm3));
        assert_eq!(chart.phrases, source.phrases);
        assert_eq!(chart.lyric_phrases, source.lyric_phrases);
    }
    #[test]
    fn shared_phrase_edits_do_not_replace_notes_or_other_owners_and_refuse_uncovered_voices() {
        let chart = fixture();
        let expected = chart.lyric_phrases[0].clone();
        let mut phrase = expected.clone();
        phrase.end += 120;
        let after = Edit::PutPhrase {
            kind: PhraseKind::Lyrics,
            index: Some(0),
            expected: Some(expected.clone()),
            phrase,
        }
        .apply(Some(&chart), 480)
        .unwrap()
        .unwrap();
        assert_eq!(after.voices, chart.voices);
        assert_eq!(after.phrases, chart.phrases);
        assert!(
            Edit::RemovePhrase {
                kind: PhraseKind::Lyrics,
                index: 0,
                expected
            }
            .apply(Some(&chart), 480)
            .is_err()
        );
        let mut stale = chart.phrases[0].clone();
        stale.end += 1;
        assert!(
            Edit::RemovePhrase {
                kind: PhraseKind::Scoring,
                index: 0,
                expected: stale
            }
            .apply(Some(&chart), 480)
            .unwrap_err()
            .to_string()
            .contains("phrase changed")
        );
    }
    #[test]
    fn same_time_notes_are_independent_across_singers_but_refused_within_one_voice() {
        let mut chart = fixture();
        chart.voices.remove(&Part::Harm3);
        let note = chart.voices[&Part::Harm2].notes[0].clone();
        let make = |part| Edit::PutNote {
            part,
            index: None,
            expected: None,
            note: note.clone(),
            scoring_phrase: None,
            lyric_phrase: None,
        };
        assert!(make(Part::Harm3).apply(Some(&chart), 480).is_ok());
        assert!(make(Part::Harm2).apply(Some(&chart), 480).is_err());
        assert_eq!(chart.voices[&Part::Harm2].notes.len(), 1);
    }
}
