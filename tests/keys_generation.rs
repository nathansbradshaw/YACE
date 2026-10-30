use chart_engine::{
    chart::{Chart, Note, Tracks},
    editor::{Action, Actor, ActorKind, Request, Store},
    game_export,
    mapping::{self, PitchNote, Transcription},
    midi, models,
    pipeline::{self, Build, IsolationOptions},
    stem_review::{AlignmentEvidence, StemId},
    timing::TempoMap,
};
use std::{collections::BTreeMap, fs, path::Path};

#[test]
fn five_lane_compression_merges_jitter_without_deleting_real_retriggers() {
    let timing = TempoMap::constant(120.).unwrap();
    let source = vec![
        candidate(1., 2., 40),
        candidate(1.005, 2.1, 41),
        candidate(1.025, 2.2, 41),
        candidate(3., 4., 100),
    ];
    let notes = chart_engine::keys::arrange(&source, &timing, 3, false).unwrap();
    assert_eq!(notes.len(), 3);
    assert_eq!(notes[0].tick, timing.tick(1.).unwrap());
    assert_eq!(
        notes[1].tick,
        timing.tick(1.025).unwrap(),
        "repeat of a source pitch already merged into the game lane must survive"
    );
    assert_eq!(notes[0].tick + notes[0].sustain, notes[1].tick);
    assert_eq!(notes[0].lane, notes[1].lane);
}

fn candidate(start: f64, end: f64, pitch: u8) -> PitchNote {
    PitchNote {
        start,
        end,
        pitch,
        confidence: 0.9,
    }
}
fn events() -> Transcription {
    let notes = vec![
        candidate(0.5, 1.5, 60),
        candidate(0.5, 1.0, 64),
        candidate(0.5, 1.25, 67),
        candidate(1.0, 1.75, 60),
        candidate(2.0, 2.4, 72),
        candidate(2.5, 2.9, 84),
    ];
    Transcription {
        piano: notes.clone(),
        pro_piano: notes,
        ..Default::default()
    }
}
fn note_spans(bytes: &[u8], label: &str) -> Vec<(u64, u8, u64)> {
    let smf = midly::Smf::parse(bytes).unwrap();
    let track = smf.tracks.iter().find(|t| t.iter().any(|e| matches!(e.kind, midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(n)) if n == label.as_bytes()))).unwrap();
    let mut tick = 0;
    let mut active = BTreeMap::new();
    let mut spans = Vec::new();
    for event in track {
        tick += event.delta.as_int() as u64;
        if let midly::TrackEventKind::Midi { message, .. } = event.kind {
            match message {
                midly::MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    assert!(
                        active.insert(key.as_int(), tick).is_none(),
                        "overlapping same-key note-ons"
                    );
                }
                midly::MidiMessage::NoteOff { key, .. }
                | midly::MidiMessage::NoteOn { key, .. } => {
                    let start = active.remove(&key.as_int()).expect("paired note off");
                    spans.push((start, key.as_int(), tick - start));
                }
                _ => {}
            }
        }
    }
    assert!(active.is_empty());
    spans.sort();
    spans
}

#[test]
fn both_arrangements_keep_playable_chords_and_fit_other_source_octaves() {
    let map = TempoMap::constant(120.).unwrap();
    let tracks = mapping::tracks(&events(), &map, true).unwrap();
    assert_eq!(tracks.len(), 8);
    assert!(tracks["ExpertKeyboard"].iter().all(|n| n.lane <= 4));
    let pro = &tracks["ExpertProKeys"];
    assert!(pro.iter().all(|n| (48..=72).contains(&n.lane)));
    for pitch in [60, 64, 67, 72] {
        assert!(pro.iter().any(|n| n.lane == pitch));
    }
    assert!(!pro.iter().any(|n| n.lane == 84 || n.lane == 48));
    assert!(
        pro.iter()
            .any(|n| n.tick == map.tick(2.5).unwrap() && n.lane == 72),
        "High source MIDI84 must be arranged, not discarded"
    );
    assert_eq!(
        pro.iter()
            .filter(|n| n.tick == map.tick(0.5).unwrap())
            .count(),
        3
    );
    assert_eq!(
        pro.iter().find(|n| n.lane == 60).unwrap().sustain,
        map.tick(1.0).unwrap() - map.tick(0.5).unwrap()
    );
    for tier in ["Easy", "Medium", "Hard"] {
        assert!(
            tracks[&format!("{tier}ProKeys")]
                .iter()
                .all(|n| pro.iter().any(|e| e.tick == n.tick && e.lane == n.lane))
        );
    }
    assert!(!chart_engine::generated_rules::assess(&tracks, &map, 4.).has_errors());
    let bytes = midi::new_midi(&map, &tracks).unwrap();
    assert_eq!(
        midi::inventory(&bytes)
            .unwrap()
            .iter()
            .filter(|(n, c)| n.starts_with("PART REAL_KEYS_") && *c > 0)
            .count(),
        4
    );
    let spans = note_spans(&bytes, "PART REAL_KEYS_X");
    assert!(
        spans.iter().any(|(_, key, _)| *key == 9),
        "High MIDI72 needs YARG's highest display window"
    );
    for n in pro {
        assert!(spans.contains(&(n.tick, n.lane, n.sustain)));
    }
    assert!(
        note_spans(&bytes, "PART KEYS")
            .iter()
            .any(|(_, key, _)| (96..=100).contains(key))
    );
}

