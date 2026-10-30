use chart_engine::{
    chart::Note,
    key_ranges,
    mapping::{self, PitchNote, Transcription},
    pro_keys,
    timing::{Tempo, TempoMap},
};

fn candidate(start: f64, end: f64, pitch: u8) -> PitchNote {
    PitchNote {
        start,
        end,
        pitch,
        confidence: 0.9,
    }
}

fn assert_playable(notes: &[Note]) {
    assert!(
        notes
            .iter()
            .all(|note| (48..=72).contains(&note.lane) && note.sustain > 0)
    );
    assert!(key_ranges::plan(notes).unwrap().uncovered.is_empty());
    for pair in notes.iter().enumerate().flat_map(|(i, n)| {
        notes[i + 1..]
            .iter()
            .filter(move |next| next.lane == n.lane)
            .take(1)
            .map(move |next| (n, next))
    }) {
        assert!(
            pair.0.tick + pair.0.sustain <= pair.1.tick,
            "same-key releases cannot overlap"
        );
    }
}

#[test]
fn every_midi_register_keeps_its_pitch_class_attack_and_source_evidence() {
    let map = TempoMap::constant(120.).unwrap();
    let source: Vec<_> = (0..=127)
        .map(|pitch| candidate(pitch as f64 * 0.75, pitch as f64 * 0.75 + 0.5, pitch))
        .collect();
    let before = serde_json::to_vec(&source).unwrap();
    let arranged = pro_keys::arrange(&source, &map).unwrap();
    assert_eq!(arranged.notes.len(), 128);
    assert_eq!(arranged.out_of_range_omitted, 0);
    assert_eq!(arranged.decisions.len(), source.len());
    assert_eq!(serde_json::to_vec(&source).unwrap(), before);
    for decision in &arranged.decisions {
        let pitch = decision.target_pitch.unwrap();
        assert_eq!(pitch % 12, decision.source.pitch % 12);
        assert_eq!(
            i16::from(pitch) - i16::from(decision.source.pitch),
            decision.octave_shift * 12
        );
        assert_eq!(decision.tick, map.tick(decision.source.start));
        assert_eq!(decision.disposition, "emitted");
    }
    assert_playable(&arranged.notes);
}

#[test]
fn nearby_high_and_low_motifs_keep_intervals_in_a_consistent_octave() {
    let map = TempoMap::constant(120.).unwrap();
    for pitches in [[80, 82, 85, 82, 80], [20, 22, 25, 22, 20]] {
        let source: Vec<_> = pitches
            .iter()
            .enumerate()
            .map(|(i, p)| candidate(i as f64 * 0.5, i as f64 * 0.5 + 0.4, *p))
            .collect();
        let arranged = pro_keys::arrange(&source, &map).unwrap();
        let shifts: Vec<_> = arranged.decisions.iter().map(|d| d.octave_shift).collect();
        assert!(
            shifts.iter().all(|shift| *shift == shifts[0]),
            "a playable short motif should keep its intervals: {shifts:?}"
        );
        assert_playable(&arranged.notes);
    }
}

#[test]
fn already_playable_chords_keep_original_pitches_and_independent_releases() {
    let map = TempoMap::constant(120.).unwrap();
    let source = vec![
        candidate(0., 2., 60),
        candidate(0., 1., 64),
        candidate(0., 1.5, 67),
    ];
    let arranged = pro_keys::arrange(&source, &map).unwrap();
    assert_eq!(arranged.octave_adapted_candidates, 0);
    assert_eq!(arranged.shortened_releases, 0);
    for decision in &arranged.decisions {
        assert_eq!(decision.source.pitch, decision.target_pitch.unwrap());
        assert_eq!(decision.source_sustain, decision.output_sustain);
    }
    assert_playable(&arranged.notes);
}

#[test]
fn wide_chords_and_held_notes_fit_without_hidden_notes_or_unexplained_loss() {
    let map = TempoMap::constant(120.).unwrap();
    for source in [
        vec![candidate(0., 3., 48), candidate(1., 2., 72)],
        vec![
            candidate(0., 1., 48),
            candidate(0., 1.5, 60),
            candidate(0., 2., 72),
        ],
        vec![
            candidate(0., 2., 24),
            candidate(0.01, 1., 84),
            candidate(0.03, 1.5, 108),
            candidate(0.04, 0.5, 36),
        ],
    ] {
        let arranged = pro_keys::arrange(&source, &map).unwrap();
        assert_eq!(arranged.out_of_range_omitted, 0);
        assert!(arranged.decisions.iter().all(|d| d.target_pitch.is_some()));
        assert_eq!(
            arranged.notes.len()
                + arranged.merged_simultaneous_candidates
                + arranged.merged_octave_collisions,
            source.len()
        );
        assert_playable(&arranged.notes);
        for tier in 0..3 {
            let reduced = pro_keys::reduce(&arranged, &map, tier);
            assert!(reduced.iter().all(|n| arranged.notes.contains(n)));
            assert_playable(&reduced);
        }
    }
}

