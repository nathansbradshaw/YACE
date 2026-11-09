use chart_engine::{
    vocal_generation::{PitchFrame, PitchSupport},
    vocal_posterior as vp,
};
fn frames() -> Vec<PitchFrame> {
    (0..100)
        .map(|i| PitchFrame {
            time_seconds: i as f64 * 0.01,
            midi: (!(40..45).contains(&i)).then_some(60.),
            yin_midi: Some(60.),
            mcleod_midi: Some(60.),
            clarity: 0.9,
            rms: 0.1,
            local_rms: Some(0.1),
            local_energy_active: Some(true),
            support: PitchSupport::Consensus,
        })
        .collect()
}
#[test]
fn posterior_chooses_supported_alternative_without_adding_or_removing_voice() {
    let mut f = frames();
    let mask: Vec<_> = f.iter().map(|f| f.midi.is_some()).collect();
    let costs = vec![std::array::from_fn(|i| if i == 26 { 0. } else { 8. }); f.len()];
    vp::decode(&mut f, &costs, 3.2, &|| false).unwrap();
    assert_eq!(mask, f.iter().map(|f| f.midi.is_some()).collect::<Vec<_>>());
    assert!(f.iter().filter_map(|f| f.midi).all(|p| p == 62.));
}
#[test]
fn flat_evidence_keeps_acoustic_pitch_and_silence() {
    let mut f = frames();
    vp::decode(&mut f, &vec![[1.; 49]; 100], 3.2, &|| false).unwrap();
    assert!(f.iter().filter_map(|f| f.midi).all(|p| p == 60.));
    let mut silent = frames();
    for f in &mut silent {
        f.midi = None
    }
    vp::decode(&mut silent, &vec![[0.; 49]; 100], 3.2, &|| false).unwrap();
    assert!(silent.iter().all(|f| f.midi.is_none()));
}
#[test]
fn invalid_costs_and_late_cancellation_are_atomic() {
    let mut f = frames();
    let before = serde_json::to_vec(&f).unwrap();
    let mut cost = vec![[1.; 49]; 100];
    cost[50][2] = f64::NAN;
    assert!(vp::decode(&mut f, &cost, 3.2, &|| false).is_err());
    assert_eq!(before, serde_json::to_vec(&f).unwrap());
    cost[50][2] = 1.;
    let calls = std::cell::Cell::new(0);
    assert!(
        vp::decode(&mut f, &cost, 3.2, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 2
        })
        .is_err()
    );
    assert_eq!(before, serde_json::to_vec(&f).unwrap());
}
#[test]
fn recovery_requires_independent_pitch_support_energy_and_duration() {
    let mut f = frames();
    for v in &mut f {
        v.midi = None;
    }
    let cost = vec![std::array::from_fn(|s| if s == 24 { 0. } else { 2. }); 100];
    let flat = vec![[0.; 49]; 100];
    let mut note = vec![Some(60.); 100];
    note[20..30].fill(None);
    for v in &mut f[40..50] {
        v.local_rms = Some(0.);
    }
    note[60..66].fill(Some(62.));
    assert_eq!(
        vp::recover(&mut f, [&cost, &cost, &flat], &note, 3, 0.08, &|| false).unwrap(),
        0
    );
    let n = vp::recover(&mut f, [&cost, &cost, &cost], &note, 3, 0.08, &|| false).unwrap();
    assert_eq!(n, 74);
    assert!(f[20..30].iter().all(|v| v.midi.is_none()));
    assert!(f[40..50].iter().all(|v| v.midi.is_none()));
    assert!(f[60..66].iter().all(|v| v.midi.is_none()));
    assert!(
        f.iter()
            .filter(|v| v.midi.is_some())
            .all(|v| v.support == PitchSupport::AcousticAlternative)
    );
}
#[test]
fn cancelled_recovery_never_partially_changes_frames() {
    let mut f = frames();
    let before = serde_json::to_vec(&f).unwrap();
    let cost = vec![[1.; 49]; 100];
    let calls = std::cell::Cell::new(0);
    assert!(
        vp::recover(
            &mut f,
            [&cost, &cost, &cost],
            &vec![Some(60.); 100],
            3,
            0.08,
            &|| {
                calls.set(calls.get() + 1);
                calls.get() > 3
            }
        )
        .is_err()
    );
    assert_eq!(before, serde_json::to_vec(&f).unwrap());
}
#[test]
fn a_short_detector_dropout_uses_the_full_supported_acoustic_span() {
    let mut f = frames();
    let cost = vec![std::array::from_fn(|s| if s == 24 { 0. } else { 2. }); 100];
    let before: Vec<_> = f.iter().map(|f| f.midi).collect();
    assert_eq!(
        vp::recover(
            &mut f,
            [&cost, &cost, &cost],
            &vec![Some(60.); 100],
            3,
            0.08,
            &|| false
        )
        .unwrap(),
        5
    );
    for (a, b) in before.iter().zip(&f) {
        if a.is_some() {
            assert_eq!(*a, b.midi);
        }
    }
    assert!(f.iter().all(|f| f.midi == Some(60.)));
}
#[test]
fn relative_energy_floor_filters_additions_and_preserves_quiet_existing_voice() {
    let mut f = frames();
    let cost = vec![std::array::from_fn(|s| if s == 24 { 0. } else { 2. }); 100];
    for v in &mut f[40..45] {
        v.local_rms = Some(0.01);
    }
    f[10].local_rms = Some(0.0001);
    assert_eq!(
        vp::recover_energy(
            &mut f,
            [&cost, &cost, &cost],
            &vec![Some(60.); 100],
            2,
            0.18,
            0.5,
            &|| false
        )
        .unwrap(),
        0
    );
    assert_eq!(f[10].midi, Some(60.));
    for v in &mut f[40..45] {
        v.local_rms = Some(0.08);
    }
    assert_eq!(
        vp::recover_energy(
            &mut f,
            [&cost, &cost, &cost],
            &vec![Some(60.); 100],
            2,
            0.18,
            0.5,
            &|| false
        )
        .unwrap(),
        5
    );
}

