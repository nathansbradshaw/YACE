//! Independent visual interpretation of the supported .chart gameplay subset.
//! No upstream implementation or assets are copied. Compatibility references:
//! YARG.Core 3beb94e526558134145bcd3e409428f759001a40, MoonNote.cs,
//! SongEntry.IniBase.cs, NoteFlagPriority.cs and MoonSongLoader.{Guitar,Drums}.cs.
//! See docs/GAMEPLAY_VIEW.md for pinned links, exact scope and source licenses.
use crate::{chart::Note, timing::TempoMap};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrumMode {
    Standard,
    Pro,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdSource {
    DefaultChartLibrary,
    SongIniFrequency,
    SongIniEighthNote,
    SongIniHopofreq,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub source_resolution: Option<u32>,
    /// Inclusive tick distance, already scaled/resolved for the source PPQ.
    pub hopo_threshold_ticks: Option<u64>,
    pub threshold_source: ThresholdSource,
    /// YARG's .chart library default is false; other formats differ.
    pub chord_hopo_cancellation: bool,
    pub drums: DrumMode,
}

impl Policy {
    pub fn unknown() -> Self {
        Self {
            source_resolution: None,
            hopo_threshold_ticks: None,
            threshold_source: ThresholdSource::Unknown,
            chord_hopo_cancellation: false,
            drums: DrumMode::Pro,
        }
    }
}

/// Resolve YARG's song-library .chart policy from normalized song.ini fields.
/// This is deliberately not its separate direct-parser or MIDI default path.
pub fn yarg_chart_policy(resolution: u32, ini: &BTreeMap<String, String>) -> Result<Policy> {
    ensure!(
        resolution > 0,
        "Gameplay view requires a positive source PPQ"
    );
    let mut source = ThresholdSource::DefaultChartLibrary;
    let explicit = ini
        .get("hopo_frequency")
        .map(|value| {
            value
                .trim()
                .parse::<i64>()
                .context("Invalid song.ini hopo_frequency")
        })
        .transpose()?;
    let threshold = if let Some(frequency) = explicit.filter(|value| *value > 0) {
        source = ThresholdSource::SongIniFrequency;
        frequency as u64
    } else {
        let ppq = u64::from(resolution);
        let base = if let Some(value) = ini.get("eighthnote_hopo") {
            source = ThresholdSource::SongIniEighthNote;
            // Match the confirmed bool-reader convention: '1' or a case-
            // insensitive 'true' prefix is true, all other present values false.
            let value = value.trim();
            let eighth = value.starts_with('1')
                || value
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("true"));
            ppq / if eighth { 2 } else { 3 }
        } else if let Some(value) = ini.get("hopofreq") {
            source = ThresholdSource::SongIniHopofreq;
            let denominator = match value
                .trim()
                .parse::<i32>()
                .context("Invalid song.ini hopofreq")?
            {
                0 => 24,
                1 => 16,
                2 => 12,
                3 => 8,
                4 => 6,
                5 => 4,
                _ => anyhow::bail!("Unsupported song.ini hopofreq; expected 0 through 5"),
            };
            4 * ppq / denominator
        } else {
            ppq / 3
        };
        base + ppq / 192
    };
    ensure!(
        threshold <= u64::from(u32::MAX),
        "HOPO threshold exceeds the confirmed game tick range"
    );
    Ok(Policy {
        source_resolution: Some(resolution),
        hopo_threshold_ticks: Some(threshold),
        threshold_source: source,
        chord_hopo_cancellation: false,
        drums: DrumMode::Pro,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GemShape {
    Fret,
    Open,
    Kick,
    DoubleKick,
    Pad,
    Tom,
    Cymbal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Technique {
    Strum,
    Hopo,
    Tap,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dynamics {
    Neutral,
    Accent,
    Ghost,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GemVisual {
    pub tick: u64,
    pub lane: u8,
    pub sustain: u64,
    pub seconds: f64,
    pub end_seconds: f64,
    pub shape: GemShape,
    pub technique: Option<Technique>,
    pub dynamics: Dynamics,
    pub attack_size: usize,
    pub natural_hopo: Option<bool>,
    pub forced_marker: bool,
    pub tap_marker: bool,
}

fn supported_track(track: &str) -> Option<bool> {
    ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .find_map(|tier| match track.strip_prefix(tier)? {
            "Single" | "DoubleBass" | "Keyboard" | "ProKeys" => Some(false),
            "Drums" => Some(true),
            _ => None,
        })
}

/// Classify a COMPLETE supported .chart track before viewport filtering.
/// Pure read-only display semantics, not scoring, format conversion or playback.
pub fn classify(
    track: &str,
    notes: &[Note],
    timing: &TempoMap,
    policy: &Policy,
) -> Result<Vec<GemVisual>> {
    let drums = supported_track(track).context("Unsupported gameplay-view track")?;
    let pro_keys = track.ends_with("ProKeys");
    let keys = pro_keys || track.ends_with("Keyboard");
    ensure!(
        policy
            .source_resolution
            .is_none_or(|resolution| resolution == timing.resolution),
        "Gameplay policy PPQ does not match source timing"
    );
    ensure!(
        timing.resolution > 0
            && timing.offset.is_finite()
            && timing.tempos.first().is_some_and(|tempo| tempo.tick == 0)
            && timing
                .tempos
                .iter()
                .all(|tempo| tempo.bpm.is_finite() && (1.0..=1000.0).contains(&tempo.bpm))
            && timing
                .tempos
                .windows(2)
                .all(|pair| pair[0].tick < pair[1].tick),
        "Invalid gameplay-view timing"
    );
    let mut positions = BTreeSet::new();
    let mut attacks: BTreeMap<u64, Vec<&Note>> = BTreeMap::new();
    for note in notes {
        ensure!(
            positions.insert((note.tick, note.lane)),
            "Duplicate gameplay-view lane/tick"
        );
        note.tick
            .checked_add(note.sustain)
            .context("Gameplay-view tick overflow")?;
        let playable = if pro_keys {
            (48..=72).contains(&note.lane)
        } else if keys {
            note.lane <= 4
        } else {
            note.lane <= 4
                || if drums {
                    note.lane == 32
                } else {
                    note.lane == 7
                }
        };
        if playable {
            attacks.entry(note.tick).or_default().push(note);
        }
    }
    let mut visuals = Vec::new();
    let mut previous: Option<(u64, BTreeSet<u8>)> = None;
    for (tick, mut attack) in attacks {
        attack.sort_by_key(|note| note.lane);
        let lanes: BTreeSet<_> = attack.iter().map(|note| note.lane).collect();
        let forced = !drums && !keys && positions.contains(&(tick, 5));
        let tap = !drums && !keys && positions.contains(&(tick, 6));
        for note in attack {
            let (shape, technique, dynamics, natural_hopo) = if keys {
                (GemShape::Fret, None, Dynamics::Neutral, None)
            } else if drums {
                let shape = match note.lane {
                    0 => GemShape::Kick,
                    32 => GemShape::DoubleKick,
                    1 => GemShape::Pad,
                    _ if policy.drums == DrumMode::Pro
                        && positions.contains(&(tick, note.lane + 64)) =>
                    {
                        GemShape::Cymbal
                    }
                    _ => GemShape::Tom,
                };
                let dynamics = if (1..=4).contains(&note.lane)
                    && positions.contains(&(tick, note.lane + 33))
                {
                    Dynamics::Accent
                } else if (1..=4).contains(&note.lane)
                    && positions.contains(&(tick, note.lane + 39))
                {
                    Dynamics::Ghost
                } else {
                    Dynamics::Neutral
                };
                (shape, None, dynamics, None)
            } else {
                let natural = if lanes.len() != 1 {
                    Some(false)
                } else if let Some((previous_tick, previous_lanes)) = &previous {
                    if previous_lanes.len() == 1 && previous_lanes.contains(&note.lane) {
                        Some(false)
                    } else {
                        policy
                            .hopo_threshold_ticks
                            .map(|threshold| tick - *previous_tick <= threshold)
                    }
                } else {
                    Some(false)
                };
                let mut technique = if tap {
                    Technique::Tap
                } else {
                    match natural.map(|natural| natural != forced) {
                        Some(true) => Technique::Hopo,
                        Some(false) => Technique::Strum,
                        None => Technique::Unknown,
                    }
                };
                if technique == Technique::Hopo
                    && policy.chord_hopo_cancellation
                    && lanes.len() == 1
                    && previous.as_ref().is_some_and(|(_, previous_lanes)| {
                        previous_lanes.len() > 1 && previous_lanes.contains(&note.lane)
                    })
                {
                    technique = Technique::Strum;
                }
                (
                    if note.lane == 7 {
                        GemShape::Open
                    } else {
                        GemShape::Fret
                    },
                    Some(technique),
                    Dynamics::Neutral,
                    natural,
                )
            };
            visuals.push(GemVisual {
                tick,
                lane: note.lane,
                sustain: note.sustain,
                seconds: timing.seconds(tick),
                end_seconds: timing.seconds(tick + note.sustain),
                shape,
                technique,
                dynamics,
                attack_size: lanes.len(),
                natural_hopo,
                forced_marker: forced,
                tap_marker: tap,
            });
        }
        previous = Some((tick, lanes));
    }
    Ok(visuals)
}
