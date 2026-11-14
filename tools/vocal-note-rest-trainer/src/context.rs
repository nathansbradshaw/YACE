//! Opt-in model-v3 features. Reuse the immutable 336-column cache; no labels.
use super::{Data, Result};
use anyhow::{Context, ensure};
pub(super) const WIDTH: usize = 418;
pub(super) fn enhance(mut d: Data) -> Result<Data> {
    let raw = d.g.rmvpe_pitch.as_ref().context("Original RMVPE")?;
    ensure!(
        !d.x.is_empty()
            && d.x.len() == raw.frames.len()
            && d.x
                .iter()
                .all(|r| r.len() == 336 && r.iter().all(|v| v.is_finite()))
            && raw.frames.iter().all(|f| f.rms.is_finite() && f.rms >= 0.),
        "Relative feature clock/schema"
    );
    let mut energy = raw.frames.iter().map(|f| f.rms).collect::<Vec<_>>();
    energy.sort_by(f64::total_cmp);
    let q95 = energy[energy.len() * 95 / 100].max(1e-9);
    for (i, row) in d.x.iter_mut().enumerate() {
        let rms = raw.frames[i].rms.max(1e-9);
        let mut extras = vec![(20. * (rms / q95).log10() / 60.).clamp(-1., 0.2) as f32];
        for radius in [25usize, 100] {
            let peak = raw.frames[i.saturating_sub(radius)..(i + radius + 1).min(raw.frames.len())]
                .iter()
                .map(|f| f.rms)
                .fold(1e-9, f64::max);
            extras.push((20. * (rms / peak).log10() / 60.).clamp(-1., 0.2) as f32);
        }
        let max = row[..128]
            .iter()
            .map(|v| *v as f64)
            .fold(f64::NEG_INFINITY, f64::max);
        let sum = row[..128]
            .iter()
            .map(|v| (*v as f64 - max).exp())
            .sum::<f64>();
        let entropy = -row[..128]
            .iter()
            .map(|v| {
                let p = (*v as f64 - max).exp() / sum;
                p * p.max(1e-30).ln()
            })
            .sum::<f64>()
            / 128f64.ln();
        let flatness =
            (row[..128].iter().map(|v| *v as f64).sum::<f64>() / 128. - max - (sum / 128.).ln())
                .exp();
        let flux = (row[128..256].iter().map(|v| v.max(0.) as f64).sum::<f64>() / 128. / 10.)
            .clamp(0., 1.);
        extras.extend([entropy as f32, flatness as f32, flux as f32]);
        ensure!(
            extras.iter().all(|v| v.is_finite()),
            "Nonfinite relative acoustic features"
        );
        row.extend(extras);
    }
    let compact =
        d.x.iter()
            .map(|r| {
                let mut v = (0..16)
                    .map(|b| r[b * 8..b * 8 + 8].iter().sum::<f32>() / 8.)
                    .collect::<Vec<_>>();
                v.extend([r[320], r[321], r[336]]);
                v
            })
            .collect::<Vec<_>>();
    for (i, row) in d.x.iter_mut().enumerate() {
        for delta in [-150isize, -50, 50, 150] {
            let center = (i as isize + delta).clamp(0, compact.len() as isize - 1) as usize;
            let lo = center.saturating_sub(12);
            let hi = (center + 13).min(compact.len());
            for k in 0..19 {
                row.push(compact[lo..hi].iter().map(|v| v[k]).sum::<f32>() / (hi - lo) as f32);
            }
        }
    }
    ensure!(
        d.x.iter()
            .all(|r| r.len() == WIDTH && r.iter().all(|v| v.is_finite())),
        "Invalid extended feature values"
    );
    d.meta.cols = WIDTH;
    d.meta
        .frontend
        .push_str("::GAIN_RELATIVE_V1_WIDE_CONTEXT_250MS_WINDOWS_AT_PLUS_MINUS_500_1500MS_V1");
    Ok(d)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Data {
        let mut d = crate::tests::data();
        d.x = vec![vec![0.; 336]; d.meta.rows];
        d
    }
    #[test]
    fn leaves_measurements_and_base_cache_values_intact() {
        let d = fixture();
        let raw = serde_json::to_value(&d.g.rmvpe_pitch).unwrap();
        let got = enhance(d).unwrap();
        assert_eq!(got.meta.cols, WIDTH);
        assert!(got.x.iter().all(|r| r[..336] == [0.; 336]));
        assert_eq!(serde_json::to_value(&got.g.rmvpe_pitch).unwrap(), raw);
    }
    #[test]
    fn source_gain_does_not_change_relative_energy_features() {
        let a = enhance(fixture()).unwrap();
        let mut d = fixture();
        for f in &mut d.g.rmvpe_pitch.as_mut().unwrap().frames {
            f.rms *= 0.01;
        }
        let b = enhance(d).unwrap();
        for (a, b) in a.x.iter().zip(b.x) {
            assert_eq!(&a[336..339], &b[336..339]);
            assert!(b.iter().all(|v| v.is_finite()));
        }
    }
    #[test]
    fn rejects_bad_clock_and_nonfinite_or_empty_inputs() {
        let mut d = fixture();
        d.x.pop();
        assert!(enhance(d).is_err());
        let mut d = fixture();
        d.x[0][0] = f32::NAN;
        assert!(enhance(d).is_err());
        let mut d = fixture();
        d.x.clear();
        d.g.rmvpe_pitch.as_mut().unwrap().frames.clear();
        assert!(enhance(d).is_err());
    }
    #[test]
    fn wide_windows_use_only_their_source() {
        let mut d = fixture();
        for row in &mut d.x {
            row[..128].fill(7.);
        }
        let got = enhance(d).unwrap();
        for row in got.x {
            for block in row[342..].chunks_exact(19) {
                assert!(block[..16].iter().all(|v| *v == 7.));
            }
        }
    }
}
