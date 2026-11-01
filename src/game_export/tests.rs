use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use std::fs;

fn chart(parts: &str) -> Chart {
    Chart::parse(format!(
        "[Song]\n{{\n Resolution = 480\n}}\n[SyncTrack]\n{{\n0 = B 120000\n480 = B 90000\n}}\n[Events]\n{{\n0 = E \"section Intro\"\n}}\n{parts}"
    ))
    .unwrap()
}

fn vocals() -> VocalChart {
    VocalChart {
        version: 1,
        resolution: 480,
        source: "Authored fixture".into(),
        phrases: vec![VocalPhrase {
            start: 0,
            end: 960,
            player: 1,
        }],
        notes: vec![VocalNote {
            midi: None,
            start: 240,
            end: 480,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Text {
                text: "Test".into(),
                join_next: false,
            },
        }],
    }
}

fn track<'a>(smf: &'a Smf<'a>, label: &str) -> &'a [midly::TrackEvent<'a>] {
    smf.tracks.iter().find(|events| events.iter().any(|event| {
        matches!(event.kind, TrackEventKind::Meta(MetaMessage::TrackName(name)) if name == label.as_bytes())
    })).unwrap()
}

fn spans(events: &[midly::TrackEvent<'_>], key: u8) -> Vec<(u64, u64)> {
    let mut tick = 0;
    let mut active = None;
    let mut result = Vec::new();
    for event in events {
        tick += u64::from(event.delta.as_int());
        match event.kind {
            TrackEventKind::Midi {
                message: MidiMessage::NoteOn { key: note, vel },
                ..
            } if note.as_int() == key && vel.as_int() > 0 => {
                assert!(active.replace(tick).is_none())
            }
            TrackEventKind::Midi {
                message: MidiMessage::NoteOff { key: note, .. },
                ..
            } if note.as_int() == key => result.push((active.take().unwrap(), tick)),
            _ => {}
        }
    }
    assert!(active.is_none());
    result
}

#[test]
fn rhythm_and_coop_preserve_each_difficulty_notes_phrases_and_techniques() {
    let mut parts = String::from("[ExpertProKeys]\n{\n0 = N 60 120\n}\n");
    for suffix in ["DoubleRhythm", "DoubleGuitar"] {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            parts.push_str(&format!(
                "[{tier}{suffix}]\n{{\n0 = N 0 240\n120 = N 1 0\n240 = N 1 0\n240 = N 5 0\n360 = N 2 0\n360 = N 6 0\n480 = N 7 120\n0 = S 2 600\n120 = E solo\n600 = E soloend\n}}\n"
            ));
        }
    }
    let source = chart(&parts);
    let bytes = song(&source, Some(&vocals())).unwrap().0;
    let smf = Smf::parse(&bytes).unwrap();
    for label in ["PART RHYTHM", "PART GUITAR COOP"] {
        let events = track(&smf, label);
        assert_eq!(spans(events, 103), [(120, 600)]);
        assert_eq!(spans(events, 116), [(0, 600)]);
        for difficulty in 0..4u8 {
            let base = 60 + difficulty * 12;
            assert_eq!(spans(events, base), [(0, 240)]);
            assert_eq!(spans(events, base + 1), [(120, 121), (240, 241)]);
            assert_eq!(spans(events, base - 1), [(480, 600)]);
            assert_eq!(
                spans(events, base + 5),
                [(120, 121), (240, 241), (480, 481)]
            );
            assert!(spans(events, base + 6).contains(&(0, 1)));
            assert!(
                events.iter().any(|event| matches!(event.kind,
                TrackEventKind::SysEx(data) if data == [b'P', b'S', 0, 0, difficulty, 4, 1, 0xf7]))
            );
        }
    }
    assert_eq!(
        crate::vocals::import_midi(&bytes).unwrap().notes,
        vocals().notes
    );
    assert_eq!(spans(track(&smf, "PART REAL_KEYS_X"), 60), [(0, 120)]);
}

