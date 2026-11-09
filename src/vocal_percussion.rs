//! Dedicated vocal percussion phrases, separate from pitched/talkie syllables.
//! MIDI keys 96/97 mean displayed/hidden percussion. Lead and harmony scoring
//! are separate; the harmony owner is HARM1. Other source tracks stay opaque.
//! Format: https://github.com/TheNathannator/GuitarGame_ChartFormats/blob/main/docs/Chart-File-Formats/mid-format/Tracks/Vocals.md
use crate::{
    song_files::game_midi::NoteAttributes,
    timing::TempoMap,
    vocal_harmonies::HarmonyChart,
    vocals::{VocalChart, VocalPhrase},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "vocal_percussion/midi.rs"]
mod midi;
pub use midi::import_midi;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Part {
    Lead,
    Harm1,
}
impl Part {
    pub fn track(self) -> &'static str {
        match self {
            Self::Lead => "PART VOCALS",
            Self::Harm1 => "HARM1",
        }
    }
    pub fn from_midi(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "PART VOCALS" => Some(Self::Lead),
            "HARM1" | "PART HARM1" => Some(Self::Harm1),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    pub start: u64,
    /// Zero-length MIDI hits are retained explicitly, without extending them.
    pub end: u64,
    pub displayed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<NoteAttributes>,
}
impl Hit {
    pub fn key(&self) -> u8 {
        if self.displayed { 96 } else { 97 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phrase {
    pub range: VocalPhrase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<NoteAttributes>,
}
impl Phrase {
    pub fn covers(&self, hit: &Hit) -> bool {
        self.range.start <= hit.start && hit.start < self.range.end && hit.end <= self.range.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub source: String,
    pub phrases: Vec<Phrase>,
    pub hits: Vec<Hit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PercussionChart {
    pub version: u32,
    pub resolution: u32,
    pub source: String,
    pub tracks: BTreeMap<Part, Track>,
}

fn attributes(value: Option<&NoteAttributes>, zero_length: bool) -> Result<()> {
    if let Some(value) = value {
        value.validate()?;
        ensure!(
            value.talkie_pitch.is_none(),
            "Percussion is not a talkie pitch"
        );
        ensure!(
            value.zero_length == zero_length,
            "Percussion MIDI duration attributes disagree with the hit span"
        );
    }
    Ok(())
}
fn separate_phrases(percussion: &[Phrase], singing: &[VocalPhrase]) -> bool {
    let mut cursor = 0;
    for phrase in percussion {
        while singing
            .get(cursor)
            .is_some_and(|other| other.end <= phrase.range.start)
        {
            cursor += 1;
        }
        if singing
            .get(cursor)
            .is_some_and(|other| other.start < phrase.range.end)
        {
            return false;
        }
    }
    true
}
impl PercussionChart {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported percussion document version or resolution"
        );
        ensure!(
            !self.source.trim().is_empty(),
            "Percussion provenance is required"
        );
        ensure!(
            !self.tracks.is_empty(),
            "Percussion chart needs an authored part"
        );
        for track in self.tracks.values() {
            ensure!(
                !track.source.trim().is_empty(),
                "Percussion part provenance is required"
            );
            ensure!(
                !track.hits.is_empty() && track.hits.len() <= 131072,
                "Percussion part needs 1..131072 hits"
            );
            ensure!(track.phrases.len() <= 16384, "Too many percussion phrases");
            ensure!(
                track
                    .hits
                    .windows(2)
                    .all(|pair| pair[0].start < pair[1].start && pair[0].end <= pair[1].start),
                "Percussion hits must be ordered, distinct and non-overlapping within a part"
            );
            ensure!(
                track
                    .phrases
                    .windows(2)
                    .all(|pair| pair[0].range.end <= pair[1].range.start),
                "Percussion phrases must be ordered and non-overlapping"
            );
            for phrase in &track.phrases {
                let range = &phrase.range;
                ensure!(
                    range.start < range.end
                        && range.end <= u64::from(u32::MAX)
                        && matches!(range.player, 1 | 2),
                    "Invalid percussion scoring phrase span or player"
                );
                attributes(phrase.midi.as_ref(), false)?;
                let first_hit = track.hits.partition_point(|hit| hit.start < range.start);
                ensure!(
                    track
                        .hits
                        .get(first_hit)
                        .is_some_and(|hit| phrase.covers(hit)),
                    "Empty percussion scoring phrase"
                );
            }
            for hit in &track.hits {
                ensure!(
                    hit.start <= hit.end && hit.end <= u64::from(u32::MAX),
                    "Invalid percussion hit span"
                );
                attributes(hit.midi.as_ref(), hit.start == hit.end)?;
                let next_phrase = track
                    .phrases
                    .partition_point(|phrase| phrase.range.start <= hit.start);
                ensure!(
                    next_phrase
                        .checked_sub(1)
                        .and_then(|index| track.phrases.get(index))
                        .is_some_and(|phrase| phrase.covers(hit)),
                    "Every percussion hit needs one complete dedicated scoring phrase"
                );
            }
        }
        Ok(())
    }

    /// Dedicated percussion phrases cannot overlap singing/scoring phrases.
    /// This never shortens, moves or deletes a singer to accommodate a hit.
    pub fn validate_with_singing(
        &self,
        lead: Option<&VocalChart>,
        harmonies: Option<&HarmonyChart>,
    ) -> Result<()> {
        self.validate()?;
        if let Some(lead) = lead {
            lead.validate()?;
            ensure!(
                lead.resolution == self.resolution,
                "Percussion/lead clock mismatch"
            );
        }
        if let Some(harmonies) = harmonies {
            harmonies.validate()?;
            ensure!(
                harmonies.resolution == self.resolution,
                "Percussion/harmony clock mismatch"
            );
        }
        for (&part, track) in &self.tracks {
            let phrases = match part {
                Part::Lead => lead.map(|lead| lead.phrases.as_slice()),
                Part::Harm1 => harmonies.map(|harmonies| harmonies.phrases.as_slice()),
            };
            ensure!(
                phrases.is_none_or(|singing| separate_phrases(&track.phrases, singing)),
                "Percussion needs a dedicated phrase outside the applicable singing phrases"
            );
        }
        Ok(())
    }

    pub fn validate_timing(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.validate()?;
        ensure!(
            timing.resolution == self.resolution && duration.is_finite() && duration > 0.,
            "Percussion/project timing mismatch"
        );
        ensure!(
            self.tracks.values().all(|track| track
                .hits
                .iter()
                .map(|hit| hit.end)
                .chain(track.phrases.iter().map(|phrase| phrase.range.end))
                .all(
                    |end| timing.seconds(end).is_finite() && timing.seconds(end) <= duration + 0.1
                )),
            "Percussion hits or phrases extend beyond the timeline"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "vocal_percussion/tests.rs"]
mod tests;
