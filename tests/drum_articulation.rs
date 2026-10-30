use chart_engine::{
    drum_articulation,
    mapping::{DrumHit, Voice},
};
const RATE: usize = 22050;
fn tom(audio: &mut [f32], time: f64, freq: f64, gain: f64) {
    for i in 0..(RATE as f64 * 0.25) as usize {
        let t = i as f64 / RATE as f64;
        audio[(time * RATE as f64) as usize + i] += (gain
            * (-t / 0.11).exp()
            * ((std::f64::consts::TAU * freq * t).sin()
                + 0.20 * (std::f64::consts::TAU * freq * 1.59 * t).sin()))
            as f32;
    }
}
fn tom_kit(freqs: &[f64], gain: f64) -> (Vec<f32>, Vec<DrumHit>) {
    let mut audio = vec![0.; RATE * (freqs.len() + 1)];
    let mut hits = vec![];
    for (i, f) in freqs.iter().enumerate() {
        let time = i as f64 + 0.1;
        tom(&mut audio, time, *f, gain);
        hits.push(DrumHit {
            time,
            voice: Voice::Tom,
            confidence: 0.9,
        });
    }
    (audio, hits)
}
#[test]
fn repeated_source_resonances_define_relative_toms_without_moving_attacks() {
    let freqs = [
        95., 155., 245., 95., 155., 245., 95., 155., 245., 95., 155., 245.,
    ];
    let (audio, hits) = tom_kit(&freqs, 0.3);
    let mut evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    // Controlled known-truth experiment, not the production policy.
    for e in &mut evidence {
        e.map_proposal = e.proposed.is_some();
    }
    let proposed = drum_articulation::proposed_hits(&hits, &evidence);
    for (i, (raw, after)) in hits.iter().zip(&proposed).enumerate() {
        assert_eq!(raw.time, after.time);
        assert_eq!(raw.confidence, after.confidence);
        assert_eq!(
            after.voice,
            [Voice::LowTom, Voice::MidTom, Voice::HighTom][i % 3]
        );
    }
    let (quiet, _) = tom_kit(&freqs, 0.03);
    let quiet = drum_articulation::analyze(&quiet, &hits).unwrap();
    assert_eq!(
        evidence.iter().map(|e| e.proposed).collect::<Vec<_>>(),
        quiet.iter().map(|e| e.proposed).collect::<Vec<_>>()
    );
    let (audio, hits) = tom_kit(&[95., 155., 95., 155., 95., 155.], 0.3);
    assert!(
        drum_articulation::analyze(&audio, &hits)
            .unwrap()
            .iter()
            .all(|e| e.proposed.is_none()),
        "Two observed resonances do not identify the missing physical kit register"
    );
}
#[test]
fn one_tom_cannot_manufacture_a_full_kit_and_overlaps_abstain() {
    let (audio, mut hits) = tom_kit(&[155.; 12], 0.3);
    assert!(
        drum_articulation::analyze(&audio, &hits)
            .unwrap()
            .iter()
            .all(|e| e.proposed.is_none())
    );
    hits.push(DrumHit {
        time: hits[0].time,
        voice: Voice::Kick,
        confidence: 0.9,
    });
    let mut evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    for e in &mut evidence {
        e.map_proposal = e.proposed.is_some();
    }
    assert_eq!(evidence[0].reason, "overlapping_same_band_attack");
    assert!(evidence[0].proposed.is_none());
}
#[test]
fn silence_invalid_pcm_and_changed_hit_identity_are_not_articulations() {
    let (mut audio, hits) = tom_kit(&[95., 155., 245., 95., 155., 245., 95., 155., 245.], 0.3);
    let evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    let mut changed = hits.clone();
    changed[0].time += 0.01;
    assert!(drum_articulation::validate(&evidence, &changed).is_err());
    assert!(
        drum_articulation::analyze(&vec![0.; audio.len()], &hits)
            .unwrap()
            .iter()
            .all(|e| e.proposed.is_none())
    );
    audio[0] = f32::NAN;
    assert!(drum_articulation::analyze(&audio, &hits).is_err());
}

