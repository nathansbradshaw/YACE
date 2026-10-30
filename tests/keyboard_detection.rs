use chart_engine::{
    generation::Request,
    keyboard_detection::{self, Evidence, KeyboardSound},
    mapping::{PitchCandidateEvidence, PitchCandidateSource, PitchNote, Transcription},
    models,
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
};
use std::collections::BTreeMap;
fn note() -> PitchNote {
    PitchNote {
        start: 0.5,
        end: 1.8,
        pitch: 60,
        confidence: 0.8,
    }
}
fn fixture(sound: KeyboardSound) -> Transcription {
    let ids = if sound == KeyboardSound::Piano {
        vec![StemId::Piano]
    } else {
        vec![StemId::Piano, StemId::Other]
    };
    let stems: Vec<_> = ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| StemAsset {
            id,
            path: std::path::PathBuf::from(format!("/not-needed/{}.wav", id.as_str())),
            sha256: format!("{i:064x}"),
        })
        .collect();
    let asset = models::assets()
        .into_iter()
        .find(|a| a.id == sound.model_id())
        .unwrap();
    let raw_notes = vec![note()];
    let evidence = Evidence {
        version: 1,
        sound,
        source_audio_sha256: "a".repeat(64),
        duration_seconds: 2.,
        stem_sha256: stems
            .iter()
            .map(|s| (s.id.as_str().into(), s.sha256.clone()))
            .collect(),
        model_id: asset.id,
        model_sha256: asset.sha256,
        implementation_sha256: keyboard_detection::implementation_hash(sound),
        method: sound.method().into(),
        score_semantics: sound.score_semantics().into(),
        decoder_parameters: if sound == KeyboardSound::Piano {
            BTreeMap::from([
                ("onset_threshold".into(), 0.3),
                ("offset_threshold".into(), 0.3),
                ("frame_threshold".into(), 0.1),
                ("pedal_offset_threshold".into(), 0.2),
            ])
        } else {
            BTreeMap::from([
                ("onset_threshold".into(), 0.5),
                ("frame_threshold".into(), 0.3),
            ])
        },
        raw_notes: raw_notes.clone(),
        basic_pitch_candidates: if sound == KeyboardSound::Synth {
            vec![PitchCandidateEvidence {
                candidate: note(),
                source: PitchCandidateSource::NetworkOnset,
                onset_score: 0.7,
                inferred_onset_score: 0.8,
            }]
        } else {
            vec![]
        },
        velocities: if sound == KeyboardSound::Piano {
            vec![100]
        } else {
            vec![]
        },
        pedals: vec![],
        muscriptor: None,
        piano_audio: None,
        piano_multiview: None,
    };
    Transcription {
        isolation: Some(StemManifest {
            version: 1,
            source_audio_sha256: "a".repeat(64),
            sample_rate: 44100,
            channels: 2,
            frames: 88200,
            alignment: AlignmentEvidence {
                method: "fixture".into(),
                details: "metadata-only evidence validation; no inference claim".into(),
            },
            stems,
            generation: None,
        }),
        piano: raw_notes.clone(),
        pro_piano: raw_notes,
        keyboard_detection: Some(evidence),
        ..Default::default()
    }
}
#[test]
fn legacy_requests_and_model_free_events_keep_defaults() {
    let request:Request=serde_json::from_value(serde_json::json!({"source":{"type":"audio","path":"song.wav"},"bpm":120,"title":"Test","artist":"Test","instruments":["piano"],"model_dir":"models"})).unwrap();
    assert_eq!(request.keyboard_sound, KeyboardSound::Piano);
    assert_eq!(
        request.piano_quality,
        keyboard_detection::PianoQuality::Standard
    );
    let old: Transcription = serde_json::from_value(
        serde_json::json!({"piano":[{"start":0.5,"end":1.8,"pitch":60,"confidence":0.8}]}),
    )
    .unwrap();
    old.validate().unwrap();
    assert!(old.keyboard_detection.is_none());
    assert!(old.keyboard_analysis.is_none());
    assert_eq!(
        serde_json::from_str::<KeyboardSound>("\"synth\"").unwrap(),
        KeyboardSound::Synth
    );
}
#[test]
fn only_requested_model_assets_are_selected() {
    let keys = vec!["piano".into(), "pro_piano".into()];
    assert_eq!(
        keyboard_detection::required_models(&keys, KeyboardSound::Piano, false),
        ["piano", "separation"]
    );
    assert_eq!(
        keyboard_detection::required_models(&keys, KeyboardSound::Synth, false),
        ["muscriptor-cond", "muscriptor-lm", "separation"]
    );
    assert_eq!(
        keyboard_detection::required_models(&["drums".into()], KeyboardSound::Piano, true),
        ["drums"]
    );
    assert_eq!(
        keyboard_detection::required_models(
            &["bass".into(), "piano".into()],
            KeyboardSound::Piano,
            false
        ),
        ["pitch", "piano", "separation"]
    );
}

#[test]
fn higher_quality_assets_are_optional_and_only_requested_for_piano_inputs() {
    use keyboard_detection::{PianoQuality, required_models_with_quality};
    let keys = vec!["piano".into(), "pro_piano".into()];
    assert_eq!(
        required_models_with_quality(
            &keys,
            KeyboardSound::Piano,
            PianoQuality::HighQuality,
            false
        ),
        ["piano", chart_engine::keyboard_separation::MODEL_ID]
    );
    assert_eq!(
        required_models_with_quality(
            &["bass".into(), "piano".into(), "vocals".into()],
            KeyboardSound::Piano,
            PianoQuality::HighQuality,
            false
        ),
        [
            "pitch",
            "piano",
            chart_engine::keyboard_separation::MODEL_ID,
            "separation"
        ]
    );
    assert_eq!(
        required_models_with_quality(
            &keys,
            KeyboardSound::Synth,
            PianoQuality::HighQuality,
            false
        ),
        keyboard_detection::required_models(&keys, KeyboardSound::Synth, false)
    );
    assert_eq!(
        required_models_with_quality(
            &["drums".into()],
            KeyboardSound::Piano,
            PianoQuality::HighQuality,
            true
        ),
        ["drums"]
    );
    assert_eq!(
        serde_json::from_str::<PianoQuality>("\"high_quality\"").unwrap(),
        PianoQuality::HighQuality
    );
    assert!(
        models::assets()
            .iter()
            .all(|a| a.id != chart_engine::keyboard_separation::MODEL_ID)
    );
}

