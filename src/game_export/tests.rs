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
