use chart_engine::{
    chart::{Chart, Note},
    drum_dynamics::{self, Kind},
    editor::{Action, Actor, ActorKind, Content, EditorNote, Project, Request, Store, Transport},
    note_groups::{self, Selection, Snapshot},
    review_regions::{Policy, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n 0 = E \"lyric Preserved\"\n}\n[ExpertDrums]\n{\n 480 = N 2 0\n 480 = N 66 0\n 480 = N 35 0\n 480 = N 41 0\n 960 = N 1 0\n 960 = N 34 0\n 1200 = N 12 17\n 0 = S 2 480\n}\n";

fn fixture(root: &Path) -> Store {
    fixture_with(root, SOURCE)
}
fn fixture_with(root: &Path, source: &str) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    fs::write(root.join("source/notes.chart"), source).unwrap();
    let audio = b"opaque audio bytes; note editing requires no decoder";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(source.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let chart = Chart::parse(source.into()).unwrap();
    let notes = chart
        .notes("ExpertDrums")
        .unwrap()
        .into_iter()
        .map(|note| EditorNote {
            midi: None,
            id: format!("n-{}-{}", note.lane, note.tick),
            note,
        })
        .collect();
    let mut tracks = BTreeMap::from([("ExpertDrums".into(), notes)]);
    if chart
        .sections
        .iter()
        .any(|section| section.name == "ExpertSingle")
    {
        tracks.insert(
            "ExpertSingle".into(),
            chart
                .notes("ExpertSingle")
                .unwrap()
                .into_iter()
                .map(|note| EditorNote {
                    midi: None,
                    id: format!("guitar-{}-{}", note.lane, note.tick),
                    note,
                })
                .collect(),
        );
    }
    let project = Project {
        drum_layout: None,
        recording: None,
        version: 1,
        revision: 0,
        title: "Articulation fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: source_hash.clone(),
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash.clone(),
        duration: 4.0,
        review: ReviewMap::build(
            4.0,
            Policy::default(),
            BTreeMap::from([
                ("chart".into(), source_hash),
                ("original_audio".into(), audio_hash),
            ]),
            None,
            Default::default(),
            vec![],
        )
        .unwrap(),
        content: Content {
            analysis_sources: Default::default(),
            star_power: Default::default(),
            solos: Default::default(),
            drum_ranges: Default::default(),
            disco_ranges: Default::default(),
            midi_drum_mix: None,
            midi_drum_attributes: None,
            bre: None,
            lane_ranges: Default::default(),
            midi_lane_ranges: None,
            sync: None,
            sections: None,
            text_events: None,
            midi_text_events: None,
            venue_events: None,

            track_origins: Default::default(),
            difficulty_links: Default::default(),
            vocals: None,
            harmonies: None,
            vocal_percussion: None,
            vocal_overdrive: None,
            vocal_review: None,
            vocal_pitch_analysis: None,
            tracks,
            decisions: vec![],
            hidden_flags: Default::default(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertDrums".into(),
        selected_region: None,
        selected_note: Some("n-2-480".into()),
        kit_audition: None,
        transport: Transport {
            playback_rate: 1.0,
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    fs::write(
        root.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    Store::new(root)
}

fn request(store: &Store, action: Action) -> Request {
    Request {
        expected_revision: store.load().unwrap().revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Articulation regression".into(),
        },
        action,
    }
}

#[test]
fn legacy_drum_profile_comes_from_retained_source_and_is_saved_on_next_action() {
    use chart_engine::drum_layout::Layout;
    let root = tempfile::tempdir().unwrap();
    let source = SOURCE.replace(" 1200 = N 12 17", " 1440 = N 5 0\n 1200 = N 12 17");
    let store = fixture_with(root.path(), &source);
    let before = fs::read(root.path().join("project.json")).unwrap();
    let project = store.load().unwrap();
    assert_eq!(project.drum_layout, Some(Layout::FiveLane));
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    store
        .apply(request(
            &store,
            Action::SelectTrack {
                track: "ExpertDrums".into(),
            },
        ))
        .unwrap();
    let saved: Project =
        serde_json::from_slice(&fs::read(root.path().join("project.json")).unwrap()).unwrap();
    assert_eq!(saved.drum_layout, Some(Layout::FiveLane));
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );

    let explicit = tempfile::tempdir().unwrap();
    let store = fixture_with(explicit.path(), &source);
    fs::write(
        explicit.path().join("source/song.ini"),
        "[song]\nfive_lane_drums = false\n",
    )
    .unwrap();
    assert_eq!(store.load().unwrap().drum_layout, Some(Layout::FourLane));
}

#[test]
fn four_pad_saves_retain_the_existing_project_schema() {
    use chart_engine::drum_layout::Layout;
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    assert_eq!(store.load().unwrap().drum_layout(), Layout::FourLane);
    store
        .apply(request(
            &store,
            Action::SelectTrack {
                track: "ExpertDrums".into(),
            },
        ))
        .unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("project.json")).unwrap()).unwrap();
    assert!(
        saved.get("drum_layout").is_none(),
        "ordinary projects must not gain a field older clients reject"
    );
    assert_eq!(store.load().unwrap().drum_layout(), Layout::FourLane);
}

#[test]
fn five_pad_edits_keep_dynamics_clipboard_history_and_profile_without_the_last_green() {
    use chart_engine::{
        drum_layout::Layout,
        export_options::{Format, Options},
    };
    let root = tempfile::tempdir().unwrap();
    let source = SOURCE.replace(" 1200 = N 12 17", " 1440 = N 5 0\n 1200 = N 12 17");
    let store = fixture_with(root.path(), &source);
    let id = "n-5-1440";
    store
        .apply(request(
            &store,
            Action::SelectNote {
                note_id: Some(id.into()),
            },
        ))
        .unwrap();
    set_dynamics(&store, &[id], Kind::Ghost);
    store
        .apply(request(
            &store,
            Action::UpdateNote {
                note_id: id.into(),
                tick: 1680,
                lane: 4,
                sustain: 0,
                cymbal: false,
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    assert_eq!(doc.drum_layout(), Layout::FiveLane);
    assert_eq!(doc.lane_rows("ExpertDrums"), [0, 32, 1, 2, 3, 4, 5]);
    assert!(
        doc.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 1680 && n.note.lane == 43)
    );
    assert!(
        !doc.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 1440 && n.note.lane == 44)
    );
    assert_eq!(
        doc.drum_layout()
            .name(doc.find_note(id).unwrap().1.note.lane),
        "Orange"
    );
    let selection = Selection {
        track: "ExpertDrums".into(),
        ids: [id.into()].into(),
        vocals: Default::default(),
    };
    let clipboard = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selection).unwrap();
    let encoded = clipboard.encode().unwrap();
    assert!(encoded.contains("five_lane_drums"));

    let other = tempfile::tempdir().unwrap();
    let four = fixture(other.path());
    let four_before = fs::read(other.path().join("project.json")).unwrap();
    assert!(
        four.apply(request(
            &four,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Paste {
                    track: "ExpertDrums".into(),
                    tick: 2400,
                    clipboard: clipboard.clone()
                }
            }
        ))
        .is_err()
    );
    assert_eq!(
        fs::read(other.path().join("project.json")).unwrap(),
        four_before
    );

    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Move {
                    selection: selected(&store, &[id]),
                    ticks: 240,
                    rows: 1,
                },
            },
        ))
        .unwrap();
    let moved = store.load().unwrap();
    assert_eq!(moved.find_note(id).unwrap().1.note.lane, 5);
    assert!(
        moved.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 1920 && n.note.lane == 44)
    );
    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Paste {
                    track: "ExpertDrums".into(),
                    tick: 2400,
                    clipboard: note_groups::Clipboard::decode(&encoded).unwrap(),
                },
            },
        ))
        .unwrap();
    let pasted = store.load().unwrap();
    assert!(
        pasted.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 2400 && n.note.lane == 4)
    );
    assert!(
        pasted.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 2400 && n.note.lane == 43)
    );
    store
        .apply(request(&store, Action::DeleteNote { note_id: id.into() }))
        .unwrap();
    let deleted = store.load().unwrap();
    assert!(
        !deleted.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.lane == 5)
    );
    assert_eq!(deleted.drum_layout(), Layout::FiveLane);
    let stage = tempfile::tempdir().unwrap();
    let actor = request(&store, Action::Undo).actor;
    let options = Options {
        include_audio: false,
        format: Format::Chart,
        ..Default::default()
    };
    store
        .write_export_with_options(&deleted, stage.path(), &actor, &options)
        .unwrap();
    let ini = fs::read_to_string(stage.path().join("song.ini")).unwrap();
    assert_eq!(
        chart_engine::song_metadata::ini_fields(&ini)["five_lane_drums"],
        "true"
    );
    let refused = tempfile::tempdir().unwrap();
    assert!(
        store
            .write_export_with_options(
                &deleted,
                refused.path(),
                &actor,
                &Options {
                    format: Format::Midi,
                    ..options
                }
            )
            .unwrap_err()
            .to_string()
            .contains("Five-lane drums require .chart")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );
    store.apply(request(&store, Action::Undo)).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.find_note(id).unwrap().1.note.lane, 5);
    assert_eq!(restored.drum_layout(), Layout::FiveLane);
}