fn mixed_piano_fixture() -> Transcription {
    let mut t = fixture(KeyboardSound::Piano);
    let mut analysis = t.isolation.clone().unwrap();
    analysis.stems = StemId::ALL
        .into_iter()
        .enumerate()
        .map(|(i, id)| StemAsset {
            id,
            path: format!("/analysis/{}.wav", id.as_str()).into(),
            sha256: format!("{:064x}", i + 100),
        })
        .collect();
    t.keyboard_detection.as_mut().unwrap().stem_sha256 = BTreeMap::from([(
        "piano".into(),
        analysis
            .stems
            .iter()
            .find(|s| s.id == StemId::Piano)
            .unwrap()
            .sha256
            .clone(),
    )]);
    t.keyboard_analysis = Some(analysis);
    t.guitar = vec![note()];
    t
}

#[test]
fn mixed_piano_evidence_uses_its_own_complete_manifest_and_preserves_primary_inputs() {
    let t = mixed_piano_fixture();
    let primary = serde_json::to_vec(t.isolation.as_ref().unwrap()).unwrap();
    t.validate().unwrap();
    assert_eq!(
        serde_json::to_vec(t.isolation.as_ref().unwrap()).unwrap(),
        primary
    );
    assert_eq!(t.keyboard_manifest().unwrap().unwrap().stems.len(), 6);
    assert_ne!(
        t.keyboard_detection.as_ref().unwrap().stem_sha256["piano"],
        t.isolation.as_ref().unwrap().stems[0].sha256
    );
    let restored: Transcription = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
    restored.validate().unwrap();
    for mutation in 0..8 {
        let mut t = mixed_piano_fixture();
        match mutation {
            0 => t.keyboard_analysis.as_mut().unwrap().source_audio_sha256 = "e".repeat(64),
            1 => t.keyboard_analysis.as_mut().unwrap().frames += 1,
            2 => {
                t.keyboard_analysis
                    .as_mut()
                    .unwrap()
                    .stems
                    .iter_mut()
                    .find(|s| s.id == StemId::Piano)
                    .unwrap()
                    .sha256 = "f".repeat(64)
            }
            3 => t.isolation.as_mut().unwrap().source_audio_sha256 = "e".repeat(64),
            4 => t
                .keyboard_analysis
                .as_mut()
                .unwrap()
                .stems
                .retain(|s| s.id != StemId::Other),
            5 => t.keyboard_detection = None,
            6 => t.keyboard_detection.as_mut().unwrap().sound = KeyboardSound::Synth,
            _ => t.isolation = None,
        }
        assert!(
            t.validate().is_err(),
            "accepted split-manifest mutation {mutation}"
        );
    }
    let legacy = fixture(KeyboardSound::Piano);
    assert!(legacy.keyboard_analysis.is_none());
    legacy.validate().unwrap();
}