fn cymbal_kit(mixed: bool) -> (Vec<f32>, Vec<DrumHit>) {
    let mut audio = vec![0.; RATE * 13];
    let mut hits = vec![];
    let mut noise = 17u64;
    for hit in 0..12 {
        let time = hit as f64 + 0.1;
        hits.push(DrumHit {
            time,
            voice: Voice::Cymbal,
            confidence: 0.9,
        });
        for i in 0..RATE / 3 {
            let t = i as f64 / RATE as f64;
            noise = noise.wrapping_mul(6364136223846793005).wrapping_add(1);
            let x = if mixed && hit % 2 == 1 {
                (std::f64::consts::TAU * 4300. * t).sin()
                    + 0.7 * (std::f64::consts::TAU * 6217. * t).sin()
                    + 0.4 * (std::f64::consts::TAU * 8143. * t).sin()
            } else {
                ((noise >> 32) as u32 as f64 / u32::MAX as f64 - 0.5) * 2.
            };
            audio[(time * RATE as f64) as usize + i] = (0.2 * (-t / 0.18).exp() * x) as f32;
        }
    }
    (audio, hits)
}
#[test]
fn separated_broad_and_tonal_cymbals_need_two_populations() {
    let (audio, hits) = cymbal_kit(true);
    let evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    for (i, e) in evidence.iter().enumerate() {
        assert_eq!(
            e.proposed,
            Some(if i % 2 == 0 {
                Voice::Crash
            } else {
                Voice::Ride
            })
        );
    }
    let (audio, hits) = cymbal_kit(false);
    assert!(
        drum_articulation::analyze(&audio, &hits)
            .unwrap()
            .iter()
            .all(|e| e.proposed.is_none())
    );
}

#[test]
fn expert_relocation_cannot_merge_hands_or_change_other_difficulties() {
    let (audio, mut hits) = tom_kit(
        &[
            95., 155., 245., 95., 155., 245., 95., 155., 245., 95., 155., 245.,
        ],
        0.3,
    );
    for i in [2usize, 5, 8, 11] {
        hits.push(DrumHit {
            time: hits[i].time,
            voice: Voice::HiHat,
            confidence: 0.95,
        });
    }
    let mut evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    for e in &mut evidence {
        e.map_proposal = e.proposed.is_some();
    }
    let mut input = chart_engine::mapping::Transcription {
        drums: hits,
        ..Default::default()
    };
    let timing = chart_engine::timing::TempoMap::constant(120.).unwrap();
    for pro in [false, true] {
        let before = chart_engine::mapping::tracks(&input, &timing, pro).unwrap();
        input.drum_articulations = evidence.clone();
        let after = chart_engine::mapping::tracks(&input, &timing, pro).unwrap();
        for name in ["EasyDrums", "MediumDrums", "HardDrums"] {
            assert_eq!(before[name], after[name]);
        }
        let heads = |notes: &Vec<chart_engine::chart::Note>| {
            let mut counts = std::collections::BTreeMap::new();
            for n in notes.iter().filter(|n| n.lane <= 4) {
                *counts.entry(n.tick).or_insert(0usize) += 1;
            }
            counts
        };
        assert_eq!(heads(&before["ExpertDrums"]), heads(&after["ExpertDrums"]));
        assert!(after["ExpertDrums"].iter().any(|n| n.lane == 4)); // noncolliding low toms survive
        input.drum_articulations.clear();
    }
}

#[test]
fn evidence_rejects_invalid_family_measurements_duplicate_or_stale_source() {
    let (audio, hits) = tom_kit(&[95., 155., 245., 95., 155., 245., 95., 155., 245.], 0.3);
    let evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    for change in 0..5 {
        let mut bad = evidence.clone();
        match change {
            0 => bad[0].proposed = Some(Voice::Ride),
            1 => bad[0].spectral_flatness = Some(1.2),
            2 => bad[0].window_end = None,
            3 => bad[0].original_confidence = 0.1,
            _ => bad.push(bad[0].clone()),
        };
        assert!(drum_articulation::validate(&bad, &hits).is_err());
    }
    let manifest = chart_engine::stem_review::StemManifest {
        version: 1,
        source_audio_sha256: "a".repeat(64),
        sample_rate: 44100,
        channels: 2,
        frames: 44100 * 12,
        alignment: chart_engine::stem_review::AlignmentEvidence {
            method: "synthetic".into(),
            details: "Known generated source clock".into(),
        },
        stems: vec![chart_engine::stem_review::StemAsset {
            id: chart_engine::stem_review::StemId::Drums,
            path: "/synthetic/drums.wav".into(),
            sha256: "b".repeat(64),
        }],
        generation: None,
    };
    let source = drum_articulation::Source::from_manifest(&manifest).unwrap();
    drum_articulation::validate_source(Some(&source), &evidence, Some(&manifest)).unwrap();
    let mut changed = manifest.clone();
    changed.stems[0].sha256 = "c".repeat(64);
    assert!(drum_articulation::validate_source(Some(&source), &evidence, Some(&changed)).is_err());
    assert!(drum_articulation::validate_source(None, &evidence, Some(&manifest)).is_err());
    let mut changed = source;
    changed.source_frames = 1;
    assert!(
        drum_articulation::validate_source(Some(&changed), &evidence, Some(&manifest)).is_err()
    );
    let legacy: chart_engine::mapping::Transcription = serde_json::from_str("{}").unwrap();
    assert!(legacy.drum_articulations.is_empty() && legacy.drum_articulation_source.is_none());
}