#[test]
fn folded_octave_jitter_merges_but_real_repeated_attacks_survive() {
    let map = TempoMap::constant(120.).unwrap();
    // Three source octaves must share at most two same-class keys in any
    // visible controller window. Their slightly different model peaks are
    // one chord, not a three-millisecond repeated-key performance.
    let chord = vec![
        candidate(1., 2., 36),
        candidate(1.003, 2.2, 60),
        candidate(1.006, 2.4, 84),
    ];
    let arranged = pro_keys::arrange(&chord, &map).unwrap();
    assert!(arranged.merged_octave_collisions > 0);
    assert_eq!(
        arranged.notes.len() + arranged.merged_octave_collisions,
        chord.len()
    );
    let merged = arranged
        .decisions
        .iter()
        .find(|d| d.disposition == "merged_octave_collision")
        .unwrap();
    assert!(merged.output_tick < merged.tick);
    assert!(merged.output_sustain.is_some());
    assert_playable(&arranged.notes);
    let mut chord_then_repeat = chord.clone();
    chord_then_repeat.push(candidate(1.025, 2.5, merged.source.pitch));
    let repeated = pro_keys::arrange(&chord_then_repeat, &map).unwrap();
    assert_eq!(
        repeated.notes.len(),
        arranged.notes.len() + 1,
        "a real retrigger of a previously merged voice must survive"
    );
    assert_eq!(
        repeated.merged_octave_collisions,
        arranged.merged_octave_collisions
    );
    assert_playable(&repeated.notes);

    for spacing in [0.020, 0.050, 0.125] {
        let repeated = vec![candidate(1., 2., 60), candidate(1. + spacing, 2.4, 60)];
        let arranged = pro_keys::arrange(&repeated, &map).unwrap();
        assert_eq!(
            arranged.notes.len(),
            2,
            "same source key attacks must survive"
        );
        assert_eq!(arranged.merged_octave_collisions, 0);
        assert_playable(&arranged.notes);
    }
    let slower_octaves = vec![
        candidate(1., 2., 36),
        candidate(1.05, 2.2, 60),
        candidate(1.1, 2.4, 84),
    ];
    let arranged = pro_keys::arrange(&slower_octaves, &map).unwrap();
    assert_eq!(arranged.notes.len(), 3);
    assert_eq!(arranged.merged_octave_collisions, 0);
    assert_playable(&arranged.notes);
}

#[test]
fn arbitrary_key_transpositions_and_tempo_changes_share_the_same_invariants() {
    let map = TempoMap::new(
        960,
        0.,
        vec![
            Tempo { tick: 0, bpm: 93. },
            Tempo {
                tick: 3840,
                bpm: 137.,
            },
        ],
    )
    .unwrap();
    for transpose in -24i16..=24 {
        let mut source = Vec::new();
        for index in 0..36 {
            for pitch in [36, 51, 60, 76, 88] {
                source.push(candidate(
                    index as f64 * 0.23,
                    index as f64 * 0.23 + 0.6,
                    (pitch + transpose) as u8,
                ));
            }
        }
        let input = Transcription {
            pro_piano: source.clone(),
            ..Default::default()
        };
        let tracks = mapping::tracks(&input, &map, true).unwrap();
        for notes in tracks.values() {
            assert_playable(notes);
        }
        let arranged = pro_keys::arrange(&source, &map).unwrap();
        assert_eq!(arranged.out_of_range_omitted, 0);
        assert_eq!(
            arranged
                .decisions
                .iter()
                .filter(|d| d.target_pitch.is_some())
                .count(),
            source.len()
        );
    }
}

#[test]
fn invalid_candidates_fail_and_low_confidence_or_pre_offset_notes_stay_explicit() {
    let map = TempoMap::new(480, 0.5, vec![]).unwrap();
    let mut weak = candidate(1., 2., 100);
    weak.confidence = 0.1;
    let arranged = pro_keys::arrange(&[candidate(0., 0.2, 100), weak], &map).unwrap();
    assert!(arranged.notes.is_empty());
    assert_eq!(arranged.decisions[0].disposition, "before_chart_start");
    assert_eq!(arranged.decisions[1].disposition, "below_confidence");
    assert!(pro_keys::arrange(&[candidate(2., 1., 60)], &map).is_err());
}

#[test]
fn future_time_slice_optimization_preserves_complete_dense_transposed_arrangements() {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    let map = TempoMap::new(
        480,
        0.125,
        vec![
            Tempo { tick: 0, bpm: 107. },
            Tempo {
                tick: 3840,
                bpm: 143.,
            },
        ],
    )
    .unwrap();
    for transpose in [-12i16, 0, 12] {
        let mut raw = Vec::new();
        for i in 0..48 {
            let start = 1. + i as f64 * 0.13;
            for (voice, step) in [0i16, 4, 7].into_iter().enumerate() {
                let pitch = (48 + transpose + step + (i % 3) as i16 * 12) as u8;
                raw.push(PitchNote {
                    start: start + voice as f64 * 0.005,
                    end: start + 0.8,
                    pitch,
                    confidence: 0.9,
                });
            }
            for edge in [start + 0.035, start + 1.5] {
                for bits in [edge.to_bits() - 1, edge.to_bits(), edge.to_bits() + 1] {
                    let at = f64::from_bits(bits);
                    raw.push(PitchNote {
                        start: at,
                        end: at + 0.1,
                        pitch: (60 + transpose) as u8,
                        confidence: 0.9,
                    });
                }
            }
        }
        for order in 0..3 {
            let mut source = raw.clone();
            match order {
                1 => source.reverse(),
                2 => source.rotate_left(17),
                _ => {}
            }
            let arranged = pro_keys::arrange_generated(&source, &map).unwrap();
            digest.update(serde_json::to_vec(&arranged).unwrap());
        }
    }
    // Frozen pre-optimization complete receipts: notes, source indices,
    // dispositions, releases, register choices and counters must all agree.
    assert_eq!(
        format!("{:x}", digest.finalize()),
        "b4379d9aa0521b9489bc226b7da777f9eff8db66e484880ddff6bc7b136a360e"
    );
}
