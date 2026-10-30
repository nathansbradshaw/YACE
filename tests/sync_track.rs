#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    sync_track::{self, Edit, Meter},
    timing::Tempo,
};
use std::{collections::BTreeSet, fs};
fn action(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "You".into(),
        },
        action,
    })?;
    Ok(())
}
fn edit(store: &Store, edit: Edit) -> anyhow::Result<()> {
    action(store, Action::EditSync { edit })
}
fn tempo(tick: u64, bpm: f64) -> Tempo {
    Tempo { tick, bpm }
}
fn meter(tick: u64, numerator: u8, denominator: u16) -> Meter {
    Meter {
        tick,
        numerator,
        denominator,
    }
}
fn initial(store: &Store, bpm: f64) -> anyhow::Result<()> {
    edit(
        store,
        Edit::PutTempo {
            previous: Some(tempo(0, 120.)),
            tempo: tempo(0, bpm),
        },
    )
}
fn content(store: &Store) -> serde_json::Value {
    serde_json::to_value(store.load().unwrap().content).unwrap()
}

#[test]
fn bar_windows_include_rounded_ticks_at_unusual_resolutions() {
    let timing = chart_engine::timing::TempoMap::new(5, 0., vec![tempo(0, 120.)]).unwrap();
    let meters = vec![meter(0, 1, 8)];
    let all = sync_track::bars(&timing, &meters, 0, 15, 100);
    assert_eq!(
        all.iter().map(|bar| bar.tick).collect::<Vec<_>>(),
        vec![0, 3, 5, 8, 10, 13, 15]
    );
    for tick in [3, 8, 13] {
        let window = sync_track::bars(&timing, &meters, tick, tick, 100);
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].tick, tick);
    }
}

#[test]
fn tempo_changes_keep_ticks_and_sources_and_undo_the_whole_edit() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let before = store.load().unwrap();
    let bytes = fs::read(root.path().join("source/notes.chart")).unwrap();
    initial(&store, 240.).unwrap();
    let after = store.load().unwrap();
    assert_eq!(after.content.tracks, before.content.tracks);
    assert!((after.timing(&store).unwrap().seconds(480) - 0.25).abs() < 1e-9);
    assert_eq!(after.undo.len(), 1);
    let changed = content(&store);
    action(&store, Action::Undo).unwrap();
    assert_eq!(
        content(&store),
        serde_json::to_value(&before.content).unwrap()
    );
    action(&store, Action::Redo).unwrap();
    assert_eq!(content(&store), changed);
    assert_eq!(
        fs::read(root.path().join("source/notes.chart")).unwrap(),
        bytes
    );
}