#[test]
fn local_dynamics_require_nearby_original_voice_and_preserve_distant_gaps() {
    let template = frames()[0].clone();
    let mut input: Vec<_> = (0..1000)
        .map(|i| {
            let mut f = template.clone();
            f.time_seconds = i as f64 * 0.01;
            f.midi = (i < 600 || (700..760).contains(&i)).then_some(60.);
            f.local_rms = Some(if i < 600 {
                0.2
            } else if (700..760).contains(&i) {
                0.04
            } else {
                0.03
            });
            f
        })
        .collect();
    let costs = vec![std::array::from_fn(|s| if s == 24 { 0. } else { 2. }); input.len()];
    let notes: Vec<_> = (0..1000)
        .map(|i| ((700..780).contains(&i) || (950..970).contains(&i)).then_some(60.))
        .collect();
    let original = input.clone();
    assert_eq!(
        vp::recover_energy(
            &mut input,
            [&costs, &costs, &costs],
            &notes,
            2,
            0.18,
            0.5,
            &|| false
        )
        .unwrap(),
        0
    );
    assert_eq!(
        vp::recover_energy_context(
            &mut input,
            [&costs, &costs, &costs],
            &notes,
            2,
            0.18,
            vp::RecoveryEnergy {
                relative_floor: 0.5,
                context_seconds: Some(0.5),
                minimum_pitch_margin: 0.,
            },
            &|| false
        )
        .unwrap(),
        20
    );
    assert!(input[760..780].iter().all(|f| f.midi == Some(60.)));
    assert!(input[950..970].iter().all(|f| f.midi.is_none()));
    for (before, after) in original.iter().zip(&input) {
        if before.midi.is_some() {
            assert_eq!(before.midi, after.midi);
        }
        assert_eq!(before.local_rms, after.local_rms);
    }
    let saved = serde_json::to_vec(&input).unwrap();
    assert!(
        vp::recover_energy_context(
            &mut input,
            [&costs, &costs, &costs],
            &notes,
            2,
            0.18,
            vp::RecoveryEnergy {
                relative_floor: 0.5,
                context_seconds: Some(f64::NAN),
                minimum_pitch_margin: 0.,
            },
            &|| false
        )
        .is_err()
    );
    assert_eq!(saved, serde_json::to_vec(&input).unwrap());
    assert!(
        vp::recover_energy_context(
            &mut input,
            [&costs, &costs, &costs],
            &notes,
            2,
            0.18,
            vp::RecoveryEnergy {
                relative_floor: 0.5,
                context_seconds: Some(0.5),
                minimum_pitch_margin: 0.,
            },
            &|| true
        )
        .is_err()
    );
    assert_eq!(saved, serde_json::to_vec(&input).unwrap());
}

