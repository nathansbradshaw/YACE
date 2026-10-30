use chart_engine::{
    chart::{self, Chart, Note, Tracks},
    difficulty_links,
    editor::{
        Action, Actor, ActorKind, Content, DecisionStatus, EditorNote, Project, Request, Store,
        Transport,
    },
    generated_rules::{RuleCode, Severity},
    review_regions::{Issue, Policy, Reason, ReviewMap},
    timing::TempoMap,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

fn n(tick: u64, lane: u8, sustain: u64) -> Note {
    Note {
        tick,
        lane,
        sustain,
    }
}
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Linked-chart regression".into(),
    }
}

fn fixture(root: &Path, generated: bool) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    let timing = TempoMap::constant(120.0).unwrap();
    let tracks = Tracks::from([
        (
            "ExpertSingle".into(),
            vec![
                n(480, 1, 720),
                n(480, 2, 720),
                n(960, 4, 360),
                n(960, 6, 0),
                n(1500, 0, 0),
                n(1500, 5, 0),
                n(3000, 99, 17),
                n(4800, 3, 0),
            ],
        ),
        ("HardSingle".into(), vec![n(800, 3, 0)]),
        ("MediumSingle".into(), vec![n(900, 4, 0)]),
        ("EasySingle".into(), vec![n(600, 4, 0)]),
        (
            "ExpertDrums".into(),
            vec![
                n(480, 0, 0),
                n(480, 1, 0),
                n(480, 2, 0),
                n(480, 66, 0),
                n(480, 41, 0),
                n(480, 3, 0),
                n(480, 67, 0),
                n(480, 36, 0),
                n(960, 32, 0),
            ],
        ),
        ("HardDrums".into(), vec![n(600, 2, 0)]),
        ("EasyDrums".into(), vec![n(600, 1, 0)]),
    ]);
    let text = chart::new_chart("Links", "Fixture", &timing, &tracks);
    fs::write(root.join("source/notes.chart"), &text).unwrap();
    let audio = b"opaque immutable audio; note operations require no inference";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let chart_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let content = Content {
        star_power: Default::default(),
        sync: None,
        sections: None,
        track_origins: Default::default(),
        vocals: None,
        vocal_review: None,
        tracks: tracks
            .into_iter()
            .map(|(track, notes)| {
                let notes = notes
                    .into_iter()
                    .map(|note| EditorNote {
                        id: format!("source-{track}-{}-{}", note.tick, note.lane),
                        note,
                    })
                    .collect();
                (track, notes)
            })
            .collect(),
        difficulty_links: Default::default(),
        decisions: vec![],
        hidden_flags: Default::default(),
        stems: None,
        keyboard_stems: None,
        stem_mix: Default::default(),
    };
    let generated = generated.then(|| {
        BTreeSet::from([
            "EasySingle".into(),
            "MediumSingle".into(),
            "HardSingle".into(),
            "HardDrums".into(),
        ])
    });
    let review = ReviewMap::build(
        12.0,
        Policy {
            window_seconds: 2.0,
            ..Default::default()
        },
        BTreeMap::from([
            ("chart".into(), chart_hash.clone()),
            ("original_audio".into(), audio_hash.clone()),
        ]),
        generated,
        BTreeSet::from([Reason::LowCandidateScore]),
        vec![
            Issue::at(0.5, "EasySingle", Reason::LowCandidateScore, &[]),
            Issue::at(5.0, "EasySingle", Reason::LowCandidateScore, &[]),
        ],
    )
    .unwrap();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Linked fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: chart_hash,
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash,
        duration: 12.0,
        review,
        content,
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: None,
        kit_audition: None,
        transport: Transport {
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
            playback_rate: 1.0,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    fs::write(
        root.join("project.json"),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    Store::new(root)
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: actor(),
            action,
        })
        .unwrap();
}
fn link(store: &Store, track: &str) {
    apply(
        store,
        Action::EnableDifficultyLink {
            track: track.into(),
            enabled: true,
        },
    );
}
fn at<'a>(project: &'a Project, track: &str, tick: u64, lane: u8) -> &'a EditorNote {
    project.content.tracks[track]
        .iter()
        .find(|n| n.note.tick == tick && n.note.lane == lane)
        .unwrap()
}
fn update(store: &Store, id: &str, tick: u64, lane: u8, sustain: u64) {
    apply(
        store,
        Action::UpdateNote {
            note_id: id.into(),
            tick,
            lane,
            sustain,
            cymbal: false,
        },
    );
}
fn id(tick: u64, lane: u8) -> String {
    format!("source-ExpertSingle-{tick}-{lane}")
}