fn update(tick: u64, lane: u8, cymbal: bool) -> Action {
    Action::UpdateNote {
        note_id: "n-2-480".into(),
        tick,
        lane,
        sustain: 0,
        cymbal,
    }
}

fn selected(store: &Store, ids: &[&str]) -> Snapshot {
    Snapshot::capture(
        &store.load().unwrap(),
        &Selection {
            track: "ExpertDrums".into(),
            ids: ids.iter().map(|id| (*id).into()).collect(),
            vocals: Default::default(),
        },
    )
    .unwrap()
}
fn set_dynamics(store: &Store, ids: &[&str], kind: Kind) {
    store
        .apply(request(
            store,
            Action::SetDrumDynamics {
                selection: selected(store, ids),
                kind,
            },
        ))
        .unwrap();
}

#[test]
fn authored_ghost_preserves_cymbal_movement_deletion_history_and_source() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    set_dynamics(&store, &["n-2-480"], Kind::Ghost);
    let doc = store.load().unwrap();
    assert!(doc.find_note("n-35-480").is_none());
    assert_eq!(doc.find_note("n-41-480").unwrap().1.note.lane, 41);
    assert_eq!(doc.find_note("n-66-480").unwrap().1.note.lane, 66);
    assert_eq!(doc.undo.len(), 1);
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
    store.apply(request(&store, Action::Redo)).unwrap();
    store.apply(request(&store, update(720, 3, true))).unwrap();
    let moved = store.load().unwrap();
    assert_eq!(
        moved.find_note("n-41-480").unwrap().1.note,
        Note {
            tick: 720,
            lane: 42,
            sustain: 0
        }
    );
    assert!(moved.is_cymbal("ExpertDrums", &moved.find_note("n-2-480").unwrap().1.note));
    store
        .apply(request(
            &store,
            Action::DeleteNote {
                note_id: "n-2-480".into(),
            },
        ))
        .unwrap();
    assert!(store.load().unwrap().find_note("n-41-480").is_none());
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(
        Store::new(root.path())
            .load()
            .unwrap()
            .find_note("n-41-480")
            .unwrap()
            .1
            .note
            .lane,
        42
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        SOURCE
    );
}