#[test]
fn tempo_edit_pauses_playback_and_invalidates_note_approvals() {
    use chart_engine::editor::DecisionStatus;
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let doc = store.load().unwrap();
    let region = doc.review.regions[0].id.clone();
    action(
        &store,
        Action::Decide {
            region_id: region.clone(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "Checked against the recording".into(),
        },
    )
    .unwrap();
    action(&store, Action::Playback { playing: true }).unwrap();
    initial(&store, 240.).unwrap();
    let changed = store.load().unwrap();
    assert!(!changed.transport.playing);
    assert!(
        changed
            .decision(&region, "ExpertSingle", &changed.timing(&store).unwrap())
            .is_none()
    );
    action(&store, Action::Playback { playing: true }).unwrap();
    action(&store, Action::Undo).unwrap();
    let restored = store.load().unwrap();
    assert!(!restored.transport.playing);
    assert!(
        restored
            .decision(&region, "ExpertSingle", &restored.timing(&store).unwrap())
            .is_some()
    );
}
#[test]
fn tempo_markers_can_be_added_moved_edited_and_removed_without_changing_offset() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    edit(
        &store,
        Edit::PutTempo {
            previous: None,
            tempo: tempo(960, 180.125),
        },
    )
    .unwrap();
    edit(
        &store,
        Edit::PutTempo {
            previous: Some(tempo(960, 180.125)),
            tempo: tempo(1440, 200.),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().timing(&store).unwrap().tempos,
        vec![tempo(0, 120.), tempo(1440, 200.)]
    );
    edit(
        &store,
        Edit::RemoveTempo {
            tempo: tempo(1440, 200.),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().timing(&store).unwrap(),
        chart_engine::timing::TempoMap::constant(120.).unwrap()
    );
}
#[test]
fn invalid_duplicate_stale_or_out_of_recording_edits_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    for operation in [
        Edit::PutTempo {
            previous: None,
            tempo: tempo(0, 140.),
        },
        Edit::PutTempo {
            previous: None,
            tempo: tempo(100, 0.),
        },
        Edit::PutTempo {
            previous: None,
            tempo: tempo(100, 120.12345),
        },
        Edit::PutTempo {
            previous: None,
            tempo: tempo(100, f64::NAN),
        },
        Edit::RemoveTempo {
            tempo: tempo(0, 120.),
        },
        Edit::PutTempo {
            previous: Some(tempo(0, 120.)),
            tempo: tempo(1, 130.),
        },
        Edit::PutTempo {
            previous: None,
            tempo: tempo(8000, 120.),
        },
        Edit::PutMeter {
            previous: None,
            meter: meter(0, 3, 4),
        },
        Edit::PutMeter {
            previous: None,
            meter: meter(100, 3, 3),
        },
        Edit::PutMeter {
            previous: None,
            meter: meter(100, 0, 4),
        },
        Edit::RemoveMeter {
            meter: meter(0, 4, 4),
        },
    ] {
        let before = fs::read(root.path().join("project.json")).unwrap();
        assert!(edit(&store, operation).is_err());
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.path().join("checkpoints"))
                .unwrap()
                .count(),
            0
        );
    }
    initial(&store, 180.).unwrap();
    assert!(initial(&store, 240.).is_err());
}
#[test]
fn slowing_the_grid_cannot_silently_push_notes_past_the_recording() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let before = content(&store);
    assert!(initial(&store, 60.).is_err());
    assert_eq!(content(&store), before);
}
#[test]
fn meter_changes_update_bar_numbers_without_changing_audio_timing() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let before = store.load().unwrap().timing(&store).unwrap();
    edit(
        &store,
        Edit::PutMeter {
            previous: Some(meter(0, 4, 4)),
            meter: meter(0, 3, 4),
        },
    )
    .unwrap();
    edit(
        &store,
        Edit::PutMeter {
            previous: None,
            meter: meter(2880, 7, 8),
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    assert_eq!(doc.timing(&store).unwrap(), before);
    let bars = sync_track::bars(&before, &doc.content.sync.unwrap().meters, 0, 7000, 100);
    assert_eq!(
        bars.iter().map(|b| (b.tick, b.number)).collect::<Vec<_>>(),
        vec![(0, 1), (1440, 2), (2880, 3), (4560, 4), (6240, 5)]
    );
}
#[test]
fn partial_bars_and_zoomed_windows_have_stable_bar_numbers() {
    let map = chart_engine::timing::TempoMap::constant(120.).unwrap();
    let meters = vec![meter(0, 4, 4), meter(960, 3, 4)];
    assert_eq!(
        sync_track::bars(&map, &meters, 0, 5000, 100)
            .iter()
            .map(|b| (b.tick, b.number))
            .collect::<Vec<_>>(),
        vec![(0, 1), (960, 2), (2400, 3), (3840, 4)]
    );
    assert_eq!(
        sync_track::bars(&map, &meters, 1000, 5000, 100)
            .iter()
            .map(|b| (b.tick, b.number))
            .collect::<Vec<_>>(),
        vec![(2400, 3), (3840, 4)]
    );
    assert!(sync_track::bars(&map, &[meter(0, 1, 128)], 0, 1_000_000, 80).len() <= 80);
}
#[test]
fn stale_note_gestures_and_generation_targets_cannot_cross_a_tempo_edit() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let doc = store.load().unwrap();
    let target = chart_engine::parts::Target::capture(&store, &doc, "ExpertSingle").unwrap();
    let selected = chart_engine::note_groups::Selection {
        track: "ExpertSingle".into(),
        ids: BTreeSet::from([doc.content.tracks["ExpertSingle"][0].id.clone()]),
        ..Default::default()
    };
    let snapshot = chart_engine::note_groups::Snapshot::capture(&doc, &selected).unwrap();
    initial(&store, 180.).unwrap();
    assert!(target.validate(&store, &store.load().unwrap()).is_err());
    assert!(
        action(
            &store,
            Action::EditNoteGroup {
                edit: chart_engine::note_groups::Edit::Move {
                    selection: snapshot,
                    ticks: 20,
                    rows: 0
                }
            }
        )
        .is_err()
    );
}
#[test]
fn linked_reductions_and_the_tempo_map_restore_together() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    action(
        &store,
        Action::CreateTrack {
            track: "HardSingle".into(),
        },
    )
    .unwrap();
    action(
        &store,
        Action::EnableDifficultyLink {
            track: "HardSingle".into(),
            enabled: true,
        },
    )
    .unwrap();
    let before = content(&store);
    initial(&store, 180.).unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks["ExpertSingle"].len(),
        3
    );
    action(&store, Action::Undo).unwrap();
    assert_eq!(content(&store), before);
    action(&store, Action::Redo).unwrap();
    assert_eq!(
        store.load().unwrap().timing(&store).unwrap().tempos[0].bpm,
        180.
    );
}
#[test]
fn chart_and_midi_exports_use_the_edited_sync_track() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    edit(
        &store,
        Edit::PutTempo {
            previous: None,
            tempo: tempo(960, 180.125),
        },
    )
    .unwrap();
    edit(
        &store,
        Edit::PutMeter {
            previous: Some(meter(0, 4, 4)),
            meter: meter(0, 7, 8),
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let chart = Chart::parse(doc.chart_text(&store).unwrap()).unwrap();
    assert_eq!(chart.timing, doc.timing(&store).unwrap());
    let bytes = chart_engine::game_export::chart_to_midi(&chart).unwrap();
    let midi = chart_engine::midi::timing(&bytes).unwrap();
    for tick in [0, 480, 960, 2400] {
        assert!((midi.seconds(tick) - chart.timing.seconds(tick)).abs() < 0.0001);
    }
    let smf = midly::Smf::parse(&bytes).unwrap();
    assert!(smf.tracks.iter().flatten().any(|e| matches!(
        e.kind,
        midly::TrackEventKind::Meta(midly::MetaMessage::TimeSignature(7, 3, 24, 8))
    )));
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(output.join("notes.chart")).unwrap(),
        chart.raw
    );
}
#[test]
fn rendering_preserves_offsets_crlf_and_unrelated_events() {
    let raw = "[Song]\r\n{\r\n Resolution = 192\r\n Offset = -0.25\r\n}\r\n[SyncTrack]\r\n{\r\n // keep\r\n 0 = TS 4\r\n 0 = B 120000\r\n 100 = X 12\r\n}\r\n[Events]\r\n{\r\n 0 = E \"section Intro\"\r\n}\r\n";
    let chart = Chart::parse(raw.into()).unwrap();
    let mut sync = sync_track::imported(&chart).unwrap();
    sync.tempos[0].bpm = 135.125;
    let rendered = sync_track::render(raw.into(), &Some(sync)).unwrap();
    for line in [
        " Offset = -0.25\r\n",
        " // keep\r\n",
        " 100 = X 12\r\n",
        " 0 = E \"section Intro\"\r\n",
    ] {
        assert!(rendered.contains(line));
    }
    assert_eq!(Chart::parse(rendered).unwrap().timing.offset, -0.25);
}
