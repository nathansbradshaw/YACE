use chart_engine::{
    models,
    piano_model::{self, Activations, Options, Transcriber},
};
use std::path::Path;

fn activations(frames: usize) -> Activations {
    Activations {
        duration_seconds: frames as f64 / 100.,
        reg_onset: vec![0.; frames * 88],
        reg_offset: vec![0.; frames * 88],
        frame: vec![0.; frames * 88],
        velocity: vec![0.5; frames * 88],
        reg_pedal_offset: vec![0.; frames],
        pedal_frame: vec![0.; frames],
    }
}
fn peak(data: &mut [f32], pitch: u8, frame: usize) {
    let key = (pitch - 21) as usize;
    for (offset, value) in [(-2, 0.1), (-1, 0.4), (0, 0.9), (1, 0.4), (2, 0.1)] {
        data[(frame as isize + offset) as usize * 88 + key] = value;
    }
}
fn held(a: &mut Activations, pitch: u8, start: usize, end: usize) {
    for frame in start..end {
        a.frame[frame * 88 + (pitch - 21) as usize] = 0.9;
    }
}

#[test]
fn regression_decoder_preserves_chord_pitch_independent_releases_velocity_and_pedal() {
    let mut a = activations(100);
    for (pitch, release, disappear) in [(60, 40, 45), (64, 55, 60), (67, 75, 80)] {
        peak(&mut a.reg_onset, pitch, 10);
        peak(&mut a.reg_offset, pitch, release);
        held(&mut a, pitch, 10, disappear);
    }
    for i in 5..90 {
        a.pedal_frame[i] = 0.8;
    }
    for (index, value) in [(68, 0.1), (69, 0.4), (70, 0.9), (71, 0.4), (72, 0.1)] {
        a.reg_pedal_offset[index] = value;
    }
    let result = piano_model::decode(&a, Options::default()).unwrap();
    assert_eq!(
        result.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
        [60, 64, 67]
    );
    assert_eq!(
        result.notes.iter().map(|n| n.start).collect::<Vec<_>>(),
        [0.1; 3]
    );
    assert_eq!(
        result.notes.iter().map(|n| n.end).collect::<Vec<_>>(),
        [0.4, 0.55, 0.75]
    );
    assert_eq!(result.velocities, [64; 3]);
    assert_eq!(result.pedals.len(), 1);
    assert_eq!(result.pedals[0].start, 0.05);
    assert_eq!(result.pedals[0].end, 0.7);
    assert_eq!(
        result.notes[0].end, 0.4,
        "Pedal must not extend a controller's key release"
    );
}

#[test]
fn retriggers_finish_previous_attack_without_duplicates_and_silence_stays_empty() {
    let mut a = activations(100);
    peak(&mut a.reg_onset, 60, 10);
    peak(&mut a.reg_onset, 60, 25);
    held(&mut a, 60, 10, 60);
    let result = piano_model::decode(&a, Options::default()).unwrap();
    assert_eq!(result.notes.len(), 2);
    assert_eq!((result.notes[0].start, result.notes[0].end), (0.1, 0.24));
    assert_eq!((result.notes[1].start, result.notes[1].end), (0.25, 0.6));
    assert!(
        piano_model::decode(&activations(100), Options::default())
            .unwrap()
            .notes
            .is_empty()
    );
    a.reg_onset[0] = f32::NAN;
    assert!(piano_model::decode(&a, Options::default()).is_err());
    a.reg_onset[0] = 0.;
    a.velocity.pop();
    assert!(piano_model::decode(&a, Options::default()).is_err());
    assert!(
        piano_model::decode(
            &activations(100),
            Options {
                onset_threshold: f32::NAN,
                ..Options::default()
            }
        )
        .is_err()
    );
}

#[test]
fn holds_over_twelve_seconds_follow_frame_offset_or_audio_end_without_a_length_cap() {
    let mut a = activations(1601);
    a.duration_seconds = 16.;
    // All three notes exceed the former six-second cap and cross both central
    // stitching seams. Their releases come from three independent sources.
    peak(&mut a.reg_onset, 60, 50);
    held(&mut a, 60, 50, 1380); // Frame release at13.8s, with no offset peak.
    peak(&mut a.reg_onset, 64, 60);
    held(&mut a, 64, 60, 1350);
    peak(&mut a.reg_offset, 64, 1320); // Supported offset before frame decay.
    peak(&mut a.reg_onset, 67, 70);
    held(&mut a, 67, 70, 1601); // Still sounding at the exact16s source end.
    for frame in 40..1500 {
        a.pedal_frame[frame] = 0.8;
    }
    for (frame, value) in [
        (1448, 0.1),
        (1449, 0.4),
        (1450, 0.9),
        (1451, 0.4),
        (1452, 0.1),
    ] {
        a.reg_pedal_offset[frame] = value;
    }
    let output = piano_model::decode(&a, Options::default()).unwrap();
    assert_eq!(
        output.notes.len(),
        3,
        "One real onset per pitch; no cap-created fragments"
    );
    for (note, (pitch, start, end)) in
        output
            .notes
            .iter()
            .zip([(60, 0.5, 13.8), (64, 0.6, 13.2), (67, 0.7, 16.)])
    {
        assert_eq!((note.pitch, note.start, note.end), (pitch, start, end));
        assert!(note.end - note.start > 12.);
        assert!(note.end <= a.duration_seconds);
    }
    assert_eq!(output.pedals.len(), 1);
    assert_eq!(output.pedals[0].end, 14.5);
    assert_eq!(
        output.notes[0].end, 13.8,
        "Pedal does not inflate key holds"
    );
}

