use chart_engine::drum_parts::{BANDS, analyse};

fn burst(rate: u32, frequency: f64, onset: f64) -> Vec<f32> {
    (0..rate as usize)
        .map(|i| {
            let t = i as f64 / rate as f64 - onset;
            if (0. ..0.12).contains(&t) {
                ((std::f64::consts::TAU * frequency * t).sin() * (-t * 35.).exp() * 0.5) as f32
            } else {
                0.
            }
        })
        .collect()
}

#[test]
fn finite_silence_and_short_edges_stay_bounded() {
    for rate in [8000, 22050, 44100, 48000, 192000] {
        for len in [1, 17, 2000] {
            let output = analyse(&vec![0.; len], rate).unwrap();
            assert_eq!(output.len(), BANDS.len());
            assert!(output.iter().all(|v| v.rms == 0. && v.attacks.is_empty()));
            let impulse = analyse(&vec![0.5; len], rate).unwrap();
            assert!(impulse.iter().all(|v| {
                v.rms.is_finite()
                    && v.rms >= 0.
                    && v.attacks
                        .iter()
                        .all(|t| *t >= 0. && *t < len as f64 / rate as f64)
            }));
        }
    }
}

#[test]
fn tone_bursts_have_source_clock_activity_without_instrument_claims() {
    for (frequency, band) in [(65., 0), (300., 1), (3500., 2), (10000., 3)] {
        let input = burst(44100, frequency, 0.4);
        let original = input.clone();
        let result = analyse(&input, 44100).unwrap();
        assert_eq!(input, original);
        let peak = result
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.rms.total_cmp(&b.1.rms))
            .unwrap()
            .0;
        assert_eq!(peak, band);
        assert!(
            result[band]
                .attacks
                .iter()
                .any(|t| (*t - 0.4).abs() <= 0.025),
            "{} {:?}",
            frequency,
            result[band].attacks
        );
        assert!(
            result
                .iter()
                .all(|r| !["kick", "snare", "ride", "crash", "toms"].contains(&r.part.as_str()))
        );
        assert_eq!(result, analyse(&input, 44100).unwrap());
        let encoded = serde_json::to_vec(&result).unwrap();
        assert_eq!(
            result,
            serde_json::from_slice::<Vec<chart_engine::drum_parts::PartEvidence>>(&encoded)
                .unwrap()
        );
    }
}

#[test]
fn absolute_gain_is_preserved_and_sustained_tone_does_not_become_many_hits() {
    let original: Vec<_> = (0..44100)
        .map(|i| (0.4 * (std::f64::consts::TAU * 300. * i as f64 / 44100.).sin()) as f32)
        .collect();
    let quiet: Vec<_> = original.iter().map(|v| v * 0.25).collect();
    let a = analyse(&original, 44100).unwrap();
    let b = analyse(&quiet, 44100).unwrap();
    assert!((a[1].rms - 0.4 / 2f64.sqrt()).abs() < 0.01);
    assert!((b[1].rms / a[1].rms - 0.25).abs() < 1e-6);
    assert!(a[1].attacks.len() <= 2, "{:?}", a[1].attacks);
}

#[test]
fn invalid_audio_fails_before_analysis() {
    assert!(analyse(&[], 44100).is_err());
    for rate in [0, 7999, 192001, u32::MAX] {
        assert!(analyse(&[0.], rate).is_err());
    }
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(analyse(&[bad], 44100).is_err());
    }
}
