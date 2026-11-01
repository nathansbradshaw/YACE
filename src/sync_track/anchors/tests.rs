use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    sync_track::{self, Meter},
};
use std::fs;

const CHART: &str = "[Song]\r\n{\r\nResolution = 480\r\nOffset = 0.25\r\n}\r\n[SyncTrack]\r\n{\r\n0 = TS 4\r\n0 = B 120000\r\n480 = B 120000\r\n960 = A 1000000\r\n960 = B 120000\r\n1920 = A 2000000\r\n1920 = B 120000\r\n}\r\n[Events]\r\n{\r\n0 = E \"section Retain\"\r\n}\r\n[ExpertSingle]\r\n{\r\n960 = N 0 120\r\n1920 = N 1 120\r\n}\r\n";

fn store(root: &std::path::Path) -> Store {
    let folder = root.join("song");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("notes.chart"), CHART).unwrap();
    fs::write(folder.join("song.ini"), "[song]\nname = Anchor fixture\n").unwrap();
    let samples = vec![0.; 44100 * 4];
    crate::models::write_audio(&folder.join("song.wav"), &samples, &samples).unwrap();
    Store::create(&folder, None, &root.join("project")).unwrap()
}

fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Anchor test".into(),
        },
        action,
    })?;
    Ok(())
}

#[test]
fn anchored_charts_open_without_changing_the_playable_clock() {
    let chart = Chart::parse(CHART.into()).unwrap();
    assert_eq!(from_chart(&chart).unwrap().len(), 2);
    assert_eq!(chart.timing.seconds(960), 1.25);
    assert_eq!(chart.timing.seconds(1920), 2.25);
    assert_eq!(sync_track::render(chart.raw.clone(), &None).unwrap(), CHART);
    let midi = crate::game_export::song(&chart, None).unwrap();
    let clock = crate::midi::timing(&midi.0).unwrap();
    assert!((clock.seconds(1920) + midi.1 as f64 / 1000. - 2.25).abs() < 1e-9);
}

#[test]
fn tempo_edit_compensates_before_anchors_and_undo_restores_exact_source() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    apply(
        &store,
        Action::EditSync {
            edit: Edit::PutTempo {
                previous: Some(Tempo { tick: 0, bpm: 120. }),
                tempo: Tempo { tick: 0, bpm: 150. },
            },
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let clock = doc.timing(&store).unwrap();
    assert_eq!(clock.tempos[1].bpm, 100.);
    assert!((clock.seconds(960) - 1.25).abs() < 1e-9);
    assert!((clock.seconds(1920) - 2.25).abs() < 1e-9);
    let text = doc.chart_text(&store).unwrap();
    assert!(text.contains("960 = A 1000000\r\n"));
    assert!(text.contains("1920 = A 2000000\r\n"));
    assert!(text.contains("0 = E \"section Retain\"\r\n"));
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(reopened.load().unwrap().timing(&reopened).unwrap(), clock);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), CHART);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), text);
}

#[test]
fn impossible_or_destructive_anchor_edits_leave_the_project_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    for edit in [
        Edit::PutTempo {
            previous: Some(Tempo { tick: 0, bpm: 120. }),
            tempo: Tempo { tick: 0, bpm: 30. },
        },
        Edit::PutTempo {
            previous: Some(Tempo {
                tick: 960,
                bpm: 120.,
            }),
            tempo: Tempo {
                tick: 1000,
                bpm: 120.,
            },
        },
        Edit::RemoveTempo {
            tempo: Tempo {
                tick: 960,
                bpm: 120.,
            },
        },
        Edit::PutTempo {
            previous: Some(Tempo {
                tick: 480,
                bpm: 120.,
            }),
            tempo: Tempo {
                tick: 480,
                bpm: 150.,
            },
        },
    ] {
        let revision = store.load().unwrap().revision;
        assert!(apply(&store, Action::EditSync { edit }).is_err());
        assert_eq!(store.load().unwrap().revision, revision);
        assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), CHART);
    }
    apply(
        &store,
        Action::EditSync {
            edit: Edit::PutMeter {
                previous: None,
                meter: Meter {
                    tick: 480,
                    numerator: 3,
                    denominator: 8,
                },
            },
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().timing(&store).unwrap().seconds(1920),
        2.25
    );
}

#[test]
fn invalid_anchor_rows_are_explicit_errors_and_unpaired_boundaries_can_be_retained() {
    for rows in [
        "960 = A bad",
        "960 = A -1",
        "960 = A 1000000 1",
        "960 = A 1000000\n960 = A 1000000",
        "960 = A 1100000",
        "0 = A 2000",
    ] {
        let source = format!(
            "[Song]\n{{\nResolution = 480\n}}\n[SyncTrack]\n{{\n0 = B 120000\n{rows}\n}}\n"
        );
        assert!(Chart::parse(source).is_err(), "{rows}");
    }
    let source = CHART.replace("960 = B 120000\r\n", "");
    let chart = Chart::parse(source).unwrap();
    let mut tempos = chart.timing.tempos.clone();
    let edit = Edit::PutTempo {
        previous: Some(tempos[0].clone()),
        tempo: Tempo { tick: 0, bpm: 150. },
    };
    tempos[0].bpm = 150.;
    constrain(&mut tempos, &chart, &edit).unwrap();
    let clock = TempoMap::new(480, 0.25, tempos).unwrap();
    assert!((clock.seconds(960) - 1.25).abs() < 1e-9);
    assert!((clock.seconds(1920) - 2.25).abs() < 1e-9);
}
