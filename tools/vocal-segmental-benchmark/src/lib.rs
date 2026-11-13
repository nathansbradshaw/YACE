//! Continuous-pitch segmentation for offline research, independent of references.
//! PELT pruning uses the superadditivity of within-segment squared error.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub start: f64,
    pub end: f64,
    pub pitch: Option<f64>,
    pub rms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub continuous_pitch: f64,
    pub midi: u8,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Boundary penalty in squared-semitone seconds; frame density independent.
    pub penalty: f64,
    /// Split a same-pitch note at an isolated envelope valley of this depth.
    /// None disables this independently testable articulation heuristic.
    pub articulation_db: Option<f64>,
    /// Bound worst-case work instead of allowing pathological input to hang.
    pub max_cost_evaluations: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            penalty: 0.02,
            articulation_db: None,
            max_cost_evaluations: 30_000_000,
        }
    }
}

#[derive(Default, Serialize)]
pub struct Decode {
    pub notes: Vec<Note>,
    pub cost_evaluations: usize,
    pub accepted_seconds: f64,
}

pub fn segment(samples: &[Sample], config: Config) -> Result<Decode> {
    ensure!(
        config.penalty.is_finite() && config.penalty > 0.0,
        "Penalty must be positive"
    );
    ensure!(
        config
            .articulation_db
            .is_none_or(|v| v.is_finite() && v > 0.0),
        "Invalid articulation threshold"
    );
    ensure!(
        samples.len() <= 400_000 && config.max_cost_evaluations > 0,
        "Input/work budget exceeded"
    );
    ensure!(
        samples.iter().all(|s| s.start.is_finite()
            && s.end.is_finite()
            && s.start >= 0.0
            && s.end > s.start
            && s.rms.is_finite()
            && s.rms >= 0.0
            && s.pitch
                .is_none_or(|p| p.is_finite() && (0.0..=127.0).contains(&p))),
        "Invalid sample"
    );
    ensure!(
        samples.windows(2).all(|s| s[0].end <= s[1].start + 1e-9),
        "Overlapping sample cells"
    );
    let mut result = Decode::default();
    let mut i = 0;
    while i < samples.len() {
        if samples[i].pitch.is_none() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < samples.len()
            && samples[i].pitch.is_some()
            && (samples[i - 1].end - samples[i].start).abs() < 1e-8
        {
            i += 1;
        }
        let run = &samples[start..i];
        result.accepted_seconds += run.iter().map(|s| s.end - s.start).sum::<f64>();
        let segments = fit_run(run, config, &mut result.cost_evaluations)?;
        for (a, b) in segments {
            let mut boundaries = vec![a];
            if let Some(db) = config.articulation_db {
                boundaries.extend(articulations(&run[a..b], db).into_iter().map(|v| v + a));
            }
            boundaries.push(b);
            for (part, pair) in boundaries.windows(2).enumerate() {
                let chunk = &run[pair[0]..pair[1]];
                let weight = chunk.iter().map(|s| s.end - s.start).sum::<f64>();
                let pitch = chunk
                    .iter()
                    .map(|s| s.pitch.unwrap() * (s.end - s.start))
                    .sum::<f64>()
                    / weight;
                let note = Note {
                    start: chunk[0].start,
                    end: chunk.last().unwrap().end,
                    continuous_pitch: pitch,
                    midi: pitch.round().clamp(0.0, 127.0) as u8,
                };
                if let Some(last) = result.notes.last_mut().filter(|last| {
                    part == 0 && last.midi == note.midi && (last.end - note.start).abs() < 1e-8
                }) {
                    let before = last.end - last.start;
                    let after = note.end - note.start;
                    last.continuous_pitch = (last.continuous_pitch * before
                        + note.continuous_pitch * after)
                        / (before + after);
                    last.end = note.end;
                } else {
                    result.notes.push(note);
                }
            }
        }
    }
    Ok(result)
}

