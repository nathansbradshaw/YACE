//! Independent harmony voices with the MIDI format's shared phrase ownership.
//! Lead vocals remain a separate document; overlapping singers never share a
//! monophonic note list. HARM1 owns scoring, HARM2 owns the second lyric range.
//! Format reference: https://github.com/TheNathannator/GuitarGame_ChartFormats/blob/main/docs/Chart-File-Formats/mid-format/Tracks/Vocals.md
use crate::{
    timing::TempoMap,
    vocals::{VocalChart, VocalNote, VocalPhrase},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Part {
    Harm1,
    Harm2,
    Harm3,
}
impl Part {
    pub const ALL: [Self; 3] = [Self::Harm1, Self::Harm2, Self::Harm3];
    pub fn track(self) -> &'static str {
        match self {
            Self::Harm1 => "HARM1",
            Self::Harm2 => "HARM2",
            Self::Harm3 => "HARM3",
        }
    }
    pub fn from_midi(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "HARM1" | "PART HARM1" => Some(Self::Harm1),
            "HARM2" | "PART HARM2" => Some(Self::Harm2),
            "HARM3" | "PART HARM3" => Some(Self::Harm3),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub source: String,
    pub notes: Vec<VocalNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarmonyChart {
    pub version: u32,
    pub resolution: u32,
    pub source: String,
    /// HARM1 scoring phrases apply to all voices, including HARM2/HARM3-only spans.
    pub phrases: Vec<VocalPhrase>,
    /// HARM2 lyric ranges must cover both HARM2 and HARM3; HARM3 owns no markers.
    pub lyric_phrases: Vec<VocalPhrase>,
    pub voices: BTreeMap<Part, Voice>,
}

fn contains(phrase: &VocalPhrase, note: &VocalNote) -> bool {
    phrase.start <= note.start && note.end <= phrase.end
}
fn phrase_shape(phrases: &[VocalPhrase]) -> Result<()> {
    ensure!(phrases.len() <= 16384, "Too many harmony phrases");
    ensure!(
        phrases.iter().all(|phrase| phrase.start < phrase.end
            && phrase.end <= u64::from(u32::MAX)
            && matches!(phrase.player, 1 | 2)),
        "Invalid harmony phrase span or player"
    );
    ensure!(
        phrases.windows(2).all(|pair| pair[0].end <= pair[1].start),
        "Harmony phrases must be ordered and non-overlapping"
    );
    Ok(())
}
impl HarmonyChart {
    /// A display/validation view of one voice. It does not transfer ownership
    /// of scoring phrases or mutate another singer's notes.
    pub fn voice_chart(&self, part: Part) -> Option<VocalChart> {
        let voice = self.voices.get(&part)?;
        Some(VocalChart {
            version: 1,
            resolution: self.resolution,
            source: voice.source.clone(),
            phrases: self
                .phrases
                .iter()
                .filter(|phrase| voice.notes.iter().any(|note| contains(phrase, note)))
                .cloned()
                .collect(),
            notes: voice.notes.clone(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported harmony document version or resolution"
        );
        ensure!(
            !self.source.trim().is_empty(),
            "Harmony source/provenance is required"
        );
        ensure!(
            !self.voices.is_empty() && self.voices.values().any(|voice| !voice.notes.is_empty()),
            "Harmony chart needs at least one authored voice"
        );
        phrase_shape(&self.phrases)?;
        phrase_shape(&self.lyric_phrases)?;
        for (&part, voice) in &self.voices {
            ensure!(
                !voice.source.trim().is_empty(),
                "Harmony voice source/provenance is required"
            );
            if !voice.notes.is_empty() {
                // Each singer is monophonic, while simultaneous notes in other
                // voices are independent and must not trigger collisions.
                self.voice_chart(part).unwrap().validate()?;
            }
        }
        ensure!(
            self.phrases.iter().all(|phrase| self
                .voices
                .values()
                .any(|voice| voice.notes.iter().any(|note| contains(phrase, note)))),
            "Empty shared harmony scoring phrase"
        );
        for (&part, voice) in &self.voices {
            if part == Part::Harm1 {
                continue;
            }
            ensure!(
                voice.notes.iter().all(|note| self
                    .lyric_phrases
                    .iter()
                    .filter(|phrase| contains(phrase, note))
                    .count()
                    == 1),
                "HARM2 lyric ranges must cover every HARM2 and HARM3 note"
            );
        }
        ensure!(
            self.lyric_phrases
                .iter()
                .all(|phrase| [Part::Harm2, Part::Harm3]
                    .into_iter()
                    .filter_map(|part| self.voices.get(&part))
                    .any(|voice| voice.notes.iter().any(|note| contains(phrase, note)))),
            "Empty HARM2 lyric range"
        );
        Ok(())
    }
    pub fn validate_timing(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.validate()?;
        ensure!(
            timing.resolution == self.resolution && duration.is_finite() && duration > 0.,
            "Harmony/project timing mismatch"
        );
        ensure!(self.voices.values().flat_map(|voice| &voice.notes).map(|note| note.end)
            .chain(self.phrases.iter().chain(&self.lyric_phrases).map(|phrase| phrase.end))
            .all(|end| timing.seconds(end).is_finite() && timing.seconds(end) <= duration + 0.1),
            "Harmony notes or phrases extend beyond the timeline");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocals::{Syllable, VocalTone};
    fn fixture() -> HarmonyChart {
        HarmonyChart {
            version: 1,
            resolution: 480,
            source: "Manual harmony fixture".into(),
            phrases: vec![VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            }],
            lyric_phrases: vec![VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            }],
            voices: Part::ALL
                .into_iter()
                .enumerate()
                .map(|(index, part)| {
                    (
                        part,
                        Voice {
                            source: format!("Independent {}", part.track()),
                            notes: vec![VocalNote {
                                start: 240,
                                end: 720,
                                tone: VocalTone::Pitched {
                                    midi: 60 + index as u8 * 4,
                                },
                                lyric: Syllable::Text {
                                    text: format!("voice{index}"),
                                    join_next: false,
                                },
                                midi: None,
                            }],
                        },
                    )
                })
                .collect(),
        }
    }
    #[test]
    fn simultaneous_singers_remain_independent_and_round_trip_the_document() {
        let mut chart = fixture();
        chart.validate().unwrap();
        let others = (
            chart.voices[&Part::Harm1].clone(),
            chart.voices[&Part::Harm3].clone(),
        );
        chart.voices.get_mut(&Part::Harm2).unwrap().notes[0].tone = VocalTone::Pitched { midi: 65 };
        chart.validate().unwrap();
        assert_eq!(
            others,
            (
                chart.voices[&Part::Harm1].clone(),
                chart.voices[&Part::Harm3].clone()
            )
        );
        let encoded = serde_json::to_vec(&chart).unwrap();
        assert_eq!(
            serde_json::from_slice::<HarmonyChart>(&encoded).unwrap(),
            chart
        );
        let voice = chart.voice_chart(Part::Harm3).unwrap();
        assert_eq!(voice.notes[0].start, 240);
        assert_eq!(voice.phrases, chart.phrases);
    }
    #[test]
    fn collisions_within_a_voice_and_uncovered_shared_ranges_refuse() {
        let mut chart = fixture();
        let duplicate = chart.voices[&Part::Harm2].notes[0].clone();
        chart
            .voices
            .get_mut(&Part::Harm2)
            .unwrap()
            .notes
            .push(duplicate);
        assert!(chart.validate().is_err());
        let mut chart = fixture();
        chart.phrases[0].end = 600;
        assert!(chart.validate().is_err());
        let mut chart = fixture();
        chart.lyric_phrases[0].end = 600;
        assert!(chart.validate().is_err());
    }
    #[test]
    fn shared_scoring_can_cover_a_span_with_no_harm1_notes() {
        let mut chart = fixture();
        chart.voices.remove(&Part::Harm1);
        chart.validate().unwrap();
        assert!(chart.voice_chart(Part::Harm1).is_none());
        assert_eq!(Part::from_midi("PART HARM3"), Some(Part::Harm3));
        assert_eq!(Part::from_midi("PART VOCALS"), None);
    }
}