#[test]
fn batch_dynamics_skip_kick_preserve_other_flags_and_refuse_stale_selection() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    store
        .apply(request(
            &store,
            Action::AddNote {
                track: "ExpertDrums".into(),
                tick: 0,
                lane: 0,
                sustain: 0,
                cymbal: false,
            },
        ))
        .unwrap();
    let kick = store.load().unwrap().selected_note.unwrap();
    let ids = ["n-2-480", "n-1-960", kick.as_str()];
    let old = selected(&store, &ids);
    let before = store.load().unwrap().chart_text(&store).unwrap();
    set_dynamics(&store, &ids, Kind::Ghost);
    let ghost = store.load().unwrap();
    let markers = drum_dynamics::markers(&ghost.content.tracks["ExpertDrums"]);
    assert_eq!(markers.get(&(480, 2)), Some(&Some(Kind::Ghost)));
    assert_eq!(markers.get(&(960, 1)), Some(&Some(Kind::Ghost)));
    assert!(ghost.find_note(&kick).is_some());
    assert!(ghost.find_note("n-66-480").is_some());
    assert!(ghost.find_note("n-12-1200").is_some());
    let accepted = fs::read(root.path().join("project.json")).unwrap();
    let error = store
        .apply(request(
            &store,
            Action::SetDrumDynamics {
                selection: old,
                kind: Kind::Accent,
            },
        ))
        .unwrap_err();
    assert!(error.to_string().contains("Notes changed"));
    assert_eq!(
        fs::read(root.path().join("project.json")).unwrap(),
        accepted
    );
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), before);
    store.apply(request(&store, Action::Redo)).unwrap();
    set_dynamics(&store, &ids, Kind::Normal);
    let normal = store.load().unwrap();
    assert!(drum_dynamics::markers(&normal.content.tracks["ExpertDrums"]).is_empty());
    assert!(normal.find_note("n-66-480").is_some());
    let accepted = fs::read(root.path().join("project.json")).unwrap();
    assert!(
        store
            .apply(request(
                &store,
                Action::SetDrumDynamics {
                    selection: selected(&store, &[&kick]),
                    kind: Kind::Accent
                }
            ))
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("project.json")).unwrap(),
        accepted
    );
}

