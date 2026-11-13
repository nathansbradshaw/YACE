//! Full-run squared-error segmentation. Variable attack rewards use the same
//! superadditive segment-cost bound as PELT; ties prefer fewer notes.
use anyhow::{Result, ensure};
pub struct Fit {
    pub spans: Vec<(usize, usize)>,
    pub cost: f64,
    pub evaluations: usize,
}
pub fn fit(pitch: &[f64], boundary: &[f64], budget: usize) -> Result<Fit> {
    ensure!(
        pitch.len() == boundary.len() && pitch.len() <= 400_000 && budget > 0,
        "Invalid segmentation shape or work budget"
    );
    ensure!(
        pitch
            .iter()
            .all(|p| p.is_finite() && (0. ..=127.).contains(p))
            && boundary
                .iter()
                .all(|p| p.is_finite() && (-4. ..=4.).contains(p)),
        "Invalid segmentation evidence"
    );
    let n = pitch.len();
    if n == 0 {
        return Ok(Fit {
            spans: vec![],
            cost: 0.,
            evaluations: 0,
        });
    }
    let mut sums = vec![0.; n + 1];
    let mut squares = sums.clone();
    for (i, p) in pitch.iter().enumerate() {
        let p = p - pitch[0];
        sums[i + 1] = sums[i] + p;
        squares[i + 1] = squares[i] + p * p;
    }
    let segment = |a: usize, b: usize| {
        ((squares[b] - squares[a]) - (sums[b] - sums[a]).powi(2) / (b - a) as f64).max(0.) * 0.01
    };
    let penalty = |a: usize| if a == 0 { 0. } else { boundary[a] };
    let mut costs = vec![f64::INFINITY; n + 1];
    let mut counts = vec![usize::MAX; n + 1];
    let mut previous = vec![0; n + 1];
    costs[0] = 0.;
    counts[0] = 0;
    let mut candidates = vec![0];
    let mut evaluations = 0;
    for end in 1..=n {
        let mut inspected = Vec::with_capacity(candidates.len());
        for &start in &candidates {
            evaluations += 1;
            ensure!(
                evaluations <= budget,
                "Segmentation work budget exceeded; source notes were not truncated"
            );
            let cost = costs[start] + segment(start, end) + penalty(start);
            let count = counts[start] + 1;
            if cost < costs[end] - 1e-12
                || ((cost - costs[end]).abs() <= 1e-12 && count < counts[end])
            {
                costs[end] = cost;
                counts[end] = count;
                previous[end] = start;
            }
            inspected.push((start, cost));
        }
        if end < n {
            let bound = costs[end] + penalty(end);
            candidates = inspected
                .into_iter()
                .filter(|(start, cost)| {
                    *cost < bound - 1e-12
                        || ((*cost - bound).abs() <= 1e-12 && counts[*start] < counts[end])
                })
                .map(|(start, _)| start)
                .collect();
            candidates.push(end);
        }
    }
    let mut spans = vec![];
    let mut end = n;
    while end > 0 {
        let start = previous[end];
        ensure!(start < end, "Non-progressing segmentation");
        spans.push((start, end));
        end = start;
    }
    spans.reverse();
    Ok(Fit {
        spans,
        cost: costs[n],
        evaluations,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn exhaustive(p: &[f64], k: &[f64]) -> f64 {
        let mut d = vec![f64::INFINITY; p.len() + 1];
        d[0] = 0.;
        for end in 1..=p.len() {
            for start in 0..end {
                let x = &p[start..end];
                let mean = x.iter().sum::<f64>() / x.len() as f64;
                let cost = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() * 0.01;
                d[end] = d[end].min(d[start] + cost + if start == 0 { 0. } else { k[start] });
            }
        }
        d[p.len()]
    }
    #[test]
    fn matches_exhaustive_with_positive_zero_and_negative_attack_costs() {
        let mut seed = 43u64;
        for n in 1..=20 {
            for _ in 0..128 {
                let mut p = vec![];
                let mut k = vec![];
                for _ in 0..n {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    p.push(60. + ((seed >> 32) % 11) as f64 * 0.17);
                    k.push([-0.16, 0., 0.08, 0.16][(seed as usize >> 8) % 4]);
                }
                let got = fit(&p, &k, 100_000).unwrap();
                assert!((got.cost - exhaustive(&p, &k)).abs() < 1e-8, "n={n}");
                assert_eq!(got.spans.first().unwrap().0, 0);
                assert_eq!(got.spans.last().unwrap().1, n);
                assert!(got.spans.windows(2).all(|w| w[0].1 == w[1].0));
            }
        }
    }
    #[test]
    fn long_flat_notes_have_no_forced_three_second_boundary() {
        let p = vec![69.; 10_000];
        let got = fit(&p, &vec![0.16; p.len()], 30_000).unwrap();
        assert_eq!(got.spans, vec![(0, p.len())]);
        assert!(got.evaluations < 30_000);
        let got = fit(&p, &vec![0.; p.len()], 30_000).unwrap();
        assert_eq!(got.spans, vec![(0, p.len())]);
    }
    #[test]
    fn strong_attack_splits_equal_pitches() {
        let got = fit(
            &[69.; 20],
            &(0..20)
                .map(|i| if i == 10 { -0.16 } else { 0.16 })
                .collect::<Vec<_>>(),
            1000,
        )
        .unwrap();
        assert_eq!(got.spans, vec![(0, 10), (10, 20)]);
    }
    #[test]
    fn invalid_data_and_exhausted_budget_fail_explicitly() {
        assert!(fit(&[60.; 20], &[0.16; 20], 1).is_err());
        assert!(fit(&[f64::NAN], &[0.16], 100).is_err());
        assert!(fit(&[60.], &[], 100).is_err());
    }
}
