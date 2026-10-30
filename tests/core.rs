use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    mapping::{self, DrumHit, PitchNote, Transcription, Voice},
    midi, models,
    pipeline::{self, Build, Metadata},
    timing::{Tempo, TempoMap},
};
use std::{collections::BTreeMap, fs};

fn tempo() -> TempoMap {
    TempoMap::new(
        480,
        0.25,
        vec![
            Tempo {
                tick: 0,
                bpm: 120.0,
            },
            Tempo {
                tick: 960,
                bpm: 60.0,
            },
        ],
    )
    .unwrap()
}
fn note(tick: u64, lane: u8) -> Note {
    Note {
        tick,
        lane,
        sustain: 0,
    }
}
fn base_chart() -> String {
    "[Song]\r\n{\r\n  Resolution = 480\r\n  Offset = 0.25\r\n}\r\n[SyncTrack]\r\n{\r\n  0 = B 120000\r\n  960 = B 60000\r\n}\r\n[Events]\r\n{\r\n  0 = E \"section intro\"\r\n}\r\n[ExpertSingle]\r\n{\r\n  480 = N 2 120\r\n  600 = S 2 240\r\n}\r\n".into()
}

#[test]
fn variable_tempo_and_offset_round_trip() {
    let map = tempo();
    assert_eq!(map.tick(0.2), None);
    assert_eq!(map.tick(0.75), Some(480));
    assert_eq!(map.tick(2.25), Some(1440));
    for tick in (0..40000).step_by(7) {
        assert_eq!(map.tick(map.seconds(tick)), Some(tick));
    }
}

#[test]
fn invalid_tempos_are_rejected() {
    assert!(TempoMap::constant(f64::NAN).is_err());
    assert!(TempoMap::constant(0.0).is_err());
    assert!(TempoMap::new(0, 0.0, vec![]).is_err());
    assert!(
        TempoMap::new(
            480,
            0.0,
            vec![
                Tempo {
                    tick: 0,
                    bpm: 120.0
                },
                Tempo { tick: 0, bpm: 60.0 }
            ]
        )
        .is_err()
    );
}

#[test]
fn enhance_chart_preserves_all_original_bytes_and_authored_tracks() {
    let original = base_chart();
    let chart = Chart::parse(original.clone()).unwrap();
    let generated = Tracks::from([
        ("ExpertDrums".into(), vec![note(480, 0)]),
        ("ExpertSingle".into(), vec![note(0, 4)]),
    ]);
    let (out, added) = chart.enhance(&generated).unwrap();
    assert!(out.starts_with(&original));
    assert_eq!(added, vec!["ExpertDrums"]);
    let again = Chart::parse(out.clone()).unwrap();
    assert_eq!(
        again.notes("ExpertSingle").unwrap(),
        vec![Note {
            tick: 480,
            lane: 2,
            sustain: 120
        }]
    );
    assert_eq!(again.enhance(&generated).unwrap(), (out, vec![]));
}

#[test]
fn empty_chart_track_keeps_its_special_events() {
    let raw = format!("{}[ExpertDrums]\n{{\n  0 = S 2 480\n}}\n", base_chart());
    let chart = Chart::parse(raw).unwrap();
    let (out, _) = chart
        .enhance(&Tracks::from([("ExpertDrums".into(), vec![note(480, 1)])]))
        .unwrap();
    assert!(out.contains("0 = S 2 480"));
    assert_eq!(
        Chart::parse(out).unwrap().notes("ExpertDrums").unwrap(),
        vec![note(480, 1)]
    );
}

#[test]
fn anchors_and_duplicate_sections_fail_instead_of_guessing() {
    assert!(Chart::parse(base_chart().replace("0 = B 120000", "0 = A 123456")).is_err());
    assert!(Chart::parse(format!("{}[Song]\n{{\n}}\n", base_chart())).is_err());
}

