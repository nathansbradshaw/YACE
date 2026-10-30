use chart_engine::transkun::{self, FrontEnd, Interval, semi_crf_decode, subframe_offset};
use std::{fs, path::PathBuf};

#[test]
fn viterbi_selects_global_optimum_and_independent_chords() {
    let t = 6;
    let k = 3;
    let mut s = vec![-20.0; t * t * k];
    let mut set = |e, b, track, v| s[(e * t + b) * k + track] = v;
    // A tempting long interval loses to two shorter events sharing an endpoint.
    set(5, 0, 0, 7.0);
    set(2, 0, 0, 4.0);
    set(5, 2, 0, 5.0);
    set(4, 1, 1, 3.0);
    set(3, 3, 2, 2.0);
    set(5, 5, 2, 1.0);
    let decoded = semi_crf_decode(&s, t, k, None).unwrap();
    assert_eq!(
        decoded[0],
        vec![Interval { begin: 0, end: 2 }, Interval { begin: 2, end: 5 }]
    );
    assert_eq!(decoded[1], vec![Interval { begin: 1, end: 4 }]);
    assert_eq!(
        decoded[2],
        vec![Interval { begin: 3, end: 3 }, Interval { begin: 5, end: 5 }]
    );
}
#[test]
fn viterbi_forced_start_skips_already_closed_events() {
    let mut s = vec![-10.0; 36];
    s[3 * 6 + 1] = 5.0;
    s[5 * 6 + 5] = 2.0;
    assert_eq!(
        semi_crf_decode(&s, 6, 1, Some(&[4])).unwrap()[0],
        vec![Interval { begin: 5, end: 5 }]
    );
    assert!(semi_crf_decode(&s, 6, 1, Some(&[6])).unwrap()[0].is_empty());
}
#[test]
fn decoder_rejects_shape_nonfinite_and_invalid_resume() {
    assert!(semi_crf_decode(&[], 0, 1, None).is_err());
    assert!(semi_crf_decode(&[0.0], 2, 1, None).is_err());
    assert!(semi_crf_decode(&[f32::NAN], 1, 1, None).is_err());
    assert!(semi_crf_decode(&[0.0], 1, 1, Some(&[2])).is_err());
    assert!(semi_crf_decode(&[0.0], 1, 1, Some(&[])).is_err());
    assert!(semi_crf_decode(&[], usize::MAX, 2, None).is_err());
}
#[test]
fn negative_and_tied_scores_do_not_invent_notes() {
    assert!(semi_crf_decode(&[-1.0; 16], 4, 1, None).unwrap()[0].is_empty());
    assert!(semi_crf_decode(&[0.0; 16], 4, 1, None).unwrap()[0].is_empty());
    assert_eq!(
        semi_crf_decode(&[2.0], 1, 1, None).unwrap()[0],
        vec![Interval { begin: 0, end: 0 }]
    );
}
#[test]
fn subframe_mean_is_stable_symmetric_and_bounded() {
    assert_eq!(subframe_offset(0.0), 0.0);
    for l in [1e-8, 0.002, 0.004, 0.1, 1.0, 10.0, 1000.0] {
        let p = subframe_offset(l);
        let n = subframe_offset(-l);
        assert!(p.is_finite() && n.is_finite() && (0.0..=0.5).contains(&p));
        assert!((p + n).abs() < 1e-9, "{l}: {p} {n}");
    }
}
#[test]
fn model_assets_are_revision_pinned() {
    for asset in transkun::assets() {
        assert!(asset.url.contains(transkun::MODEL_REVISION));
        assert_eq!(asset.sha256.len(), 64);
        assert!(asset.filename.starts_with("transkun/"));
    }
}
fn read_floats(path: PathBuf) -> Vec<f32> {
    fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn model_dir() -> PathBuf {
    std::env::var_os("TRANSKUN_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/transkun"))
}
fn reference_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("TRANSKUN_REFERENCE_DIR")
            .expect("Set TRANSKUN_REFERENCE_DIR to pinned upstream numeric fixtures"),
    )
}
#[test]
#[ignore = "requires pinned public model assets and upstream reference fixtures"]
fn native_frontend_matches_upstream_numeric_reference() {
    let dir = reference_dir();
    let audio = read_floats(dir.join("ref3b_audio.f32"));
    let reference = read_floats(dir.join("ref3b_features.f32"));
    let (actual, frames) = FrontEnd::load(&model_dir())
        .unwrap()
        .compute(&audio)
        .unwrap();
    assert_eq!(frames, 66);
    assert_eq!(actual.len(), reference.len());
    let max = actual
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    let rms = (actual
        .iter()
        .zip(&reference)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / actual.len() as f64)
        .sqrt();
    eprintln!("Transkun frontend numeric reference: max_abs={max}, rms={rms}");
    assert!(max < 5e-4 && rms < 2e-5);
}
fn compare_intervals(actual: Vec<Vec<Interval>>, reference: &serde_json::Value) {
    for (i, list) in actual.iter().enumerate() {
        let expected: Vec<[usize; 2]> =
            serde_json::from_value(reference[i.to_string()].clone()).unwrap();
        let actual: Vec<_> = list.iter().map(|iv| [iv.begin, iv.end]).collect();
        assert_eq!(actual, expected, "track {i}");
    }
}
#[test]
#[ignore = "requires pinned upstream semi-CRF numeric fixtures"]
fn native_decoder_matches_real_and_synthetic_reference() {
    let dir = reference_dir();
    for (name, t) in [("ref3c_syn", 6), ("ref3c", 66)] {
        let scores = read_floats(dir.join(format!("{name}_S.f32")));
        let reference: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(format!("{name}_intervals.json"))).unwrap())
                .unwrap();
        compare_intervals(semi_crf_decode(&scores, t, 90, None).unwrap(), &reference);
    }
    let reference: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("ref3c_forced_intervals.json")).unwrap())
            .unwrap();
    let forced: Vec<usize> = serde_json::from_value(reference["forcedStartPos"].clone()).unwrap();
    compare_intervals(
        semi_crf_decode(
            &read_floats(dir.join("ref3c_syn_S.f32")),
            6,
            90,
            Some(&forced),
        )
        .unwrap(),
        &reference["intervals"],
    );
}