#[test]
fn fast_fills_use_real_short_windows_and_never_borrow_the_next_hit() {
    let mut audio = vec![0.; RATE * 3];
    let mut hits = vec![];
    for i in 0..12 {
        let time = 0.1 + i as f64 * 0.1;
        tom(&mut audio, time, [95., 155., 245.][i % 3], 0.3);
        hits.push(DrumHit {
            time,
            voice: Voice::Tom,
            confidence: 0.9,
        });
    }
    let evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    for (index, e) in evidence.iter().take(11).enumerate() {
        assert_eq!(e.window_samples, Some(1024));
        assert!(e.window_end.unwrap() <= hits[index + 1].time);
        if let Some(hz) = e.resonance_hz
            && e.proposed.is_some()
        {
            assert!(hz * 1024. / RATE as f64 >= 3.5);
        }
    }
    assert!(evidence.iter().any(|e| e.resonance_hz.is_some()));
}

#[test]
fn relocation_cannot_replace_a_selected_tom_with_a_discarded_cymbal() {
    use chart_engine::{mapping, timing::TempoMap};
    let hits = vec![
        DrumHit {
            time: 0.5,
            voice: Voice::Snare,
            confidence: 0.95,
        },
        DrumHit {
            time: 0.5,
            voice: Voice::Tom,
            confidence: 0.60,
        },
        DrumHit {
            time: 0.5,
            voice: Voice::Cymbal,
            confidence: 0.80,
        },
    ];
    let mut evidence = drum_articulation::analyze(&vec![0.; RATE * 2], &hits).unwrap();
    let mut frames = vec![[0.; 8]; 201];
    frames[50][4] = 0.95;
    drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 2.).unwrap();
    assert!(evidence.iter().any(|e| e.map_proposal));
    let mut input = mapping::Transcription {
        drums: hits,
        ..Default::default()
    };
    let timing = TempoMap::constant(120.).unwrap();
    let before = mapping::tracks(&input, &timing, true).unwrap();
    input.drum_articulations = evidence;
    let after = mapping::tracks(&input, &timing, true).unwrap();
    assert_eq!(before["ExpertDrums"], after["ExpertDrums"]);
    let played = mapping::played_drum_articulations(&input, &timing);
    assert!(
        played
            .iter()
            .any(|(i, _, voice)| *i == 1 && *voice == Voice::Tom)
    );
    assert!(!played.iter().any(|(i, _, _)| *i == 2));
}

#[test]
fn short_tail_review_is_valid_when_chart_tick_rounds_after_raw_hit() {
    use chart_engine::{chart, mapping, review, timing::TempoMap};
    let hits = vec![DrumHit {
        time: 1.984,
        voice: Voice::Cymbal,
        confidence: 0.9,
    }];
    let evidence = drum_articulation::analyze(&vec![0.; RATE * 2], &hits).unwrap();
    assert!(evidence[0].window_end.is_none());
    let events = mapping::Transcription {
        drums: hits,
        drum_articulations: evidence,
        ..Default::default()
    };
    let timing = TempoMap::constant(120.).unwrap();
    let document = chart::Chart::parse(chart::new_chart(
        "Tail",
        "Fixture",
        &timing,
        &mapping::tracks(&events, &timing, true).unwrap(),
    ))
    .unwrap();
    review::generation_map(
        &document,
        &events,
        2.,
        Default::default(),
        ["ExpertDrums".into()].into_iter().collect(),
    )
    .unwrap();
}

#[test]
fn articulation_review_requires_a_played_generated_head() {
    use chart_engine::{chart, mapping, review, review_regions::Reason, timing::TempoMap};
    let hits = vec![
        DrumHit {
            time: 0.5,
            voice: Voice::Snare,
            confidence: 0.95,
        },
        DrumHit {
            time: 0.5,
            voice: Voice::HiHat,
            confidence: 0.9,
        },
        DrumHit {
            time: 0.5,
            voice: Voice::Cymbal,
            confidence: 0.8,
        },
    ];
    let audio = vec![0.; RATE * 2];
    let evidence = drum_articulation::analyze(&audio, &hits).unwrap();
    let mut input = mapping::Transcription {
        drums: hits,
        drum_articulations: evidence,
        ..Default::default()
    };
    let timing = TempoMap::constant(120.).unwrap();
    let build = |events: &mapping::Transcription| {
        chart::Chart::parse(chart::new_chart(
            "Synthetic review",
            "Fixture",
            &timing,
            &mapping::tracks(events, &timing, true).unwrap(),
        ))
        .unwrap()
    };
    let generated = ["ExpertDrums".into()].into_iter().collect();
    let document = build(&input);
    let map = review::generation_map(&document, &input, 2., Default::default(), generated).unwrap();
    assert!(
        !map.regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == Reason::DrumArticulation)
    );
    input.drums.remove(1);
    input.drum_articulations = drum_articulation::analyze(&audio, &input.drums).unwrap();
    let document = build(&input);
    let map = review::generation_map(
        &document,
        &input,
        2.,
        Default::default(),
        ["ExpertDrums".into()].into_iter().collect(),
    )
    .unwrap();
    let issue = map
        .regions
        .iter()
        .flat_map(|r| &r.issues)
        .find(|i| i.reason == Reason::DrumArticulation)
        .unwrap();
    assert_eq!(issue.evidence["proposal_applied"], 0.);
    // Recovery removes an earlier raw hit in its baseline. The remaining
    // articulation must still reference the same cymbal, not the old index.
    input.drum_recovered_hits = vec![input.drums[0].clone()];
    let recovered_map = review::generation_map(
        &document,
        &input,
        2.,
        Default::default(),
        ["ExpertDrums".into()].into_iter().collect(),
    )
    .unwrap();
    assert!(
        recovered_map
            .regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| { i.reason == Reason::RecoveredPercussionAttack })
    );
    let authored = review::generation_map(
        &document,
        &input,
        2.,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    assert!(
        !authored
            .regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(|i| i.reason == Reason::DrumArticulation)
    );
}

