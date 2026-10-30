//! Deterministic checks for newly generated five-fret/four-pad chart tracks.
//! Findings never rewrite notes or certify their musical accuracy.
use crate::{chart::Tracks, timing::TempoMap};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const BOUNDARY_TOLERANCE_SECONDS: f64 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleCode {
    InvalidTiming,
    InvalidDuration,
    UnsupportedTrack,
    UnsupportedLane,
    DuplicateLane,
    TickOverflow,
    OrphanCymbalMarker,
    OrphanControlMarker,
    DrumSustain,
    ControlMarkerSustain,
    OpenColoredChord,
    BeyondRecording,
    FastRepeat,
    HandCrowding,
    SustainRetriggerOverlap,
    LowerTierAttackAbsentFromExpert,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub severity: Severity,
    pub code: RuleCode,
    pub track: String,
    pub tick: u64,
    pub seconds: f64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleReport {
    pub version: u32,
    pub assessed_tracks: usize,
    pub assessed_note_events: usize,
    pub findings: Vec<Finding>,
    pub musical_acceptance: String,
}

impl RuleReport {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }
}

fn kind(track: &str) -> Option<(&str, bool)> {
    ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .find_map(|tier| {
            let instrument = track.strip_prefix(tier)?;
            matches!(
                instrument,
                "Single" | "DoubleBass" | "Drums" | "Keyboard" | "ProKeys"
            )
            .then_some((instrument, instrument == "Drums"))
        })
}

fn playable(drums: bool, lane: u8) -> bool {
    lane <= 4 || if drums { lane == 32 } else { lane == 7 }
}

fn supported_lane(drums: bool, lane: u8) -> bool {
    if drums {
        matches!(lane, 0..=4 | 32 | 34..=37 | 40..=43 | 66..=68)
    } else {
        lane <= 7
    }
}