#[test]
fn rhythm_and_coop_export_reopen_preserves_working_master_and_original_song() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("song");
    fs::create_dir(&folder).unwrap();
    let source = chart(
        "[ExpertSingle]\n{\n0 = N 0 0\n}\n[ExpertDoubleRhythm]\n{\n120 = N 7 240\n}\n[HardDoubleGuitar]\n{\n240 = N 4 120\n}\n",
    );
    fs::write(folder.join("notes.chart"), &source.raw).unwrap();
    fs::write(
        folder.join("song.ini"),
        "[song]\nname = Extra parts\ncustom = keep\n",
    )
    .unwrap();
    let samples = vec![0.; 44100 * 3];
    crate::models::write_audio(&folder.join("song.wav"), &samples, &samples).unwrap();
    let store = Store::create(&folder, None, &root.path().join("project")).unwrap();
    let actor = Actor {
        kind: ActorKind::Human,
        name: "Export fixture".into(),
    };
    store
        .apply(Request {
            expected_revision: 0,
            actor: actor.clone(),
            action: Action::SetVocals { document: vocals() },
        })
        .unwrap();
    let output = root.path().join("export");
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor,
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    assert_eq!(spans(track(&smf, "PART RHYTHM"), 95), [(120, 360)]);
    assert_eq!(spans(track(&smf, "PART GUITAR COOP"), 88), [(240, 360)]);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.load().unwrap().chart_text(&reopened).unwrap(),
        source.raw
    );
    assert_eq!(
        fs::read_to_string(folder.join("notes.chart")).unwrap(),
        source.raw
    );
    assert!(
        fs::read_to_string(output.join("song.ini"))
            .unwrap()
            .contains("custom = keep")
    );
}

#[test]
fn rhythm_and_coop_still_refuse_orphan_controls_and_unknown_events() {
    for suffix in ["DoubleRhythm", "DoubleGuitar"] {
        for rows in ["0 = N 5 0", "0 = N 0 0\n0 = E custom"] {
            let source = chart(&format!("[Expert{suffix}]\n{{\n{rows}\n}}\n"));
            assert!(song(&source, Some(&vocals())).is_err());
        }
    }
}

#[test]
fn six_fret_exports_every_lane_difficulty_and_part_with_mixed_vocals_and_keys() {
    let mut parts = String::from("[ExpertProKeys]\n{\n0 = N 60 120\n}\n");
    for suffix in ["GHLGuitar", "GHLBass", "GHLRhythm", "GHLCoop", "GHLKeys"] {
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            parts.push_str(&format!("[{tier}{suffix}]\n{{\n"));
            for (index, lane) in [0, 1, 2, 3, 4, 8, 7].iter().enumerate() {
                parts.push_str(&format!("{} = N {lane} 30\n", index * 240));
            }
            parts.push_str("120 = N 8 30\n240 = N 5 0\n480 = N 6 0\n0 = S 2 1680\n120 = E solo\n1440 = E soloend\n}\n");
        }
    }
    let source = chart(&parts);
    let bytes = song(&source, Some(&vocals())).unwrap().0;
    let smf = Smf::parse(&bytes).unwrap();
    for label in [
        "PART GUITAR GHL",
        "PART BASS GHL",
        "PART RHYTHM GHL",
        "PART GUITAR COOP GHL",
        "PART KEYS GHL",
    ] {
        let events = track(&smf, label);
        assert_eq!(spans(events, 103), [(120, 1440)]);
        assert_eq!(spans(events, 116), [(0, 1680)]);
        for difficulty in 0..4u8 {
            let shift = difficulty * 12;
            for (key, start) in [
                (59, 0),
                (60, 240),
                (61, 480),
                (62, 720),
                (63, 960),
                (58, 1440),
            ] {
                assert_eq!(spans(events, key + shift), [(start, start + 30)]);
            }
            assert_eq!(spans(events, 64 + shift), [(120, 150), (1200, 1230)]);
            assert!(spans(events, 65 + shift).contains(&(120, 121)));
            assert!(spans(events, 66 + shift).contains(&(240, 241)));
            assert!(
                events.iter().any(|event| matches!(event.kind,
                TrackEventKind::SysEx(data) if data == [b'P', b'S', 0, 0, difficulty, 4, 1, 0xf7]))
            );
        }
    }
    assert_eq!(
        crate::vocals::import_midi(&bytes).unwrap().notes,
        vocals().notes
    );
    assert_eq!(spans(track(&smf, "PART REAL_KEYS_X"), 60), [(0, 120)]);
}

#[test]
fn unsupported_six_fret_controls_are_not_silently_discarded() {
    for row in ["0 = N 9 0", "0 = N 5 0", "0 = N 8 0\n0 = E custom"] {
        let source = chart(&format!("[ExpertGHLGuitar]\n{{\n{row}\n}}\n"));
        assert!(song(&source, Some(&vocals())).is_err());
    }
}