#[test]
fn short_consensus_requires_separation_from_the_runner_up_pitch() {
    let mut input = frames();
    let original: Vec<_> = input.iter().map(|f| f.midi).collect();
    let notes = vec![Some(60.); input.len()];
    let ambiguous = vec![
        std::array::from_fn(|s| match s {
            24 => 0.,
            25 => 0.1,
            _ => 2.,
        });
        input.len()
    ];
    assert_eq!(
        vp::recover_energy_context(
            &mut input,
            [&ambiguous, &ambiguous, &ambiguous],
            &notes,
            3,
            0.08,
            vp::RecoveryEnergy {
                relative_floor: 0.5,
                context_seconds: None,
                minimum_pitch_margin: 0.25
            },
            &|| false
        )
        .unwrap(),
        0
    );
    assert_eq!(original, input.iter().map(|f| f.midi).collect::<Vec<_>>());
    let clear = vec![std::array::from_fn(|s| if s == 24 { 0. } else { 2. }); input.len()];
    assert_eq!(
        vp::recover_energy_context(
            &mut input,
            [&clear, &clear, &clear],
            &notes,
            3,
            0.08,
            vp::RecoveryEnergy {
                relative_floor: 0.5,
                context_seconds: None,
                minimum_pitch_margin: 0.25
            },
            &|| false
        )
        .unwrap(),
        5
    );
    assert!(input.iter().all(|f| f.midi == Some(60.)));
}

#[test]
fn frozen_short_recovery_preserves_the_base_and_rejects_ambiguous_short_notes() {
    let mut f = frames();
    for (i, v) in f.iter_mut().enumerate() {
        v.midi = (i < 10).then_some(57.);
    }
    let original = f.clone();
    let mut notes = vec![None; f.len()];
    notes[20..31].fill(Some(60.));
    notes[40..51].fill(Some(62.));
    notes[60..85].fill(Some(64.));
    let costs: Vec<_> = (0..f.len())
        .map(|i| {
            std::array::from_fn(|s| {
                let p = notes[i].unwrap_or(60.) as usize;
                if s + 36 == p {
                    0.
                } else if s + 36 == p + 1 && i >= 40 {
                    0.1
                } else {
                    2.
                }
            })
        })
        .collect();
    assert_eq!(
        vp::recover_short_consensus(&mut f, [&costs, &costs, &costs], &notes, &|| false).unwrap(),
        36
    );
    assert!(f[..10].iter().all(|f| f.midi == Some(57.)));
    assert!(f[20..31].iter().all(|f| f.midi == Some(60.)));
    assert!(f[40..51].iter().all(|f| f.midi.is_none()));
    assert!(f[60..85].iter().all(|f| f.midi == Some(64.)));
    let mut cancelled = original.clone();
    let calls = std::cell::Cell::new(0);
    assert!(
        vp::recover_short_consensus(&mut cancelled, [&costs, &costs, &costs], &notes, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 150
        })
        .is_err()
    );
    assert_eq!(
        serde_json::to_vec(&original).unwrap(),
        serde_json::to_vec(&cancelled).unwrap()
    );
}