#[test]
fn direct_expert_reduction_preserves_chord_parentage_source_controls_and_stable_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), false);
    let original = store.load().unwrap().content.tracks["ExpertSingle"].clone();
    let source = fs::read(temp.path().join("source/notes.chart")).unwrap();
    let before = store.load().unwrap();
    let timing = before.timing(&store).unwrap();
    let preview = difficulty_links::preview(&before, "EasySingle", &timing).unwrap();
    assert!(preview.removed_count > 0 && preview.added_count > 0 && !preview.rules.has_errors());
    link(&store, "EasySingle");
    link(&store, "MediumSingle");
    link(&store, "HardSingle");
    let project = store.load().unwrap();
    let easy = at(&project, "EasySingle", 480, 1);
    let ancestry = &project.content.difficulty_links.links["EasySingle"].ancestry[&easy.id];
    assert_eq!(ancestry.parent_ids, vec![id(480, 1), id(480, 2)]);
    assert_eq!(ancestry.source_lanes, vec![1, 2]);
    assert_eq!(ancestry.mapped_lane, 1);
    assert!(
        ancestry
            .rule_ids
            .iter()
            .any(|r| r == "fixed_tier_lane_compression")
    );
    assert!(
        project.content.tracks["EasySingle"]
            .iter()
            .filter(|n| n.note.lane <= 4)
            .all(|n| n.note.lane <= 2)
    );
    assert!(
        project.content.tracks["MediumSingle"]
            .iter()
            .filter(|n| n.note.lane <= 4)
            .all(|n| n.note.lane <= 3)
    );
    assert_eq!(at(&project, "HardSingle", 480, 1).note.lane, 1);
    let stable = at(&project, "EasySingle", 960, 2).id.clone();
    let tap = at(&project, "EasySingle", 960, 6).id.clone();
    let remote = at(&project, "EasySingle", 4800, 2).clone();
    update(&store, &id(960, 4), 1080, 3, 360);
    let project = store.load().unwrap();
    assert_eq!(at(&project, "EasySingle", 1080, 2).id, stable);
    assert_eq!(at(&project, "EasySingle", 1080, 6).id, tap);
    assert_eq!(at(&project, "EasySingle", 4800, 2), &remote);
    assert_eq!(
        project.content.tracks["ExpertSingle"]
            .iter()
            .find(|n| n.note.lane == 99),
        original.iter().find(|n| n.note.lane == 99)
    );
    assert_eq!(
        fs::read(temp.path().join("source/notes.chart")).unwrap(),
        source
    );
    let view = difficulty_links::snapshot(&project, &timing);
    assert!(
        view.tracks
            .iter()
            .find(|t| t.track == "HardSingle")
            .unwrap()
            .findings
            .iter()
            .any(|f| f.code == "forced_context_review")
    );
}

