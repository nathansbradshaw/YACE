use chart_engine::{
    timing::{Tempo, TempoMap},
    vocal_benchmark::{Reference, ReferenceFrame, ReferenceNote, score, score_exported_notes},
    vocal_generation::{Generation, NoteEvidence, PitchFrame, PitchSupport, Policy, Status},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
fn fixture() -> (Reference, Generation) {
    let t = TempoMap::new(480, 0., vec![Tempo { tick: 0, bpm: 120. }]).unwrap();
    let reference = Reference {
        reference_kind: "authored_chart".into(),
        audio_sha256: "a".repeat(64),
        duration_seconds: 1.,
        provenance: "Synthetic reviewed target chart on the recording clock".into(),
        reviewed: true,
        frames: (0..100)
            .map(|i| ReferenceFrame {
                time_seconds: i as f64 * 0.01 + 0.005,
                midi: Some(60.),
                backing_midi: vec![],
            })
            .collect(),
        notes: vec![ReferenceNote {
            start_seconds: 0.,
            end_seconds: 1.,
            midi: 60,
        }],
        words: vec![],
    };
    let g = Generation {
        version: 1,
        audio_sha256: "a".repeat(64),
        vocal_stem_sha256: "b".repeat(64),
        duration_seconds: 1.,
        algorithm: "Test detector differs from assembled notes".into(),
        timing: Some(t),
        policy: Policy::default(),
        status: Status::ReviewRequired,
        chart: Some(VocalChart {
            version: 1,
            resolution: 480,
            source: "Synthetic chart".into(),
            phrases: vec![VocalPhrase {
                midi: None,
                start: 0,
                end: 960,
                player: 1,
            }],
            notes: vec![VocalNote {
                start: 0,
                end: 960,
                tone: VocalTone::Pitched { midi: 60 },
                lyric: Syllable::Missing,
                midi: None,
            }],
        }),
        pitch_candidates: vec![],
        pitch_frames: (0..100)
            .map(|i| PitchFrame {
                time_seconds: i as f64 * 0.01,
                midi: Some(72.),
                yin_midi: Some(72.),
                mcleod_midi: Some(72.),
                clarity: 0.9,
                rms: 0.1,
                local_rms: Some(0.1),
                local_energy_active: Some(true),
                support: PitchSupport::Consensus,
            })
            .collect(),
        words: None,
        lyrics: None,
        issues: vec![],
        warnings: vec![],
        neural_pitch: None,
        vocal_pitch: None,
        rmvpe_pitch: None,
        note_evidence: vec![NoteEvidence {
            models: vec![],
            start: 0,
            end: 960,
            selected_midi: 60,
            alternatives: vec![72],
            support: vec![PitchSupport::AcousticAlternative],
            uncertain: true,
            reasons: vec!["Uncertainty must not hide this output note from evaluation".into()],
        }],
    };
    (reference, g)
}
#[test]
fn detector_success_cannot_hide_wrong_exported_pitches() {
    let (r, mut g) = fixture();
    assert_eq!(score(&r, &g).unwrap().incorrect_pitch_frames, 100);
    assert_eq!(
        score_exported_notes(&r, &g).unwrap().correct_pitch_frames,
        100
    );
    for f in &mut g.pitch_frames {
        f.midi = Some(60.)
    }
    g.chart.as_mut().unwrap().notes[0].tone = VocalTone::Pitched { midi: 72 };
    assert_eq!(score(&r, &g).unwrap().correct_pitch_frames, 100);
    let m = score_exported_notes(&r, &g).unwrap();
    assert_eq!(m.incorrect_pitch_frames, 100);
    assert_eq!(m.octave_error_frames, 100);
}
#[test]
fn gaps_and_extra_singing_are_visible_without_lyrics() {
    let (mut r, mut g) = fixture();
    g.chart.as_mut().unwrap().notes[0].start = 480;
    let m = score_exported_notes(&r, &g).unwrap();
    assert_eq!(m.detected_voiced_frames, 50);
    assert_eq!(m.missed_voiced_frames, 50);
    g.chart.as_mut().unwrap().notes[0].start = 0;
    for f in &mut r.frames[50..] {
        f.midi = None
    }
    r.notes[0].end_seconds = 0.5;
    let m = score_exported_notes(&r, &g).unwrap();
    assert_eq!(m.reference_unvoiced_frames, 50);
    assert_eq!(m.false_voiced_frames, 50);
}
#[test]
fn missing_chart_and_missing_timing_are_distinct() {
    let (r, mut g) = fixture();
    g.timing = None;
    assert!(score_exported_notes(&r, &g).is_err());
    g.chart = None;
    let m = score_exported_notes(&r, &g).unwrap();
    assert_eq!(m.missed_voiced_frames, 100);
    assert_eq!(m.detected_voiced_frames, 0);
}
#[test]
fn model_observations_and_source_bound_import_retain_review_evidence() {
    let (_, mut g) = fixture();
    let model = chart_engine::vocal_posterior::ModelFrames {
        id: "Test independent pitch band".into(),
        model_sha256: "c".repeat(64),
        implementation_sha256: "d".repeat(64),
        midi: vec![Some(60.); 100],
    };
    chart_engine::vocal_posterior::annotate(&mut g, &[model]).unwrap();
    let evidence = &g.note_evidence[0].models[0];
    assert_eq!(evidence.selected_note_support_frames, 100);
    assert_eq!(evidence.selected_midi, Some(60));
    let bytes = serde_json::to_vec(&g).unwrap();
    match chart_engine::vocal_review::import_document(&bytes).unwrap() {
        chart_engine::editor::Action::SetVocalGeneration { document, review } => {
            assert_eq!(Some(document), g.chart);
            assert_eq!(
                review.note_evidence[0].models[0].selected_note_support_frames,
                100
            );
            review.validate().unwrap();
        }
        _ => panic!("Analysis must retain source-bound review data"),
    }
    g.note_evidence[0].models[0].inspected_frames = 0;
    assert!(chart_engine::vocal_review::import_document(&serde_json::to_vec(&g).unwrap()).is_err());
}