#[test]
fn detailed_cymbal_votes_are_owned_classification_only_and_reject_cowbells() {
    let hits: Vec<_> = (0..8)
        .map(|i| DrumHit {
            time: 0.5 + i as f64,
            voice: Voice::Cymbal,
            confidence: 0.73,
        })
        .collect();
    let mut evidence = drum_articulation::analyze(&vec![0.; RATE * 9], &hits).unwrap();
    let mut frames = vec![[0.; 8]; 901];
    frames[50][4] = 0.9; // ride
    frames[150][6] = 0.85; // crash
    frames[250][5] = 0.95; // bell/cowbell, not proof of ride
    frames[350][4] = 0.59; // weak
    frames[450][4] = 0.75;
    frames[450][6] = 0.65; // insufficient family margin
    frames[550][4] = 0.9;
    frames[550][0] = 0.95; // another family dominates
    frames[655][4] = 0.95; // outside the existing attack's35ms window
    frames[750][4] = 0.7;
    frames[750][5] = 0.95; // clear bell class vetoes ride
    drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 9.).unwrap();
    assert_eq!(evidence[0].proposed, Some(Voice::Ride));
    assert!(evidence[0].map_proposal);
    assert_eq!(evidence[1].proposed, Some(Voice::Crash));
    assert!(
        evidence[2..]
            .iter()
            .all(|e| e.proposed.is_none() && !e.map_proposal)
    );
    assert_eq!(evidence[2].reason, "ambiguous_ride_bell_or_cowbell");
    let after = drum_articulation::proposed_hits(&hits, &evidence);
    for (a, b) in hits.iter().zip(after) {
        assert_eq!(a.time, b.time);
        assert_eq!(a.confidence, b.confidence);
    }
    frames[0][0] = f32::NAN;
    assert!(drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 9.).is_err());
}

#[test]
fn only_bounded_sigmoid_rounding_is_normalized_with_raw_evidence_retained() {
    let hits = vec![DrumHit {
        time: 0.5,
        voice: Voice::Cymbal,
        confidence: 0.8,
    }];
    let mut evidence = drum_articulation::analyze(&vec![0.; RATE], &hits).unwrap();
    let mut frames = vec![[0.; 8]; 101];
    frames[50][4] = 1. + drum_articulation::SCORE_EPSILON / 2.;
    frames[50][0] = -5.9604645e-8;
    drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 1.).unwrap();
    let model = evidence[0].cymbal_model.as_ref().unwrap();
    assert_eq!(model.class_peaks[4], 1.);
    assert!(model.raw_class_peaks[4] > 1.);
    assert_eq!(model.clipped_values, 2);
    frames[50][4] = 1.01;
    assert!(drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 1.).is_err());
    frames[50][4] = 0.9;
    frames[0][0] = -0.01;
    assert!(drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 1.).is_err());
}

#[test]
fn a_midpoint_model_frame_cannot_classify_two_distinct_cymbal_attacks() {
    let hits = vec![
        DrumHit {
            time: 0.5,
            voice: Voice::Cymbal,
            confidence: 0.8,
        },
        DrumHit {
            time: 0.56,
            voice: Voice::Cymbal,
            confidence: 0.8,
        },
    ];
    let mut evidence = drum_articulation::analyze(&vec![0.; RATE * 2], &hits).unwrap();
    let mut frames = vec![[0.; 8]; 201];
    frames[53][4] = 0.9;
    drum_articulation::classify_cymbals(&frames, &hits, &mut evidence, 2.).unwrap();
    assert_eq!(evidence.iter().filter(|e| e.map_proposal).count(), 1);
}