#[test]
fn manual_easier_edits_and_suppressions_survive_expert_changes_without_touching_harder() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), false);
    for track in ["EasySingle", "MediumSingle", "HardSingle"] {
        link(&store, track);
    }
    let project = store.load().unwrap();
    let master = project.content.tracks["ExpertSingle"].clone();
    let hard = project.content.tracks["HardSingle"].clone();
    let edited = at(&project, "EasySingle", 480, 1).id.clone();
    let suppressed = at(&project, "EasySingle", 960, 2).id.clone();
    update(&store, &edited, 720, 2, 0);
    apply(
        &store,
        Action::DeleteNote {
            note_id: suppressed.clone(),
        },
    );
    let project = store.load().unwrap();
    assert_eq!(project.content.tracks["ExpertSingle"], master);
    assert_eq!(project.content.tracks["HardSingle"], hard);
    apply(
        &store,
        Action::DeleteNote {
            note_id: id(480, 1),
        },
    );
    update(&store, &id(960, 4), 1200, 4, 0);
    let project = store.load().unwrap();
    assert_eq!(at(&project, "EasySingle", 720, 2).id, edited);
    assert!(
        !project.content.tracks["EasySingle"]
            .iter()
            .any(|n| n.note.tick == 1200 && n.note.lane <= 4)
    );
    assert!(
        project.content.tracks["HardSingle"]
            .iter()
            .any(|n| n.note.tick == 1200 && n.note.lane == 4)
    );
    let snapshot = difficulty_links::snapshot(&project, &project.timing(&store).unwrap());
    let easy = snapshot
        .tracks
        .iter()
        .find(|t| t.track == "EasySingle")
        .unwrap();
    assert!(
        easy.overrides
            .iter()
            .any(|o| o.note_id == edited && o.orphaned && !o.suppressed)
    );
    assert!(
        easy.overrides
            .iter()
            .any(|o| o.note_id == suppressed && o.suppressed)
    );
    assert!(easy.findings.iter().any(|f| f.code == "orphan_override"));
    apply(
        &store,
        Action::ClearDifficultyOverrides {
            track: "EasySingle".into(),
            note_id: Some(edited.clone()),
        },
    );
    assert!(
        !store.load().unwrap().content.tracks["EasySingle"]
            .iter()
            .any(|n| n.id == edited)
    );
    apply(&store, Action::Undo);
    assert_eq!(
        at(
            &Store::new(temp.path()).load().unwrap(),
            "EasySingle",
            720,
            2
        )
        .id,
        edited
    );
    apply(&store, Action::Redo);
    apply(
        &store,
        Action::ClearDifficultyOverrides {
            track: "EasySingle".into(),
            note_id: None,
        },
    );
    assert!(
        store.load().unwrap().content.tracks["EasySingle"]
            .iter()
            .any(|n| n.note.tick == 1200 && n.note.lane == 2)
    );
}

#[test]
fn independent_generated_and_paused_states_are_explicit_and_legacy_content_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), true);
    let mut project = store.load().unwrap();
    let timing = project.timing(&store).unwrap();
    assert!(
        !difficulty_links::snapshot(&project, &timing)
            .tracks
            .iter()
            .find(|t| t.track == "EasySingle")
            .unwrap()
            .has_link
    );
    let authored = project.content.tracks["EasyDrums"].clone();
    difficulty_links::enable_generated(
        &mut project.content,
        project.review.generated_tracks.as_ref(),
        &timing,
        project.duration,
    )
    .unwrap();
    assert!(
        project
            .content
            .difficulty_links
            .links
            .contains_key("EasySingle")
    );
    assert!(
        !project
            .content
            .difficulty_links
            .links
            .contains_key("EasyDrums")
    );
    assert_eq!(project.content.tracks["EasyDrums"], authored);
    let mut value = serde_json::to_value(&project.content).unwrap();
    value.as_object_mut().unwrap().remove("difficulty_links");
    let legacy: Content = serde_json::from_value(value).unwrap();
    assert!(legacy.difficulty_links.links.is_empty());
    link(&store, "EasySingle");
    let before = store.load().unwrap().content.tracks["EasySingle"].clone();
    apply(
        &store,
        Action::EnableDifficultyLink {
            track: "EasySingle".into(),
            enabled: false,
        },
    );
    update(&store, &id(960, 4), 1080, 3, 0);
    let project = store.load().unwrap();
    assert_eq!(project.content.tracks["EasySingle"], before);
    let state = difficulty_links::snapshot(&project, &timing)
        .tracks
        .into_iter()
        .find(|t| t.track == "EasySingle")
        .unwrap();
    assert!(state.has_link && !state.linked);
    link(&store, "EasySingle");
    assert!(
        store.load().unwrap().content.tracks["EasySingle"]
            .iter()
            .any(|n| n.note.tick == 1080)
    );
}