#[test]
fn gm_crash_and_ride_follow_clone_hero_conventions() {
    assert_eq!(Voice::from_gm(49), Some(Voice::Crash));
    assert_eq!(Voice::Crash.lane(), 4);
    assert_eq!(Voice::Ride.lane(), 3);
    let input = Transcription {
        drums: vec![
            DrumHit {
                time: 1.0,
                voice: Voice::Crash,
                confidence: 0.9,
            },
            DrumHit {
                time: 1.5,
                voice: Voice::Ride,
                confidence: 0.9,
            },
        ],
        ..Default::default()
    };
    let tracks = mapping::tracks(&input, &TempoMap::constant(120.0).unwrap(), true).unwrap();
    assert_eq!(
        tracks["ExpertDrums"],
        vec![note(960, 4), note(960, 68), note(1440, 3), note(1440, 67)]
    );
    assert!(
        mapping::tracks(&input, &TempoMap::constant(120.0).unwrap(), false).unwrap()["ExpertDrums"]
            .iter()
            .all(|n| n.lane < 5)
    );
}

#[test]
fn two_hands_plus_one_foot_limit_and_same_lane_dedup() {
    let input = Transcription {
        drums: [
            Voice::Kick,
            Voice::Snare,
            Voice::HiHat,
            Voice::MidTom,
            Voice::Ride,
            Voice::Crash,
        ]
        .into_iter()
        .map(|voice| DrumHit {
            time: 1.0,
            voice,
            confidence: 0.9,
        })
        .collect(),
        ..Default::default()
    };
    let tracks = mapping::tracks(&input, &TempoMap::constant(120.0).unwrap(), true).unwrap();
    let hits: Vec<_> = tracks["ExpertDrums"]
        .iter()
        .filter(|n| n.lane < 5)
        .map(|n| n.lane)
        .collect();
    assert_eq!(hits, vec![0, 1, 3]);
}

#[test]
fn model_class_jitter_cannot_bypass_two_hand_limit() {
    let events = Transcription {
        drums: vec![
            DrumHit {
                time: 1.0,
                voice: Voice::Snare,
                confidence: 0.9,
            },
            DrumHit {
                time: 1.0,
                voice: Voice::Cymbal,
                confidence: 0.8,
            },
            DrumHit {
                time: 1.01,
                voice: Voice::HiHat,
                confidence: 0.3,
            },
            DrumHit {
                time: 1.01,
                voice: Voice::Kick,
                confidence: 0.9,
            },
            DrumHit {
                time: 1.15,
                voice: Voice::HiHat,
                confidence: 0.8,
            },
        ],
        ..Default::default()
    };
    let tracks = mapping::tracks(&events, &TempoMap::constant(120.0).unwrap(), true).unwrap();
    let notes = &tracks["ExpertDrums"];
    assert_eq!(
        notes
            .iter()
            .filter(|n| n.tick == 960 && n.lane <= 4)
            .map(|n| n.lane)
            .collect::<Vec<_>>(),
        vec![0, 1, 4]
    );
    assert!(notes.iter().any(|n| n.tick == 1104 && n.lane == 2));
}

#[test]
fn inference_nms_keeps_the_stronger_cymbal_peak_but_preserves_real_gaps() {
    let mut values = vec![0.0; 150 * 5];
    values[100 * 5 + 4] = 0.7;
    values[105 * 5 + 4] = 0.95;
    values[120 * 5 + 4] = 0.9;
    let hits = models::drum_peaks(&values);
    let cymbals: Vec<_> = hits
        .iter()
        .filter(|h| h.voice == Voice::Cymbal)
        .map(|h| h.time)
        .collect();
    assert_eq!(cymbals, vec![1.05, 1.2]);
}

#[test]
fn expert_bass_has_a_minimum_retrigger_gap() {
    let events = Transcription {
        bass: [1.0, 1.03, 1.07, 1.3]
            .into_iter()
            .map(|start| PitchNote {
                start,
                end: start + 0.3,
                pitch: 40,
                confidence: 0.9,
            })
            .collect(),
        ..Default::default()
    };
    let timing = TempoMap::constant(120.0).unwrap();
    let notes = &mapping::tracks(&events, &timing, true).unwrap()["ExpertDoubleBass"];
    assert_eq!(notes.len(), 3);
    assert!(
        notes
            .windows(2)
            .all(|w| timing.seconds(w[1].tick) - timing.seconds(w[0].tick) >= 0.055)
    );
}