#[test]
fn six_fret_parts_survive_project_export_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("song");
    fs::create_dir(&folder).unwrap();
    let source = chart(
        "[ExpertSingle]\n{\n0 = N 0 0\n}\n[ExpertGHLGuitar]\n{\n120 = N 8 240\n}\n[HardGHLBass]\n{\n240 = N 7 120\n}\n[ExpertKeyboard]\n{\n0 = N 0 0\n120 = N 1 0\n120 = N 5 0\n240 = N 2 0\n240 = N 6 0\n480 = N 7 120\n}\n",
    );
    fs::write(folder.join("notes.chart"), &source.raw).unwrap();
    fs::write(folder.join("song.ini"), "[song]\nname = Six fret fixture\n").unwrap();
    let samples = vec![0.; 44100 * 3];
    crate::models::write_audio(&folder.join("song.wav"), &samples, &samples).unwrap();
    let store = Store::create(&folder, None, &root.path().join("project")).unwrap();
    let actor = Actor {
        kind: ActorKind::Human,
        name: "Six fret export".into(),
    };
    store
        .apply(Request {
            expected_revision: 0,
            actor: actor.clone(),
            action: Action::SetVocals { document: vocals() },
        })
        .unwrap();
    let output = root.path().join("export");
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor,
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let smf = Smf::parse(&bytes).unwrap();
    assert_eq!(spans(track(&smf, "PART GUITAR GHL"), 100), [(120, 360)]);
    assert_eq!(spans(track(&smf, "PART BASS GHL"), 82), [(240, 360)]);
    assert_eq!(spans(track(&smf, "PART KEYS"), 95), [(480, 600)]);
    assert!(spans(track(&smf, "PART KEYS"), 102).contains(&(120, 121)));
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.load().unwrap().chart_text(&reopened).unwrap(),
        source.raw
    );
    assert_eq!(
        fs::read_to_string(folder.join("notes.chart")).unwrap(),
        source.raw
    );
}

#[test]
fn five_lane_keys_preserves_open_notes_and_difficulty_scoped_techniques() {
    let mut parts = String::from("[ExpertProKeys]\n{\n0 = N 60 120\n}\n");
    for tier in ["Easy", "Medium", "Hard", "Expert"] {
        parts.push_str(&format!("[{tier}Keyboard]\n{{\n0 = N 0 240\n120 = N 1 0\n240 = N 1 0\n240 = N 5 0\n360 = N 2 0\n360 = N 6 0\n480 = N 7 120\n0 = S 2 600\n}}\n"));
    }
    let source = chart(&parts);
    let bytes = song(&source, Some(&vocals())).unwrap().0;
    let smf = Smf::parse(&bytes).unwrap();
    let events = track(&smf, "PART KEYS");
    for difficulty in 0..4u8 {
        let base = 60 + difficulty * 12;
        assert_eq!(spans(events, base), [(0, 240)]);
        assert_eq!(spans(events, base - 1), [(480, 600)]);
        assert_eq!(
            spans(events, base + 5),
            [(120, 121), (240, 241), (480, 481)]
        );
        assert!(events.iter().any(|event| matches!(event.kind,
            TrackEventKind::SysEx(data) if data == [b'P', b'S', 0, 0, difficulty, 4, 1, 0xf7])));
    }
    assert_eq!(spans(events, 116), [(0, 600)]);
    assert_eq!(
        crate::vocals::import_midi(&bytes).unwrap().notes,
        vocals().notes
    );
    let notes = source.notes("ExpertKeyboard").unwrap();
    let policy = crate::gameplay_view::yarg_chart_policy(480, &Default::default()).unwrap();
    let piano =
        crate::gameplay_view::classify("ExpertKeyboard", &notes, &source.timing, &policy).unwrap();
    assert!(piano.iter().all(|gem| gem.technique.is_none()));
    assert!(piano.iter().all(|gem| gem.lane <= 4));
}

#[test]
fn five_lane_keys_rejects_orphan_flags_and_unrecognized_lanes() {
    for row in ["0 = N 5 0", "0 = N 6 0", "0 = N 8 0"] {
        let source = chart(&format!("[ExpertKeyboard]\n{{\n{row}\n}}\n"));
        assert!(song(&source, Some(&vocals())).is_err());
    }
}
