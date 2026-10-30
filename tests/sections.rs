#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    sections::{self, Edit, Marker},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use std::fs;
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "You".into(),
    }
}
fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: actor(),
        action,
    })?;
    Ok(())
}
fn marker(tick: u64, name: &str) -> Marker {
    Marker {
        tick,
        name: name.into(),
    }
}
fn edit(store: &Store, edit: Edit) -> anyhow::Result<()> {
    apply(store, Action::EditSection { edit })
}
fn add(store: &Store, tick: u64, name: &str) {
    edit(
        store,
        Edit::Put {
            previous: None,
            marker: marker(tick, name),
        },
    )
    .unwrap();
}
fn text(store: &Store) -> String {
    store.load().unwrap().chart_text(store).unwrap()
}
fn existing(store: &Store, events: &str) {
    use sha2::{Digest, Sha256};
    let mut doc = store.load().unwrap();
    let source = fs::read_to_string(store.root.join("source/notes.chart")).unwrap();
    let source = source.replace("[Events]\n{\n}", &format!("[Events]\n{{\n{events}}}"));
    fs::write(store.root.join("source/notes.chart"), &source).unwrap();
    doc.source_chart_sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
    doc.review
        .input_sha256
        .insert("chart".into(), doc.source_chart_sha256.clone());
    support::save(store, &doc);
}

#[test]
fn old_projects_import_sorted_sections_without_rewriting_source_bytes() {
    let source = "[Song]\r\n{\r\n Resolution = 480\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n}\r\n[Events]\r\n{\r\n 960 = E \"section Chorus\"\r\n 10 = E \"music_start\"\r\n 0 = E section Intro\r\n 20 = E \"sectionish untouched\"\r\n}\r\n";
    let chart = Chart::parse(source.into()).unwrap();
    assert_eq!(
        sections::imported(&chart),
        vec![marker(0, "Intro"), marker(960, "Chorus")]
    );
    assert_eq!(sections::render(source.into(), &None).unwrap(), source);
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let mut json = serde_json::to_value(store.load().unwrap()).unwrap();
    json["content"].as_object_mut().unwrap().remove("sections");
    fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&json).unwrap(),
    )
    .unwrap();
    assert!(store.load().unwrap().content.sections.is_none());
}

#[test]
fn section_edits_are_individually_undoable_and_do_not_move_notes_or_sources() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let original = store.load().unwrap();
    let source = fs::read(root.path().join("source/notes.chart")).unwrap();
    add(&store, 0, "Intro");
    add(&store, 1920, "Verse");
    edit(
        &store,
        Edit::Put {
            previous: Some(marker(1920, "Verse")),
            marker: marker(2400, "Chorus"),
        },
    )
    .unwrap();
    assert_eq!(
        store.load().unwrap().content.sections.unwrap(),
        vec![marker(0, "Intro"), marker(2400, "Chorus")]
    );
    edit(
        &store,
        Edit::Remove {
            marker: marker(0, "Intro"),
        },
    )
    .unwrap();
    assert_eq!(store.load().unwrap().undo.len(), 4);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.sections.unwrap().len(), 2);
    apply(&store, Action::Undo).unwrap();
    assert!(text(&store).contains("section Verse"));
    apply(&store, Action::Redo).unwrap();
    assert!(text(&store).contains("section Chorus"));
    assert_eq!(
        store.load().unwrap().content.tracks,
        original.content.tracks
    );
    assert_eq!(
        fs::read(root.path().join("source/notes.chart")).unwrap(),
        source
    );
    assert_eq!(
        store.load().unwrap().timing(&store).unwrap().tempos,
        original.timing(&store).unwrap().tempos
    );
}

#[test]
fn undo_restores_imported_markers_and_removing_all_is_distinct_from_legacy_none() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    existing(&store, " 0 = E \"section Intro\"\n");
    let original = text(&store);
    edit(
        &store,
        Edit::Remove {
            marker: marker(0, "Intro"),
        },
    )
    .unwrap();
    assert_eq!(store.load().unwrap().content.sections, Some(vec![]));
    assert!(!text(&store).contains("section Intro"));
    apply(&store, Action::Undo).unwrap();
    assert!(store.load().unwrap().content.sections.is_none());
    assert_eq!(text(&store), original);
    apply(&store, Action::Redo).unwrap();
    assert!(store.load().unwrap().content.sections.unwrap().is_empty());
}