#[test]
fn transcription_rejects_truncated_supplied_stems_before_inference() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("song.wav");
    let stems = root.path().join("stems");
    fs::create_dir(&stems).unwrap();
    models::write_audio(&original, &vec![0.0; 88200], &vec![0.0; 88200]).unwrap();
    models::write_audio(
        &stems.join("drums.wav"),
        &vec![0.0; 44100],
        &vec![0.0; 44100],
    )
    .unwrap();
    let err =
        pipeline::transcribe(&original, root.path(), Some(&stems), &["drums".into()]).unwrap_err();
    assert!(err.to_string().contains("Supplied stems require"));
}

#[test]
fn rejected_negative_or_nonfinite_events_do_not_get_silently_clamped() {
    let mut input = Transcription {
        drums: vec![DrumHit {
            time: -1.0,
            voice: Voice::Kick,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    assert!(mapping::tracks(&input, &tempo(), true).is_err());
    input.drums[0].time = f64::NAN;
    assert!(input.validate().is_err());
}

#[test]
fn pitched_mapping_is_stable_for_repeats_and_input_order() {
    let input = Transcription {
        guitar: [60, 64, 67, 72, 60, 64, 67, 72]
            .into_iter()
            .enumerate()
            .map(|(i, pitch)| PitchNote {
                start: i as f64 * 0.5,
                end: i as f64 * 0.5 + 0.8,
                pitch,
                confidence: 0.9,
            })
            .collect(),
        ..Default::default()
    };
    let map = TempoMap::constant(120.0).unwrap();
    let tracks = mapping::tracks(&input, &map, true).unwrap();
    let expert = &tracks["ExpertSingle"];
    assert_eq!(
        expert.iter().take(4).map(|n| n.lane).collect::<Vec<_>>(),
        expert.iter().skip(4).map(|n| n.lane).collect::<Vec<_>>()
    );
    assert!(
        expert
            .windows(2)
            .all(|w| w[0].tick + w[0].sustain < w[1].tick)
    );
    let mut reverse = input;
    reverse.guitar.reverse();
    assert_eq!(tracks, mapping::tracks(&reverse, &map, true).unwrap());
    assert!(tracks["EasySingle"].iter().all(|n| n.lane <= 2));
    assert!(tracks["MediumSingle"].iter().all(|n| n.lane <= 3));
}

#[test]
fn midi_additions_preserve_original_track_chunks() {
    let map = TempoMap::constant(120.0).unwrap();
    let original = midi::new_midi(
        &map,
        &Tracks::from([(
            "ExpertSingle".into(),
            vec![Note {
                tick: 480,
                lane: 2,
                sustain: 240,
            }],
        )]),
    )
    .unwrap();
    let generated = Tracks::from([
        (
            "ExpertDrums".into(),
            vec![note(960, 0), note(960, 3), note(960, 67)],
        ),
        ("ExpertSingle".into(), vec![note(0, 4)]),
    ]);
    let (out, added) = midi::enhance(&original, &generated).unwrap();
    assert_eq!(added, vec!["PART DRUMS"]);
    assert_eq!(&out[14..original.len()], &original[14..]);
    let inv = midi::inventory(&out).unwrap();
    assert_eq!(inv.iter().find(|(n, _)| n == "PART GUITAR").unwrap().1, 1);
    assert_eq!(midi::enhance(&out, &generated).unwrap(), (out, vec![]));
}

#[test]
fn midi_global_tom_flags_do_not_modify_existing_drums() {
    let map = TempoMap::constant(120.0).unwrap();
    let old = Tracks::from([("ExpertDrums".into(), vec![note(480, 3), note(480, 67)])]);
    let original = midi::new_midi(&map, &old).unwrap();
    let missing = Tracks::from([("EasyDrums".into(), vec![note(480, 3)])]);
    assert_eq!(
        midi::enhance(&original, &missing).unwrap(),
        (original, vec![])
    );
}

#[test]
fn midi_export_emits_tom_flags_only_for_toms() {
    let map = TempoMap::constant(120.0).unwrap();
    let data = midi::new_midi(
        &map,
        &Tracks::from([(
            "ExpertDrums".into(),
            vec![
                note(0, 2),
                note(480, 3),
                note(480, 67),
                note(960, 4),
                note(960, 68),
            ],
        )]),
    )
    .unwrap();
    let smf = midly::Smf::parse(&data).unwrap();
    let keys: Vec<_> = smf.tracks[1]
        .iter()
        .filter_map(|e| match e.kind {
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { key, .. },
                ..
            } => Some(key.as_int()),
            _ => None,
        })
        .collect();
    assert!(keys.contains(&110));
    assert!(!keys.contains(&111));
    assert!(!keys.contains(&112));
}

#[test]
fn onset_decoders_reject_silence_and_pick_separated_hits() {
    assert!(models::drum_peaks(&vec![0.0; 500]).is_empty());
    let mut activations = vec![0.0; 500];
    activations[20 * 5] = 0.9;
    activations[60 * 5 + 1] = 0.8;
    let hits = models::drum_peaks(&activations);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].time, 0.2);
    assert_eq!(hits[1].voice, Voice::Snare);
    assert!(models::decode_pitch(&vec![0.0; 8800], &vec![0.0; 8800]).is_empty());
}