/// Assess generated tracks only, never untouched authored/source tracks.
/// The accepted lane profile is five-fret guitar/bass and four-pad drums,
/// including double kick and recognized articulation/control markers.
pub fn assess(tracks: &Tracks, timing: &TempoMap, duration: f64) -> RuleReport {
    let mut report = RuleReport {
        version: 1,
        assessed_tracks: tracks.len(),
        assessed_note_events: tracks.values().map(Vec::len).sum(),
        findings: Vec::new(),
        musical_acceptance: "unverified".into(),
    };
    let valid_timing = timing.resolution > 0
        && timing.offset.is_finite()
        && timing.tempos.first().is_some_and(|tempo| tempo.tick == 0)
        && timing
            .tempos
            .iter()
            .all(|tempo| tempo.bpm.is_finite() && (1.0..=1000.0).contains(&tempo.bpm))
        && timing
            .tempos
            .windows(2)
            .all(|pair| pair[0].tick < pair[1].tick);
    if !valid_timing {
        report.findings.push(Finding {
            severity: Severity::Error,
            code: RuleCode::InvalidTiming,
            track: String::new(),
            tick: 0,
            seconds: 0.0,
            reason: "Cannot assess generated note timing with an invalid tempo map".into(),
        });
    }
    if !duration.is_finite() || duration <= 0.0 {
        report.findings.push(Finding {
            severity: Severity::Error,
            code: RuleCode::InvalidDuration,
            track: String::new(),
            tick: 0,
            seconds: 0.0,
            reason: "Recording duration must be finite and positive".into(),
        });
    }
    if report.has_errors() {
        return report;
    }
    let mut add = |severity, code, track: &str, tick, reason: String| {
        report.findings.push(Finding {
            severity,
            code,
            track: track.into(),
            tick,
            seconds: timing.seconds(tick),
            reason,
        });
    };
    for (track, original) in tracks {
        let Some((instrument, drums)) = kind(track) else {
            add(
                Severity::Error,
                RuleCode::UnsupportedTrack,
                track,
                0,
                "Track is outside the generated five-fret/four-pad profile".into(),
            );
            continue;
        };
        let pro_keys = instrument == "ProKeys";
        let is_playable = |lane| {
            if pro_keys {
                (48..=72).contains(&lane)
            } else if instrument == "Keyboard" {
                lane <= 4
            } else {
                playable(drums, lane)
            }
        };
        let is_supported = |lane| {
            if pro_keys {
                (48..=72).contains(&lane)
            } else if instrument == "Keyboard" {
                lane <= 4
            } else {
                supported_lane(drums, lane)
            }
        };
        let mut ordered = original.clone();
        ordered.sort();
        let mut positions = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for note in &ordered {
            if !positions.insert((note.tick, note.lane)) {
                duplicates.insert((note.tick, note.lane));
            }
        }
        for (tick, lane) in duplicates {
            add(
                Severity::Error,
                RuleCode::DuplicateLane,
                track,
                tick,
                format!("Lane {lane} occurs more than once at the same tick"),
            );
        }
        // Repeated malformed events should not inflate the softer checks.
        ordered.dedup_by_key(|note| (note.tick, note.lane));
        for note in &ordered {
            if !is_supported(note.lane) {
                add(
                    Severity::Error,
                    RuleCode::UnsupportedLane,
                    track,
                    note.tick,
                    format!(
                        "Lane {} is outside this generated track's supported profile",
                        note.lane
                    ),
                );
                continue;
            }
            let Some(end_tick) = note.tick.checked_add(note.sustain) else {
                add(
                    Severity::Error,
                    RuleCode::TickOverflow,
                    track,
                    note.tick,
                    "Note tick plus sustain overflows the supported tick range".into(),
                );
                continue;
            };
            if is_playable(note.lane) {
                if drums && note.sustain > 0 {
                    add(
                        Severity::Error,
                        RuleCode::DrumSustain,
                        track,
                        note.tick,
                        "Generated drum strikes must have zero sustain".into(),
                    );
                }
                if timing.seconds(end_tick) > duration + BOUNDARY_TOLERANCE_SECONDS {
                    add(
                        Severity::Error,
                        RuleCode::BeyondRecording,
                        track,
                        note.tick,
                        format!(
                            "Note extends beyond recording plus {:.0} ms boundary tolerance",
                            BOUNDARY_TOLERANCE_SECONDS * 1000.0
                        ),
                    );
                }
            } else {
                if note.sustain > 0 {
                    add(
                        Severity::Error,
                        RuleCode::ControlMarkerSustain,
                        track,
                        note.tick,
                        "Generated control markers must have zero sustain".into(),
                    );
                }
                let target = if drums {
                    match note.lane {
                        66..=68 => Some(note.lane - 64),
                        34..=37 => Some(note.lane - 33),
                        40..=43 => Some(note.lane - 39),
                        _ => None,
                    }
                } else {
                    None
                };
                let attached = target.map_or_else(
                    || {
                        (0..=4)
                            .chain(std::iter::once(7))
                            .any(|lane| positions.contains(&(note.tick, lane)))
                    },
                    |lane| positions.contains(&(note.tick, lane)),
                );
                if !attached {
                    add(
                        Severity::Error,
                        if drums && (66..=68).contains(&note.lane) {
                            RuleCode::OrphanCymbalMarker
                        } else {
                            RuleCode::OrphanControlMarker
                        },
                        track,
                        note.tick,
                        format!(
                            "Control lane {} has no matching playable gem at this attack",
                            note.lane
                        ),
                    );
                }
            }
            if !drums
                && note.lane == 7
                && (0..=4).any(|lane| positions.contains(&(note.tick, lane)))
            {
                add(
                    Severity::Error,
                    RuleCode::OpenColoredChord,
                    track,
                    note.tick,
                    "A five-fret open note cannot share an attack with colored gems".into(),
                );
            }
        }

        let gems: Vec<_> = ordered
            .iter()
            .filter(|note| is_playable(note.lane))
            .collect();
        let mut by_lane: BTreeMap<u8, Vec<_>> = BTreeMap::new();
        for note in &gems {
            by_lane.entry(note.lane).or_default().push(*note);
        }
        for (lane, notes) in by_lane {
            for pair in notes.windows(2) {
                let gap = timing.seconds(pair[1].tick) - timing.seconds(pair[0].tick);
                if gap > 0.0 && gap < 0.050 - 1e-9 {
                    add(
                        Severity::Review,
                        RuleCode::FastRepeat,
                        track,
                        pair[1].tick,
                        format!(
                            "Lane {lane} repeats after {:.1} ms; check the real roll/picking pattern before changing it",
                            gap * 1000.0
                        ),
                    );
                }
                if !drums
                    && pair[0]
                        .tick
                        .checked_add(pair[0].sustain)
                        .is_some_and(|end| end > pair[1].tick)
                {
                    add(
                        Severity::Review,
                        RuleCode::SustainRetriggerOverlap,
                        track,
                        pair[1].tick,
                        format!(
                            "Lane {lane} retriggers before its prior sustain ends; check the intended release"
                        ),
                    );
                }
            }
        }
        if drums {
            let hands: Vec<_> = gems
                .iter()
                .filter(|note| (1..=4).contains(&note.lane))
                .collect();
            let mut flagged = BTreeSet::new();
            for (i, first) in hands.iter().enumerate() {
                let count = hands[i..]
                    .iter()
                    .take_while(|note| {
                        timing.seconds(note.tick) - timing.seconds(first.tick) <= 0.020 + 1e-9
                    })
                    .count();
                if count > 2 && flagged.insert(first.tick) {
                    add(
                        Severity::Review,
                        RuleCode::HandCrowding,
                        track,
                        first.tick,
                        format!(
                            "{count} hand strikes occur within 20 ms; check kit assignment and physical playability"
                        ),
                    );
                }
            }
        }
        if !track.starts_with("Expert")
            && let Some(expert) = tracks.get(&format!("Expert{instrument}"))
        {
            let expert_ticks: BTreeSet<_> = expert
                .iter()
                .filter(|note| is_playable(note.lane))
                .map(|note| note.tick)
                .collect();
            let attack_ticks: BTreeSet<_> = gems.iter().map(|note| note.tick).collect();
            for tick in attack_ticks.difference(&expert_ticks) {
                add(Severity::Review, RuleCode::LowerTierAttackAbsentFromExpert, track, *tick,
                    "Lower-tier attack is absent at the exact Expert tick; verify this is a deliberate reduction choice".into());
            }
        }
    }
    report.findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.track.cmp(&b.track))
            .then(a.tick.cmp(&b.tick))
            .then(a.code.cmp(&b.code))
            .then(a.reason.cmp(&b.reason))
    });
    report
}