/// Exact penalized least-squares segmentation over each voiced run. Unlike an
/// integer frame path, segment centers remain continuous until final emission.
fn fit_run(
    samples: &[Sample],
    config: Config,
    evaluations: &mut usize,
) -> Result<Vec<(usize, usize)>> {
    let n = samples.len();
    let origin = samples[0].pitch.unwrap();
    let mut w = vec![0.0; n + 1];
    let mut x = vec![0.0; n + 1];
    let mut xx = vec![0.0; n + 1];
    for (i, s) in samples.iter().enumerate() {
        let d = s.end - s.start;
        let p = s.pitch.unwrap() - origin;
        w[i + 1] = w[i] + d;
        x[i + 1] = x[i] + d * p;
        xx[i + 1] = xx[i] + d * p * p;
    }
    let cost = |a: usize, b: usize| {
        let total = w[b] - w[a];
        let sum = x[b] - x[a];
        ((xx[b] - xx[a]) - sum * sum / total).max(0.0)
    };
    let mut dp = vec![f64::INFINITY; n + 1];
    let mut back = vec![0; n + 1];
    dp[0] = -config.penalty;
    let mut candidates = vec![0];
    for b in 1..=n {
        let mut costs = Vec::with_capacity(candidates.len());
        for &a in &candidates {
            *evaluations += 1;
            ensure!(
                *evaluations <= config.max_cost_evaluations,
                "Segmentation work budget exceeded"
            );
            let c = dp[a] + cost(a, b) + config.penalty;
            costs.push(c);
            if c < dp[b] {
                dp[b] = c;
                back[b] = a;
            }
        }
        // If a path through b is no worse even after one extra boundary, a
        // predecessor cannot win at any later time: SSE(a,c)>=SSE(a,b)+SSE(b,c).
        let bound = dp[b] + config.penalty;
        candidates = candidates
            .into_iter()
            .zip(costs)
            .filter_map(|(a, c)| (c < bound).then_some(a))
            .collect();
        candidates.push(b);
    }
    let mut result = vec![];
    let mut b = n;
    while b > 0 {
        let a = back[b];
        result.push((a, b));
        b = a;
    }
    result.reverse();
    Ok(result)
}

