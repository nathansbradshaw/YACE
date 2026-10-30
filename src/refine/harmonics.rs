#[derive(Clone, Copy, Debug)]
pub(crate) struct HarmonicFit {
    pub(crate) residual: f64,
    pub(crate) variance: f64,
    pub(crate) fundamental_power: f64,
    pub(crate) strongest_power: f64,
    pub(crate) phase: f64,
}

/// Fit the harmonics jointly rather than projecting each through a short Hann
/// window independently. Nearby sinusoidal basis functions are not orthogonal
/// over a fractional period; a bright octave can otherwise look like a weak
/// fundamental. DC and a linear trend absorb slow nonperiodic attack movement.
pub(crate) fn harmonic_fit(
    signal: &[f32],
    frequency: f64,
    fundamental: bool,
) -> Option<HarmonicFit> {
    if signal.len() < 24 || !frequency.is_finite() || frequency <= 0. {
        return None;
    }
    let first = if fundamental { 1 } else { 2 };
    let columns = 2 + (6 - first) * 4;
    let mut normal = vec![vec![0.; columns + 1]; columns];
    let mut rows = Vec::with_capacity(signal.len());
    let (mut sum, mut weight_sum) = (0., 0.);
    for (index, sample) in signal.iter().enumerate() {
        let x = index as f64 / (signal.len() - 1) as f64;
        let weight = 0.5 - 0.5 * (std::f64::consts::TAU * x).cos();
        let phase = std::f64::consts::TAU * frequency * index as f64 / 11025.;
        let mut row = vec![1., x * 2. - 1.];
        for harmonic in first..=5 {
            let sin = (phase * harmonic as f64).sin();
            let cos = (phase * harmonic as f64).cos();
            row.extend([sin, cos, (x * 2. - 1.) * sin, (x * 2. - 1.) * cos]);
        }
        for i in 0..columns {
            for j in 0..columns {
                normal[i][j] += row[i] * row[j] * weight;
            }
            normal[i][columns] += row[i] * (*sample as f64) * weight;
        }
        sum += *sample as f64 * weight;
        weight_sum += weight;
        rows.push((row, weight, *sample as f64));
    }
    let scale = normal
        .iter()
        .enumerate()
        .map(|(index, row)| row[index].abs())
        .fold(0., f64::max);
    for column in 0..columns {
        let pivot = (column..columns).max_by(|a, b| {
            normal[*a][column]
                .abs()
                .total_cmp(&normal[*b][column].abs())
        })?;
        if normal[pivot][column].abs() < scale * 1e-10 {
            return None;
        }
        normal.swap(column, pivot);
        let divisor = normal[column][column];
        for value in &mut normal[column][column..=columns] {
            *value /= divisor;
        }
        let pivot_row = normal[column].clone();
        for (i, row) in normal.iter_mut().enumerate() {
            if i == column {
                continue;
            }
            let factor = row[column];
            for (value, pivot) in row[column..=columns]
                .iter_mut()
                .zip(&pivot_row[column..=columns])
            {
                *value -= factor * pivot;
            }
        }
    }
    let beta: Vec<_> = normal.iter().map(|row| row[columns]).collect();
    if beta.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mean = sum / weight_sum.max(f64::EPSILON);
    let (mut residual, mut variance) = (0., 0.);
    for (row, weight, sample) in rows {
        let predicted = row
            .iter()
            .zip(&beta)
            .map(|(value, beta)| value * beta)
            .sum::<f64>();
        residual += (sample - predicted).powi(2) * weight;
        variance += (sample - mean).powi(2) * weight;
    }
    let powers: Vec<_> = beta[2..]
        .chunks_exact(4)
        .map(|pair| pair[0] * pair[0] + pair[1] * pair[1])
        .collect();
    Some(HarmonicFit {
        residual,
        variance,
        fundamental_power: if fundamental { powers[0] } else { 0. },
        strongest_power: powers.into_iter().fold(0., f64::max),
        phase: if fundamental {
            beta[3].atan2(beta[2])
        } else {
            0.
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_period_fit_does_not_invent_a_fundamental_from_octave_leakage() {
        for frequency in [61.735_f64, 73.416, 82.407] {
            for cycles in [1.75, 2.5, 4.] {
                for phase in 0..8 {
                    for real_fundamental in [false, true] {
                        let length = (11025. * cycles / frequency).ceil() as usize;
                        let signal: Vec<_> = (0..length)
                            .map(|index| {
                                let angle =
                                    std::f64::consts::TAU * frequency * index as f64 / 11025.;
                                let ramp = index as f64 / (length - 1) as f64;
                                (0.3 * ramp * (2. * angle + phase as f64 * 0.7).sin()
                                    + 0.1 * (3. * angle - 0.4).cos()
                                    + if real_fundamental {
                                        0.08 * (angle + 0.3).sin()
                                    } else {
                                        0.
                                    }) as f32
                            })
                            .collect();
                        let fit = harmonic_fit(&signal, frequency, true).unwrap();
                        let expected = if real_fundamental {
                            0.08f64.powi(2)
                        } else {
                            0.
                        };
                        assert!(
                            (fit.fundamental_power - expected).abs() < 1e-7,
                            "{frequency}/{cycles}/{phase}: {fit:?}"
                        );
                        assert!(fit.residual / fit.variance < 1e-10, "{fit:?}");
                    }
                }
            }
        }
    }
}