#[test]
fn mixed_generation_retains_both_real_source_bound_manifest_sidecars() {
    use chart_engine::{pipeline, timing::TempoMap};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("song.wav");
    let pcm = vec![0.; 88200];
    models::write_audio(&source, &pcm, &pcm).unwrap();
    let source_hash = models::hash_file(&source).unwrap();
    let mut t = mixed_piano_fixture();
    for (prefix, manifest) in [
        ("primary", t.isolation.as_mut().unwrap()),
        ("analysis", t.keyboard_analysis.as_mut().unwrap()),
    ] {
        manifest.source_audio_sha256 = source_hash.clone();
        manifest.stems = StemId::ALL
            .into_iter()
            .enumerate()
            .map(|(i, id)| {
                let path = temp.path().join(format!("{prefix}-{}.wav", id.as_str()));
                let mut stem = pcm.clone();
                stem[100 + i] = if prefix == "analysis" { 0.25 } else { 0.125 };
                models::write_audio(&path, &stem, &stem).unwrap();
                StemAsset {
                    id,
                    path,
                    sha256: models::hash_file(
                        &temp.path().join(format!("{prefix}-{}.wav", id.as_str())),
                    )
                    .unwrap(),
                }
            })
            .collect();
    }
    let e = t.keyboard_detection.as_mut().unwrap();
    e.source_audio_sha256 = source_hash;
    e.stem_sha256 = BTreeMap::from([(
        "piano".into(),
        t.keyboard_analysis
            .as_ref()
            .unwrap()
            .stems
            .iter()
            .find(|s| s.id == StemId::Piano)
            .unwrap()
            .sha256
            .clone(),
    )]);
    let out = temp.path().join("generated");
    let report = pipeline::build(pipeline::Build {
        audio: &source,
        output: &out,
        existing: None,
        timing: Some(TempoMap::constant(120.).unwrap()),
        transcription: &t,
        meta: pipeline::Metadata {
            name: "Split source fixture".into(),
            artist: "Local test".into(),
            duration: 2.,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    let primary: StemManifest =
        serde_json::from_slice(&std::fs::read(out.join("chart-engine-isolation.json")).unwrap())
            .unwrap();
    let analysis: StemManifest = serde_json::from_slice(
        &std::fs::read(out.join("chart-engine-keyboard-isolation.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&primary).unwrap(),
        serde_json::to_value(t.isolation.as_ref().unwrap()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&analysis).unwrap(),
        serde_json::to_value(t.keyboard_analysis.as_ref().unwrap()).unwrap()
    );
    assert_eq!(
        report["keyboard_analysis_manifest"],
        "chart-engine-keyboard-isolation.json"
    );
    assert!(out.join("notes.mid").is_file());
    let saved: Transcription = serde_json::from_slice(
        &std::fs::read(out.join("chart-engine-transcription.json")).unwrap(),
    )
    .unwrap();
    saved.validate().unwrap();
}
#[test]
fn piano_and_synth_evidence_roundtrip_without_installed_models_or_stems() {
    for sound in [KeyboardSound::Piano, KeyboardSound::Synth] {
        let original = fixture(sound);
        original.validate().unwrap();
        let restored: Transcription =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.keyboard_detection.unwrap().raw_notes.len(), 1);
    }
}
fn residual_fixture() -> Transcription {
    use chart_engine::keyboard_audio as audio;
    let mut t = fixture(KeyboardSound::Piano);
    let manifest = t.isolation.as_mut().unwrap();
    manifest.stems = audio::CONTRIBUTORS
        .iter()
        .enumerate()
        .map(|(i, id)| StemAsset {
            id: *id,
            path: std::path::PathBuf::from(format!("/offline/{}.wav", id.as_str())),
            sha256: format!("{:064x}", i + 1),
        })
        .collect();
    let receipt = audio::Receipt {
        version: 1,
        recipe: audio::RECIPE.into(),
        source_audio_sha256: manifest.source_audio_sha256.clone(),
        contributing_stem_sha256: audio::contributing_hashes(manifest).unwrap(),
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
    let e = t.keyboard_detection.as_mut().unwrap();
    e.version = 4;
    e.method = keyboard_detection::piano_method();
    e.stem_sha256 = receipt.contributing_stem_sha256.clone();
    e.piano_audio = Some(receipt);
    t
}
#[test]
fn piano_v4_roundtrips_offline_without_fake_piano_contributor() {
    let t = residual_fixture();
    t.validate().unwrap();
    let restored: Transcription = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
    restored.validate().unwrap();
    assert!(
        !restored
            .keyboard_detection
            .unwrap()
            .stem_sha256
            .contains_key("piano")
    );
    for mutation in 0..7 {
        let mut t = residual_fixture();
        let e = t.keyboard_detection.as_mut().unwrap();
        match mutation {
            0 => e.piano_audio = None,
            1 => e.piano_audio.as_mut().unwrap().gain = 4.,
            2 => e.piano_audio.as_mut().unwrap().source_frames += 1,
            3 => e
                .piano_audio
                .as_mut()
                .unwrap()
                .contributing_stem_sha256
                .clear(),
            4 => e.method = KeyboardSound::Piano.method().into(),
            5 => e.version = 1,
            _ => e.sound = KeyboardSound::Synth,
        }
        assert!(t.validate().is_err(), "mutation {mutation}");
    }
}
#[test]
fn piano_recovery_requires_corroboration_not_an_isolated_leaked_chord() {
    use keyboard_detection::{PianoIndex, PianoView, piano_selected_indices as select};
    let n = |start: f64, pitch: u8, confidence: f32| PitchNote {
        start,
        end: start + 0.2,
        pitch,
        confidence,
    };
    let residual = vec![n(1.1, 72, 0.4), n(1.2, 61, 0.9), n(4., 60, 0.9)];
    assert!(select(&[], &residual).is_empty());
    let chord = vec![n(1., 60, 0.8), n(1.005, 64, 0.9), n(1.01, 67, 0.9)];
    assert!(
        select(&chord, &residual)
            .iter()
            .all(|i| i.view == PianoView::Baseline)
    );
    let baseline = vec![n(1., 60, 0.8), n(1.4, 64, 0.7), n(1.6, 67, 0.4)];
    let selected = select(&baseline, &residual);
    assert!(selected.contains(&PianoIndex {
        view: PianoView::Residual,
        index: 0
    }));
    assert!(!selected.contains(&PianoIndex {
        view: PianoView::Residual,
        index: 1
    }));
    assert!(!selected.contains(&PianoIndex {
        view: PianoView::Residual,
        index: 2
    }));
    assert!(selected.contains(&PianoIndex {
        view: PianoView::Baseline,
        index: 0
    }));
    assert!(selected.contains(&PianoIndex {
        view: PianoView::Baseline,
        index: 1
    }));
    assert!(!selected.contains(&PianoIndex {
        view: PianoView::Baseline,
        index: 2
    }));
    let aligned = vec![n(1.02, 72, 0.4)];
    assert!(!select(&baseline, &aligned).contains(&PianoIndex {
        view: PianoView::Baseline,
        index: 0
    }));
}
fn multiview_fixture() -> Transcription {
    use keyboard_detection::{PianoView, PianoViews};
    let mut t = residual_fixture();
    let manifest = t.isolation.as_mut().unwrap();
    manifest.stems.push(StemAsset {
        id: StemId::Piano,
        path: "/offline/piano.wav".into(),
        sha256: "d".repeat(64),
    });
    let mut second = note();
    second.start = 1.1;
    second.pitch = 64;
    let baseline = chart_engine::piano_model::Output {
        method: chart_engine::piano_model::METHOD.into(),
        notes: vec![note(), second],
        velocities: vec![80, 90],
        pedals: vec![],
    };
    let mut residual = baseline.clone();
    residual.notes[0].pitch = 72;
    let indices = keyboard_detection::piano_selected_indices(&baseline.notes, &residual.notes);
    let views = PianoViews {
        baseline,
        residual,
        selection_policy: keyboard_detection::PIANO_SELECTION.into(),
        selected_indices: indices,
    };
    let e = t.keyboard_detection.as_mut().unwrap();
    e.version = 6;
    e.method = keyboard_detection::piano_multiview_method();
    e.stem_sha256.insert("piano".into(), "d".repeat(64));
    e.raw_notes = views
        .selected_indices
        .iter()
        .map(|i| match i.view {
            PianoView::Baseline => views.baseline.notes[i.index].clone(),
            PianoView::Residual => views.residual.notes[i.index].clone(),
        })
        .collect();
    e.velocities = views
        .selected_indices
        .iter()
        .map(|i| match i.view {
            PianoView::Baseline => views.baseline.velocities[i.index],
            PianoView::Residual => views.residual.velocities[i.index],
        })
        .collect();
    e.piano_multiview = Some(views);
    t.piano = e.raw_notes.clone();
    t.pro_piano = e.raw_notes.clone();
    t
}
#[test]
fn piano_v6_retains_both_raw_views_and_rejects_changed_selection_or_binding() {
    let t = multiview_fixture();
    t.validate().unwrap();
    let restored: Transcription = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
    restored.validate().unwrap();
    for mutation in 0..8 {
        let mut t = multiview_fixture();
        let e = t.keyboard_detection.as_mut().unwrap();
        match mutation {
            0 => e.piano_multiview = None,
            1 => e.piano_multiview.as_mut().unwrap().selected_indices.clear(),
            2 => e.piano_multiview.as_mut().unwrap().baseline.notes[0].confidence = f32::NAN,
            3 => e.piano_multiview.as_mut().unwrap().selection_policy = "union".into(),
            4 => {
                e.stem_sha256.remove("piano");
            }
            5 => e.piano_multiview.as_mut().unwrap().residual.velocities[0] = 1,
            6 => e.piano_multiview.as_mut().unwrap().selected_indices[0].index = 999,
            _ => e.version = 4,
        }
        assert!(t.validate().is_err(), "mutation {mutation}");
    }
}
#[test]
fn piano_v6_one_residual_cannot_erase_two_fifty_ms_baseline_repeats() {
    use keyboard_detection::{PianoIndex, PianoView};
    let n = |start| PitchNote {
        start,
        end: start + 0.2,
        pitch: 60,
        confidence: 0.9,
    };
    let baseline = vec![n(1.), n(1.05)];
    let residual = vec![n(1.025)];
    let before = serde_json::to_vec(&(&baseline, &residual)).unwrap();
    let selected = keyboard_detection::piano_selected_indices(&baseline, &residual);
    assert_eq!(selected.len(), 2);
    assert!(selected.contains(&PianoIndex {
        view: PianoView::Residual,
        index: 0
    }));
    assert!(selected.contains(&PianoIndex {
        view: PianoView::Baseline,
        index: 1
    }));
    assert_eq!(serde_json::to_vec(&(&baseline, &residual)).unwrap(), before);
}
#[test]
fn piano_v6_interval_matching_is_maximum_cardinality_against_exhaustive_assignments() {
    use keyboard_detection::PianoView;
    fn best(b: &[f64], r: &[f64], i: usize, used: u8) -> usize {
        if i == b.len() {
            return 0;
        }
        let mut result = best(b, r, i + 1, used);
        for (j, &time) in r.iter().enumerate() {
            if used & (1 << j) == 0 && (b[i] - time).abs() <= 0.05 {
                result = result.max(1 + best(b, r, i + 1, used | (1 << j)));
            }
        }
        result
    }
    let n = |start, pitch| PitchNote {
        start,
        end: start + 0.2,
        pitch,
        confidence: 0.9,
    };
    for bm in 0..16 {
        for rm in 0..16 {
            let times = [1., 1.04, 1.08, 1.12];
            let b: Vec<_> = times
                .iter()
                .enumerate()
                .filter(|(i, _)| bm & (1 << i) != 0)
                .map(|(_, &t)| t)
                .collect();
            let r: Vec<_> = times
                .iter()
                .enumerate()
                .filter(|(i, _)| rm & (1 << i) != 0)
                .map(|(_, &t)| t)
                .collect();
            let mut baseline: Vec<_> = b.iter().map(|&t| n(t, 60)).collect();
            // Two unrelated attack groups qualify the local phrase without
            // supplying a matching pitch class or affecting reference counts.
            baseline.extend([n(0.5, 61), n(0.7, 61)]);
            let residual: Vec<_> = r.iter().map(|&t| n(t, 72)).collect();
            for reverse in [false, true] {
                if reverse {
                    baseline.reverse();
                }
                let selected = keyboard_detection::piano_selected_indices(&baseline, &residual);
                let retained = selected
                    .iter()
                    .filter(|i| i.view == PianoView::Baseline && baseline[i.index].pitch == 60)
                    .count();
                assert_eq!(
                    b.len() - retained,
                    best(&b, &r, 0, 0),
                    "baseline mask {bm}, residual mask {rm}, reverse {reverse}"
                );
            }
        }
    }
}

#[test]
fn piano_v6_time_slice_optimization_preserves_dense_boundary_and_unsorted_selections() {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for transpose in [-24i16, -12, 0, 12, 24] {
        let mut baseline = Vec::new();
        let mut residual = Vec::new();
        for i in 0..96 {
            let start = 3. + (i / 3) as f64 * 0.1 + [0., 0.02, 0.05][i % 3];
            let pitch = (60 + transpose + [0, 4, 7, 12][i % 4]) as u8;
            baseline.push(PitchNote {
                start,
                end: start + 0.2,
                pitch,
                confidence: [0.65, 0.649, 0.9][i % 3],
            });
            for delta in [-1.5, -0.75, -0.05, 0., 0.05, 0.75, 1.5] {
                let edge: f64 = start + delta;
                for bits in [edge.to_bits() - 1, edge.to_bits(), edge.to_bits() + 1] {
                    let at = f64::from_bits(bits);
                    residual.push(PitchNote {
                        start: at,
                        end: at + 0.2,
                        pitch,
                        confidence: 0.4,
                    });
                }
            }
        }
        for order in 0..4 {
            let (mut b, mut r) = (baseline.clone(), residual.clone());
            match order {
                1 => {
                    b.reverse();
                    r.reverse();
                }
                2 => {
                    b.rotate_left(17);
                    r.rotate_left(37);
                }
                3 => {
                    b.sort_by_key(|n| n.pitch);
                    r.sort_by_key(|n| n.pitch);
                }
                _ => {}
            }
            let indices = keyboard_detection::piano_selected_indices(&b, &r);
            digest.update(serde_json::to_vec(&indices).unwrap());
        }
    }
    // Captured from the frozen pre-optimization v6 implementation. Covers
    // exact and adjacent-f64 inclusive thresholds, every selected raw index,
    // input reordering, weak attacks and pitch-class transpositions.
    assert_eq!(
        format!("{:x}", digest.finalize()),
        "1e0cb33ef7ed6fab6dc7be3bd2b9ea317c7e927d649f132485b2bc11592c5b0a"
    );
}
#[test]
fn piano_v5_saved_many_to_one_selection_remains_valid_and_cannot_be_relabelled_v6() {
    use keyboard_detection::{PianoIndex, PianoView};
    let mut t = multiview_fixture();
    let e = t.keyboard_detection.as_mut().unwrap();
    e.version = 5;
    e.method = format!(
        "{};{}",
        keyboard_detection::piano_method(),
        keyboard_detection::PIANO_SELECTION_V5
    );
    let views = e.piano_multiview.as_mut().unwrap();
    views.baseline.notes = vec![
        PitchNote {
            start: 1.,
            end: 1.2,
            pitch: 60,
            confidence: 0.9,
        },
        PitchNote {
            start: 1.05,
            end: 1.25,
            pitch: 60,
            confidence: 0.9,
        },
    ];
    views.residual.notes = vec![PitchNote {
        start: 1.025,
        end: 1.225,
        pitch: 60,
        confidence: 0.9,
    }];
    views.residual.velocities = vec![80];
    views.selection_policy = keyboard_detection::PIANO_SELECTION_V5.into();
    views.selected_indices = vec![PianoIndex {
        view: PianoView::Residual,
        index: 0,
    }];
    e.raw_notes = views.residual.notes.clone();
    e.velocities = views.residual.velocities.clone();
    t.piano = e.raw_notes.clone();
    t.pro_piano = e.raw_notes.clone();
    t.validate().unwrap();
    let restored: Transcription = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
    restored.validate().unwrap();
    let e = t.keyboard_detection.as_mut().unwrap();
    e.version = 6;
    e.method = keyboard_detection::piano_multiview_method();
    e.piano_multiview.as_mut().unwrap().selection_policy =
        keyboard_detection::PIANO_SELECTION.into();
    assert!(
        t.validate().is_err(),
        "old suppressed repeat must not pass v6 validation"
    );
}
#[test]
#[ignore = "requires YACE_V5_RECEIPT pointing to a saved local v5 production receipt"]
fn actual_saved_piano_v5_receipt_validates_without_model_or_audio_access() {
    let path = std::env::var("YACE_V5_RECEIPT").expect("set YACE_V5_RECEIPT");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let t: Transcription = serde_json::from_value(saved["transcription"].clone()).unwrap();
    assert_eq!(t.keyboard_detection.as_ref().unwrap().version, 5);
    t.validate().unwrap();
}
#[test]
fn changed_source_model_stem_scores_raw_notes_and_source_clock_are_rejected() {
    for sound in [KeyboardSound::Piano, KeyboardSound::Synth] {
        for mutation in 0..12 {
            let mut t = fixture(sound);
            let e = t.keyboard_detection.as_mut().unwrap();
            match mutation {
                0 => e.source_audio_sha256 = "b".repeat(64),
                1 => e.model_sha256 = "b".repeat(64),
                2 => {
                    e.stem_sha256.insert("piano".into(), "b".repeat(64));
                }
                3 => e.method = "unspecified".into(),
                4 => e.score_semantics = "probability of correctness".into(),
                5 => e.raw_notes[0].end = 3.,
                6 => t.piano[0].pitch = 61,
                7 => e.implementation_sha256 = "missing".into(),
                8 => {
                    e.decoder_parameters.insert("onset_threshold".into(), 0.99);
                }
                9 => e.raw_notes[0].confidence = f32::NAN,
                10 => e.duration_seconds = 3.,
                _ => t.isolation = None,
            }
            assert!(
                t.validate().is_err(),
                "accepted mutation{mutation} for{sound:?}"
            );
        }
    }
}
#[test]
fn velocities_and_pedal_events_remain_separate_and_source_bounded() {
    let mut t = fixture(KeyboardSound::Piano);
    t.keyboard_detection
        .as_mut()
        .unwrap()
        .pedals
        .push(chart_engine::piano_model::Pedal {
            start: 0.4,
            end: 2.,
        });
    t.validate().unwrap();
    assert_eq!(t.pro_piano[0].end, 1.8);
    t.keyboard_detection.as_mut().unwrap().velocities.clear();
    assert!(t.validate().is_err());
    let mut t = fixture(KeyboardSound::Piano);
    t.keyboard_detection
        .as_mut()
        .unwrap()
        .pedals
        .push(chart_engine::piano_model::Pedal {
            start: 0.4,
            end: 2.1,
        });
    assert!(t.validate().is_err());
    let mut t = fixture(KeyboardSound::Synth);
    t.keyboard_detection
        .as_mut()
        .unwrap()
        .basic_pitch_candidates[0]
        .onset_score = f32::NAN;
    assert!(t.validate().is_err());
}

#[test]
#[ignore = "requires verified local Kong model, FFmpeg and prepared piano benchmark fixture"]
fn native_piano_production_pipeline_preserves_evidence_and_builds_playable_drafts() {
    use chart_engine::{
        pipeline::{self, IsolationOptions},
        timing::TempoMap,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("output/keyboard-benchmark-20261027/corpus/fixture-piano/mix.wav");
    let temporary = tempfile::tempdir().unwrap();
    let piano = temporary.path().join("piano.wav");
    std::fs::copy(&source, &piano).unwrap();
    let silence = temporary.path().join("other.wav");
    let pcm = models::decode(&source, 44100, 1).unwrap();
    let zero = vec![0.; pcm.len()];
    models::write_audio(&silence, &zero, &zero).unwrap();
    let mut assets = BTreeMap::from([(StemId::Piano, piano), (StemId::Other, silence.clone())]);
    for id in [StemId::Drums, StemId::Bass, StemId::Vocals] {
        let path = temporary.path().join(format!("{}.wav", id.as_str()));
        std::fs::copy(&silence, &path).unwrap();
        assets.insert(id, path);
    }
    let manifest=StemManifest::build(&source,assets,AlignmentEvidence{method:"Prepared Rust/FluidSynth piano fixture".into(),details:"Piano equals standalone synthesized source clip; remaining stems are equal-length silence. Validates native production integration, not real-recording separation or musical accuracy.".into()}).unwrap();
    let manifest_path = temporary.path().join("isolation.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let models = root.join("models");
    {
        let sound = KeyboardSound::Piano;
        let transcription = pipeline::transcribe_with_isolation(
            &source,
            &models,
            IsolationOptions {
                manifest: Some(&manifest_path),
                keyboard_sound: sound,
                ..Default::default()
            },
            &["piano".into(), "pro_piano".into()],
        )
        .unwrap();
        transcription.validate().unwrap();
        assert!(!transcription.piano.is_empty());
        assert_eq!(transcription.piano.len(), transcription.pro_piano.len());
        let evidence = transcription.keyboard_detection.as_ref().unwrap();
        assert_eq!(evidence.sound, sound);
        assert_eq!(evidence.raw_notes.len(), transcription.piano.len());
        assert_eq!(evidence.model_id, sound.model_id());
        assert!(
            evidence
                .raw_notes
                .iter()
                .all(|n| n.end <= pcm.len() as f64 / 44100. + 1. / 22050.)
        );
        let output = temporary.path().join(format!("{sound:?}"));
        pipeline::build(pipeline::Build {
            audio: &source,
            output: &output,
            existing: None,
            timing: Some(TempoMap::constant(120.).unwrap()),
            transcription: &transcription,
            meta: pipeline::Metadata {
                name: "Native keyboard pipeline fixture".into(),
                artist: "Rust synthesis verification".into(),
                ..pipeline::metadata(&source).unwrap()
            },
            pro: true,
            lookup_lyrics: false,
        })
        .unwrap();
        assert!(output.join("notes.mid").is_file());
        let saved: Transcription = serde_json::from_slice(
            &std::fs::read(output.join("chart-engine-transcription.json")).unwrap(),
        )
        .unwrap();
        saved.validate().unwrap();
        assert_eq!(
            saved.keyboard_detection.unwrap().raw_notes.len(),
            evidence.raw_notes.len()
        );
        eprintln!(
            "Native production {sound:?}: {} raw notes, bound evidence, playable MIDI",
            evidence.raw_notes.len()
        );
    }
}

fn mu_fixture() -> Transcription {
    use chart_engine::muscriptor::{self, ChunkTrace, NoteDecoder, Options, Output};
    use sha2::{Digest, Sha256};
    let tokens = vec![
        1134,
        53,
        1135 + 29,
        1133,
        1004 + 60,
        1135 + 48,
        1004 + 64,
        1135 + 33,
        1004 + 48,
        1265 + 36,
        153,
        1132,
        1135 + 29,
        1004 + 60,
        1135 + 48,
        1004 + 64,
        1135 + 33,
        1004 + 48,
        1,
    ];
    let mut decoder = NoteDecoder::new();
    decoder.begin_chunk(0., 2.).unwrap();
    for &token in tokens.iter().take_while(|t| **t != 1) {
        decoder.feed(token);
    }
    let (notes, diagnostics) = decoder.finish(2.);
    let assets: BTreeMap<_, _> = muscriptor::assets()
        .into_iter()
        .map(|a| (a.filename, a.sha256))
        .collect();
    let output = Output {
        method: muscriptor::METHOD.into(),
        duration_seconds: 2.,
        options: Options {
            instrument_groups: vec![32],
            ..Default::default()
        },
        model_assets: assets.clone(),
        notes,
        chunks: vec![ChunkTrace {
            start: 0.,
            end: 2.,
            instrument_ids: vec![34],
            forced_tokens: vec![],
            generated_tokens: tokens,
            emitted_eos: true,
            reached_token_limit: false,
            elapsed_seconds: 0.1,
        }],
        diagnostics,
    };
    let indices = keyboard_detection::synth_selected_indices(&output.notes);
    let selected: Vec<_> = indices
        .iter()
        .map(|&i| {
            let n = &output.notes[i];
            PitchNote {
                start: n.start,
                end: n.end,
                pitch: n.pitch,
                confidence: 1.,
            }
        })
        .collect();
    let mut t = fixture(KeyboardSound::Synth);
    let e = t.keyboard_detection.as_mut().unwrap();
    e.version = 3;
    e.model_id = "muscriptor-small".into();
    e.model_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&assets).unwrap()));
    e.method = muscriptor::METHOD.into();
    e.score_semantics = keyboard_detection::SYNTH_PRIORITY.into();
    e.stem_sha256.clear();
    e.decoder_parameters = BTreeMap::from([("nominal_arrangement_priority".into(), 1.)]);
    e.raw_notes = selected.clone();
    e.basic_pitch_candidates.clear();
    e.muscriptor = Some(keyboard_detection::SynthEvidence {
        output,
        selection_policy: keyboard_detection::SYNTH_SELECTION.into(),
        selected_indices: indices,
    });
    t.piano = selected.clone();
    t.pro_piano = selected;
    t
}
#[test]
fn muscriptor_v2_retains_all_raw_voices_but_selects_general_lead_harmony_without_confidence() {
    let t = mu_fixture();
    t.validate().unwrap();
    let e = t.keyboard_detection.as_ref().unwrap();
    let m = e.muscriptor.as_ref().unwrap();
    assert_eq!(m.output.notes.len(), 4);
    assert_eq!(e.raw_notes.len(), 2);
    assert_eq!(
        m.selected_indices
            .iter()
            .map(|&i| m.output.notes[i].program)
            .collect::<Vec<_>>(),
        [29, 48]
    );
    assert!(e.raw_notes.iter().all(|n| n.confidence == 1.));
    assert!(e.score_semantics.contains("not a model confidence"));
    let json = serde_json::to_value(&t).unwrap();
    assert!(
        json["keyboard_detection"]["muscriptor"]["output"]["notes"][0]
            .get("confidence")
            .is_none()
    );
    let restored: Transcription = serde_json::from_value(json).unwrap();
    restored.validate().unwrap();
    let all: Vec<_> = (0..=129)
        .map(|program| chart_engine::muscriptor::Note {
            start: 0.,
            end: 1.,
            pitch: 60,
            program,
            instrument: chart_engine::muscriptor::instrument_name(program),
            is_drum: program == 128,
            release: chart_engine::muscriptor::Release::Explicit,
        })
        .collect();
    let indices = keyboard_detection::synth_selected_indices(&all);
    assert!(
        indices.contains(&29)
            && indices.contains(&43)
            && indices.contains(&61)
            && indices.contains(&80)
            && indices.contains(&88)
    );
    assert!((32..=39).all(|p| !indices.contains(&p)));
    assert!(!indices.contains(&128));
}
#[test]
fn muscriptor_v2_mutations_reject_substituted_models_tokens_source_selection_and_fake_confidence() {
    for mutation in 0..17 {
        let mut t = mu_fixture();
        let e = t.keyboard_detection.as_mut().unwrap();
        let m = e.muscriptor.as_mut().unwrap();
        match mutation {
            0 => e.source_audio_sha256 = "f".repeat(64),
            1 => e.model_sha256 = "f".repeat(64),
            2 => {
                m.output
                    .model_assets
                    .insert("muscriptor/muscriptor-small-lm.onnx".into(), "f".repeat(64));
            }
            3 => m.output.options.instrument_groups = vec![29],
            4 => m.selected_indices.push(1),
            5 => m.selected_indices = vec![usize::MAX],
            6 => m.selection_policy = "program29-only".into(),
            7 => e.raw_notes[0].confidence = 0.7,
            8 => m.output.notes[0].release = chart_engine::muscriptor::Release::SourceEndUnknown,
            9 => m.output.chunks[0].generated_tokens[2] += 1,
            10 => m.output.chunks[0].emitted_eos = false,
            11 => m.output.chunks[0].generated_tokens.insert(0, 1),
            12 => m.output.chunks[0].forced_tokens = vec![1134],
            13 => m.output.diagnostics.ignored_special_tokens += 1,
            14 => m.output.duration_seconds = 3.,
            15 => e.score_semantics = "model confidence 100%".into(),
            _ => e.sound = KeyboardSound::Piano,
        }
        assert!(
            t.validate().is_err(),
            "accepted MuScriptor mutation{mutation}"
        );
    }
}
#[test]
fn changed_original_source_is_rejected_before_any_synth_model_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("changed.wav");
    std::fs::write(&path, b"changed-source").unwrap();
    let t = mu_fixture();
    let error = keyboard_detection::detect(
        &path,
        t.isolation.as_ref().unwrap(),
        temp.path(),
        KeyboardSound::Synth,
    )
    .unwrap_err();
    assert!(error.to_string().contains("source differs"), "{error:#}");
}
#[test]
#[ignore = "requires pinned local MuScriptor assets and approved Final Countdown benchmark clip"]
fn native_approved_synth_clip_transcribes_exports_and_reloads_full_evidence() {
    use chart_engine::{
        pipeline::{self, IsolationOptions},
        timing::TempoMap,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let clip = root.join("output/keyboard-benchmark-20261027/corpus/final-countdown-0");
    let source = clip.join("mix.wav");
    let output = root.join("output/keyboard-benchmark-20261027/muscriptor-production-validation");
    std::fs::create_dir_all(&output).unwrap();
    let mut stems = BTreeMap::new();
    for id in [StemId::Piano, StemId::Other] {
        // Benchmark excerpts are44.1kHz mono. Explicitly duplicate channels
        // without resampling, trimming or shifting before review attachment.
        let samples = models::decode(&clip.join(format!("{}.wav", id.as_str())), 44100, 1).unwrap();
        let path = output.join(format!("{}-stereo.wav", id.as_str()));
        if !path.exists() {
            models::write_audio(&path, &samples, &samples).unwrap();
        } else {
            assert_eq!(models::decode(&path, 44100, 1).unwrap(), samples);
        }
        stems.insert(id, path);
    }
    let manifest=StemManifest::build(&source,stems,AlignmentEvidence{method:"Prepared source-bound benchmark excerpt; explicit channel duplication".into(),details:"Original mix, piano and other are the same16s source-clock excerpt used in the approved listening comparison. Mono44.1kHz review stems have channels duplicated without resampling/trim/shift. MuScriptor reads only the unchanged full mix.".into()}).unwrap();
    let manifest_path = output.join("isolation.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let transcription = pipeline::transcribe_with_isolation(
        &source,
        &root.join("models"),
        IsolationOptions {
            manifest: Some(&manifest_path),
            keyboard_sound: KeyboardSound::Synth,
            ..Default::default()
        },
        &["piano".into(), "pro_piano".into()],
    )
    .unwrap();
    transcription.validate().unwrap();
    let e = transcription.keyboard_detection.as_ref().unwrap();
    let m = e.muscriptor.as_ref().unwrap();
    assert_eq!(e.version, 3);
    assert!(!e.raw_notes.is_empty());
    assert!(
        m.output
            .chunks
            .iter()
            .all(|c| c.emitted_eos && !c.reached_token_limit)
    );
    assert_eq!(m.output.diagnostics.missing_release_at_eof, 0);
    std::fs::write(
        output.join("events.json"),
        serde_json::to_vec_pretty(&transcription).unwrap(),
    )
    .unwrap();
    let song = output.join("song");
    pipeline::build(pipeline::Build {
        audio: &source,
        output: &song,
        existing: None,
        timing: Some(TempoMap::constant(118.).unwrap()),
        transcription: &transcription,
        meta: pipeline::Metadata {
            name: "Final Countdown synth production check".into(),
            artist: "Europe".into(),
            ..pipeline::metadata(&source).unwrap()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    assert!(song.join("notes.mid").is_file());
    let saved: Transcription = serde_json::from_slice(
        &std::fs::read(song.join("chart-engine-transcription.json")).unwrap(),
    )
    .unwrap();
    saved.validate().unwrap();
    assert_eq!(
        saved
            .keyboard_detection
            .as_ref()
            .unwrap()
            .muscriptor
            .as_ref()
            .unwrap()
            .selected_indices,
        m.selected_indices
    );
    let project = output.join("project");
    let store = chart_engine::editor::Store::create(&song, None, &project).unwrap();
    assert!(!store.load().unwrap().content.tracks["ExpertProKeys"].is_empty());
    eprintln!(
        "MuScriptor production: {} rawvoices, {} selected, playable MIDI+reloaded evidence+editor",
        m.output.notes.len(),
        e.raw_notes.len()
    );
}

fn synth_note(
    start: f64,
    pitch: u8,
    program: u16,
    duration: f64,
) -> chart_engine::muscriptor::Note {
    chart_engine::muscriptor::Note {
        start,
        end: start + duration,
        pitch,
        program,
        instrument: chart_engine::muscriptor::instrument_name(program),
        is_drum: program == 128,
        release: chart_engine::muscriptor::Release::Explicit,
    }
}
#[test]
fn suspect_bass_labels_retain_coherent_upper_phrases_without_retaining_low_bass() {
    let mut notes = vec![
        synth_note(0.1, 35, 33, 0.2),
        synth_note(0.3, 43, 32, 0.2),
        synth_note(0.5, 47, 39, 0.2),
    ];
    for (i, pitch) in [53, 60, 62, 59, 60].into_iter().enumerate() {
        notes.push(synth_note(
            i as f64 * 0.2,
            pitch,
            if i % 2 == 0 { 32 } else { 39 },
            0.15,
        ));
    }
    assert_eq!(
        keyboard_detection::synth_selected_indices(&notes),
        (3..8).collect::<Vec<_>>()
    );
    let mut reversed = notes.clone();
    reversed.reverse();
    let selected = keyboard_detection::synth_selected_indices(&reversed);
    let mut actual: Vec<_> = selected
        .iter()
        .map(|&i| {
            (
                reversed[i].pitch,
                (reversed[i].start * 1000.).round() as u64,
            )
        })
        .collect();
    actual.sort();
    let mut expected: Vec<_> = notes[3..]
        .iter()
        .map(|n| (n.pitch, (n.start * 1000.).round() as u64))
        .collect();
    expected.sort();
    assert_eq!(actual, expected);
}
#[test]
fn bass_rescue_requires_phrase_evidence_not_a_high_note_or_chord_flash() {
    let notes = vec![
        synth_note(0., 60, 33, 0.2),
        synth_note(0., 64, 33, 0.2),
        synth_note(0., 67, 33, 0.2),
        synth_note(2., 62, 33, 0.2),
        synth_note(4., 65, 33, 0.2),
        synth_note(4.5, 72, 33, 0.01),
    ];
    assert!(keyboard_detection::synth_selected_indices(&notes).is_empty());
    let notes: Vec<_> = (0..10)
        .map(|i| synth_note(i as f64 * 0.1, 40 + i % 8, 33, 0.08))
        .collect();
    assert!(keyboard_detection::synth_selected_indices(&notes).is_empty());
}
#[test]
fn bass_rescue_preserves_fifty_ms_repeats_in_a_supported_phrase_and_other_labels() {
    let mut notes: Vec<_> = (0..8)
        .map(|i| synth_note(i as f64 * 0.05, 60, 35, 0.05))
        .collect();
    notes.push(synth_note(0., 30, 29, 0.02));
    notes.push(synth_note(0., 80, 128, 0.1));
    assert_eq!(
        keyboard_detection::synth_selected_indices(&notes),
        (0..9).collect::<Vec<_>>()
    );
}
#[test]
fn v2_historical_bass_selection_remains_valid_when_v3_would_rescue_the_phrase() {
    use chart_engine::muscriptor;
    let mut t = mu_fixture();
    let e = t.keyboard_detection.as_mut().unwrap();
    let m = e.muscriptor.as_mut().unwrap();
    let tokens = vec![
        1134,
        1135 + 33,
        1133,
        13,
        1004 + 60,
        23,
        1132,
        1004 + 60,
        33,
        1133,
        1004 + 62,
        43,
        1132,
        1004 + 62,
        53,
        1133,
        1004 + 64,
        63,
        1132,
        1004 + 64,
        1,
    ];
    let mut decoder = muscriptor::NoteDecoder::new();
    decoder.begin_chunk(0., 2.).unwrap();
    for &token in tokens.iter().take_while(|t| **t != 1) {
        decoder.feed(token);
    }
    let (notes, diagnostics) = decoder.finish(2.);
    m.output.notes = notes;
    m.output.diagnostics = diagnostics;
    m.output.chunks[0].generated_tokens = tokens;
    assert_eq!(
        keyboard_detection::synth_selected_indices(&m.output.notes),
        [0, 1, 2]
    );
    e.version = 2;
    m.selection_policy = keyboard_detection::SYNTH_SELECTION_V2.into();
    m.selected_indices.clear();
    e.raw_notes.clear();
    t.piano.clear();
    t.pro_piano.clear();
    t.validate().unwrap();
    let restored: Transcription = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
    restored.validate().unwrap();
    t.keyboard_detection.as_mut().unwrap().version = 3;
    assert!(t.validate().is_err());
}
#[test]
#[ignore = "requires preserved local approved C/D MuScriptor receipts; no neural inference"]
fn approved_final_countdown_c_and_d_selected_indices_remain_identical() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for kind in ["unconditional", "lead-conditioned"] {
        let path = root.join(format!(
            "output/keyboard-benchmark-20261027/muscriptor-smoke/final-countdown-0-mix-{kind}.json"
        ));
        let output: chart_engine::muscriptor::Output =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let old: Vec<_> = output
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.is_drum && n.program != 128 && !(32..=39).contains(&n.program))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            keyboard_detection::synth_selected_indices(&output.notes),
            old,
            "approved{kind} changed"
        );
    }
}