fn articulations(samples: &[Sample], threshold: f64) -> Vec<usize> {
    if samples.len() < 15 {
        return vec![];
    }
    let level: Vec<f64> = samples
        .iter()
        .map(|s| 20.0 * s.rms.max(1e-8).log10())
        .collect();
    let mut prefix = vec![0.0; level.len() + 1];
    for (i, &v) in level.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
    let mut candidates = vec![];
    for i in 1..samples.len() - 1 {
        let time = samples[i].start;
        if time - samples[0].start < 0.08 || samples.last().unwrap().end - time < 0.08 {
            continue;
        }
        let average = |lo: f64, hi: f64| {
            let a = samples.partition_point(|s| s.start < time + lo);
            let b = samples.partition_point(|s| s.start < time + hi);
            (b > a).then(|| (prefix[b] - prefix[a]) / (b - a) as f64)
        };
        let Some(left) = average(-0.08, -0.025) else {
            continue;
        };
        let Some(right) = average(0.025, 0.08) else {
            continue;
        };
        let Some(valley) = average(-0.012, 0.012) else {
            continue;
        };
        let depth = left.min(right) - valley;
        if depth >= threshold {
            candidates.push((i, depth));
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut accepted: Vec<usize> = vec![];
    for (i, _) in candidates {
        if accepted
            .iter()
            .all(|&j| (samples[i].start - samples[j].start).abs() >= 0.10)
        {
            accepted.push(i);
        }
    }
    accepted.sort_unstable();
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    fn samples(p: &[Option<f64>], dt: f64) -> Vec<Sample> {
        p.iter()
            .enumerate()
            .map(|(i, &pitch)| Sample {
                start: i as f64 * dt,
                end: (i + 1) as f64 * dt,
                pitch,
                rms: 0.1,
            })
            .collect()
    }
    #[test]
    fn pruned_solution_matches_exhaustive_dynamic_program() {
        let mut random = 17u64;
        for n in 2..70 {
            for penalty in [0.0001, 0.01, 0.1, 1.0] {
                let pitches: Vec<_> = (0..n)
                    .map(|_| {
                        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                        Some(60.0 + ((random >> 32) % 80) as f64 / 10.0)
                    })
                    .collect();
                let s = samples(&pitches, 0.01);
                let result = fit_run(
                    &s,
                    Config {
                        penalty,
                        ..Default::default()
                    },
                    &mut 0,
                )
                .unwrap();
                let mut dp = vec![f64::INFINITY; n + 1];
                dp[0] = -penalty;
                for b in 1..=n {
                    for a in 0..b {
                        let mean =
                            s[a..b].iter().map(|s| s.pitch.unwrap()).sum::<f64>() / (b - a) as f64;
                        let c = s[a..b]
                            .iter()
                            .map(|s| (s.pitch.unwrap() - mean).powi(2) * 0.01)
                            .sum::<f64>();
                        dp[b] = dp[b].min(dp[a] + c + penalty);
                    }
                }
                let actual = result
                    .iter()
                    .map(|&(a, b)| {
                        let mean =
                            s[a..b].iter().map(|s| s.pitch.unwrap()).sum::<f64>() / (b - a) as f64;
                        s[a..b]
                            .iter()
                            .map(|s| (s.pitch.unwrap() - mean).powi(2) * 0.01)
                            .sum::<f64>()
                    })
                    .sum::<f64>()
                    + penalty * (result.len() - 1) as f64;
                assert!((actual - dp[n]).abs() < 1e-8, "n={n}, penalty={penalty}");
            }
        }
    }
    #[test]
    fn keeps_short_semitone_and_never_fills_rest() {
        let p = [
            vec![Some(60.0); 30],
            vec![Some(61.0); 4],
            vec![Some(60.0); 30],
            vec![None; 2],
            vec![Some(72.0); 20],
        ]
        .concat();
        let s = samples(&p, 0.01);
        let d = segment(
            &s,
            Config {
                penalty: 0.01,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            d.notes.iter().map(|n| n.midi).collect::<Vec<_>>(),
            [60, 61, 60, 72]
        );
        assert!((d.notes[1].end - d.notes[1].start - 0.04).abs() < 1e-8);
        assert!(
            (d.notes.iter().map(|n| n.end - n.start).sum::<f64>() - d.accepted_seconds).abs()
                < 1e-8
        );
    }
    #[test]
    fn frame_density_does_not_change_note_solution() {
        let a = [vec![Some(60.2); 20], vec![Some(63.2); 30]].concat();
        let b: Vec<_> = a.iter().flat_map(|&p| [p, p]).collect();
        let a = segment(&samples(&a, 0.01), Config::default()).unwrap();
        let b = segment(&samples(&b, 0.005), Config::default()).unwrap();
        assert_eq!(a.notes.len(), b.notes.len());
        for (a, b) in a.notes.iter().zip(b.notes) {
            assert!((a.start - b.start).abs() < 1e-8);
            assert!((a.end - b.end).abs() < 1e-8);
            assert_eq!(a.midi, b.midi);
        }
    }
    #[test]
    fn sub_semitone_motion_does_not_create_repeated_midi_attacks() {
        let p: Vec<_> = (0..400)
            .map(|i| Some(60.0 + 0.35 * (i as f64 * 0.3).sin()))
            .collect();
        let d = segment(
            &samples(&p, 0.01),
            Config {
                penalty: 0.005,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(d.notes.len(), 1);
        assert_eq!(d.notes[0].midi, 60);
    }
    #[test]
    fn envelope_rearticulation_splits_without_deleting_pitch() {
        let mut s = samples(&vec![Some(60.0); 100], 0.01);
        for cell in &mut s[47..53] {
            cell.rms = 0.01;
        }
        let d = segment(
            &s,
            Config {
                articulation_db: Some(3.0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(d.notes.len(), 2);
        assert!((d.notes[0].end - d.notes[1].start).abs() < 1e-8);
        assert!((d.notes.iter().map(|n| n.end - n.start).sum::<f64>() - 1.0).abs() < 1e-8);
    }
    #[test]
    fn constant_pitch_pruning_is_linear_and_budget_is_enforced() {
        let s = samples(&vec![Some(60.0); 10_000], 0.01);
        let d = segment(&s, Config::default()).unwrap();
        assert_eq!(d.notes.len(), 1);
        assert!(d.cost_evaluations < 30_000);
        assert!(
            segment(
                &s,
                Config {
                    max_cost_evaluations: 1,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_overlapping_and_nonfinite_evidence() {
        let mut s = samples(&[Some(60.0); 2], 0.01);
        s[1].start = 0.005;
        assert!(segment(&s, Config::default()).is_err());
        s[1].start = 0.01;
        s[0].pitch = Some(f64::NAN);
        assert!(segment(&s, Config::default()).is_err());
    }
}