#[test]
fn manual_additions_win_future_collisions_and_have_no_invented_expert_parent() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), false);
    link(&store, "EasySingle");
    apply(
        &store,
        Action::AddNote {
            track: "EasySingle".into(),
            tick: 2400,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    let manual = store.load().unwrap().selected_note.unwrap();
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 2400,
            lane: 2,
            sustain: 0,
            cymbal: false,
        },
    );
    let project = store.load().unwrap();
    let state = difficulty_links::snapshot(&project, &project.timing(&store).unwrap())
        .tracks
        .into_iter()
        .find(|t| t.track == "EasySingle")
        .unwrap();
    assert_eq!(at(&project, "EasySingle", 2400, 1).id, manual);
    assert_eq!(
        project.content.tracks["EasySingle"]
            .iter()
            .filter(|n| n.note.tick == 2400 && n.note.lane == 1)
            .count(),
        1
    );
    assert!(
        state
            .overrides
            .iter()
            .any(|o| o.note_id == manual && o.ancestry.is_none() && !o.orphaned)
    );
    assert!(state.findings.iter().any(|f| f.code == "manual_collision"));
}

#[test]
fn drum_reduction_keeps_valid_articulations_and_fast_manual_patterns_remain_visible_review() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), false);
    link(&store, "HardDrums");
    link(&store, "EasyDrums");
    let project = store.load().unwrap();
    assert!(
        !project.content.tracks["HardDrums"]
            .iter()
            .any(|n| n.note.lane == 2 || n.note.lane == 32 || n.note.lane == 41)
    );
    assert!(
        project.content.tracks["HardDrums"]
            .iter()
            .any(|n| n.note.lane == 67)
    );
    assert!(
        project.content.tracks["HardDrums"]
            .iter()
            .any(|n| n.note.lane == 36)
    );
    apply(
        &store,
        Action::AddNote {
            track: "HardDrums".into(),
            tick: 490,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    let project = store.load().unwrap();
    let state = difficulty_links::snapshot(&project, &project.timing(&store).unwrap())
        .tracks
        .into_iter()
        .find(|t| t.track == "HardDrums")
        .unwrap();
    assert!(
        state
            .rules
            .findings
            .iter()
            .any(|f| f.code == RuleCode::FastRepeat && f.severity == Severity::Review)
    );
    assert!(!state.rules.has_errors());
    let bytes = fs::read(temp.path().join("project.json")).unwrap();
    let error = store.apply(Request {
        expected_revision: project.revision,
        actor: actor(),
        action: Action::AddNote {
            track: "HardDrums".into(),
            tick: 490,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    });
    assert!(error.is_err());
    assert_eq!(fs::read(temp.path().join("project.json")).unwrap(), bytes);
}

#[test]
fn expert_section_rework_updates_lowers_preserves_remote_notes_and_scopes_review_decisions() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path(), false);
    link(&store, "EasySingle");
    let project = store.load().unwrap();
    let local = project
        .review
        .regions
        .iter()
        .find(|r| r.start < 2.0)
        .unwrap()
        .id
        .clone();
    let remote_region = project
        .review
        .regions
        .iter()
        .find(|r| r.start >= 4.0)
        .unwrap()
        .id
        .clone();
    for region_id in [&local, &remote_region] {
        apply(
            &store,
            Action::Decide {
                region_id: region_id.clone(),
                track: "EasySingle".into(),
                status: DecisionStatus::Reviewed,
                comment: "Checked this exact local arrangement".into(),
            },
        );
    }
    let remote = at(&store.load().unwrap(), "EasySingle", 4800, 2).clone();
    for source in [id(480, 1), id(480, 2), id(960, 4), id(1500, 0)] {
        apply(&store, Action::DeleteNote { note_id: source });
    }
    for (tick, lane) in [(400, 0), (800, 3), (1200, 4)] {
        apply(
            &store,
            Action::AddNote {
                track: "ExpertSingle".into(),
                tick,
                lane,
                sustain: 0,
                cymbal: false,
            },
        );
    }
    let project = store.load().unwrap();
    let timing = project.timing(&store).unwrap();
    assert_eq!(at(&project, "EasySingle", 4800, 2), &remote);
    assert!(project.decision(&local, "EasySingle", &timing).is_none());
    assert!(
        project
            .decision(&remote_region, "EasySingle", &timing)
            .is_some()
    );
    assert!(
        project.content.tracks["EasySingle"]
            .iter()
            .any(|n| n.note.tick == 800 && n.note.lane == 2)
    );
    assert!(project.content.decisions.len() == 2);
    assert!(
        store.inspect().unwrap()["difficulty_links"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["track"] == "EasySingle" && t["linked"] == true)
    );
    let output = temp.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("editor-review.json")).unwrap()).unwrap();
    assert_eq!(report["musical_acceptance"], "unverified");
    assert!(report["difficulty_link_state"]["links"]["EasySingle"].is_object());
    assert_eq!(
        Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap())
            .unwrap()
            .notes("EasySingle")
            .unwrap(),
        project.content.tracks["EasySingle"]
            .iter()
            .map(|n| n.note.clone())
            .collect::<Vec<_>>()
    );
}