/// Opt-in integration regression: use an already exposed development source,
/// its bound native stem manifest and pinned models. No authored chart input.
#[test]
#[ignore = "Native combined inference on exposed development audio and installed models"]
fn native_combined_drum_guitar_bass_preserves_source_binding_through_build() {
    use chart_engine::{
        bass_analysis, chart::Chart, mapping::Transcription, models, pipeline, stem_review::StemId,
        timing::TempoMap,
    };
    use std::{fs, path::PathBuf};
    let required = |name: &str| {
        PathBuf::from(
            std::env::var_os(name)
                .unwrap_or_else(|| panic!("Set {name} to an exposed development input")),
        )
    };
    let audio = required("YACE_COMBINED_AUDIO");
    let manifest_path = required("YACE_COMBINED_MANIFEST");
    let model_dir = required("YACE_COMBINED_MODELS");
    let output = required("YACE_COMBINED_OUTPUT");
    assert!(
        !output.exists(),
        "Preserve prior diagnostics; choose a new output"
    );
    let detailed_model = model_dir.join("oaf_drums.onnx");
    assert_eq!(
        models::hash_file(&detailed_model).unwrap(),
        drum_articulation::CYMBAL_MODEL_SHA256,
        "Exercise the actual detailed-model integration, not the missing-model fallback"
    );
    let manifest = pipeline::read_isolation(&audio, &manifest_path).unwrap();
    let source_sha = models::hash_file(&audio).unwrap();
    let original_manifest_sha = models::hash_file(&manifest_path).unwrap();
    let original_stems: Vec<_> = manifest
        .stems
        .iter()
        .map(|s| (s.path.clone(), s.sha256.clone()))
        .collect();
    let wanted = ["drums".into(), "guitar".into(), "bass".into()];
    // This failed inside Bass::infer at events.validate when articulation's
    // source witness existed before out.isolation had been attached.
    let events = pipeline::transcribe_with_isolation(
        &audio,
        &model_dir,
        pipeline::IsolationOptions {
            manifest: Some(&manifest_path),
            ..Default::default()
        },
        &wanted,
    )
    .unwrap();
    events.validate().unwrap();
    assert!(
        !events.drums.is_empty() && !events.guitar.is_empty() && !events.bass_candidates.is_empty()
    );
    assert!(!events.drum_articulations.is_empty());
    assert!(
        events
            .drum_articulations
            .iter()
            .all(|e| matches!(e.original, Voice::Tom | Voice::Cymbal))
    );
    assert!(
        events
            .drum_articulations
            .iter()
            .any(|e| e.cymbal_model.is_some()),
        "Detailed classifier must actually run"
    );
    let witness = events.drum_articulation_source.as_ref().unwrap();
    assert_eq!(witness.recording_sha256, source_sha);
    let bass_stem = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Bass)
        .unwrap();
    let drum_stem = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Drums)
        .unwrap();
    assert_eq!(witness.stem_sha256, drum_stem.sha256);
    assert_eq!(
        events.bass_decoder.as_ref().unwrap().bass_audio_sha256,
        bass_stem.sha256
    );
    assert_eq!(
        events.bass_analysis.as_ref().unwrap().bass_audio_sha256,
        bass_stem.sha256
    );
    assert_eq!(
        events.isolation.as_ref().unwrap().source_audio_sha256,
        source_sha
    );
    let bytes = serde_json::to_vec(&events).unwrap();
    let mut replay: Transcription = serde_json::from_slice(&bytes).unwrap();
    replay.validate().unwrap();
    let raw_bass = serde_json::to_value((
        &replay.bass_candidates,
        &replay.bass_short_candidates,
        &replay.bass_decoder,
    ))
    .unwrap();
    let raw_drums = serde_json::to_value((
        &replay.drums,
        &replay.drum_articulations,
        &replay.drum_articulation_source,
    ))
    .unwrap();
    bass_analysis::analyze(&mut replay, &bass_stem.path).unwrap();
    assert_eq!(
        raw_bass,
        serde_json::to_value((
            &replay.bass_candidates,
            &replay.bass_short_candidates,
            &replay.bass_decoder
        ))
        .unwrap()
    );
    assert_eq!(
        raw_drums,
        serde_json::to_value((
            &replay.drums,
            &replay.drum_articulations,
            &replay.drum_articulation_source
        ))
        .unwrap()
    );
    let mut metadata = pipeline::metadata(&audio).unwrap();
    metadata.name = "Combined native regression".into();
    metadata.artist = "Blind development control".into();
    let report = pipeline::build(pipeline::Build {
        audio: &audio,
        output: &output,
        existing: None,
        timing: Some(TempoMap::constant(120.).unwrap()),
        transcription: &events,
        meta: metadata,
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    let saved_path = output.join("chart-engine-transcription.json");
    let saved: Transcription = serde_json::from_slice(&fs::read(&saved_path).unwrap()).unwrap();
    saved.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&events).unwrap()
    );
    let chart = Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap()).unwrap();
    for track in ["ExpertDrums", "ExpertSingle", "ExpertDoubleBass"] {
        assert!(!chart.notes(track).unwrap().is_empty());
    }
    let _: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    assert_eq!(models::hash_file(&audio).unwrap(), source_sha);
    assert_eq!(
        models::hash_file(&manifest_path).unwrap(),
        original_manifest_sha
    );
    for (path, sha) in original_stems {
        assert_eq!(models::hash_file(&path).unwrap(), sha);
    }
    fs::write(output.join("combined-native-regression.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","scope":"source binding, combined native inference, replay and build; not musical acceptance","binary_sha256":models::hash_file(&std::env::current_exe().unwrap()).unwrap(),"source_sha256":source_sha,"input_manifest_sha256":original_manifest_sha,"articulation_model_sha256":models::hash_file(&detailed_model).unwrap(),"saved_transcription_sha256":models::hash_file(&saved_path).unwrap(),"raw_drums":events.drums.len(),"articulation_regions":events.drum_articulations.len(),"bass_raw_candidates":events.bass_candidates.len(),"bass_decoder":events.bass_decoder,"report":report})).unwrap()).unwrap();
}

