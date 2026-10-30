use chart_engine::mt3::{self, Event, Segment};
fn s(start: f64, tokens: &[u32]) -> Segment {
    Segment {
        start,
        tokens: tokens.to_vec(),
        eos: true,
        token_limit_reached: false,
    }
}
const TIE: u32 = 339;
const ON: u32 = 338;
const OFF: u32 = 337;
fn shift(i: u32) -> u32 {
    3 + i
}
fn pitch(p: u8) -> u32 {
    209 + p as u32
}
fn program(p: u8) -> u32 {
    340 + p as u32
}
#[test]
fn exact_pinned_vocabulary_ranges_match_midi_program_and_binary_velocity() {
    for (token, event) in [
        (3, Event::Shift(0)),
        (208, Event::Shift(205)),
        (209, Event::Pitch(0)),
        (336, Event::Pitch(127)),
        (337, Event::Velocity(0)),
        (338, Event::Velocity(1)),
        (339, Event::Tie),
        (340, Event::Program(0)),
        (467, Event::Program(127)),
        (468, Event::Drum(0)),
        (595, Event::Drum(127)),
        (1, Event::Eos),
    ] {
        assert_eq!(mt3::event(token), Some(event));
    }
    assert_eq!(mt3::event(0), None);
    assert_eq!(mt3::event(2), None);
    assert_eq!(mt3::event(596), None);
}
#[test]
fn yourmt3_shift_tokens_are_absolute_even_when_consecutive() {
    let stream = s(
        0.,
        &[
            TIE,
            program(80),
            shift(150),
            shift(50),
            ON,
            pitch(60),
            shift(150),
            OFF,
            pitch(60),
        ],
    );
    let (notes, dropped, _) = mt3::decode_segments(&[stream], 2.1).unwrap();
    assert_eq!(dropped, 0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].start, 0.5);
    assert_eq!(notes[0].end, 1.5);
    assert_eq!(notes[0].program, 80);
    assert_eq!(notes[0].velocity, 127);
}
#[test]
fn tied_holds_survive_sample_exact_seams_and_retriggers_stay_separate() {
    let boundary = 32767. / 16000.;
    let a = s(0., &[TIE, program(81), shift(50), ON, pitch(64)]);
    let b = s(
        boundary,
        &[
            program(81),
            pitch(64),
            TIE,
            shift(10),
            ON,
            pitch(64),
            shift(80),
            OFF,
            pitch(64),
        ],
    );
    let (notes, dropped, _) = mt3::decode_segments(&[a, b], 4.).unwrap();
    assert_eq!(dropped, 0);
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].start, 0.5);
    assert!((notes[0].end - 2.15).abs() < 1e-9);
    assert_eq!(notes[1].start, notes[0].end);
    assert!((notes[1].end - 2.85).abs() < 1e-9);
}
#[test]
fn program_identity_drums_and_unannounced_releases_remain_distinct() {
    let boundary = 32767. / 16000.;
    let a = s(
        0.,
        &[
            TIE,
            shift(50),
            ON,
            program(0),
            pitch(60),
            program(80),
            pitch(60),
            468 + 36,
        ],
    );
    let b = s(
        boundary,
        &[program(80), pitch(60), TIE, shift(50), OFF, pitch(60)],
    );
    let (notes, dropped, _) = mt3::decode_segments(&[a, b], 4.).unwrap();
    assert_eq!(dropped, 0);
    assert_eq!(notes.len(), 3);
    let piano = notes.iter().find(|n| !n.is_drum && n.program == 0).unwrap();
    let synth = notes
        .iter()
        .find(|n| !n.is_drum && n.program == 80)
        .unwrap();
    assert_eq!(piano.end, boundary);
    assert!((synth.end - 2.55).abs() < 1e-9);
    let drum = notes.iter().find(|n| n.is_drum).unwrap();
    assert_eq!(drum.pitch, 36);
    assert!((drum.end - drum.start - 0.01).abs() < 1e-9);
}
#[test]
fn source_end_clips_padding_notes_and_refuses_wrong_segment_clocks() {
    let a = s(
        0.,
        &[
            TIE,
            program(80),
            shift(10),
            ON,
            pitch(60),
            shift(190),
            OFF,
            pitch(60),
            shift(200),
            ON,
            pitch(67),
        ],
    );
    let (notes, _, clipped) = mt3::decode_segments(&[a], 0.8).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].end, 0.8);
    assert_eq!(clipped, 1);
    assert!(mt3::decode_segments(&[s(0.1, &[])], 1.).is_err());
    assert!(mt3::decode_segments(&[s(0., &[999])], 1.).is_err());
}
#[test]
#[ignore = "requires pinned local MT3 assets and prepared corpus; real native neural inference"]
fn native_mt3_controlled_sanity_and_final_countdown_saves_raw_program_notes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut model = mt3::Transcriber::load(&root.join("models/mt3")).unwrap();
    let silence = model.infer_samples(&vec![0.; 32767]).unwrap();
    assert!(
        silence.notes.is_empty(),
        "Silence yielded{} notes",
        silence.notes.len()
    );
    let fixture =
        root.join("output/keyboard-benchmark-20261027/corpus/fixture-square-lead/mix.wav");
    let fixture = model.transcribe(&fixture).unwrap();
    assert!(
        !fixture.notes.is_empty(),
        "Controlled lead yielded no notes"
    );
    assert!(
        fixture
            .notes
            .iter()
            .all(|n| n.end <= fixture.duration_seconds)
    );
    let output = root.join("output/keyboard-benchmark-20261027/mt3-candidate");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        output.join("controlled-square-lead.json"),
        serde_json::to_vec_pretty(&fixture).unwrap(),
    )
    .unwrap();
    for name in ["keyboard-bed", "mix"] {
        let audio = root.join(format!(
            "output/keyboard-benchmark-20261027/corpus/final-countdown-0/{name}.wav"
        ));
        let result = model.transcribe(&audio).unwrap();
        assert!(
            result
                .notes
                .iter()
                .all(|n| n.start >= 0. && n.end <= result.duration_seconds && n.end > n.start)
        );
        let notes = result.notes.len();
        let programs: std::collections::BTreeSet<_> = result
            .notes
            .iter()
            .filter(|n| !n.is_drum)
            .map(|n| n.program)
            .collect();
        std::fs::write(
            output.join(format!("final-countdown-{name}.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        eprintln!("MT3 Final Countdown{name}: {notes} notes, programs{programs:?}");
    }
}

#[test]
fn malformed_tie_and_missing_state_do_not_invent_attacks() {
    let (notes, dropped, _) =
        mt3::decode_segments(&[s(0., &[program(80), shift(50), ON, pitch(60)])], 2.).unwrap();
    assert!(notes.is_empty());
    assert!(dropped > 0);
    let (notes, dropped, _) = mt3::decode_segments(
        &[s(
            0.,
            &[
                TIE,
                shift(50),
                pitch(60),
                program(80),
                pitch(64),
                ON,
                pitch(67),
                shift(100),
                OFF,
                pitch(67),
            ],
        )],
        2.,
    )
    .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].pitch, 67);
    assert_eq!(dropped, 2);
}

#[test]
fn long_tied_holds_end_at_real_source_eof_without_ten_second_truncation() {
    let mut segments = vec![s(0., &[TIE, program(80), shift(10), ON, pitch(60)])];
    for i in 1..7 {
        segments.push(s(
            i as f64 * 32767. / 16000.,
            &[program(80), pitch(60), TIE],
        ));
    }
    let (notes, dropped, _) = mt3::decode_segments(&segments, 14.2).unwrap();
    assert_eq!(dropped, 0);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].start, 0.1);
    assert_eq!(notes[0].end, 14.2);
}