fn expert_only_fixture(root: &Path, newline: &str, final_newline: bool) -> Store {
    let store = fixture(root, false);
    let mut project = store.load().unwrap();
    // Unknown authored sections, events, comments, BOM and line endings must
    // survive a newly created lower difficulty byte-for-byte. Keyboard is now
    // supported, so use a genuinely unknown custom instrument section here.
    let raw = "\u{feff}[Song]\n{\n  Name = \"Authored Expert only\"\n  Resolution = 960\n  Offset = 0.125\n}\n[SyncTrack]\n{\n  0 = B 120000\n  1920 = B 150000\n}\n[Events]\n{\n  960 = E \"section Original phrase\"\n}\n[ExpertSingle]\n{\n  // Preserve this authored spacing.\n  0 = N 1 0\n  960 = N 2 480\n  960 = N 4 480\n  1440 = E solo\n  2880 = N 0 0\n}\n[ExpertCustomKeyboard]\n{\n  123 = N 88 99\n}\n[CustomAuthoredData]\n{\n  secret = preserved exactly\n}";
    let mut raw = raw.replace('\n', newline);
    if final_newline {
        raw.push_str(newline);
    }
    let chart = Chart::parse(raw.clone()).unwrap();
    let notes = chart.notes("ExpertSingle").unwrap();
    project.content.tracks = BTreeMap::from([(
        "ExpertSingle".into(),
        notes
            .into_iter()
            .map(|note| EditorNote {
                id: format!("expert-only-{}-{}", note.tick, note.lane),
                note,
            })
            .collect(),
    )]);
    project.source_chart_sha256 = format!("{:x}", Sha256::digest(raw.as_bytes()));
    project.content.difficulty_links = Default::default();
    project.review = ReviewMap::build(
        project.duration,
        Policy::default(),
        BTreeMap::from([
            ("chart".into(), project.source_chart_sha256.clone()),
            ("original_audio".into(), project.audio_sha256.clone()),
        ]),
        None,
        BTreeSet::new(),
        vec![],
    )
    .unwrap();
    project.selected_track = "ExpertSingle".into();
    project.selected_note = None;
    project.selected_region = None;
    fs::write(root.join("source/notes.chart"), raw).unwrap();
    fs::write(
        root.join("project.json"),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    store.load().unwrap();
    store
}

#[test]
fn expert_only_can_preview_create_all_lower_tracks_append_preserving_authored_bytes_and_undo_selected_row()
 {
    for (newline, final_newline) in [("\n", true), ("\r\n", false)] {
        let temp = tempfile::tempdir().unwrap();
        let store = expert_only_fixture(temp.path(), newline, final_newline);
        let original = fs::read_to_string(temp.path().join("source/notes.chart")).unwrap();
        let original_expert = store.load().unwrap().content.tracks["ExpertSingle"].clone();
        for track in ["EasySingle", "MediumSingle", "HardSingle"] {
            let before = store.load().unwrap();
            let timing = before.timing(&store).unwrap();
            let preview = difficulty_links::preview(&before, track, &timing).unwrap();
            assert_eq!(preview.before_count, 0);
            assert!(
                preview.after_count > 0
                    && preview.added_count == preview.after_count
                    && !preview.rules.has_errors()
            );
            link(&store, track);
            let project = store.load().unwrap();
            assert_eq!(project.content.tracks["ExpertSingle"], original_expert);
            assert!(!project.content.tracks[track].is_empty());
            assert!(project.content.difficulty_links.links[track].enabled);
            assert!(project.content.decisions.is_empty());
            assert!(
                project.review.generated_tracks.is_none(),
                "New lineage must not fabricate original generation provenance"
            );
            assert!(project.chart_text(&store).unwrap().starts_with(&original));
        }
        apply(
            &store,
            Action::SelectTrack {
                track: "HardSingle".into(),
            },
        );
        apply(&store, Action::Undo);
        let undone = store.load().unwrap();
        assert!(!undone.content.tracks.contains_key("HardSingle"));
        assert_eq!(undone.selected_track, "ExpertSingle");
        assert!(!undone.chart_text(&store).unwrap().contains("[HardSingle]"));
        apply(&store, Action::Redo);
        let project = store.load().unwrap();
        let text = project.chart_text(&store).unwrap();
        assert!(text.starts_with(&original));
        if newline == "\r\n" {
            assert!(!text.replace("\r\n", "").contains('\n'));
        }
        let emitted = Chart::parse(text).unwrap();
        for track in ["EasySingle", "MediumSingle", "HardSingle"] {
            assert_eq!(
                emitted.notes(track).unwrap(),
                project.content.tracks[track]
                    .iter()
                    .map(|item| item.note.clone())
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            fs::read_to_string(temp.path().join("source/notes.chart")).unwrap(),
            original
        );
        let output = temp.path().join("export");
        apply(
            &store,
            Action::Export {
                output: output.clone(),
            },
        );
        assert!(
            fs::read_to_string(output.join("notes.chart"))
                .unwrap()
                .starts_with(&original)
        );
        assert_eq!(
            fs::read(output.join("song.ogg")).unwrap(),
            fs::read(temp.path().join("source/song.ogg")).unwrap()
        );
        // Undoing all three creations removes their rows and emitted sections.
        for _ in 0..3 {
            apply(&store, Action::Undo);
        }
        let restored = store.load().unwrap();
        assert_eq!(restored.content.tracks.len(), 1);
        assert_eq!(restored.chart_text(&store).unwrap(), original);
    }
}

#[test]
fn new_lower_track_authorization_and_empty_expert_fail_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let store = expert_only_fixture(temp.path(), "\n", false);
    for track in [
        "ExpertDoubleBass",
        "HardDoubleBass",
        "ExpertSingle",
        "EasyKeyboard",
        "HardSingleBogus",
    ] {
        let before = fs::read(temp.path().join("project.json")).unwrap();
        let project = store.load().unwrap();
        let timing = project.timing(&store).unwrap();
        assert!(difficulty_links::preview(&project, track, &timing).is_err());
        assert!(
            store
                .apply(Request {
                    expected_revision: project.revision,
                    actor: actor(),
                    action: Action::EnableDifficultyLink {
                        track: track.into(),
                        enabled: true
                    }
                })
                .is_err()
        );
        assert_eq!(fs::read(temp.path().join("project.json")).unwrap(), before);
    }
    let mut forged = store.load().unwrap();
    forged.content.tracks.insert(
        "EasySingle".into(),
        vec![EditorNote {
            id: "unrelated-new-note".into(),
            note: n(0, 0, 0),
        }],
    );
    let chart =
        Chart::parse(fs::read_to_string(temp.path().join("source/notes.chart")).unwrap()).unwrap();
    assert!(chart_engine::editor_validation::validate(&forged, &chart).is_err());
    assert!(forged.chart_text(&store).is_err());
    for item in store.load().unwrap().content.tracks["ExpertSingle"].clone() {
        apply(&store, Action::DeleteNote { note_id: item.id });
    }
    let before = fs::read(temp.path().join("project.json")).unwrap();
    let project = store.load().unwrap();
    let timing = project.timing(&store).unwrap();
    assert!(difficulty_links::preview(&project, "EasySingle", &timing).is_err());
    assert!(
        store
            .apply(Request {
                expected_revision: project.revision,
                actor: actor(),
                action: Action::EnableDifficultyLink {
                    track: "EasySingle".into(),
                    enabled: true
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(temp.path().join("project.json")).unwrap(), before);
    assert!(!project.content.tracks.contains_key("EasySingle"));
}

#[test]
fn empty_new_reduction_is_not_exported_as_a_dead_section_and_regrows_from_expert() {
    let temp = tempfile::tempdir().unwrap();
    let store = expert_only_fixture(temp.path(), "\r\n", false);
    link(&store, "EasySingle");
    for item in store.load().unwrap().content.tracks["ExpertSingle"].clone() {
        apply(&store, Action::DeleteNote { note_id: item.id });
    }
    let empty = store.load().unwrap();
    assert!(empty.content.tracks["EasySingle"].is_empty());
    assert!(!empty.chart_text(&store).unwrap().contains("[EasySingle]"));
    let output = temp.path().join("empty-export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    assert!(
        !fs::read_to_string(output.join("notes.chart"))
            .unwrap()
            .contains("[EasySingle]")
    );
    apply(
        &store,
        Action::AddNote {
            track: "ExpertSingle".into(),
            tick: 1920,
            lane: 4,
            sustain: 0,
            cymbal: false,
        },
    );
    let regrown = store.load().unwrap();
    assert!(!regrown.content.tracks["EasySingle"].is_empty());
    assert!(regrown.chart_text(&store).unwrap().contains("[EasySingle]"));
}

#[test]
fn creating_missing_lower_never_inherits_or_invalidates_unchanged_expert_approval() {
    let temp = tempfile::tempdir().unwrap();
    let store = expert_only_fixture(temp.path(), "\n", true);
    let mut project = store.load().unwrap();
    project.review = ReviewMap::build(
        project.duration,
        Policy::default(),
        project.review.input_sha256.clone(),
        None,
        BTreeSet::from([Reason::CoarseGridDeviation]),
        vec![Issue::at(
            0.6,
            "ExpertSingle",
            Reason::CoarseGridDeviation,
            &[],
        )],
    )
    .unwrap();
    let region = project.review.regions[0].id.clone();
    fs::write(
        temp.path().join("project.json"),
        serde_json::to_vec_pretty(&project).unwrap(),
    )
    .unwrap();
    apply(
        &store,
        Action::Decide {
            region_id: region.clone(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "Reviewed only this immutable Expert arrangement".into(),
        },
    );
    link(&store, "EasySingle");
    let project = store.load().unwrap();
    let timing = project.timing(&store).unwrap();
    assert_eq!(
        project
            .decision(&region, "ExpertSingle", &timing)
            .unwrap()
            .status,
        DecisionStatus::Reviewed
    );
    assert!(project.decision(&region, "EasySingle", &timing).is_none());
    assert_eq!(project.content.decisions.len(), 1);
    assert!(project.review.generated_tracks.is_none());
    assert_eq!(
        project.review.regions[0].tracks,
        BTreeSet::from(["ExpertSingle".into()])
    );
}