#[test]
fn invalid_or_stale_edits_leave_the_project_and_checkpoints_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    add(&store, 0, "Intro");
    for new in [
        marker(0, "Collision"),
        marker(100, ""),
        marker(100, "line\nbreak"),
        marker(100, "quote\""),
        marker(100, "[bracket]"),
        marker(100, "name\\escape"),
        marker(100, " leading"),
        marker(4000, "Outside"),
    ] {
        let before = fs::read(root.path().join("project.json")).unwrap();
        let checkpoints = fs::read_dir(root.path().join("checkpoints"))
            .unwrap()
            .count();
        assert!(
            edit(
                &store,
                Edit::Put {
                    previous: None,
                    marker: new
                }
            )
            .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.path().join("checkpoints"))
                .unwrap()
                .count(),
            checkpoints
        );
    }
    edit(
        &store,
        Edit::Put {
            previous: Some(marker(0, "Intro")),
            marker: marker(0, "Opening"),
        },
    )
    .unwrap();
    assert!(
        edit(
            &store,
            Edit::Remove {
                marker: marker(0, "Intro")
            }
        )
        .is_err()
    );
    assert!(text(&store).contains("section Opening"));
}

#[test]
fn section_rendering_preserves_crlf_comments_and_unrelated_global_events() {
    let raw = "[Song]\r\n{\r\n Resolution = 480\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n 0 = TS 4\r\n}\r\n[Events]\r\n{\r\n // keep this comment\r\n 1 = E \"music_start\"\r\n 0 = E \"section Intro\"\r\n 2 = E \"sectionish not-a-marker\"\r\n 3 = E \"lighting ()\"\r\n}\r\n[ExpertSingle]\r\n{\r\n 480 = N 1 0\r\n}\r\n";
    let rendered = sections::render(
        raw.into(),
        &Some(vec![marker(0, "Entrada"), marker(960, "Coro – regreso")]),
    )
    .unwrap();
    for line in [
        " // keep this comment\r\n",
        " 1 = E \"music_start\"\r\n",
        " 2 = E \"sectionish not-a-marker\"\r\n",
        " 3 = E \"lighting ()\"\r\n",
        " 480 = N 1 0\r\n",
        " 0 = TS 4\r\n",
    ] {
        assert!(rendered.contains(line));
    }
    assert!(!rendered.contains("section Intro"));
    assert_eq!(
        sections::imported(&Chart::parse(rendered).unwrap()).len(),
        2
    );
}

#[test]
fn charts_without_an_events_section_gain_one_only_after_an_edit() {
    let raw = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n";
    assert_eq!(sections::render(raw.into(), &Some(vec![])).unwrap(), raw);
    let rendered = sections::render(raw.into(), &Some(vec![marker(0, "Intro")])).unwrap();
    assert!(rendered.starts_with(raw));
    assert_eq!(
        sections::imported(&Chart::parse(rendered).unwrap()),
        vec![marker(0, "Intro")]
    );
}

#[test]
fn section_export_retains_chart_and_midi_events_with_vocal_pitch_uncertainty() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    existing(&store, " 0 = E \"music_start\"\n 0 = E \"section Intro\"\n");
    add(&store, 1920, "Coro");
    let output = root.path().join("chart-export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let chart = Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap()).unwrap();
    assert_eq!(
        sections::imported(&chart),
        vec![marker(0, "Intro"), marker(1920, "Coro")]
    );
    apply(
        &store,
        Action::SetVocals {
            document: VocalChart {
                version: 1,
                resolution: 480,
                source: "Practice".into(),
                phrases: vec![VocalPhrase {
                    start: 480,
                    end: 720,
                    player: 1,
                }],
                notes: vec![VocalNote {
                    start: 480,
                    end: 720,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Missing,
                }],
            },
        },
    )
    .unwrap();
    let output = root.path().join("midi-export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let smf = midly::Smf::parse(&bytes).unwrap();
    let mut found = false;
    for track in &smf.tracks {
        let mut tick = 0u64;
        for event in track {
            tick += event.delta.as_int() as u64;
            if matches!(
                event.kind,
                midly::TrackEventKind::Meta(midly::MetaMessage::Text(b"[section Coro]"))
            ) {
                assert_eq!(tick, 1920);
                found = true;
            }
        }
    }
    assert!(found);
    assert_eq!(
        chart_engine::vocals::import_midi(&bytes).unwrap().notes[0].lyric,
        Syllable::Missing
    );
}

#[test]
fn note_proposals_refuse_to_drop_section_changes() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let base = store.load().unwrap();
    add(&store, 0, "Intro");
    assert!(
        chart_engine::collaboration::Proposal::between(
            &base,
            &store.load().unwrap(),
            "notes".into(),
            "Notes".into(),
            String::new(),
            actor()
        )
        .is_err()
    );
}
