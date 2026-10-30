//! Known-onset controls and an explicit source-bound development replay.
use super::{PitchActivations, guitar_harmonics::frequency, pitch_frame_seconds};
use crate::mapping::{PitchCandidateEvidence, PitchCandidateSource};
use std::f64::consts::TAU;
#[cfg(test)]
mod tests {
    use super::super::guitar_polyphonic::recover as joint_short_diagnostic;
    #[test]
    fn ordinary_network_splits_are_preserved_without_duplicate_recovery() {
        let mut input = PitchActivations {
            frames: vec![0.; 150 * 88],
            onsets: vec![0.; 150 * 88],
            audio: vec![0.; 2 * 22050],
        };
        for frame in 20..100 {
            input.frames[frame * 88 + 31] = 0.8;
        }
        for frame in [20, 60] {
            input.onsets[frame * 88 + 31] = 0.9;
        }
        let short = super::super::recover_short_plucks(&input, &vec![0.; 2 * 11025], 40..=88);
        let baseline: Vec<PitchCandidateEvidence> = short.candidates.clone();
        assert_eq!(baseline.len(), 2);
        let (_, proposed, receipts) = joint_short_diagnostic(
            &input,
            &vec![0.; 2 * 11025],
            &baseline,
            &short.short_candidates,
        );
        assert!(proposed.is_empty() && receipts.is_empty());
        assert_eq!(
            serde_json::to_value(baseline).unwrap(),
            serde_json::to_value(short.candidates).unwrap()
        );
    }
    use super::*;
    #[test]
    #[ignore = "requires explicit exposed GUITAR_REPLAY_* paths; no model inference"]
    fn replay_exposed_network_evidence() {
        use std::{fs, path::Path};
        let arg = |key: &str| std::env::var(format!("GUITAR_REPLAY_{key}")).unwrap();
        let events_path = arg("EVENTS");
        let stem = arg("STEM");
        let receipt_path = arg("RECEIPT");
        let preparation_path = arg("PREPARATION");
        let output = arg("OUTPUT");
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        let prep: serde_json::Value =
            serde_json::from_slice(&fs::read(&preparation_path).unwrap()).unwrap();
        let stem_hash = crate::models::hash_file(Path::new(&stem)).unwrap();
        assert_eq!(
            receipt["revised_events_sha256"],
            crate::models::hash_file(Path::new(&events_path)).unwrap()
        );
        assert_eq!(receipt["source_sha256"], prep["source_sha256"]);
        assert!(
            prep["roles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|role| role["role"] == "guitar" && role["output_sha256"] == stem_hash)
        );
        let events: crate::mapping::Transcription =
            serde_json::from_slice(&fs::read(&events_path).unwrap()).unwrap();
        let audio22 = crate::models::decode(Path::new(&stem), 22050, 1).unwrap();
        let audio11 = crate::models::decode(Path::new(&stem), 11025, 1).unwrap();
        let duration = audio22.len() as f64 / 22050.;
        let count = (0..)
            .take_while(|i| pitch_frame_seconds(*i) < duration)
            .count();
        let mut onsets = vec![0f32; count * 88];
        let mut cells = 0;
        for evidence in events
            .guitar_candidates
            .iter()
            .chain(events.guitar_short_candidates.iter().map(|c| &c.evidence))
        {
            if evidence.source != PitchCandidateSource::NetworkOnset {
                continue;
            }
            let mut lo = 0;
            let mut hi = count;
            while lo < hi {
                let mid = (lo + hi) / 2;
                if pitch_frame_seconds(mid) < evidence.candidate.start {
                    lo = mid + 1
                } else {
                    hi = mid
                }
            }
            if lo < count && (pitch_frame_seconds(lo) - evidence.candidate.start).abs() < 1e-9 {
                let cell = lo * 88 + (evidence.candidate.pitch - 21) as usize;
                if onsets[cell] == 0. {
                    cells += 1;
                }
                onsets[cell] = onsets[cell].max(evidence.onset_score);
            }
        }
        let baseline = events.clone();
        let mut input = PitchActivations {
            audio: audio22,
            frames: vec![],
            onsets,
        };
        let native = std::env::var("GUITAR_REPLAY_MODEL").ok();
        let native_inference = native.is_some();
        if let Some(model) = &native {
            input = super::super::pitch_activations(Path::new(&stem), Path::new(model)).unwrap();
            let native_baseline = super::super::recover_short_plucks(&input, &audio11, 40..=88);
            assert_eq!(
                serde_json::to_value(&native_baseline.candidates).unwrap(),
                serde_json::to_value(&events.guitar_candidates).unwrap(),
                "Native baseline must reproduce source-bound prior candidates"
            );
        }
        if let Ok(cache) = std::env::var("GUITAR_REPLAY_ACTIVATIONS") {
            let data: serde_json::Value =
                serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
            assert_eq!(data["guitar_stem_sha256"], stem_hash);
            input.frames = serde_json::from_value(data["frames"].clone()).unwrap();
            input.onsets = serde_json::from_value(data["onsets"].clone()).unwrap();
            assert_eq!(input.frames.len(), input.onsets.len());
            let fresh = super::super::recover_short_plucks(&input, &audio11, 40..=88);
            assert_eq!(
                serde_json::to_value(&fresh.candidates).unwrap(),
                serde_json::to_value(&events.guitar_candidates).unwrap()
            );
        }
        let (joint_short, proposed, receipts) = joint_short_diagnostic(
            &input,
            &audio11,
            &events.guitar_candidates,
            &events.guitar_short_candidates,
        );
        fs::create_dir(&output).unwrap();
        if let Some(model) = native {
            fs::write(Path::new(&output).join("activations.json"), serde_json::to_vec(&serde_json::json!({
                "guitar_stem_sha256":stem_hash,"model_sha256":crate::models::hash_file(Path::new(&model)).unwrap(),
                "frames":input.frames,"onsets":input.onsets,
                "clock":"pitch_frame_seconds: retained BasicPitch source-clock frames"
            })).unwrap()).unwrap();
        }
        let timing = crate::timing::TempoMap::constant(120.).unwrap();
        let mut joint = baseline.clone();
        joint.guitar_polyphonic_notes = receipts;
        joint.guitar_polyphonic_diagnostics = joint_short.clone();
        joint
            .guitar
            .extend(proposed.iter().map(|e| e.candidate.clone()));
        joint.guitar_candidates.extend(proposed.clone());
        joint
            .guitar
            .sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        joint.guitar_candidates.sort_by(|a, b| {
            a.candidate
                .start
                .total_cmp(&b.candidate.start)
                .then(a.candidate.pitch.cmp(&b.candidate.pitch))
        });
        joint.validate().unwrap();
        let native_parity_model = std::env::var("GUITAR_REPLAY_NATIVE_PARITY").ok();
        if let Some(model) = &native_parity_model {
            let inference =
                super::super::guitar_pitch_with_evidence(Path::new(&stem), Path::new(model))
                    .unwrap();
            assert_eq!(
                serde_json::to_value(&inference.candidates).unwrap(),
                serde_json::to_value(&joint.guitar_candidates).unwrap(),
                "Public default native Guitar inference must reproduce the source-bound proposal"
            );
            assert_eq!(
                serde_json::to_value(&inference.polyphonic_notes).unwrap(),
                serde_json::to_value(&joint.guitar_polyphonic_notes).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&inference.short_candidates).unwrap(),
                serde_json::to_value(&joint.guitar_short_candidates).unwrap()
            );
        }
        assert_eq!(
            serde_json::to_value(&joint.bass).unwrap(),
            serde_json::to_value(&baseline.bass).unwrap()
        );
        let before_tracks = crate::mapping::tracks(&baseline, &timing, true).unwrap();
        let after_tracks = crate::mapping::tracks(&joint, &timing, true).unwrap();
        assert_eq!(
            serde_json::to_value(&before_tracks["ExpertDoubleBass"]).unwrap(),
            serde_json::to_value(&after_tracks["ExpertDoubleBass"]).unwrap()
        );
        for (name, events) in [("baseline", &baseline), ("joint-proposed", &joint)] {
            fs::write(
                Path::new(&output).join(format!("{name}-events.json")),
                serde_json::to_vec_pretty(events).unwrap(),
            )
            .unwrap();
            let tracks = crate::mapping::tracks(events, &timing, true).unwrap();
            fs::write(
                Path::new(&output).join(format!("{name}.chart")),
                crate::chart::new_chart("Exposed guitar diagnostic", "YACE", &timing, &tracks),
            )
            .unwrap();
        }
        let report = serde_json::json!({"guitar_stem_sha256":stem_hash,"input_events_sha256":crate::models::hash_file(Path::new(&events_path)).unwrap(),
            "raw_bass_preserved":true,"mapped_expert_bass_preserved":true,
            "public_native_api_parity":native_parity_model.is_some(),"public_native_model_sha256":native_parity_model.as_ref().map(|model|crate::models::hash_file(Path::new(model)).unwrap()),
            "recorded_network_cells":cells,"joint_short_diagnostic_only":joint_short,"proposed_short_notes":proposed.len(),
            "condition":if native_parity_model.is_some() { "Fresh public native Guitar API parity checked against saved-evidence replay; source/model hash-bound; no reference inputs" } else if native_inference { "Fresh native BasicPitch inference; source/model hash-bound; no reference inputs" } else { "Saved-evidence replay; source-bound hypotheses and audio; no model call or reference inputs" }});
        fs::write(
            Path::new(&output).join("replay.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        eprintln!("{report}");
        if let Ok(expected) = std::env::var("GUITAR_REPLAY_EXPECTED_CHART") {
            assert_eq!(
                fs::read(expected).unwrap(),
                fs::read(Path::new(&output).join("joint-proposed.chart")).unwrap(),
                "Production extraction must preserve numerically frozen development chart"
            );
        }
    }

    #[test]
    fn joint_short_diagnostic_measures_distorted_chords() {
        for pitches in [vec![52], vec![52, 57, 60], vec![45, 52, 57, 60, 64]] {
            for saturated in [false, true] {
                let start = super::super::pitch_frame_seconds(20);
                let end = super::super::pitch_frame_seconds(28);
                let waveform = |rate: usize| {
                    (0..rate)
                        .map(|i| {
                            let t = i as f64 / rate as f64;
                            if t < start || t >= end {
                                0.
                            } else {
                                let phase = t - start;
                                let x = pitches
                                    .iter()
                                    .map(|p| {
                                        (TAU * frequency(*p) * phase).sin()
                                            + 0.3 * (TAU * 2. * frequency(*p) * phase).sin()
                                    })
                                    .sum::<f64>();
                                (0.25
                                    * (if saturated { (x * 2.).tanh() } else { x })
                                    * (phase / 0.003).min(1.)
                                    * (-phase / 0.2).exp()) as f32
                            }
                        })
                        .collect::<Vec<_>>()
                };
                let mut input = PitchActivations {
                    frames: vec![0.; 100 * 88],
                    onsets: vec![0.; 100 * 88],
                    audio: waveform(22050),
                };
                for p in &pitches {
                    for frame in 20..28 {
                        input.frames[frame * 88 + (*p - 21) as usize] = 0.45;
                    }
                    input.onsets[20 * 88 + (*p - 21) as usize] = 0.65;
                }
                let short = super::super::recover_short_plucks(&input, &waveform(11025), 40..=88);
                let (result, proposed, receipts) = joint_short_diagnostic(
                    &input,
                    &waveform(11025),
                    &short.candidates,
                    &short.short_candidates,
                );
                eprintln!("saturated {saturated} chord {pitches:?}: {result:?}");
                assert!(short.candidates.is_empty());
                assert_eq!(result.values().sum::<usize>(), pitches.len());
                if pitches.len() == 1 || !saturated {
                    assert_eq!(proposed.len(), 1, "{pitches:?} {result:?}");
                }
                if !proposed.is_empty() {
                    let events = crate::mapping::Transcription {
                        guitar: proposed.iter().map(|e| e.candidate.clone()).collect(),
                        guitar_candidates: proposed,
                        guitar_short_candidates: short.short_candidates,
                        guitar_polyphonic_notes: receipts,
                        ..Default::default()
                    };
                    events.validate().unwrap();
                    let mut broken = events.clone();
                    broken.guitar_polyphonic_notes[0].window_end += 0.1;
                    assert!(broken.validate().is_err());
                    let mut broken = events.clone();
                    broken.guitar_short_candidates.clear();
                    assert!(broken.validate().is_err());
                    let mut broken = events.clone();
                    broken.guitar[0].start += 0.001;
                    assert!(broken.validate().is_err());
                    let mut broken = events.clone();
                    broken.guitar_polyphonic_notes[0].evidence.onset_score = 0.49;
                    assert!(broken.validate().is_err());
                    let mut broken = events.clone();
                    broken.guitar_source = crate::mapping::GuitarSource::Piano;
                    assert!(broken.validate().is_err());
                    let timing = crate::timing::TempoMap::constant(120.).unwrap();
                    let mut tracks = crate::mapping::tracks(&events, &timing, true).unwrap();
                    let chart = crate::chart::Chart::parse(crate::chart::new_chart(
                        "Test", "Test", &timing, &tracks,
                    ))
                    .unwrap();
                    let generated = std::collections::BTreeSet::from(["ExpertSingle".into()]);
                    let map = crate::review::generation_map(
                        &chart,
                        &events,
                        1.,
                        Default::default(),
                        generated.clone(),
                    )
                    .unwrap();
                    assert!(
                        map.regions.iter().flat_map(|r| &r.issues).any(|i| i.reason
                            == crate::review_regions::Reason::RecoveredPolyphonicGuitar)
                    );
                    tracks.get_mut("ExpertSingle").unwrap().clear();
                    let removed = crate::chart::Chart::parse(crate::chart::new_chart(
                        "Test", "Test", &timing, &tracks,
                    ))
                    .unwrap();
                    let map = crate::review::generation_map(
                        &removed,
                        &events,
                        1.,
                        Default::default(),
                        generated,
                    )
                    .unwrap();
                    assert!(
                        !map.regions.iter().flat_map(|r| &r.issues).any(|i| i.reason
                            == crate::review_regions::Reason::RecoveredPolyphonicGuitar)
                    );
                    let map = crate::review::generation_map(
                        &chart,
                        &events,
                        1.,
                        Default::default(),
                        Default::default(),
                    )
                    .unwrap();
                    assert!(
                        !map.regions.iter().flat_map(|r| &r.issues).any(|i| i.reason
                            == crate::review_regions::Reason::RecoveredPolyphonicGuitar)
                    );
                }
            }
        }
    }
    #[test]
    fn joint_short_candidate_rejects_continuous_gain_modulation_wrong_pitch_and_noise() {
        for kind in 0..7 {
            for pitch in [40, 52, 64, 76, 88] {
                for phase in [0., 0.7, 1.7] {
                    let (start, end) = (
                        super::super::pitch_frame_seconds(20),
                        super::super::pitch_frame_seconds(28),
                    );
                    let waveform = |rate: usize| {
                        (0..rate)
                            .map(|i| {
                                let t = i as f64 / rate as f64;
                                let f = frequency(pitch);
                                let carrier = TAU * f * t + phase;
                                match kind {
                                    0 => (0.15 * carrier.sin()) as f32,
                                    1 => {
                                        ((if t < start { 0.025 } else { 0.15 }) * carrier.sin())
                                            as f32
                                    }
                                    2 => {
                                        (0.12 * (0.6 + 0.4 * (TAU * 8. * t).sin()) * carrier.sin())
                                            as f32
                                    }
                                    3 => {
                                        (0.15 * (carrier + 0.1 * (TAU * 6. * t).sin()).sin()) as f32
                                    }
                                    4 => {
                                        if t >= start && t < end {
                                            (0.2 * (TAU * frequency(pitch + 1) * t + phase).sin())
                                                as f32
                                        } else {
                                            0.
                                        }
                                    }
                                    5 => {
                                        if t >= start && t < end {
                                            let mut state =
                                                (i as u64).wrapping_add(phase.to_bits());
                                            state = (state ^ (state >> 30))
                                                .wrapping_mul(0xbf58476d1ce4e5b9);
                                            state = (state ^ (state >> 27))
                                                .wrapping_mul(0x94d049bb133111eb);
                                            state ^= state >> 31;
                                            ((state >> 32) as u32 as f64 / u32::MAX as f64 * 0.4
                                                - 0.2)
                                                as f32
                                        } else {
                                            0.
                                        }
                                    }
                                    _ => {
                                        if t >= start && t < end {
                                            (0.2 * (TAU * frequency(pitch - 12) * t + phase).sin()
                                                + 0.15 * carrier.sin())
                                                as f32
                                        } else {
                                            0.
                                        }
                                    }
                                }
                            })
                            .collect::<Vec<_>>()
                    };
                    let mut input = PitchActivations {
                        frames: vec![0.; 100 * 88],
                        onsets: vec![0.; 100 * 88],
                        audio: waveform(22050),
                    };
                    for frame in 20..28 {
                        input.frames[frame * 88 + (pitch - 21) as usize] = 0.45;
                    }
                    input.onsets[20 * 88 + (pitch - 21) as usize] = 0.65;
                    let short =
                        super::super::recover_short_plucks(&input, &waveform(11025), 40..=88);
                    let (result, _, _) = joint_short_diagnostic(
                        &input,
                        &waveform(11025),
                        &short.candidates,
                        &short.short_candidates,
                    );
                    assert!(
                        !result.contains_key("supported_joint_harmonics")
                            && !result.contains_key("otherwise_supported_but_partial_source_fit"),
                        "kind {kind} pitch {pitch} phase {phase}: {result:?}"
                    );
                }
            }
        }
    }
}
