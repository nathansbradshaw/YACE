// SPDX-License-Identifier: LGPL-2.1-or-later
// Native-compatible float resampling, adapted from FFmpeg n8.1.2
// libswresample/resample.c and resample_template.c.
// Copyright (c) 2004-2012 Michael Niedermayer.
// Rust adaptation: Kaiser phase tables, source-clock bounds and channel matrix.
// See licenses/FFmpeg-LGPL-2.1.txt and THIRD_PARTY.md.
use anyhow::{Result, ensure};

fn i0(x: f64) -> f64 {
    let mut term = 1.;
    let mut sum = 1.;
    let quarter = x * x / 4.;
    for k in 1..64 {
        term *= quarter / (k * k) as f64;
        sum += term;
        if term <= sum * f64::EPSILON {
            break;
        }
    }
    sum
}
fn reflected(mut index: i64, frames: usize) -> usize {
    if frames == 1 {
        return 0;
    }
    while index < 0 || index >= frames as i64 {
        index = if index < 0 {
            -index
        } else {
            2 * frames as i64 - index - 1
        };
    }
    index as usize
}
pub fn convert(
    stereo: &[f32],
    input_rate: u32,
    input_channels: usize,
    rate: u32,
    channels: u32,
) -> Result<Vec<f32>> {
    ensure!(
        input_rate > 0
            && rate > 0
            && [1, 2].contains(&channels)
            && !stereo.is_empty()
            && stereo.len().is_multiple_of(2),
        "Invalid resampling format"
    );
    let frames = stereo.len() / 2;
    let input_channels = input_channels.clamp(1, 2);
    let matrix_after =
        (channels as f64 / input_channels as f64) < (rate as f64 / input_rate as f64);
    let gain = std::f32::consts::FRAC_1_SQRT_2;
    let prepared = if !matrix_after && input_channels == 2 && channels == 1 {
        Some(
            stereo
                .chunks_exact(2)
                .map(|s| s[0] * gain + s[1] * gain)
                .collect::<Vec<_>>(),
        )
    } else if !matrix_after && input_channels == 1 && channels == 2 {
        Some(stereo.chunks_exact(2).map(|s| s[0] * gain).collect())
    } else {
        None
    };
    let work_channels = if prepared.is_some() {
        1
    } else {
        input_channels
    };
    let mut a = rate;
    let mut b = input_rate;
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let phases = (rate / a).min(1024) as usize;
    let factor = (rate as f64 * 0.97 / input_rate as f64).min(1.);
    let taps = (32. / factor).ceil() as usize;
    let taps = if taps > 1 {
        taps.next_multiple_of(2)
    } else {
        1
    };
    ensure!(taps <= 131072, "Unsupported resampling ratio");
    let center = (taps - 1) / 2;
    let half = if phases.is_multiple_of(2) {
        phases / 2 + 1
    } else {
        phases
    };
    let mut filters = vec![Vec::<f32>::new(); phases + 1];
    let mut norm = 0.;
    for phase in 0..half {
        let fraction = phase as f64 / phases as f64;
        let mut values = Vec::with_capacity(taps);
        let mut numerator =
            (std::f64::consts::PI * fraction).sin() * if center % 2 == 1 { 1. } else { -1. };
        for tap in 0..taps {
            let distance = tap as f64 - center as f64 - fraction;
            let x = std::f64::consts::PI * distance * factor;
            let sinc = if x == 0. {
                1.
            } else if factor == 1. {
                numerator / x
            } else {
                x.sin() / x
            };
            let window = 2. * distance / taps as f64;
            values.push(sinc * i0(9. * (1. - window * window).max(0.).sqrt()));
            numerator = -numerator;
        }
        if phase == 0 {
            norm = values.iter().sum();
        }
        filters[phase] = values.into_iter().map(|v| (v / norm) as f32).collect();
        if phases.is_multiple_of(2) && phase > 0 {
            filters[phases - phase] = filters[phase].iter().rev().copied().collect();
        }
    }
    filters[phases] = vec![0.; taps];
    filters[phases][0] = if taps.is_multiple_of(8) {
        filters[0][taps - 1]
    } else {
        0.
    };
    for tap in 1..taps {
        filters[phases][tap] = filters[0][tap - 1];
    }
    // SWR flushes a reflection of the remaining input buffer. The tail can
    // produce one fewer frame than a simple ceil(rate ratio), depending on
    // the final phase; match that clock rather than inventing another sample.
    let reflection = if frames <= taps {
        frames.div_ceil(2)
    } else {
        let before_flush = ((frames - taps / 2) as u64 * rate as u64).div_ceil(input_rate as u64);
        let consumed = (before_flush * input_rate as u64 / rate as u64) as usize - center;
        (frames - consumed).min(taps).div_ceil(2)
    };
    let count = if frames + reflection < taps + 1 {
        0
    } else {
        let available = (frames + reflection + center + 1).saturating_sub(taps);
        (available as u64 * rate as u64).div_ceil(input_rate as u64) as usize
    };
    let mut output = Vec::with_capacity(count * channels as usize);
    for frame in 0..count {
        let clock = frame as u64 * input_rate as u64;
        let source = (clock / rate as u64) as i64 - center as i64;
        let phase_clock = (clock % rate as u64) * phases as u64;
        let phase = (phase_clock / rate as u64) as usize;
        let alpha = (phase_clock % rate as u64) as f32 / rate as f32;
        let filtered = |filter: &[f32], channel: usize| {
            let mut even = 0f32;
            let mut odd = 0f32;
            for (tap, &coefficient) in filter.iter().enumerate() {
                let index = reflected(source + tap as i64, frames);
                let sample = if let Some(prepared) = &prepared {
                    prepared[index]
                } else {
                    stereo[index * 2 + channel]
                };
                if tap % 2 == 0 {
                    even += sample * coefficient;
                } else {
                    odd += sample * coefficient;
                }
            }
            even + odd
        };
        let mut values = [0f32; 2];
        for (channel, value) in values.iter_mut().enumerate().take(work_channels) {
            let first = filtered(&filters[phase], channel);
            *value = if alpha == 0. {
                first
            } else {
                first * (1. - alpha) + filtered(&filters[phase + 1], channel) * alpha
            };
        }
        if channels == 1 {
            output.push(if work_channels == 2 {
                values[0] * gain + values[1] * gain
            } else {
                values[0]
            });
        } else if work_channels == 1 {
            let value = values[0]
                * if matrix_after && input_channels == 1 {
                    gain
                } else {
                    1.
                };
            output.extend([value, value]);
        } else {
            output.extend(values);
        }
    }
    Ok(output)
}