#[test]
fn melodia_recovers_a_sustained_pitch_without_an_onset_peak() {
    let mut frames = vec![0.0; 100 * 88];
    for t in 10..80 {
        frames[t * 88 + 48] = 0.8;
    }
    let notes = models::decode_pitch(&frames, &vec![0.0; 100 * 88]);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].pitch, 69);
    assert!((notes[0].start - 10.0 * 256.0 / 22050.0).abs() < 0.02);
}

#[test]
fn lrc_handles_multiple_timestamps_and_offsets_without_fabricated_words() {
    let lines = pipeline::lyric_events(
        "[00:00.10]before\n[00:01.00][00:02.00]hello world\n[ar:Artist]\n[00:99.0]bad",
        &tempo(),
        5.0,
    );
    assert!(!lines.contains("before"));
    assert!(!lines.contains("bad"));
    assert_eq!(lines.matches("hello world").count(), 2);
    assert!(!lines.contains("Artist"));
}

#[test]
fn new_chart_round_trips_utf8_and_escaping() {
    let raw = chart::new_chart("Canción \"A\"", "A\\B", &tempo(), &BTreeMap::new());
    let doc = Chart::parse(raw).unwrap();
    assert_eq!(doc.timing.tick(2.25), Some(1440));
}

#[test]
fn build_is_transactional_and_never_overwrites_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let original = base_chart();
    fs::write(source.join("notes.chart"), &original).unwrap();
    fs::write(
        source.join("song.ini"),
        "[song]\nname = Test\nartist = Artist\ndiff_drums = -1\ncustom = keep\n",
    )
    .unwrap();
    fs::write(source.join("album.png"), b"artwork-is-preserved").unwrap();
    let audio = source.join("song.ogg");
    fs::write(&audio, b"fixture audio bytes; no decoding needed by build").unwrap();
    let output = temp.path().join("enhanced");
    let events = Transcription {
        drums: vec![DrumHit {
            time: 0.75,
            voice: Voice::Kick,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    let request = || Build {
        audio: &audio,
        output: &output,
        existing: Some(&source),
        timing: None,
        transcription: &events,
        meta: Metadata {
            name: "Test".into(),
            artist: "Artist".into(),
            duration: 5.0,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    };
    pipeline::build(request()).unwrap();
    let markers: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    assert_eq!(markers.musical_acceptance, "unverified");
    assert_eq!(
        markers.input_sha256["chart"],
        models::hash_file(&output.join("notes.chart")).unwrap()
    );
    assert!(
        markers
            .generated_tracks
            .as_ref()
            .unwrap()
            .contains("ExpertDrums")
    );
    assert!(
        !markers
            .generated_tracks
            .as_ref()
            .unwrap()
            .contains("ExpertSingle")
    );
    assert_eq!(
        fs::read_to_string(source.join("notes.chart")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(output.join("album.png")).unwrap(),
        b"artwork-is-preserved"
    );
    let ini = fs::read_to_string(output.join("song.ini")).unwrap();
    assert!(ini.contains("custom = keep"));
    assert!(!ini.contains("diff_drums = -1"));
    assert_eq!(
        Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap())
            .unwrap()
            .notes("ExpertDrums")
            .unwrap(),
        vec![note(480, 0)]
    );
    assert!(pipeline::build(request()).is_err());
    assert!(pipeline::destination(&source.join("inside"), Some(&source)).is_err());
}

#[test]
fn empty_transcription_and_legacy_delay_leave_no_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("notes.chart"), base_chart()).unwrap();
    fs::write(source.join("song.ini"), "[song]\ndelay = 50\n").unwrap();
    let audio = source.join("song.ogg");
    fs::write(&audio, b"fixture").unwrap();
    let output = temp.path().join("out");
    let events = Transcription {
        drums: vec![DrumHit {
            time: 1.0,
            voice: Voice::Kick,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    assert!(
        pipeline::build(Build {
            audio: &audio,
            output: &output,
            existing: Some(&source),
            timing: None,
            transcription: &events,
            meta: Metadata::default(),
            pro: true,
            lookup_lyrics: false
        })
        .unwrap_err()
        .to_string()
        .contains("delay")
    );
    assert!(!output.exists());
    assert!(
        pipeline::build(Build {
            audio: &audio,
            output: &output,
            existing: None,
            timing: Some(tempo()),
            transcription: &Transcription::default(),
            meta: Metadata::default(),
            pro: true,
            lookup_lyrics: false
        })
        .is_err()
    );
    assert!(!output.exists());
}

#[test]
fn midi_enhancement_emits_markers_only_for_added_instruments() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let map = TempoMap::constant(123.456).unwrap();
    let original = midi::new_midi(
        &map,
        &Tracks::from([("ExpertSingle".into(), vec![note(960, 2)])]),
    )
    .unwrap();
    fs::write(source.join("notes.mid"), &original).unwrap();
    let audio = source.join("song.ogg");
    fs::write(&audio, b"preserved audio fixture").unwrap();
    let output = temp.path().join("out");
    let events = Transcription {
        drums: vec![DrumHit {
            time: 1.0,
            voice: Voice::Kick,
            confidence: 0.4,
        }],
        guitar: vec![PitchNote {
            start: 1.0,
            end: 1.5,
            pitch: 60,
            confidence: 0.3,
        }],
        ..Default::default()
    };
    pipeline::build(Build {
        audio: &audio,
        output: &output,
        existing: Some(&source),
        timing: None,
        transcription: &events,
        meta: Metadata {
            name: "Test".into(),
            artist: "Artist".into(),
            duration: 5.0,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    let result = fs::read(output.join("notes.mid")).unwrap();
    assert_eq!(&result[14..original.len()], &original[14..]);
    let markers: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    let generated = markers.generated_tracks.unwrap();
    assert!(generated.contains("ExpertDrums"));
    assert!(!generated.contains("ExpertSingle"));
    assert!(
        markers
            .regions
            .iter()
            .flat_map(|r| &r.issues)
            .any(
                |i| i.reason == chart_engine::review_regions::Reason::LowCandidateScore
                    && i.track == "ExpertDrums"
            )
    );
    assert!(
        markers
            .regions
            .iter()
            .flat_map(|r| &r.issues)
            .all(|i| i.track.ends_with("Drums"))
    );
}

#[test]
fn buffered_stereo_export_preserves_channels_and_samples() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("stereo.wav");
    let left: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.1).sin() * 0.25).collect();
    let right: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.15).cos() * 0.2).collect();
    models::write_audio(&path, &left, &right).unwrap();
    let decoded = models::decode(&path, 44100, 2).unwrap();
    assert_eq!(decoded.len(), left.len() * 2);
    for (i, pair) in decoded.chunks_exact(2).enumerate() {
        assert!((pair[0] - left[i]).abs() < 0.00001);
        assert!((pair[1] - right[i]).abs() < 0.00001);
    }
    assert!(models::write_audio(&path, &left, &right).is_err());
}

#[test]
fn publication_does_not_replace_a_concurrent_empty_folder() {
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join("stage");
    let output = root.path().join("output");
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join("receipt"), "complete").unwrap();
    fs::create_dir(&output).unwrap();
    assert!(pipeline::publish(&staged, &output).is_err());
    assert!(staged.join("receipt").exists());
    assert!(!output.join("receipt").exists());
    fs::remove_dir(&output).unwrap();
    pipeline::publish(&staged, &output).unwrap();
    assert!(output.join("receipt").exists());
}