/// Deterministic harmonic piano-like fixture, generated in Rust. Held fundamentals
/// and decaying partials provide known pitch/onset truth independent of the model.
fn piano_fixture() -> (Vec<f32>, Vec<(u8, f64, f64)>) {
    let rate = transkun::SAMPLE_RATE;
    let truth = vec![
        (60, 0.40, 1.15),
        (64, 0.40, 1.15),
        (67, 0.40, 1.15),
        (62, 1.65, 2.40),
        (65, 1.65, 2.40),
        (69, 1.65, 2.40),
    ];
    let mut audio = vec![0.0f32; rate * 3];
    for &(pitch, start, end) in &truth {
        let frequency = 440.0 * 2f64.powf((pitch as f64 - 69.0) / 12.0);
        for (i, sample) in audio
            .iter_mut()
            .enumerate()
            .take(((end + 0.18) * rate as f64) as usize)
            .skip((start * rate as f64) as usize)
        {
            let t = i as f64 / rate as f64 - start;
            let attack = (t / 0.006).min(1.0);
            let release = if t > end - start {
                (-(t - (end - start)) * 32.0).exp()
            } else {
                1.0
            };
            let mut value = 0.0;
            for h in 1..=8 {
                let partial = h as f64;
                value += (std::f64::consts::TAU * frequency * partial * t).sin()
                    / partial.powf(1.8)
                    * (-t * (0.7 + 0.2 * partial)).exp();
            }
            *sample += (0.16 * value * attack * release) as f32;
        }
    }
    (audio, truth)
}
#[test]
#[ignore = "runs real Transkun backbone and attribute-head ONNX inference"]
fn native_onnx_transcribes_known_harmonic_chords() {
    let (audio, truth) = piano_fixture();
    let result = transkun::transcribe_samples(&audio, &model_dir()).unwrap();
    eprintln!(
        "Transkun harmonic fixture: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
    let matches = truth
        .iter()
        .filter(|(pitch, start, _)| {
            result
                .notes
                .iter()
                .any(|n| n.pitch == *pitch && (n.start - *start).abs() < 0.12)
        })
        .count();
    // This piano-trained model is imperfect on additive synthetic timbres. The
    // smoke gate requires recovery of the full first chord; the printed 6-note
    // recall remains evidence rather than being presented as perfect accuracy.
    eprintln!("Known harmonic note matches: {matches}/{}", truth.len());
    for pitch in [60, 64, 67] {
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.pitch == pitch && (n.start - 0.40).abs() < 0.12),
            "Known first-chord pitch {pitch} missing"
        );
    }
    assert!(
        result.notes.len() <= truth.len() + 2,
        "Too many extra events on known fixture"
    );
    assert!(
        result
            .notes
            .iter()
            .all(|n| n.start >= 0.0 && n.end <= 3.0 && n.end > n.start && n.velocity > 0)
    );
}
