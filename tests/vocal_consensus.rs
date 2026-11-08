use chart_engine::{
    vocal_consensus,
    vocal_generation::{PitchFrame, PitchSupport},
    vocal_pitch::{Evidence, Frame},
};
fn fixture() -> (Vec<PitchFrame>, Vec<f32>, Evidence, Vec<Frame>) {
    let signal: Vec<_> = (0..22050)
        .map(|i| (std::f64::consts::TAU * 220. * i as f64 / 22050.).sin() as f32 * 0.2)
        .collect();
    let frames = (0..100)
        .map(|i| PitchFrame {
            time_seconds: i as f64 * 220. / 22050.,
            midi: (!(20..70).contains(&i)).then_some(57.),
            yin_midi: None,
            mcleod_midi: None,
            clarity: 0.9,
            rms: 0.1,
            local_rms: Some(0.1),
            local_energy_active: Some(true),
            support: PitchSupport::Consensus,
        })
        .collect();
    let model: Vec<_> = (0..101)
        .map(|i| Frame {
            time_seconds: i as f64 * 0.01,
            frequency_hz: Some(220.),
            confidence: 0.9,
            rms: 0.1,
        })
        .collect();
    let primary = chart_engine::vocal_rmvpe::finish_pcm(&vec![0.; 16000], model.clone()).unwrap();
    (frames, signal, primary, model)
}
#[test]
fn independent_models_recover_periodic_voice_without_words_and_keep_existing_voice() {
    let (mut frames, signal, primary, other) = fixture();
    let before = frames.clone();
    let recovered =
        vocal_consensus::recover(&mut frames, &signal, &primary, &other, &|| false).unwrap();
    assert_eq!(recovered.len(), 50);
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.midi, Some(57.));
        if before[i].midi.is_some() {
            assert_eq!(
                serde_json::to_value(f).unwrap(),
                serde_json::to_value(&before[i]).unwrap()
            );
        } else {
            assert_eq!(f.support, PitchSupport::AcousticAlternative);
            assert_eq!(f.clarity, 0.5);
        }
    }
}
#[test]
fn recovery_requires_two_models_energy_stability_and_actual_periodicity() {
    for variant in 0..6 {
        let (mut frames, mut signal, primary, mut other) = fixture();
        match variant {
            0 => {
                for f in &mut other {
                    f.frequency_hz = Some(440.);
                }
            }
            1 => {
                for f in &mut other {
                    f.confidence = 0.49;
                }
            }
            2 => {
                for f in &mut frames {
                    f.local_energy_active = Some(false);
                }
            }
            3 => signal.fill(0.),
            4 => {
                for (i, f) in other.iter_mut().enumerate() {
                    if i % 5 != 0 {
                        f.confidence = 0.;
                    }
                }
            }
            _ => {
                for (i, s) in signal.iter_mut().enumerate() {
                    *s = (std::f64::consts::TAU * 300. * i as f64 / 22050.).sin() as f32 * 0.2;
                }
            }
        }
        assert!(
            vocal_consensus::recover(&mut frames, &signal, &primary, &other, &|| false)
                .unwrap()
                .is_empty(),
            "variant {variant}"
        );
    }
}
#[test]
fn cancellation_and_invalid_contours_leave_the_draft_unchanged() {
    let (mut frames, signal, primary, mut other) = fixture();
    let before = serde_json::to_value(&frames).unwrap();
    let calls = std::cell::Cell::new(0);
    assert!(
        vocal_consensus::recover(&mut frames, &signal, &primary, &other, &|| {
            let n = calls.get() + 1;
            calls.set(n);
            n >= 3
        })
        .is_err()
    );
    assert_eq!(serde_json::to_value(&frames).unwrap(), before);
    other[10].time_seconds = f64::NAN;
    assert!(vocal_consensus::recover(&mut frames, &signal, &primary, &other, &|| false).is_err());
    assert_eq!(serde_json::to_value(&frames).unwrap(), before);
}