#[test]
#[ignore = "Private exposed development recording; no inference or target input to analysis"]
fn development_articulation_proposals() {
    let events = std::path::PathBuf::from(std::env::var_os("YACE_DRUM_EVENTS").unwrap());
    let stem = std::path::PathBuf::from(std::env::var_os("YACE_DRUM_STEM").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("YACE_DRUM_OUTPUT").unwrap());
    assert!(!output.exists());
    std::fs::create_dir(&output).unwrap();
    let mut events: chart_engine::mapping::Transcription =
        serde_json::from_slice(&std::fs::read(events).unwrap()).unwrap();
    if let Some(manifest) = &events.isolation {
        let bound = manifest
            .stems
            .iter()
            .find(|s| s.id == chart_engine::stem_review::StemId::Drums)
            .unwrap();
        assert_eq!(
            chart_engine::models::hash_file(&stem).unwrap(),
            bound.sha256,
            "Development audio must be the actual hash-bound analyzed stem"
        );
    }
    let audio = chart_engine::models::decode(&stem, RATE as u32, 1).unwrap();
    let cached = std::env::var_os("YACE_DRUM_EVIDENCE_CACHE");
    let mut evidence = if let Some(cache) = &cached {
        let cache = std::path::PathBuf::from(cache);
        assert_eq!(
            chart_engine::models::hash_file(&cache).unwrap(),
            std::env::var("YACE_DRUM_EVIDENCE_CACHE_SHA256").unwrap()
        );
        let source: serde_json::Value =
            serde_json::from_slice(&std::fs::read(cache.with_file_name("source.json")).unwrap())
                .unwrap();
        assert_eq!(
            source["stem_sha256"],
            chart_engine::models::hash_file(&stem).unwrap()
        );
        assert_eq!(
            source["events_sha256"],
            chart_engine::models::hash_file(&std::path::PathBuf::from(
                std::env::var_os("YACE_DRUM_EVENTS").unwrap()
            ))
            .unwrap()
        );
        let evidence: Vec<drum_articulation::Evidence> =
            serde_json::from_slice(&std::fs::read(cache).unwrap()).unwrap();
        drum_articulation::validate(&evidence, &events.drums).unwrap();
        evidence
    } else {
        drum_articulation::analyze(&audio, &events.drums).unwrap()
    };
    if let Some(model) = std::env::var_os("YACE_DRUM_DETAIL_MODEL")
        && cached.is_none()
    {
        let model = std::path::PathBuf::from(model);
        let (frames, duration) = if let Some(cache) = std::env::var_os("YACE_DRUM_FRAME_CACHE") {
            let cache = std::path::PathBuf::from(cache);
            let metadata: serde_json::Value =
                serde_json::from_slice(&std::fs::read(cache.with_extension("json")).unwrap())
                    .unwrap();
            let expected = std::env::var("YACE_DRUM_FRAME_CACHE_SHA256").unwrap();
            assert_eq!(chart_engine::models::hash_file(&cache).unwrap(), expected);
            assert_eq!(
                metadata["stem_sha256"],
                chart_engine::models::hash_file(&stem).unwrap()
            );
            assert_eq!(
                metadata["model_sha256"],
                drum_articulation::CYMBAL_MODEL_SHA256
            );
            assert_eq!(
                chart_engine::models::hash_file(&model).unwrap(),
                drum_articulation::CYMBAL_MODEL_SHA256
            );
            assert_eq!(metadata["classes"], 8);
            assert_eq!(metadata["rate"], 100);
            let bytes = std::fs::read(&cache).unwrap();
            assert!(bytes.len() <= 360001 * 32 && bytes.len() % 32 == 0);
            let frames: Vec<[f32; 8]> = bytes
                .chunks_exact(32)
                .map(|row| {
                    std::array::from_fn(|i| {
                        f32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
                    })
                })
                .collect();
            assert_eq!(metadata["frames"].as_u64().unwrap() as usize, frames.len());
            let pcm = chart_engine::models::decode(&stem, 44100, 1).unwrap();
            let duration = pcm.len() as f64 / 44100.;
            assert!((metadata["duration"].as_f64().unwrap() - duration).abs() < 1e-9);
            eprintln!(
                "Cached source peak={}, out_of_nominal_range={}, normalized=false",
                pcm.iter().map(|v| v.abs()).fold(0., f32::max),
                pcm.iter().filter(|v| v.abs() > 1.).count()
            );
            (frames, duration)
        } else {
            drum_articulation::infer_cymbal_frames(&stem, &model).unwrap()
        };
        let mut bytes = Vec::with_capacity(frames.len() * 32);
        for value in frames.iter().flatten() {
            bytes.extend(value.to_le_bytes());
        }
        std::fs::write(output.join("raw-cymbal-frames.f32"), bytes).unwrap();
        std::fs::write(output.join("raw-cymbal-frames.json"),serde_json::to_vec_pretty(&serde_json::json!({"rate":100,"classes":8,"frames":frames.len(),"duration":duration,"model_sha256":drum_articulation::CYMBAL_MODEL_SHA256,"stem_sha256":chart_engine::models::hash_file(&stem).unwrap()})).unwrap()).unwrap();
        drum_articulation::classify_cymbals(&frames, &events.drums, &mut evidence, duration)
            .unwrap();
    }
    std::fs::write(output.join("source.json"),serde_json::to_vec_pretty(&serde_json::json!({"stem_sha256":chart_engine::models::hash_file(&stem).unwrap(),"events_sha256":chart_engine::models::hash_file(&std::path::PathBuf::from(std::env::var_os("YACE_DRUM_EVENTS").unwrap())).unwrap(),"probe_binary_sha256":chart_engine::models::hash_file(&std::env::current_exe().unwrap()).unwrap(),"sample_rate":RATE,"frames":audio.len(),"algorithm":drum_articulation::ALGORITHM})).unwrap()).unwrap();
    std::fs::write(
        output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    let timing = chart_engine::timing::TempoMap::constant(120.).unwrap();
    let before = chart_engine::mapping::tracks(&events, &timing, true).unwrap();
    events.drum_articulations = evidence;
    events.drum_articulation_source = events
        .isolation
        .as_ref()
        .map(drum_articulation::Source::from_manifest)
        .transpose()
        .unwrap();
    let after = chart_engine::mapping::tracks(&events, &timing, true).unwrap();
    let heads = |notes: &Vec<chart_engine::chart::Note>| {
        let mut counts = std::collections::BTreeMap::new();
        for n in notes.iter().filter(|n| n.lane <= 4) {
            *counts.entry(n.tick).or_insert(0usize) += 1;
        }
        counts
    };
    assert_eq!(
        heads(&before["ExpertDrums"]),
        heads(&after["ExpertDrums"]),
        "Exact standard head clock/count invariant"
    );
    assert_eq!(
        before.get("ExpertDoubleBass"),
        after.get("ExpertDoubleBass")
    );
    for (name, tracks) in [("baseline", before), ("proposed", after)] {
        std::fs::write(
            output.join(format!("{name}.chart")),
            chart_engine::chart::new_chart(
                "Development articulation",
                "Source-only analysis",
                &timing,
                &tracks,
            ),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "Independent evaluation of exposed development reports only"]
fn summarize_development_articulations() {
    use serde_json::{Value, json};
    use std::{collections::BTreeMap, fs};
    let root = std::path::PathBuf::from(std::env::var_os("YACE_DRUM_DEV_ROOT").unwrap());
    let destination = root.join("summary-v3.json");
    assert!(!destination.exists());
    let load =
        |p: &std::path::Path| -> Value { serde_json::from_slice(&fs::read(p).unwrap()).unwrap() };
    let mut pools: BTreeMap<String, BTreeMap<u64, [u64; 3]>> = BTreeMap::new();
    let inputs: Vec<Value> = std::env::var_os("YACE_DRUM_CASES")
        .map(|p| load(std::path::Path::new(&p)).as_array().unwrap().clone())
        .unwrap_or_default();
    let mut cases = vec![];
    for entry in fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if !path.join("baseline-comparison.json").exists() {
            continue;
        }
        let before = load(&path.join("baseline-comparison.json"));
        let after = load(&path.join("proposed-comparison.json"));
        let evidence = load(&path.join("evidence.json"));
        let mut reasons = BTreeMap::<String, usize>::new();
        let mut proposed = 0;
        let mut enabled = 0;
        for e in evidence.as_array().unwrap() {
            *reasons
                .entry(e["reason"].as_str().unwrap().into())
                .or_default() += 1;
            proposed += usize::from(!e["proposed"].is_null());
            enabled += usize::from(e["map_proposal"] == true);
        }
        let mut effects = json!(null);
        if let Some(input) = inputs
            .iter()
            .find(|i| i["key"] == path.file_name().unwrap().to_str().unwrap())
        {
            let mut events: chart_engine::mapping::Transcription =
                serde_json::from_slice(&fs::read(input["events"].as_str().unwrap()).unwrap())
                    .unwrap();
            events.drum_articulations = serde_json::from_value(evidence.clone()).unwrap();
            events.drum_articulation_source = events
                .isolation
                .as_ref()
                .map(drum_articulation::Source::from_manifest)
                .transpose()
                .unwrap();
            let timing = chart_engine::timing::TempoMap::constant(120.).unwrap();
            let played = chart_engine::mapping::played_drum_articulations(&events, &timing);
            let relocated = played
                .iter()
                .filter(|(index, _, voice)| *voice != events.drums[*index].voice)
                .count();
            let accepted_played = played
                .iter()
                .filter(|(index, _, _)| {
                    events
                        .drum_articulations
                        .iter()
                        .any(|e| e.hit_index == *index && e.map_proposal)
                })
                .count();
            effects = json!({"played_relocated_heads":relocated,"accepted_played_heads":accepted_played,"accepted_unplayed":enabled-accepted_played,"accepted_collision_fallback":played.iter().filter(|(index,_,voice)| events.drum_articulations.iter().any(|e| e.hit_index==*index && e.map_proposal && e.proposed!=Some(*voice))).count()});
        }
        let mut raw_scores = json!(null);
        let raw_path = path.join("raw-cymbal-frames.f32");
        if raw_path.exists() {
            let bytes = fs::read(raw_path).unwrap();
            assert_eq!(bytes.len() % 32, 0);
            let values: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            raw_scores = json!({"minimum":values.iter().copied().fold(f32::INFINITY,f32::min),"maximum":values.iter().copied().fold(f32::NEG_INFINITY,f32::max),"nonfinite":values.iter().filter(|v| !v.is_finite()).count(),"outside_zero_one":values.iter().filter(|v| **v<0. || **v>1.).count()});
        }
        let mut measurements = vec![];
        for instrument in ["drums", "pro_drums"] {
            let mut row = json!({"instrument":instrument});
            for (variant, document) in [("baseline", &before), ("proposed", &after)] {
                let result = document["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["instrument"] == instrument)
                    .unwrap();
                assert_eq!(result["success"], true);
                let report = &result["report"];
                let attacks = if instrument == "pro_drums" {
                    &report["pro_drum_hits"]["attacks"]
                } else {
                    &report["attacks"]
                };
                row[variant] = attacks.clone();
                let mut add = |label: String, attacks: &Value| {
                    for a in attacks.as_array().unwrap() {
                        let total = pools
                            .entry(label.clone())
                            .or_default()
                            .entry(a["tolerance_ms"].as_u64().unwrap())
                            .or_default();
                        for (i, name) in ["matched", "reference_attacks", "generated_attacks"]
                            .iter()
                            .enumerate()
                        {
                            total[i] += a[name].as_u64().unwrap();
                        }
                    }
                };
                add(format!("{instrument}/{variant}"), attacks);
                if instrument == "pro_drums" {
                    for class in report["pro_drum_hits"]["lane_articulation_classes"]
                        .as_array()
                        .unwrap()
                    {
                        add(
                            format!(
                                "{instrument}/{}/{variant}",
                                class["class"].as_str().unwrap()
                            ),
                            &class["attacks"],
                        );
                    }
                }
            }
            measurements.push(row);
        }
        cases.push(json!({"case":path.file_name().unwrap().to_str().unwrap(),"proposals":proposed,"enabled_proposals":enabled,"mapped_effects":effects,"raw_scores":raw_scores,"reasons":reasons,"measurements":measurements}));
    }
    cases.sort_by(|a, b| a["case"].as_str().cmp(&b["case"].as_str()));
    let pooled:Vec<_>=pools.into_iter().map(|(label,rows)|json!({"label":label,"metrics":rows.into_iter().map(|(t,[m,r,g])|json!({"tolerance_ms":t,"matched":m,"reference":r,"generated":g,"precision":if g>0{Some(m as f64/g as f64)}else{None},"recall":if r>0{Some(m as f64/r as f64)}else{None},"f1":if r+g>0{Some(2.*m as f64/(r+g)as f64)}else{None}})).collect::<Vec<_>>()})).collect();
    fs::write(
        destination,
        serde_json::to_vec_pretty(&json!({"cases":cases,"pooled":pooled})).unwrap(),
    )
    .unwrap();
}
