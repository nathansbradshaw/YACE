use chart_engine::{
    vocal_generation::{PitchFrame, PitchSupport},
    vocal_note_consensus as nc,
    vocal_pitch::{self, Frame},
    vocal_rmvpe,
};
fn fixture() -> (
    Vec<PitchFrame>,
    vocal_pitch::Evidence,
    vocal_pitch::Evidence,
    nc::Evidence,
) {
    let pcm = vec![0f32; 16384];
    let frame = |time| Frame {
        time_seconds: time,
        frequency_hz: Some(220.),
        confidence: 0.9,
        rms: 0.1,
    };
    let mut primary =
        vocal_rmvpe::finish_pcm(&pcm, (0..103).map(|i| frame(i as f64 * 0.01)).collect()).unwrap();
    let mut swift =
        vocal_pitch::finish_pcm(&pcm, (0..64).map(|i| frame(i as f64 * 0.016)).collect()).unwrap();
    for e in [&mut primary, &mut swift] {
        e.audio_sha256 = "a".repeat(64);
        e.vocal_stem_sha256 = "b".repeat(64)
    }
    let notes = nc::Evidence {
        algorithm: nc::NOTE_ALGORITHM.into(),
        implementation_sha256: chart_engine::vocal_note_recipe::implementation_hash(),
        checkpoint_revision: "3c8332bf43adae35f6e4d64971862f2f6139b310".into(),
        stem_sha256: "b".repeat(64),
        encoder_sha256: nc::ENCODER_SHA.into(),
        decoder_sha256: nc::DECODER_SHA.into(),
        rwbd_sha256: nc::RWBD_SHA.into(),
        duration_seconds: 1.024,
        notes: vec![nc::Note {
            start_seconds: 0.,
            end_seconds: 1.024,
            midi: 57,
        }],
    };
    let frames = (0..100)
        .map(|i| PitchFrame {
            time_seconds: i as f64 * 0.01,
            midi: (!(30..40).contains(&i)).then_some(60.),
            yin_midi: Some(60.),
            mcleod_midi: Some(60.),
            clarity: 0.9,
            rms: 0.1,
            local_rms: Some(0.1),
            local_energy_active: Some(true),
            support: PitchSupport::Consensus,
        })
        .collect();
    (frames, primary, swift, notes)
}
#[test]
fn corroborated_pitch_correction_keeps_every_voiced_frame_and_gap() {
    let (mut f, p, s, n) = fixture();
    let mask: Vec<_> = f.iter().map(|f| f.midi.is_some()).collect();
    assert_eq!(nc::refine(&mut f, &p, &s, &n, &|| false).unwrap(), 90);
    assert_eq!(mask, f.iter().map(|f| f.midi.is_some()).collect::<Vec<_>>());
    assert!(f.iter().filter_map(|f| f.midi).all(|p| p == 57.));
    assert!(
        f.iter()
            .filter(|f| f.midi.is_some())
            .all(|f| f.support == PitchSupport::AcousticAlternative && f.clarity <= 0.5)
    );
}
#[test]
fn disagreement_low_confidence_silence_and_unsupported_octaves_cannot_rewrite() {
    for mode in 0..4 {
        let (mut f, p, mut s, mut n) = fixture();
        match mode {
            0 => n.notes[0].midi = 69,
            1 => s.frames.iter_mut().for_each(|f| f.confidence = 0.49),
            2 => f.iter_mut().for_each(|f| f.midi = None),
            _ => n.notes[0].midi = 30,
        };
        let before = serde_json::to_vec(&f).unwrap();
        assert_eq!(nc::refine(&mut f, &p, &s, &n, &|| false).unwrap(), 0);
        assert_eq!(before, serde_json::to_vec(&f).unwrap());
    }
}
#[test]
fn wrong_bindings_nonfinite_values_and_cancellation_are_atomic() {
    for mode in 0..5 {
        let (mut f, p, mut s, mut n) = fixture();
        match mode {
            0 => n.encoder_sha256 = "c".repeat(64),
            1 => n.stem_sha256 = "c".repeat(64),
            2 => s.pcm_sha256 = "c".repeat(64),
            3 => n.notes[0].start_seconds = f64::NAN,
            _ => {}
        }
        let before = serde_json::to_vec(&f).unwrap();
        assert!(nc::refine(&mut f, &p, &s, &n, &|| mode == 4).is_err());
        assert_eq!(before, serde_json::to_vec(&f).unwrap());
    }
}
