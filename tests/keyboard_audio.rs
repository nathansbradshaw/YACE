use chart_engine::{
    keyboard_audio as audio,
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
};
use std::{collections::BTreeMap, path::PathBuf};

#[test]
fn residual_preserves_phase_clock_and_uses_one_scalar() {
    let original = [0.04, -0.04, 0.02, 0.];
    let drums = [0.01, -0.01, 0.01, 0.];
    let bass = [0.01, -0.01, 0., 0.];
    let vocals = [0.; 4];
    let mixed = audio::mix_aligned(&original, &drums, &bass, &vocals).unwrap();
    assert_eq!(mixed.samples.len(), original.len());
    assert_eq!(mixed.gain, 4.);
    for (a, b) in mixed.samples.iter().zip([0.08, -0.08, 0.04, 0.]) {
        assert!((*a - b).abs() < 1e-7);
    }
    assert!(mixed.residual_peak < 0.021);
    assert_ne!(mixed.residual_pcm_sha256, mixed.analysis_pcm_sha256);
}

#[test]
fn headroom_bounds_outliers_without_clipping_or_trimming() {
    let dc = vec![0.1; 480000];
    let silence = vec![0.; dc.len()];
    let constant = audio::mix_aligned(&dc, &silence, &silence, &silence).unwrap();
    assert_eq!(constant.samples.len(), dc.len());
    let mut signal = vec![0.; 1000];
    signal[0] = 2.;
    signal[800] = -1.;
    let mixed =
        audio::mix_aligned(&signal, &vec![0.; 1000], &vec![0.; 1000], &vec![0.; 1000]).unwrap();
    assert_eq!(mixed.samples.len(), 1000);
    assert_eq!(mixed.gain, 0.475);
    assert!((mixed.samples[0] - 0.95).abs() < 1e-7);
    assert!((mixed.samples[800] + 0.475).abs() < 1e-7);
    assert_eq!(mixed.samples[999], 0.);
    let zero = audio::mix_aligned(&[0.; 20], &[0.; 20], &[0.; 20], &[0.; 20]).unwrap();
    assert_eq!(zero.gain, 1.);
    assert_eq!(zero.samples, vec![0.; 20]);
}

#[test]
fn alignment_and_nonfinite_input_fail_closed() {
    assert!(audio::mix_aligned(&[], &[], &[], &[]).is_err());
    assert!(audio::mix_aligned(&[0., 1.], &[0.], &[0., 0.], &[0., 0.]).is_err());
    assert!(audio::mix_aligned(&[f32::NAN], &[0.], &[0.], &[0.]).is_err());
    assert!(audio::mix_aligned(&[0.], &[f32::INFINITY], &[0.], &[0.]).is_err());
}

pub fn bound_fixture() -> (StemManifest, audio::Receipt) {
    let stems = audio::CONTRIBUTORS
        .iter()
        .enumerate()
        .map(|(i, id)| StemAsset {
            id: *id,
            path: PathBuf::from(format!("/offline/{}.wav", id.as_str())),
            sha256: format!("{:064x}", i + 1),
        })
        .collect();
    let manifest = StemManifest {
        version: 1,
        source_audio_sha256: "a".repeat(64),
        sample_rate: 44100,
        channels: 2,
        frames: 88200,
        alignment: AlignmentEvidence {
            method: "fixture".into(),
            details: "metadata-only validation; audio intentionally unavailable".into(),
        },
        stems,
        generation: None,
    };
    let receipt = audio::Receipt {
        version: 1,
        recipe: audio::RECIPE.into(),
        source_audio_sha256: manifest.source_audio_sha256.clone(),
        contributing_stem_sha256: audio::contributing_hashes(&manifest).unwrap(),
        source_sample_rate: 44100,
        source_frames: 88200,
        sample_rate: 16000,
        channels: 1,
        sample_count: 32000,
        target_rms: 0.1,
        maximum_gain: 4.,
        peak_ceiling: 0.95,
        gain: 2.,
        residual_rms: 0.05,
        residual_peak: 0.2,
        analysis_rms: 0.1,
        analysis_peak: 0.4,
        residual_pcm_sha256: "b".repeat(64),
        analysis_pcm_sha256: "c".repeat(64),
    };
    (manifest, receipt)
}

#[test]
fn saved_recipe_validates_offline_and_rejects_provenance_or_gain_changes() {
    let (manifest, receipt) = bound_fixture();
    receipt.validate(&manifest).unwrap();
    let restored: audio::Receipt =
        serde_json::from_slice(&serde_json::to_vec(&receipt).unwrap()).unwrap();
    restored.validate(&manifest).unwrap();
    for mutation in 0..12 {
        let mut r = receipt.clone();
        match mutation {
            0 => r.source_audio_sha256 = "d".repeat(64),
            1 => r.source_frames += 2,
            2 => r.sample_count -= 10,
            3 => r
                .contributing_stem_sha256
                .insert("drums".into(), "d".repeat(64))
                .map(|_| ())
                .unwrap(),
            4 => {
                r.contributing_stem_sha256
                    .insert("piano".into(), "d".repeat(64));
            }
            5 => r.gain = 3.,
            6 => r.analysis_peak = 0.8,
            7 => r.target_rms = 0.2,
            8 => r.residual_rms = f64::NAN,
            9 => r.analysis_pcm_sha256 = "invalid".into(),
            10 => r.recipe = "unbound mixture".into(),
            _ => r.source_sample_rate = 16000,
        }
        assert!(r.validate(&manifest).is_err(), "mutation {mutation}");
    }
    let mut missing = manifest.clone();
    missing.stems.retain(|s| s.id != StemId::Drums);
    assert!(receipt.validate(&missing).is_err());
    assert_eq!(receipt.contributing_stem_sha256.len(), 3);
    assert_eq!(
        receipt
            .contributing_stem_sha256
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        BTreeMap::from([
            ("bass".to_owned(), ()),
            ("drums".into(), ()),
            ("vocals".into(), ())
        ])
        .into_keys()
        .collect::<Vec<_>>()
    );
}
