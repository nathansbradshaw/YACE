//! Model-agnostic review markers shared by report, future editor and AI adapters.
//! Scores are evidence, never calibrated probabilities or musical acceptance.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    WeakAttackSupport,
    PitchDisagreement,
    LowCandidateScore,
    FastRepeat,
    HandCrowding,
    CoarseGridDeviation,
    SustainOverlap,
    ReductionDifference,
    UnchartedStemAttack,
    StemAttackTiming,
    PianoSourceReview,
    ProKeysRangeCoverage,
    ProKeysArrangement,
    RecoveredShortNote,
    RecoveredPolyphonicGuitar,
    RecoveredPercussionAttack,
    MelodySourceReview,
    DrumArticulation,
}

impl Reason {
    pub const ALL: [Self; 18] = [
        Self::WeakAttackSupport,
        Self::PitchDisagreement,
        Self::LowCandidateScore,
        Self::FastRepeat,
        Self::HandCrowding,
        Self::CoarseGridDeviation,
        Self::SustainOverlap,
        Self::ReductionDifference,
        Self::UnchartedStemAttack,
        Self::StemAttackTiming,
        Self::PianoSourceReview,
        Self::ProKeysRangeCoverage,
        Self::ProKeysArrangement,
        Self::RecoveredShortNote,
        Self::RecoveredPolyphonicGuitar,
        Self::RecoveredPercussionAttack,
        Self::MelodySourceReview,
        Self::DrumArticulation,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::WeakAttackSupport => "Weak attack support",
            Self::PitchDisagreement => "Pitch disagreement",
            Self::LowCandidateScore => "Low candidate score",
            Self::FastRepeat => "Fast repeat",
            Self::HandCrowding => "Hand crowding",
            Self::CoarseGridDeviation => "Timing to inspect",
            Self::SustainOverlap => "Sustain retrigger",
            Self::ReductionDifference => "Difficulty relationship",
            Self::UnchartedStemAttack => "Uncharted stem attack",
            Self::StemAttackTiming => "Stem attack timing",
            Self::PianoSourceReview => "Piano source to review",
            Self::ProKeysRangeCoverage => "Pro Keys display range",
            Self::ProKeysArrangement => "Octave / release arrangement",
            Self::RecoveredShortNote => "Recovered short note",
            Self::RecoveredPolyphonicGuitar => "Additional guitar candidate",
            Self::RecoveredPercussionAttack => "Recovered percussion attack",
            Self::MelodySourceReview => "Melody source to review",
            Self::DrumArticulation => "Drum articulation",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::DrumArticulation => {
                "Review source-bound drum-class evidence against the isolated stem. An optional detailed model can propose ride/crash for an existing cymbal hit; ambiguous bell/cowbell votes and unvalidated tom-register proposals retain the generic lane. Model scores and relative resonances do not prove physical kit identity"
            }
            Self::ProKeysArrangement => {
                "Review octave-equivalent pitch placement, merged simultaneous unisons or shortened overlapping releases used to fit the game keyboard. Original detections remain in the arrangement evidence"
            }
            Self::PianoSourceReview => {
                "Confirm piano/keyboard is present in the source; inspect separation bleed, chord pitches, independent releases and controller reach before accepting this draft"
            }
            Self::ProKeysRangeCoverage => {
                "Simultaneously sounding pitches cannot fit a single YARG keyboard window. All pitches are retained; review chord voices, held-note releases and range transitions before game play"
            }
            Self::WeakAttackSupport => {
                "No nearby isolated-instrument attack supports this candidate within the measured tolerance; listen before changing it because soft attacks may evade detection"
            }
            Self::PitchDisagreement => {
                "Bass pitch evidence disagrees or crosses a possible transition; check note ownership, harmonics and stem leakage before changing notes"
            }
            Self::LowCandidateScore => {
                "Nearby generated candidate has a low source score; check instrument identity and note choice"
            }
            Self::FastRepeat => {
                "Same lane repeats within 50 ms; check whether the rapid figure is intentional and playable"
            }
            Self::HandCrowding => {
                "More than two hand hits within 20 ms; check kit assignment and physical playability"
            }
            Self::CoarseGridDeviation => {
                "Attack is over 35 ms from the coarse straight/triplet grid; check expressive timing before quantizing"
            }
            Self::SustainOverlap => {
                "A new attack retriggers a lane while its previous sustain is held; check the intended release and fingering"
            }
            Self::ReductionDifference => {
                "A lower difficulty adds an attack absent from Expert; check that it preserves the musical rhythm"
            }
            Self::UnchartedStemAttack => {
                "Strong isolated-stem attack has no nearby Expert attack; inspect bleed, sustain coverage and arrangement before considering a note"
            }
            Self::StemAttackTiming => {
                "Stem evidence disagrees with the original attack timing; listen to the isolated instrument and inspect the measured boundary or flux residual before approving"
            }
            Self::RecoveredShortNote => {
                "A short melodic attack was recovered using network onset, isolated-stem attack and acoustic pitch evidence; listen before approving its timing, instrument identity and playability"
            }
            Self::RecoveredPolyphonicGuitar => {
                "A short network hypothesis contributes measured harmonic energy near a guitar-stem attack. Evidence scores are uncalibrated; listen before approving the pitch, timing, source identity and arrangement"
            }
            Self::RecoveredPercussionAttack => {
                "A strong percussion model peak below the fixed contrast threshold has nearby source-attack support. Review instrument identity, timing and playability; percussion bleed can still produce false additions"
            }
            Self::MelodySourceReview => {
                "This five-fret Guitar part follows an explicitly selected other-instrument or piano stem. Review the intended melody, mixed voices, separation bleed and note choices; the chart does not establish a literal guitar performance"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Attention,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    /// Recording seconds, including chart offset/tempo conversion when applicable.
    pub start: f64,
    pub end: f64,
    pub track: String,
    pub reason: Reason,
    pub priority: Priority,
    /// Named measurements with units in their keys. Absent evidence stays absent.
    pub evidence: BTreeMap<String, f64>,
}

impl Issue {
    pub fn at(time: f64, track: &str, reason: Reason, evidence: &[(&str, f64)]) -> Self {
        Self {
            start: time,
            end: time,
            track: track.into(),
            reason,
            priority: if reason == Reason::HandCrowding {
                Priority::High
            } else {
                Priority::Attention
            },
            evidence: evidence.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    /// Content-derived and bound to inputs and policy. Never reuse approval on a new ID.
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub tracks: BTreeSet<String>,
    pub priority: Priority,
    pub status: ReviewStatus,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Fixed windows prevent a chain of nearby flags from merging an entire song.
    pub window_seconds: f64,
    /// Uncalibrated heuristic; not a probability that a note is correct.
    pub low_candidate_score_below: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            window_seconds: 12.0,
            low_candidate_score_below: 0.45,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewMap {
    pub version: u32,
    pub duration_seconds: f64,
    pub policy: Policy,
    /// Hashes of chart, transcription, generation receipt and all checked audio.
    pub input_sha256: BTreeMap<String, String>,
    /// None means origin is unknown; model scores must then stay unassigned.
    pub generated_tracks: Option<BTreeSet<String>>,
    /// Checks run, including those which produced zero flags.
    pub checks_performed: BTreeSet<Reason>,
    pub musical_acceptance: String,
    pub unflagged_sections: String,
    pub coverage_limits: Vec<String>,
    pub regions: Vec<Region>,
}

impl ReviewMap {
    pub fn build(
        duration: f64,
        policy: Policy,
        input_sha256: BTreeMap<String, String>,
        generated_tracks: Option<BTreeSet<String>>,
        checks_performed: BTreeSet<Reason>,
        issues: Vec<Issue>,
    ) -> Result<Self> {
        ensure!(
            duration.is_finite() && duration > 0.0,
            "Invalid review duration"
        );
        ensure!(
            policy.window_seconds.is_finite()
                && policy.window_seconds >= 1.0
                && policy.low_candidate_score_below.is_finite()
                && (0.0..=1.0).contains(&policy.low_candidate_score_below),
            "Invalid review policy"
        );
        let context = serde_json::to_vec(&(
            1,
            &input_sha256,
            &policy,
            duration,
            &generated_tracks,
            &checks_performed,
        ))?;
        let mut windows: BTreeMap<u64, Vec<Issue>> = BTreeMap::new();
        for mut issue in issues {
            ensure!(
                issue.start.is_finite()
                    && issue.end.is_finite()
                    && issue.end >= issue.start
                    && !issue.track.is_empty()
                    && issue.evidence.values().all(|v| v.is_finite()),
                "Invalid timed review issue"
            );
            // Notes may legitimately precede the recording with a negative chart offset.
            if issue.end < 0.0 || issue.start >= duration {
                continue;
            }
            issue.start = issue.start.max(0.0);
            issue.end = issue.end.min(duration);
            // Long uncertain notes appear in every affected window, with exact issue
            // spans retained. A point on a boundary belongs to the following window.
            let first = (issue.start / policy.window_seconds).floor() as u64;
            let last = if issue.end > issue.start {
                ((issue.end / policy.window_seconds).ceil() as u64).saturating_sub(1)
            } else {
                first
            };
            for window in first..=last {
                windows.entry(window).or_default().push(issue.clone());
            }
        }
        let mut regions = Vec::new();
        for (window, mut issues) in windows {
            // Sort before hashing so provider iteration order cannot change marker IDs.
            issues.sort_by(|a, b| {
                a.start
                    .total_cmp(&b.start)
                    .then(a.end.total_cmp(&b.end))
                    .then(a.track.cmp(&b.track))
                    .then(a.reason.cmp(&b.reason))
                    .then(a.priority.cmp(&b.priority))
                    .then_with(|| {
                        serde_json::to_string(&a.evidence)
                            .unwrap()
                            .cmp(&serde_json::to_string(&b.evidence).unwrap())
                    })
            });
            issues.dedup();
            let start = window as f64 * policy.window_seconds;
            let end = (start + policy.window_seconds).min(duration);
            let mut hash = Sha256::new();
            hash.update(&context);
            hash.update(serde_json::to_vec(&(start, end, &issues))?);
            regions.push(Region {
                id: format!("review-{:x}", hash.finalize()),
                start,
                end,
                tracks: issues.iter().map(|i| i.track.clone()).collect(),
                priority: issues.iter().map(|i| i.priority).max().unwrap(),
                status: ReviewStatus::Pending,
                issues,
            });
        }
        Ok(Self {
            version: 1,
            duration_seconds: duration,
            policy,
            input_sha256,
            generated_tracks,
            checks_performed,
            musical_acceptance: "unverified".into(),
            unflagged_sections: "unverified; absence of a flag is not approval".into(),
            coverage_limits: vec![
                "Source scores (including model scores) are uncalibrated candidate scores, not probabilities of musical correctness. Candidate-to-chart association uses nearby attacks, not exact note provenance.".into(),
                "Flux measures timing support, not drum identity or completeness. YIN comparison after bass refinement measures consistency, not independent accuracy.".into(),
                "Pattern flags are review prompts, not automatic errors. These checks do not establish riff consistency, strumming feel, solo quality, missing notes or enjoyable playability.".into(),
                "Piano source/range prompts and optional vocal evidence do not measure musical correctness. Drum articulation and other instruments still require listening review.".into(),
            ],
            regions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(issues: Vec<Issue>, hash: &str) -> ReviewMap {
        ReviewMap::build(
            25.0,
            Policy::default(),
            BTreeMap::from([("chart".into(), hash.into())]),
            None,
            BTreeSet::new(),
            issues,
        )
        .unwrap()
    }

    #[test]
    fn bounded_windows_keep_exact_times_long_notes_offsets_and_priorities() {
        let mut long = Issue::at(
            11.5,
            "ExpertSingle",
            Reason::LowCandidateScore,
            &[("candidate_score", 0.3)],
        );
        long.end = 24.5;
        let map = build(
            vec![
                long,
                Issue::at(-0.1, "ExpertDrums", Reason::FastRepeat, &[]),
                Issue::at(12.0, "ExpertDrums", Reason::HandCrowding, &[]),
                Issue::at(25.0, "ExpertDrums", Reason::FastRepeat, &[]),
            ],
            "a",
        );
        assert_eq!(map.regions.len(), 3);
        assert_eq!((map.regions[2].start, map.regions[2].end), (24.0, 25.0));
        assert_eq!(map.regions[0].issues.len(), 1);
        assert_eq!(map.regions[1].priority, Priority::High);
        assert!(
            map.regions
                .iter()
                .all(|r| r.status == ReviewStatus::Pending)
        );
        assert_eq!(map.regions[2].issues[0].start, 11.5);
    }

    #[test]
    fn ids_survive_order_and_duplicates_but_not_changed_inputs() {
        let a = Issue::at(2.0, "ExpertDrums", Reason::WeakAttackSupport, &[]);
        let b = Issue::at(
            3.0,
            "ExpertDoubleBass",
            Reason::PitchDisagreement,
            &[("candidate_score", 0.99)],
        );
        let map = build(vec![a.clone(), b.clone()], "a");
        assert_eq!(
            map.regions,
            build(vec![b.clone(), a.clone(), a.clone()], "a").regions
        );
        assert_ne!(
            map.regions[0].id,
            build(vec![a, b], "changed").regions[0].id
        );
        assert_eq!(
            map.regions[0].issues.len(),
            2,
            "high model scores must not hide contradictory evidence"
        );
        let round_trip: ReviewMap =
            serde_json::from_slice(&serde_json::to_vec(&map).unwrap()).unwrap();
        assert_eq!(round_trip.regions, map.regions);
        assert_eq!(build(vec![], "a").musical_acceptance, "unverified");
    }

    #[test]
    fn invalid_evidence_and_policy_are_rejected() {
        let issue = Issue::at(f64::NAN, "ExpertDrums", Reason::FastRepeat, &[]);
        assert!(
            ReviewMap::build(
                25.0,
                Policy::default(),
                BTreeMap::new(),
                None,
                BTreeSet::new(),
                vec![issue]
            )
            .is_err()
        );
        let policy = Policy {
            window_seconds: 0.0,
            ..Default::default()
        };
        assert!(
            ReviewMap::build(25.0, policy, BTreeMap::new(), None, BTreeSet::new(), vec![]).is_err()
        );
    }
}
