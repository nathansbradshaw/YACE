use chart_engine::{keyboard_voicing, mapping::PitchNote, pro_keys, timing::TempoMap};
fn n(start: f64, end: f64, pitch: u8, confidence: f32) -> PitchNote {
    PitchNote {
        start,
        end,
        pitch,
        confidence,
    }
}
#[test]
fn octave_coonsets_collapse_but_repeated_source_attacks_survive_in_every_register() {
    for shift in 0..=60 {
        let raw = vec![
            n(1., 2., 24 + shift, 0.9),
            n(1.005, 2.1, 48 + shift, 0.9),
            n(1.025, 2.2, 48 + shift, 0.9),
        ];
        let before = serde_json::to_vec(&raw).unwrap();
        let selection = keyboard_voicing::select(&raw);
        assert_eq!(selection.voices.len(), 2);
        assert_eq!(selection.voices[0].source_indices.len(), 2);
        assert_eq!(selection.voices[0].note.start, 1.);
        assert_eq!(selection.voices[1].note.start, 1.025);
        let arranged = pro_keys::arrange(&raw, &TempoMap::constant(120.).unwrap()).unwrap();
        assert_eq!(arranged.notes.len(), 2);
        assert_eq!(arranged.decisions.len(), 3);
        assert_eq!(arranged.merged_octave_collisions, 1);
        assert_eq!(serde_json::to_vec(&raw).unwrap(), before);
    }
}
#[test]
fn weak_short_isolated_flashes_are_reviewable_but_quiet_sustains_and_fast_melody_survive() {
    let raw = vec![
        n(0., 0.03, 60, 0.5),
        n(1., 1.03, 62, 0.9),
        n(1.02, 1.05, 62, 0.9),
        n(2., 3., 64, 0.4),
        n(4., 4.03, 65, 0.5),
        n(4.5, 4.7, 65, 0.8),
    ];
    let selection = keyboard_voicing::select(&raw);
    assert_eq!(selection.weak_short_candidates, [0]);
    assert_eq!(selection.voices.len(), 5);
    let arranged = pro_keys::arrange(&raw, &TempoMap::constant(120.).unwrap()).unwrap();
    assert_eq!(arranged.filtered_short_candidates, 1);
    assert_eq!(arranged.decisions[0].disposition, "weak_short_isolated");
    assert!(arranged.decisions[0].target_pitch.is_none());
    assert_eq!(arranged.decisions.len(), raw.len());
}
#[test]
fn distinct_chord_tones_are_not_reduced_to_one_melody() {
    let raw = vec![
        n(0., 1., 60, 0.9),
        n(0.005, 0.8, 64, 0.9),
        n(0.009, 1.2, 67, 0.9),
    ];
    assert_eq!(keyboard_voicing::select(&raw).voices.len(), 3);
    assert_eq!(
        pro_keys::arrange(&raw, &TempoMap::constant(120.).unwrap())
            .unwrap()
            .notes
            .len(),
        3
    );
}

#[test]
fn generated_register_uses_one_musical_octave_convention_for_all_transpositions() {
    let map = TempoMap::constant(120.).unwrap();
    for base in 60..=72 {
        let raw = vec![
            n(0., 0.2, base, 0.9),
            n(0.4, 0.6, base + 2, 0.9),
            n(0.8, 1., base + 4, 0.9),
            n(1.2, 1.4, base, 0.9),
        ];
        let plan = pro_keys::arrange_generated(&raw, &map).unwrap();
        assert_eq!(plan.decisions[0].target_pitch, Some(base - 12));
        assert_eq!(
            plan.decisions[3].target_pitch,
            plan.decisions[0].target_pitch
        );
        assert!(plan.notes.iter().all(|n| (48..=72).contains(&n.lane)));
        assert!(
            chart_engine::key_ranges::plan(&plan.notes)
                .unwrap()
                .uncovered
                .is_empty()
        );
        for d in plan.decisions {
            assert_eq!(d.target_pitch.unwrap() % 12, d.source.pitch % 12);
        }
    }
}

#[test]
fn a_repeat_of_the_lower_merged_octave_survives_and_every_shortened_source_is_counted() {
    let raw = vec![
        n(1., 3., 60, 0.9),
        n(1.005, 3., 72, 0.9),
        n(1.025, 2., 60, 0.9),
    ];
    let map = TempoMap::constant(120.).unwrap();
    let plan = pro_keys::arrange_generated(&raw, &map).unwrap();
    assert_eq!(plan.notes.len(), 2);
    assert_eq!(plan.shortened_releases, 2);
    let keys = chart_engine::keys::arrange(&raw, &map, 3, false).unwrap();
    assert_eq!(keys.len(), 2);
}

#[test]
fn rejected_confidence_and_pre_chart_notes_cannot_extend_or_hide_an_eligible_attack() {
    let map = TempoMap::new(480, 0.5, vec![]).unwrap();
    let raw = vec![
        n(0.49, 3., 60, 0.9),
        n(0.51, 0.7, 72, 0.9),
        n(0.515, 10., 60, 0.1),
    ];
    let plan = pro_keys::arrange_generated(&raw, &map).unwrap();
    assert_eq!(plan.notes.len(), 1);
    assert_eq!(plan.decisions[0].disposition, "before_chart_start");
    assert_eq!(plan.decisions[2].disposition, "below_confidence");
    assert_eq!(plan.notes[0].tick, map.tick(0.51).unwrap());
    assert_eq!(
        plan.notes[0].tick + plan.notes[0].sustain,
        map.tick(0.7).unwrap()
    );
    let keys = chart_engine::keys::arrange(&raw, &map, 3, false).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].tick, map.tick(0.51).unwrap());
}
