//! Research-only note rejection. Protects supplied correct training examples;
//! this is not a guarantee for another recording. No pitch or timing is edited.
use crate::Note;
use anyhow::{Result, ensure};
use chart_engine::vocal_benchmark::Reference;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct Example {
    pub features: Vec<f64>,
    pub correct: bool,
    pub recording: usize,
}
/// Training labels only. A match at another pitch must not protect a false note.
pub fn has_reference_support(r: &Reference, n: &Note) -> bool {
    r.notes.iter().any(|r| {
        r.midi == n.midi
            && ((n.end.min(r.end_seconds) > n.start.max(r.start_seconds))
                || ((n.start - r.start_seconds).abs() <= 0.05
                    && (n.end - r.end_seconds).abs()
                        <= 0.05_f64.max(0.2 * (r.end_seconds - r.start_seconds))))
    })
}
/// Caller owns recording exclusion and feature construction. No held-out labels
/// may enter these examples. Protection applies only to supplied correct data.
pub fn fit(points: &[Example]) -> Result<Rejector> {
    ensure!(
        !points.is_empty() && points.len() <= 50000,
        "Example budget exceeded"
    );
    let f = points[0].features.len();
    ensure!(
        (1..=64).contains(&f)
            && points.iter().all(|p| p.features.len() == f
                && p.features.iter().all(|v| v.is_finite())
                && p.recording < 16)
            && points.iter().any(|p| p.correct),
        "Invalid features, recording ID, or missing protection examples"
    );
    Ok(fit_inner(points))
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Rejector {
    bounds: Vec<[f64; 2]>,
    rules: Vec<Vec<Condition>>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Condition {
    feature: usize,
    threshold: f64,
    below: bool,
}
impl Condition {
    fn matches(&self, x: &[f64]) -> bool {
        if self.below {
            x[self.feature] <= self.threshold
        } else {
            x[self.feature] > self.threshold
        }
    }
}
impl Rejector {
    pub fn keeps(&self, features: &[f64]) -> bool {
        self.predict(features) != 0.
    }
    pub fn limited(&self, count: usize) -> Self {
        let mut m = self.clone();
        m.rules.truncate(count);
        m
    }
    fn predict(&self, x: &[f64]) -> f64 {
        if x.len() != self.bounds.len() || x.iter().any(|v| !v.is_finite()) {
            return 1.;
        }
        if x.iter()
            .zip(&self.bounds)
            .any(|(&v, b)| v < b[0] - 1e-9 || v > b[1] + 1e-9)
        {
            return 1.;
        }
        if self.rules.iter().any(|rule| {
            !rule.is_empty()
                && rule.len() <= 2
                && rule.iter().all(|c| c.feature < x.len() && c.matches(x))
        }) {
            0.
        } else {
            1.
        }
    }
}
fn fit_inner(points: &[Example]) -> Rejector {
    let f = points[0].features.len();
    let mut cuts = vec![];
    let mut bounds = vec![];
    for j in 0..f {
        let mut values: Vec<_> = points.iter().map(|p| p.features[j]).collect();
        values.sort_by(f64::total_cmp);
        bounds.push([values[0], values[values.len() - 1]]);
        let mut cut: Vec<_> = (1..16).map(|i| values[values.len() * i / 16]).collect();
        cut.dedup();
        cuts.push(cut);
    }
    let words = points.len().div_ceil(64);
    let mut positive = vec![0u64; words];
    let mut negative = positive.clone();
    let mut sources = vec![positive.clone(); 16];
    for (i, p) in points.iter().enumerate() {
        let bit = 1u64 << (i % 64);
        if p.correct {
            positive[i / 64] |= bit;
        } else {
            negative[i / 64] |= bit;
            sources[p.recording][i / 64] |= bit;
        }
    }
    let mut predicates = vec![];
    for (f, cut) in cuts.iter().enumerate() {
        for &t in cut {
            for below in [true, false] {
                let c = Condition {
                    feature: f,
                    threshold: t,
                    below,
                };
                let mut bits = vec![0u64; words];
                for (i, p) in points.iter().enumerate() {
                    if c.matches(&p.features) {
                        bits[i / 64] |= 1u64 << (i % 64);
                    }
                }
                predicates.push((c, bits));
            }
        }
    }
    let mut candidates: Vec<(u32, usize, usize, Vec<u64>)> = vec![];
    for i in 0..predicates.len() {
        for j in i..predicates.len() {
            if predicates[i]
                .1
                .iter()
                .zip(&predicates[j].1)
                .zip(&positive)
                .any(|((&a, &b), &p)| a & b & p != 0)
            {
                continue;
            }
            let cover: Vec<_> = predicates[i]
                .1
                .iter()
                .zip(&predicates[j].1)
                .zip(&negative)
                .map(|((&a, &b), &n)| a & b & n)
                .collect();
            let n: u32 = cover.iter().map(|v| v.count_ones()).sum();
            if n < 20 {
                continue;
            }
            let count = sources
                .iter()
                .filter(|s| {
                    s.iter()
                        .zip(&cover)
                        .map(|(&a, &b)| (a & b).count_ones())
                        .sum::<u32>()
                        >= 5
                })
                .count();
            if count < 3 {
                continue;
            }
            candidates.push((n, i, j, cover));
            if candidates.len() >= 4096 {
                candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
                candidates.truncate(2048);
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    candidates.truncate(2048);
    let mut uncovered = negative;
    let mut rules = vec![];
    for _ in 0..8 {
        let best = candidates
            .iter()
            .enumerate()
            .map(|(k, c)| {
                (
                    k,
                    c.3.iter()
                        .zip(&uncovered)
                        .map(|(&a, &b)| (a & b).count_ones())
                        .sum::<u32>(),
                )
            })
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
        let Some((index, gain)) = best else {
            break;
        };
        if gain == 0 {
            break;
        }
        let c = &candidates[index];
        let mut rule = vec![predicates[c.1].0.clone()];
        if c.1 != c.2 {
            rule.push(predicates[c.2].0.clone());
        }
        for (u, &n) in uncovered.iter_mut().zip(&c.3) {
            *u &= !n;
        }
        rules.push(rule);
    }
    let model = Rejector { bounds, rules };
    assert!(
        points
            .iter()
            .filter(|p| p.correct)
            .all(|p| model.predict(&p.features) == 1.),
        "A fitted rejection rule touched a protected correct note"
    );
    model
}
#[cfg(test)]
mod tests {
    use super::*;
    fn examples() -> Vec<Example> {
        let mut p = vec![];
        for s in 0..3 {
            for _ in 0..8 {
                p.push(Example {
                    features: vec![0.],
                    correct: false,
                    recording: s,
                });
            }
        }
        for _ in 0..8 {
            p.push(Example {
                features: vec![1.],
                correct: true,
                recording: 3,
            });
        }
        p
    }
    #[test]
    fn supported_negatives_can_be_rejected_without_touching_protection_examples() {
        let p = examples();
        let m = fit(&p).unwrap();
        assert!(!m.keeps(&[0.]));
        assert!(p.iter().filter(|p| p.correct).all(|p| m.keeps(&p.features)));
        assert!(m.keeps(&[2.]));
        assert!(m.keeps(&[f64::NAN]));
        assert!(m.keeps(&[]));
        assert!(m.limited(0).keeps(&[0.]));
        let copy: Rejector = serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(!copy.keeps(&[0.]));
    }
    #[test]
    fn a_single_recording_or_an_ambiguous_positive_prevents_rejection() {
        let mut p = examples();
        for x in &mut p {
            x.recording = 0;
        }
        assert!(fit(&p).unwrap().keeps(&[0.]));
        let mut p = examples();
        p.push(Example {
            features: vec![0.],
            correct: true,
            recording: 4,
        });
        assert!(fit(&p).unwrap().keeps(&[0.]));
    }
    #[test]
    fn conjunction_protects_both_neighboring_correct_patterns() {
        let mut p = examples();
        for e in &mut p {
            e.features = if e.correct {
                vec![1., 0.]
            } else {
                vec![0., 0.]
            };
        }
        p.push(Example {
            features: vec![0., 1.],
            correct: true,
            recording: 4,
        });
        let m = fit(&p).unwrap();
        assert!(!m.keeps(&[0., 0.]));
        for count in 0..=8 {
            assert!(m.limited(count).keeps(&[1., 0.]));
            assert!(m.limited(count).keeps(&[0., 1.]));
        }
        assert!(m.keeps(&[0., -1.]));
    }
    #[test]
    fn malformed_training_data_is_rejected() {
        assert!(fit(&[]).is_err());
        let mut p = examples();
        p[0].features = vec![f64::NAN];
        assert!(fit(&p).is_err());
        let mut p = examples();
        p[0].recording = 16;
        assert!(fit(&p).is_err());
    }
    #[test]
    fn labels_protect_the_actual_pitch_and_short_tolerance_matches() {
        use chart_engine::vocal_benchmark::ReferenceNote;
        let r = Reference {
            reference_kind: "authored_chart".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 10.,
            provenance: "Synthetic note-validity test".into(),
            reviewed: true,
            frames: vec![],
            words: vec![],
            notes: vec![ReferenceNote {
                start_seconds: 1.,
                end_seconds: 1.01,
                midi: 60,
            }],
        };
        assert!(has_reference_support(
            &r,
            &Note {
                start: 1.,
                end: 1.01,
                midi: 60
            }
        ));
        assert!(!has_reference_support(
            &r,
            &Note {
                start: 1.,
                end: 1.01,
                midi: 72
            }
        ));
        assert!(has_reference_support(
            &r,
            &Note {
                start: 1.04,
                end: 1.05,
                midi: 60
            }
        ));
    }
}