#[test]
fn repeated_keys_around_stitch_seams_keep_every_onset_and_release() {
    let mut a = activations(2201);
    a.duration_seconds = 22.;
    // True40ms repetitions straddle the7.5s/12.5s retained-window seams.
    // The first hold also exceeds six seconds before its real retrigger.
    let onsets = [50, 748, 752, 1248, 1252, 1700];
    for frame in onsets {
        peak(&mut a.reg_onset, 60, frame);
    }
    held(&mut a, 60, 50, 2000);
    let output = piano_model::decode(&a, Options::default()).unwrap();
    assert_eq!(
        output.notes.len(),
        onsets.len(),
        "Seams cannot erase or invent retriggers"
    );
    for (index, note) in output.notes.iter().enumerate() {
        assert_eq!(note.pitch, 60);
        assert_eq!(note.start, onsets[index] as f64 / 100.);
        let expected_end = if index + 1 < onsets.len() {
            (onsets[index + 1] - 1) as f64 / 100.
        } else {
            20.
        };
        assert_eq!(note.end, expected_end);
        assert!(note.end > note.start && note.end <= a.duration_seconds);
    }
    assert!(
        output
            .notes
            .windows(2)
            .all(|pair| pair[0].end < pair[1].start)
    );
}

#[test]
#[ignore = "requires verified Kong ONNX checkpoint; no Python"]
fn native_piano_specific_onnx_detects_known_chords_and_rejects_silence() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/piano_kong.onnx");
    let mut model = Transcriber::load(&path).unwrap();
    let mut signal = vec![0.; 16000 * 6];
    for (start, end, pitches) in [(0.6, 1.6, [60, 64, 67]), (2.5, 3.5, [62, 65, 69])] {
        for (index, sample) in signal.iter_mut().enumerate() {
            let t = index as f64 / 16000.;
            if t >= start && t < end {
                let elapsed = t - start;
                let envelope =
                    (elapsed / 0.01).min(1.) * (-1.3 * elapsed).exp() * ((end - t) / 0.04).min(1.);
                for pitch in pitches {
                    let frequency = 440. * 2f64.powf((pitch as f64 - 69.) / 12.);
                    *sample += (envelope
                        * ((std::f64::consts::TAU * frequency * elapsed).sin()
                            + 0.25 * (2. * std::f64::consts::TAU * frequency * elapsed).sin())
                        * 0.08) as f32;
                }
            }
        }
    }
    let emissions = model.infer_samples(&signal).unwrap();
    let output = piano_model::decode(&emissions, Options::default()).unwrap();
    let pitches: std::collections::BTreeSet<_> = output.notes.iter().map(|n| n.pitch).collect();
    assert!(
        [60, 64, 67].iter().filter(|p| pitches.contains(p)).count() >= 2,
        "Known piano chord should retain multiple real voices: {output:?}"
    );
    assert!(
        output
            .notes
            .iter()
            .all(|n| n.start >= 0. && n.end <= 6. && n.end > n.start)
    );
    let silent = piano_model::decode(
        &model.infer_samples(&vec![0.; 16000 * 6]).unwrap(),
        Options::default(),
    )
    .unwrap();
    assert!(
        silent.notes.is_empty(),
        "Silent input generated piano candidates: {silent:?}"
    );
    println!(
        "Real Kong ONNX inference: {} notes,pitches{pitches:?},{} pedal events; silence rejected",
        output.notes.len(),
        output.pedals.len()
    );
}

#[test]
#[ignore = "requires verified Kong model and locally prepared In the End piano-stem corpus"]
fn native_in_the_end_intro_stays_in_source_time_and_saves_raw_candidates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let audio = root.join("output/keyboard-benchmark-20261027/corpus/in-the-end-0/piano.wav");
    let model = root.join("models/piano_kong.onnx");
    let output = Transcriber::load(&model)
        .unwrap()
        .transcribe(&audio, Options::default())
        .unwrap();
    assert!(!output.notes.is_empty());
    assert!(
        output
            .notes
            .iter()
            .all(|n| n.start >= 0. && n.end > n.start && n.end <= 16.)
    );
    assert_eq!(output.notes.len(), output.velocities.len());
    if let Some(path) = std::env::var_os("YACE_KONG_SMOKE_OUTPUT") {
        let path = std::path::PathBuf::from(path);
        assert!(!path.exists(), "Refusing to replace existing evidence");
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"model_sha256":piano_model::MODEL_SHA256,"audio_sha256":models::hash_file(&audio).unwrap(),"output":output,"musical_acceptance":"unverified; raw transcription of unannotated real recording"})).unwrap()).unwrap();
    }
    println!(
        "In the End piano-stem intro: {} raw Kong candidates, first{:?}; source-clock bounds valid, accuracy unverified",
        output.notes.len(),
        &output.notes[..output.notes.len().min(8)]
    );
}
