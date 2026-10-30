use chart_engine::guitar_attacks::{Attack, detect};
use std::f64::consts::TAU;

fn noise(index: usize, seed: u64) -> f64 {
    let mut state = (index as u64).wrapping_add(seed);
    state = (state ^ (state >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94d049bb133111eb);
    state ^= state >> 31;
    (state >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.
}
fn bright_chord(time: f64, phase: f64, vibrato: bool, rate: u32) -> f64 {
    [110., 146.832384, 195.997718]
        .iter()
        .map(|frequency| {
            let base = TAU * frequency * time
                + phase
                + if vibrato {
                    frequency * 0.008 * (1. - (TAU * 5. * time).cos()) / 5.
                } else {
                    0.
                };
            (1..=20)
                .filter(|h| frequency * (*h as f64) < rate as f64 * 0.44)
                .map(|h| ((base * h as f64).sin()) / h as f64)
                .sum::<f64>()
        })
        .sum::<f64>()
        .tanh()
}
fn strums(rate: u32, truth: &[f64], phase: f64) -> Vec<f32> {
    (0..(rate as f64 * 1.9) as usize)
        .map(|index| {
            let time = index as f64 / rate as f64;
            truth
                .iter()
                .enumerate()
                .map(|(n, start)| {
                    let t = time - start;
                    if !(0. ..0.26).contains(&t) {
                        return 0.;
                    }
                    let pick = 0.2 * noise(index, 19 + n as u64 * 103) * (-t / 0.006).exp();
                    let string = 0.18
                        * bright_chord(t, phase + n as f64 * 0.7, false, rate)
                        * (t / 0.002).min(1.)
                        * (-t / 0.09).exp();
                    pick + string
                })
                .sum::<f64>() as f32
        })
        .collect()
}

#[test]
fn repeated_distorted_strums_match_known_sample_clock_once_each() {
    let truth = [0.4, 0.75, 1.1, 1.45];
    for rate in [16000, 22050, 44100, 48000] {
        for phase in [0., 0.7, 1.9] {
            let attacks = detect(&strums(rate, &truth, phase), rate).unwrap();
            assert_eq!(
                attacks.len(),
                truth.len(),
                "rate {rate} phase {phase}: {attacks:?}"
            );
            for (attack, expected) in attacks.iter().zip(truth) {
                assert!(
                    (attack.seconds - expected).abs() <= 0.025,
                    "rate {rate}: {attack:?} expected {expected}"
                );
                assert!(
                    attack.strength.is_finite()
                        && attack.strength > 0.
                        && attack.strength <= 1. + 1e-12
                );
                assert!(attack.bands >= 3 && attack.bands <= 5);
            }
            assert!(
                attacks
                    .windows(2)
                    .all(|pair| pair[1].seconds - pair[0].seconds >= 0.055)
            );
        }
    }
}

#[test]
fn source_offset_and_level_do_not_become_a_tempo_or_pitch_assumption() {
    let rate = 22050;
    let original = strums(rate, &[0.4, 0.75, 1.1, 1.45], 0.3);
    let padding = (0.237 * rate as f64).round() as usize;
    let mut delayed = vec![0.; padding];
    delayed.extend(original.iter().map(|sample| sample * 0.2));
    let before = detect(&original, rate).unwrap();
    let after = detect(&delayed, rate).unwrap();
    assert_eq!(before.len(), 4);
    assert_eq!(after.len(), before.len());
    for (a, b) in before.iter().zip(&after) {
        assert!((b.seconds - a.seconds - padding as f64 / rate as f64).abs() <= 0.012);
    }
    let encoded = serde_json::to_vec(&after).unwrap();
    let decoded: Vec<Attack> = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(after, decoded);
}

#[test]
fn held_vibrato_gain_and_amplitude_modulation_do_not_create_strums() {
    for rate in [22050, 44100] {
        for kind in 0..4 {
            let audio: Vec<_> = (0..rate * 2)
                .map(|index| {
                    let t = index as f64 / rate as f64;
                    let gain = match kind {
                        2 => 0.15 + 0.75 * ((t - 0.6) / 0.15).clamp(0., 1.),
                        3 => 0.5 + 0.25 * (TAU * 7. * t).sin(),
                        _ => 0.6,
                    };
                    (gain * bright_chord(t, 0.4, kind == 1, rate)) as f32
                })
                .collect();
            let attacks = detect(&audio, rate).unwrap();
            assert!(
                attacks.is_empty(),
                "held kind {kind} rate {rate}: {attacks:?}"
            );
        }
    }
}

#[test]
fn stationary_noise_and_smooth_noise_gain_are_not_picked_notes() {
    for seed in [11, 981, 76543] {
        for moving_gain in [false, true] {
            let rate = 22050;
            let audio: Vec<_> = (0..rate * 2)
                .map(|i| {
                    let t = i as f64 / rate as f64;
                    let gain = if moving_gain {
                        0.1 + 0.2 * (t / 1.5).min(1.)
                    } else {
                        0.2
                    };
                    (noise(i as usize, seed) * gain) as f32
                })
                .collect();
            assert!(
                detect(&audio, rate).unwrap().is_empty(),
                "seed {seed} moving {moving_gain}"
            );
        }
    }
}

#[test]
fn silence_short_context_and_invalid_inputs_are_explicit() {
    assert!(detect(&[], 44100).unwrap().is_empty());
    assert!(detect(&vec![0.; 44100], 44100).unwrap().is_empty());
    assert!(detect(&[0.1; 10], 44100).unwrap().is_empty());
    assert!(detect(&[f32::NAN], 44100).is_err());
    assert!(detect(&[f32::INFINITY], 44100).is_err());
    assert!(detect(&[0.; 10], 0).is_err());
    assert!(detect(&[0.; 10], 384000).is_err());
}