#[test]
fn authored_dynamics_survive_group_move_and_typed_clipboard_paste() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    set_dynamics(&store, &["n-2-480"], Kind::Ghost);
    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Move {
                    selection: selected(&store, &["n-2-480"]),
                    ticks: 240,
                    rows: 1,
                },
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    let selection = Selection {
        track: "ExpertDrums".into(),
        ids: ["n-2-480".into()].into(),
        vocals: Default::default(),
    };
    let clipboard = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selection).unwrap();
    let clipboard = note_groups::Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Paste {
                    track: "ExpertDrums".into(),
                    tick: 2400,
                    clipboard,
                },
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    let notes = &doc.content.tracks["ExpertDrums"];
    for (tick, lane) in [(720, 3), (2400, 3)] {
        assert_eq!(
            drum_dynamics::markers(notes).get(&(tick, lane)),
            Some(&Some(Kind::Ghost))
        );
        let note = notes
            .iter()
            .find(|n| n.note.tick == tick && n.note.lane == lane)
            .unwrap();
        assert!(doc.is_cymbal("ExpertDrums", &note.note));
    }
    assert!(doc.find_note("n-34-960").is_some());
    store.apply(request(&store, Action::Undo)).unwrap();
    assert!(
        !store.load().unwrap().content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 2400)
    );
}

#[test]
fn mixed_dynamics_export_to_game_midi_without_losing_cymbal_kick_or_other_parts() {
    let source = format!(
        "{}[ExpertSingle]\n{{\n 480 = N 0 120\n 480 = N 6 0\n}}\n",
        SOURCE.replace(" 1200 = N 12 17\n", " 720 = N 32 0\n")
    );
    let root = tempfile::tempdir().unwrap();
    let store = fixture_with(root.path(), &source);
    set_dynamics(&store, &["n-2-480"], Kind::Ghost);
    let doc = store.load().unwrap();
    let chart = Chart::parse(doc.chart_text(&store).unwrap()).unwrap();
    let bytes = chart_engine::game_export::song(&chart, None).unwrap().0;
    let inventory = chart_engine::midi::inventory(&bytes).unwrap();
    assert!(inventory.iter().any(|(name, _)| name == "PART GUITAR"));
    assert!(inventory.iter().any(|(name, _)| name == "PART DRUMS"));
    let midi = midly::Smf::parse(&bytes).unwrap();
    let velocities: Vec<_> = midi
        .tracks
        .iter()
        .flatten()
        .filter_map(|e| match e.kind {
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { vel, .. },
                ..
            } if matches!(vel.as_int(), 1 | 127) => Some(vel.as_int()),
            _ => None,
        })
        .collect();
    assert!(velocities.contains(&1) && velocities.contains(&127));
    let reopened = root.path().join("reimport");
    fs::create_dir(&reopened).unwrap();
    fs::write(reopened.join("notes.mid"), bytes).unwrap();
    fs::write(reopened.join("song.ini"), "[song]\npro_drums = true\n").unwrap();
    chart_engine::song_layout::prepare(&reopened).unwrap();
    chart_engine::song_files::game_midi::prepare(&reopened).unwrap();
    let restored = Chart::parse(fs::read_to_string(reopened.join("notes.chart")).unwrap()).unwrap();
    let notes = restored.notes("ExpertDrums").unwrap();
    for (tick, lane) in [
        (480, 2),
        (480, 66),
        (480, 41),
        (720, 32),
        (960, 1),
        (960, 34),
    ] {
        assert!(
            notes.iter().any(|n| n.tick == tick && n.lane == lane),
            "missing {tick}/{lane}"
        );
    }
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );
}