fn build_project(root: &Path, audio: &Path, events: &Transcription) -> Store {
    let song = root.join("song");
    pipeline::build(Build {
        audio,
        output: &song,
        existing: None,
        timing: Some(TempoMap::constant(120.).unwrap()),
        transcription: events,
        meta: pipeline::Metadata {
            name: "Piano fixture".into(),
            artist: "Rust test".into(),
            duration: 4.,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    assert!(song.join("notes.mid").is_file());
    let evidence: serde_json::Value = serde_json::from_slice(
        &fs::read(song.join("chart-engine-pro-keys-arrangement.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["arrangement"]["out_of_range_omitted"], 0);
    assert_eq!(
        evidence["arrangement"]["decisions"]
            .as_array()
            .unwrap()
            .len(),
        events.pro_piano.len()
    );
    assert!(
        evidence["source_transcription_sha256"]
            .as_str()
            .is_some_and(|sha| sha
                == models::hash_file(&song.join("chart-engine-transcription.json")).unwrap())
    );
    assert!(!song.join("notes.chart").exists());
    assert!(
        pipeline::editable_chart_file(&song)
            .unwrap()
            .ends_with("chart-engine-working/notes.chart")
    );
    Store::create(&song, None, &root.join("project")).unwrap()
}

#[test]
fn piano_only_song_opens_edits_undoes_and_exports_one_midi_with_bound_recovery_master() {
    let root = tempfile::tempdir().unwrap();
    let audio = root.path().join("audio.wav");
    let samples = vec![0.; 44100 * 4];
    models::write_audio(&audio, &samples, &samples).unwrap();
    let store = build_project(root.path(), &audio, &events());
    let doc = store.load().unwrap();
    assert_eq!(doc.content.tracks.len(), 8);
    assert!(
        doc.review
            .regions
            .iter()
            .any(|r| r.tracks.contains("ExpertProKeys"))
    );
    assert!(chart_engine::octave_adapter::snapshot(&doc, &doc.timing(&store).unwrap()).is_ok());
    let id = doc.content.tracks["ExpertProKeys"][0].id.clone();
    let apply = |action| {
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Piano test".into(),
                },
                action,
            })
            .unwrap()
    };
    apply(Action::SelectTrack {
        track: "ExpertProKeys".into(),
    });
    apply(Action::UpdateNote {
        note_id: id.clone(),
        tick: 480,
        lane: 61,
        sustain: 120,
        cymbal: false,
    });
    assert_eq!(
        store.load().unwrap().find_note(&id).unwrap().1.note.lane,
        61
    );
    apply(Action::Undo);
    assert_ne!(
        store.load().unwrap().find_note(&id).unwrap().1.note.lane,
        61
    );
    apply(Action::Redo);
    let vocals = chart_engine::vocals::VocalChart {
        version: 1,
        resolution: 480,
        source: "Supplied test vocal".into(),
        phrases: vec![chart_engine::vocals::VocalPhrase {
            start: 400,
            end: 900,
            player: 1,
        }],
        notes: vec![chart_engine::vocals::VocalNote {
            start: 480,
            end: 800,
            tone: chart_engine::vocals::VocalTone::Pitched { midi: 60 },
            lyric: chart_engine::vocals::Syllable::Text {
                text: "Test".into(),
                join_next: false,
            },
        }],
    };
    apply(Action::SetVocals {
        document: vocals.clone(),
    });
    let export = root.path().join("export");
    apply(Action::Export {
        output: export.clone(),
    });
    assert!(export.join("notes.mid").is_file());
    assert!(!export.join("notes.chart").exists());
    assert!(
        note_spans(
            &fs::read(export.join("notes.mid")).unwrap(),
            "PART REAL_KEYS_X"
        )
        .contains(&(480, 61, 120))
    );
    assert_eq!(
        chart_engine::vocals::import_midi(&fs::read(export.join("notes.mid")).unwrap())
            .unwrap()
            .notes,
        vocals.notes
    );
    assert!(export.join("editor-source/notes.chart").is_file());
    let reopened = Store::create(&export, None, &root.path().join("reopened")).unwrap();
    assert!(
        reopened.load().unwrap().content.tracks["ExpertProKeys"]
            .iter()
            .any(|n| n.note.tick == 480 && n.note.lane == 61)
    );
    assert_eq!(
        reopened.load().unwrap().content.vocals.unwrap().notes,
        vocals.notes
    );
    let master = pipeline::editable_chart_file(&root.path().join("song")).unwrap();
    fs::write(master, "[Song]\n{\n}\n").unwrap();
    assert!(pipeline::editable_chart_file(&root.path().join("song")).is_err());
}

#[test]
fn midi_named_piano_is_not_misclassified_as_guitar_and_strict_export_rejects_bad_pro_pitches() {
    let bytes = [
        b"MThd\0\0\0\x06\0\x01\0\x01\x01\xe0".as_slice(),
        b"MTrk\0\0\0\x19\0\xff\x03\x05Piano\0\x90\x3c\x64\x83\x60\x80\x3c\0\0\xff\x2f\0".as_slice(),
    ]
    .concat();
    let imported = midi::import(&bytes).unwrap();
    assert_eq!(imported.piano.len(), 1);
    assert_eq!(imported.pro_piano.len(), 1);
    assert!(imported.guitar.is_empty());
    let map = TempoMap::constant(120.).unwrap();
    let chart = Chart::parse(chart_engine::chart::new_chart(
        "",
        "",
        &map,
        &Tracks::from([(
            "ExpertProKeys".into(),
            vec![Note {
                tick: 0,
                lane: 84,
                sustain: 10,
            }],
        )]),
    ))
    .unwrap();
    assert!(game_export::song(&chart, None).is_err());
    assert_eq!(chart_engine::timeline::lane_rows("ExpertProKeys").len(), 25);
    assert!(!chart_engine::editor::playable("ExpertKeyboard", 7));
}

#[test]
#[ignore = "requires local verified Kong piano model and FFmpeg"]
fn real_piano_stem_inference_produces_both_editable_playable_drafts() {
    let temporary = tempfile::tempdir().unwrap();
    let saved = std::env::var_os("YACE_PIANO_EVIDENCE_DIR").map(std::path::PathBuf::from);
    let root = saved.as_deref().unwrap_or(temporary.path());
    if saved.is_some() {
        fs::create_dir(root).unwrap();
    }
    let mut signal = vec![0.; 44100 * 4];
    for (start, end, pitches) in [(0.5, 1.5, vec![60, 64, 67]), (2., 3.2, vec![62, 65, 69])] {
        for (index, sample) in signal.iter_mut().enumerate() {
            let t = index as f64 / 44100.;
            if t >= start && t < end {
                let elapsed = t - start;
                let envelope =
                    (elapsed / 0.01).min(1.) * (-1.3 * elapsed).exp() * ((end - t) / 0.04).min(1.);
                for pitch in &pitches {
                    let frequency = 440. * 2f64.powf((*pitch as f64 - 69.) / 12.);
                    *sample += (envelope
                        * ((std::f64::consts::TAU * frequency * elapsed).sin()
                            + 0.25 * (2. * std::f64::consts::TAU * frequency * elapsed).sin())
                        * 0.08) as f32;
                }
            }
        }
    }
    let audio = root.join("synthetic-piano.wav");
    models::write_audio(&audio, &signal, &signal).unwrap();
    let stems = root.join("stems");
    fs::create_dir(&stems).unwrap();
    let zeros = vec![0.; signal.len()];
    for id in StemId::ALL {
        let samples = if id == StemId::Piano { &signal } else { &zeros };
        models::write_audio(
            &stems.join(format!("{}.wav", id.as_str())),
            samples,
            samples,
        )
        .unwrap();
    }
    let manifest = root.join("isolation.json");
    pipeline::import_isolation(&audio,&stems,&manifest,AlignmentEvidence {method:"Rust synthesis source-clock fixture".into(),details:"Piano stem equals the synthesized input; five other stems are zeros of identical sample count. This exercises supplied binding and real pitch inference, not source separation quality.".into()}).unwrap();
    let model_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let transcription = pipeline::transcribe_with_isolation(
        &audio,
        &model_dir,
        IsolationOptions {
            manifest: Some(&manifest),
            keyboard_sound: chart_engine::keyboard_detection::KeyboardSound::Piano,
            ..Default::default()
        },
        &["piano".into(), "pro_piano".into()],
    )
    .unwrap();
    assert!(
        transcription.drums.is_empty()
            && transcription.bass.is_empty()
            && transcription.guitar.is_empty()
    );
    assert!(!transcription.piano.is_empty());
    assert_eq!(transcription.piano.len(), transcription.pro_piano.len());
    let detected: std::collections::BTreeSet<_> =
        transcription.piano.iter().map(|n| n.pitch).collect();
    assert!(
        [60, 64, 67].iter().filter(|p| detected.contains(p)).count() >= 2,
        "Expected multiple C-major chord voices from real model, got {detected:?}"
    );
    fs::write(
        root.join("events.json"),
        serde_json::to_vec_pretty(&transcription).unwrap(),
    )
    .unwrap();
    let store = build_project(root, &audio, &transcription);
    let output = root.join("game-song");
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Native piano inference fixture".into(),
            },
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let inventory = midi::inventory(&fs::read(output.join("notes.mid")).unwrap()).unwrap();
    assert!(inventory.iter().any(|(n, c)| n == "PART KEYS" && *c > 0));
    assert!(
        inventory
            .iter()
            .any(|(n, c)| n == "PART REAL_KEYS_X" && *c > 0)
    );
    println!(
        "Real Basic Pitch on source-bound synthetic piano: {} candidates, pitches {detected:?}; game inventory {inventory:?}; evidence {}",
        transcription.piano.len(),
        root.display()
    );
}
