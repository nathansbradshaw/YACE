//! Known-truth acoustic controls. Harmonic/formant fixtures are a controlled
//! voiced source, not evidence of accuracy on recordings or vocal identity.
use chart_engine::{
    clean::{AlignedWord, WordTimings},
    timing::TempoMap,
    vocal_generation::{self, Policy},
};
const RATE: usize = 22050;
fn vowel(duration: f64, start: f64, end: f64, midi: f64, glide: f64) -> Vec<f32> {
    let mut phase = 0.;
    (0..(duration * RATE as f64) as usize)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            let pitch = midi
                + 0.22 * (std::f64::consts::TAU * 5.5 * t).sin()
                + glide * ((t - start) / (end - start)).clamp(0., 1.);
            let frequency = 440. * 2f64.powf((pitch - 69.) / 12.);
            phase += std::f64::consts::TAU * frequency / RATE as f64;
            if t < start || t >= end {
                return 0.;
            }
            let envelope = ((t - start) / 0.015).min((end - t) / 0.015).clamp(0., 1.);
            let value = (1..=20)
                .map(|h| {
                    let f = h as f64 * frequency;
                    let formant = 1.
                        + 3. * (-0.5 * ((f - 750.) / 150.).powi(2)).exp()
                        + 2. * (-0.5 * ((f - 1200.) / 180.).powi(2)).exp()
                        + (-0.5 * ((f - 2500.) / 250.).powi(2)).exp();
                    (phase * h as f64).sin() * formant / (h * h) as f64
                })
                .sum::<f64>();
            (envelope * value * 0.14) as f32
        })
        .collect()
}
fn words(duration: f64, start: f64, end: f64) -> WordTimings {
    WordTimings{version:1,audio_sha256:"a".repeat(64),duration_seconds:duration,source:"Controlled acoustic word estimate; true vowel endpoint is separately specified by fixture".into(),words:vec![AlignedWord{text:Some("stay".into()),start:Some(start),end:Some(end)}]}
}
#[test]
fn vibrato_and_glide_pitch_frames_follow_known_contour_without_filling_breaths() {
    let signal = vowel(2., 0.3, 1.5, 57., 3.);
    let words = words(2., 0.3, 1.5);
    let timing = TempoMap::constant(120.).unwrap();
    let report = vocal_generation::generate_pcm(
        &signal,
        &"a".repeat(64),
        &"b".repeat(64),
        2.,
        &timing,
        Some(&words),
        None,
        &Policy::default(),
    )
    .unwrap();
    let supported: Vec<_> = report
        .pitch_frames
        .iter()
        .filter(|f| f.time_seconds > 0.4 && f.time_seconds < 1.4)
        .filter_map(|f| f.midi.map(|p| (f.time_seconds, p)))
        .collect();
    assert!(
        supported.len() > 65,
        "insufficient known voiced coverage: {}",
        supported.len()
    );
    for (t, p) in supported {
        let truth = 57.
            + 0.22 * (std::f64::consts::TAU * 5.5 * t).sin()
            + 3. * ((t - 0.3) / 1.2).clamp(0., 1.);
        assert!((p - truth).abs() < 0.55, "pitch error at{t}: {p} vs{truth}");
    }
    assert!(
        report
            .pitch_frames
            .iter()
            .filter(|f| f.time_seconds < 0.24 || f.time_seconds > 1.56)
            .all(|f| f.midi.is_none())
    );
}
#[test]
fn unpitched_breath_after_singing_cannot_extend_a_note() {
    let mut signal = vowel(2., 0.3, 0.85, 57., 0.);
    let mut seed = 111u32;
    for (i, sample) in signal.iter_mut().enumerate() {
        let t = i as f64 / RATE as f64;
        if (0.9..1.3).contains(&t) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *sample = 0.12 * (seed as f64 / u32::MAX as f64 * 2. - 1.) as f32;
        }
    }
    let timing = TempoMap::constant(120.).unwrap();
    let words = words(2., 0.3, 0.8);
    let report = vocal_generation::generate_pcm(
        &signal,
        &"a".repeat(64),
        &"b".repeat(64),
        2.,
        &timing,
        Some(&words),
        None,
        &Policy {
            local_energy_gate: true,
            ..Policy::default()
        },
    )
    .unwrap();
    assert!(
        report
            .chart
            .unwrap()
            .notes
            .iter()
            .all(|n| timing.seconds(n.end) < 0.9)
    );
    assert!(
        report
            .pitch_frames
            .iter()
            .filter(|f| (0.96..1.25).contains(&f.time_seconds))
            .all(|f| f.midi.is_none())
    );
}

#[test]
fn local_energy_prevents_analysis_window_from_turning_silence_into_singing() {
    let timing = TempoMap::constant(120.).unwrap();
    for duration in [0.05, 0.06, 0.1, 0.5] {
        let signal = vowel(1., 0.3, 0.3 + duration, 64., 0.);
        let words = words(1., 0.2, 0.9);
        let report = vocal_generation::generate_pcm(
            &signal,
            &"a".repeat(64),
            &"b".repeat(64),
            1.,
            &timing,
            Some(&words),
            None,
            &Policy {
                minimum_note_seconds: 0.030,
                local_energy_gate: true,
                ..Policy::default()
            },
        )
        .unwrap();
        let chart = report
            .chart
            .expect("Actual voiced fixture should survive activity gate");
        for note in chart.notes {
            assert!(
                timing.seconds(note.start) >= 0.294,
                "Analysis-window onsetpadding became a note"
            );
            assert!(
                timing.seconds(note.end) <= 0.3 + duration + 0.006,
                "Analysis-window tailpadding became a note"
            );
        }
        assert!(
            report
                .pitch_frames
                .iter()
                .filter(|f| f.time_seconds < 0.294 || f.time_seconds > 0.3 + duration + 0.006)
                .all(|f| f.midi.is_none())
        );
        assert!(
            report
                .pitch_frames
                .iter()
                .any(|f| f.local_energy_active == Some(false) && f.yin_midi.is_some()),
            "Original rejected window estimates must remain available as diagnostics"
        );
    }
}