#[test]
fn articulated_drums_move_delete_undo_and_reopen_without_orphaning_markers() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    store.apply(request(&store, update(720, 3, true))).unwrap();
    let project = store.load().unwrap();
    for (id, lane) in [("n-35-480", 36), ("n-41-480", 42)] {
        let (_, marker) = project.find_note(id).unwrap();
        assert_eq!(
            marker.note,
            Note {
                tick: 720,
                lane,
                sustain: 0
            }
        );
    }
    assert!(
        project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 720 && event.note.lane == 67)
    );
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 480)
    );
    assert_eq!(project.find_note("n-34-960").unwrap().1.note.lane, 34);
    let inspection = store.inspect().unwrap();
    let note = inspection["octave_view"]["tracks"][0]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == "n-2-480")
        .unwrap();
    assert_eq!(note["lane_name"], "blueCymbal");
    assert_eq!(note["flags"]["accent"], true);
    assert_eq!(note["flags"]["ghost"], true);
    assert_eq!(note["editable"], true);
    let text = project.chart_text(&store).unwrap();
    assert!(text.contains("0 = S 2 480\n"));
    assert!(text.contains("0 = E \"lyric Preserved\"\n"));
    assert!(text.contains("1200 = N 12 17\n"));
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
    store.apply(request(&store, Action::Redo)).unwrap();
    let reopened = Store::new(root.path());
    assert_eq!(
        reopened
            .load()
            .unwrap()
            .find_note("n-35-480")
            .unwrap()
            .1
            .note
            .lane,
        36
    );
    reopened
        .apply(request(
            &reopened,
            Action::DeleteNote {
                note_id: "n-2-480".into(),
            },
        ))
        .unwrap();
    let project = reopened.load().unwrap();
    assert!(project.find_note("n-35-480").is_none());
    assert!(project.find_note("n-41-480").is_none());
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| [36, 42, 67].contains(&event.note.lane))
    );
    assert!(project.find_note("n-34-960").is_some());
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        SOURCE
    );
}

#[test]
fn changing_an_articulated_pad_to_kick_removes_incompatible_controls_reversibly() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    store.apply(request(&store, update(720, 0, false))).unwrap();
    let project = store.load().unwrap();
    assert_eq!(project.find_note("n-2-480").unwrap().1.note.lane, 0);
    for marker in ["n-35-480", "n-41-480", "n-66-480"] {
        assert!(project.find_note(marker).is_none());
    }
    assert!(
        !project.content.tracks["ExpertDrums"]
            .iter()
            .any(|event| event.note.tick == 720 && [33, 39, 64].contains(&event.note.lane))
    );
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
}

#[test]
fn articulation_collision_rejects_the_whole_move_without_losing_marker_ids() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut project = store.load().unwrap();
    project
        .content
        .tracks
        .get_mut("ExpertDrums")
        .unwrap()
        .push(EditorNote {
            midi: None,
            id: "destination-accent".into(),
            note: Note {
                tick: 720,
                lane: 36,
                sustain: 0,
            },
        });
    fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let before = fs::read(root.path().join("project.json")).unwrap();
    let error = store
        .apply(request(&store, update(720, 3, true)))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("articulation marker already exists")
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    assert!(store.load().unwrap().find_note("n-35-480").is_some());
    assert!(
        store
            .load()
            .unwrap()
            .find_note("destination-accent")
            .is_some()
    );
}

#[test]
fn double_kick_selection_move_clipboard_and_delete_keep_the_ordinary_kick_and_unknowns() {
    let source = SOURCE.replace(
        " 480 = N 2 0",
        " 480 = N 0 0\n 480 = N 32 0\n 480 = N 96 9\n 480 = N 2 0",
    );
    let root = tempfile::tempdir().unwrap();
    let store = fixture_with(root.path(), &source);
    store
        .apply(request(
            &store,
            Action::SelectNote {
                note_id: Some("n-32-480".into()),
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    assert!(!doc.is_cymbal("ExpertDrums", &doc.find_note("n-32-480").unwrap().1.note));
    store
        .apply(request(
            &store,
            Action::UpdateNote {
                note_id: "n-32-480".into(),
                tick: 720,
                lane: 32,
                sustain: 0,
                cymbal: false,
            },
        ))
        .unwrap();
    let selection = selected(&store, &["n-32-480"]);
    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Move {
                    selection,
                    ticks: 240,
                    rows: 0,
                },
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    let selection = Selection {
        track: "ExpertDrums".into(),
        ids: ["n-32-480".into()].into(),
        vocals: Default::default(),
    };
    let clipboard = note_groups::copy(&doc, &doc.timing(&store).unwrap(), &selection).unwrap();
    let clipboard = note_groups::Clipboard::decode(&clipboard.encode().unwrap()).unwrap();
    store
        .apply(request(
            &store,
            Action::EditNoteGroup {
                edit: note_groups::Edit::Paste {
                    track: "ExpertDrums".into(),
                    tick: 1440,
                    clipboard,
                },
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    assert!(
        doc.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 1440 && n.note.lane == 32)
    );
    store
        .apply(request(
            &store,
            Action::DeleteNote {
                note_id: "n-32-480".into(),
            },
        ))
        .unwrap();
    assert!(store.load().unwrap().find_note("n-32-480").is_none());
    store.apply(request(&store, Action::Undo)).unwrap();
    let doc = Store::new(root.path()).load().unwrap();
    assert_eq!(doc.find_note("n-32-480").unwrap().1.note.tick, 960);
    assert_eq!(doc.find_note("n-0-480").unwrap().1.note.tick, 480);
    assert_eq!(
        doc.find_note("n-96-480").unwrap().1.note,
        Note {
            tick: 480,
            lane: 96,
            sustain: 9
        }
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );
}

#[test]
fn double_kick_placement_coexists_with_kick_and_rejects_cymbals_and_sustains_atomically() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    for lane in [0, 32] {
        store
            .apply(request(
                &store,
                Action::AddNote {
                    track: "ExpertDrums".into(),
                    tick: 720,
                    lane,
                    sustain: 0,
                    cymbal: false,
                },
            ))
            .unwrap();
    }
    let doc = store.load().unwrap();
    let id = doc.content.tracks["ExpertDrums"]
        .iter()
        .find(|n| n.note.tick == 720 && n.note.lane == 32)
        .unwrap()
        .id
        .clone();
    let before = fs::read(root.path().join("project.json")).unwrap();
    for (sustain, cymbal) in [(1, false), (0, true)] {
        assert!(
            store
                .apply(request(
                    &store,
                    Action::UpdateNote {
                        note_id: id.clone(),
                        tick: 960,
                        lane: 32,
                        sustain,
                        cymbal
                    }
                ))
                .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    }
    store
        .apply(request(
            &store,
            Action::DeleteNote {
                note_id: id.clone(),
            },
        ))
        .unwrap();
    let doc = store.load().unwrap();
    assert!(doc.find_note(&id).is_none());
    assert!(
        doc.content.tracks["ExpertDrums"]
            .iter()
            .any(|n| n.note.tick == 720 && n.note.lane == 0)
    );
    store.apply(request(&store, Action::Undo)).unwrap();
    assert_eq!(
        store.load().unwrap().find_note(&id).unwrap().1.note.lane,
        32
    );
}

#[test]
fn authored_double_kick_roundtrips_as_midi95_and_lower_tier_midi_refuses_loss() {
    let source = SOURCE.replace(" 1200 = N 12 17\n", "");
    let root = tempfile::tempdir().unwrap();
    let store = fixture_with(root.path(), &source);
    set_dynamics(&store, &["n-2-480"], Kind::Ghost);
    for lane in [0, 32] {
        store
            .apply(request(
                &store,
                Action::AddNote {
                    track: "ExpertDrums".into(),
                    tick: 720,
                    lane,
                    sustain: 0,
                    cymbal: false,
                },
            ))
            .unwrap();
    }
    let text = store.load().unwrap().chart_text(&store).unwrap();
    let chart = Chart::parse(text.clone()).unwrap();
    let bytes = chart_engine::game_export::song(&chart, None).unwrap().0;
    let midi = midly::Smf::parse(&bytes).unwrap();
    let pitches: Vec<_> = midi
        .tracks
        .iter()
        .flatten()
        .filter_map(|e| match e.kind {
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { key, vel },
                ..
            } if vel.as_int() > 0 => Some(key.as_int()),
            _ => None,
        })
        .collect();
    assert!(pitches.contains(&95) && pitches.contains(&96));
    let reopened = root.path().join("double-kick-midi-only");
    fs::create_dir(&reopened).unwrap();
    fs::write(reopened.join("notes.mid"), bytes).unwrap();
    fs::write(reopened.join("song.ini"), "[song]\npro_drums = true\n").unwrap();
    chart_engine::song_layout::prepare(&reopened).unwrap();
    chart_engine::song_files::game_midi::prepare(&reopened).unwrap();
    let restored = Chart::parse(fs::read_to_string(reopened.join("notes.chart")).unwrap()).unwrap();
    let notes = restored.notes("ExpertDrums").unwrap();
    for (tick, lane) in [(720, 0), (720, 32), (480, 2), (480, 66), (480, 41)] {
        assert!(
            notes.iter().any(|n| n.tick == tick && n.lane == lane),
            "missing {tick}/{lane}"
        );
    }
    let hard = Chart::parse(text.replace("ExpertDrums", "HardDrums")).unwrap();
    assert!(
        format!(
            "{:#}",
            chart_engine::game_export::song(&hard, None).unwrap_err()
        )
        .contains("Double kick MIDI export requires Expert drums")
    );
    assert!(
        hard.notes("HardDrums")
            .unwrap()
            .iter()
            .any(|n| n.lane == 32)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );
}

#[test]
fn moving_imported_drum_hits_preserves_note_off_intervals_and_midi_attributes() {
    for lane in [2, 32] {
        let source = if lane == 2 {
            SOURCE.replace("480 = N 2 0", "480 = N 2 1")
        } else {
            SOURCE.replace("480 = N 2 0", "480 = N 32 1\n 480 = N 96 9\n 480 = N 2 0")
        };
        let root = tempfile::tempdir().unwrap();
        let store = fixture_with(root.path(), &source);
        let id = format!("n-{lane}-480");
        let attributes = chart_engine::song_files::game_midi::NoteAttributes {
            channel: 3,
            velocity: 29,
            off_velocity: 7,
            off_as_note_on: false,
            zero_length: false,
            talkie_pitch: None,
        };
        let mut doc = store.load().unwrap();
        doc.content
            .tracks
            .get_mut("ExpertDrums")
            .unwrap()
            .iter_mut()
            .find(|n| n.id == id)
            .unwrap()
            .midi = Some(attributes.clone());
        fs::write(
            root.path().join("project.json"),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        store
            .apply(request(
                &store,
                Action::UpdateNote {
                    note_id: id.clone(),
                    tick: 720,
                    lane,
                    sustain: 1,
                    cymbal: lane == 2,
                },
            ))
            .unwrap();
        let doc = store.load().unwrap();
        let (_, note) = doc.find_note(&id).unwrap();
        assert_eq!(
            note.note,
            Note {
                tick: 720,
                lane,
                sustain: 1
            }
        );
        assert_eq!(note.midi, Some(attributes.clone()));
        assert_eq!(doc.is_cymbal("ExpertDrums", &note.note), lane == 2);
        let before = fs::read(root.path().join("project.json")).unwrap();
        assert!(
            store
                .apply(request(
                    &store,
                    Action::UpdateNote {
                        note_id: id.clone(),
                        tick: 960,
                        lane,
                        sustain: 2,
                        cymbal: false
                    }
                ))
                .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        store.apply(request(&store, Action::Undo)).unwrap();
        let doc = store.load().unwrap();
        let (_, note) = doc.find_note(&id).unwrap();
        assert_eq!(note.note.tick, 480);
        assert_eq!(note.note.sustain, 1);
        assert_eq!(note.midi, Some(attributes));
        if lane == 32 {
            assert!(doc.find_note("n-96-480").is_some());
        }
        assert_eq!(
            fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
            source
        );
    }
}
