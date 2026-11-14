#[path = "support/assistant_project.rs"]
mod support;
use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Request, Store},
    text_events::{self, Edit, Event, Scope},
};
use sha2::{Digest, Sha256};
use std::fs;

fn apply(store: &Store, action: Action) -> anyhow::Result<()> {
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
    apply(store, Action::EditTextEvent { edit })
}
fn text(store: &Store) -> String {
    store.load().unwrap().chart_text(store).unwrap()
}
fn local(track: &str) -> Scope {
    Scope::Local {
        track: track.into(),
    }
}
fn put(previous: Option<Event>, scope: Scope, tick: u64, text: &str) -> Edit {
    Edit::Put {
        previous,
        scope,
        tick,
        text: text.into(),
    }
}
fn install(store: &Store) -> String {
    let mut project = store.load().unwrap();
    let path = store.root.join("source/notes.chart");
    let source = fs::read_to_string(&path).unwrap()
        .replace("[Events]\n{\n}", "[Events]\n{\n // opaque comment\n 0 = E \"section Intro\"\n 1 = E \"music_start\"\n 1 = E \"music_start\"\n 2 = E \"quoted\" // keep foreign syntax\n 3 = E odd // keep comment\n 99999 = E \"after recording\"\n}")
        .replace("[ExpertSingle]\n{", "[ExpertSingle]\n{\n 120 = E \"local cue\"\n 200 = E solo\n 300 = E soloend")
        .replace("[ExpertDrums]\n{", "[ExpertDrums]\n{\n 0 = E \"mix 3 drums0d\"")
        + "[UnknownVenue]\n{\n 77 = E \"foreign light\"\n}\n";
    let source = source.replace('\n', "\r\n");
    fs::write(&path, &source).unwrap();
    project.source_chart_sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
    project
        .review
        .input_sha256
        .insert("chart".into(), project.source_chart_sha256.clone());
    support::save(store, &project);
    source
}
fn events(store: &Store) -> Vec<Event> {
    let source =
        Chart::parse(fs::read_to_string(store.root.join("source/notes.chart")).unwrap()).unwrap();
    text_events::current(&store.load().unwrap(), &source)
        .unwrap()
        .events
}

#[test]
fn duplicate_imports_and_unknown_rows_keep_exact_bytes_and_dedicated_owners() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let source = install(&store);
    let imported = events(&store);
    assert_eq!(imported.len(), 4);
    assert_ne!(imported[0].id, imported[1].id);
    assert_eq!(imported[0].text, imported[1].text);
    assert_eq!(text(&store), source);
    edit(
        &store,
        put(Some(imported[0].clone()), Scope::Global, 1, "opening cue"),
    )
    .unwrap();
    let rendered = text(&store);
    assert_eq!(
        rendered,
        source.replacen(" 1 = E \"music_start\"", "  1 = E \"opening cue\"", 1)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        source
    );
}

#[test]
fn add_edit_move_remove_are_individually_undoable_across_scopes() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let source = install(&store);
    let original_tracks = store.load().unwrap().content.tracks;
    edit(&store, put(None, Scope::Global, 480, "new cue")).unwrap();
    let added = events(&store)
        .into_iter()
        .find(|e| e.text == "new cue")
        .unwrap();
    edit(
        &store,
        put(Some(added), local("ExpertDrums"), 960, "new drum cue"),
    )
    .unwrap();
    let moved = events(&store)
        .into_iter()
        .find(|e| e.text == "new drum cue")
        .unwrap();
    let rendered = Chart::parse(text(&store)).unwrap();
    let section = rendered
        .sections
        .iter()
        .find(|s| s.name == "ExpertDrums")
        .unwrap();
    assert!(
        rendered.raw[section.body_start..section.body_end].contains("960 = E \"new drum cue\"")
    );
    edit(&store, Edit::Remove { event: moved }).unwrap();
    assert!(!text(&store).contains("new drum cue"));
    for _ in 0..3 {
        apply(&store, Action::Undo).unwrap();
    }
    assert_eq!(text(&store), source);
    assert!(store.load().unwrap().content.text_events.is_none());
    for _ in 0..3 {
        apply(&store, Action::Redo).unwrap();
    }
    assert!(!text(&store).contains("new drum cue"));
    assert_eq!(store.load().unwrap().content.tracks, original_tracks);
}

#[test]
fn imported_scope_move_and_dedicated_section_edit_do_not_overwrite_each_other() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    install(&store);
    let original = events(&store)
        .into_iter()
        .find(|e| e.text == "local cue")
        .unwrap();
    edit(
        &store,
        put(Some(original), Scope::Global, 480, "global cue"),
    )
    .unwrap();
    apply(
        &store,
        Action::EditSection {
            edit: chart_engine::sections::Edit::Put {
                previous: Some(chart_engine::sections::Marker {
                    tick: 0,
                    name: "Intro".into(),
                }),
                marker: chart_engine::sections::Marker {
                    tick: 0,
                    name: "Opening".into(),
                },
            },
        },
    )
    .unwrap();
    let rendered = text(&store);
    assert!(!rendered.contains("local cue"));
    assert!(rendered.contains("section Opening"));
    assert!(rendered.contains("480 = E \"global cue\""));
    assert!(rendered.contains(" 200 = E solo\r\n"));
    assert!(rendered.contains(" 0 = E \"mix 3 drums0d\"\r\n"));
}

#[test]
fn invalid_stale_and_dedicated_edits_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    install(&store);
    for bad in [
        put(None, Scope::Global, 0, ""),
        put(None, Scope::Global, 0, "\"break"),
        put(None, Scope::Global, 0, "line\nbreak"),
        put(None, Scope::Global, 0, "section Verse"),
        put(None, Scope::Global, 0, "[coda]"),
        put(None, local("ExpertSingle"), 0, "solo"),
        put(None, local("ExpertDrums"), 0, "[mix 3 drums0]"),
        put(None, local("HardSingle"), 0, "absent difficulty"),
        put(None, Scope::Global, 100000, "outside"),
    ] {
        let before = fs::read(root.path().join("project.json")).unwrap();
        let checkpoints = fs::read_dir(root.path().join("checkpoints"))
            .unwrap()
            .count();
        assert!(edit(&store, bad).is_err());
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(root.path().join("checkpoints"))
                .unwrap()
                .count(),
            checkpoints
        );
    }
    let original = events(&store)[0].clone();
    edit(
        &store,
        put(Some(original.clone()), Scope::Global, 1, "changed"),
    )
    .unwrap();
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(edit(&store, Edit::Remove { event: original }).is_err());
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    let outside = events(&store)
        .into_iter()
        .find(|e| e.tick == 99999)
        .unwrap();
    edit(
        &store,
        put(
            Some(outside),
            Scope::Global,
            99999,
            "retained outside timing",
        ),
    )
    .unwrap();
    assert!(text(&store).contains("99999 = E \"retained outside timing\""));
}

#[test]
fn missing_global_section_is_added_and_source_binding_is_checked() {
    let raw = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n";
    let source = Chart::parse(raw.into()).unwrap();
    let mut document = text_events::imported(&source).unwrap();
    document.events.push(Event {
        id: "new:1".into(),
        scope: Scope::Global,
        tick: 0,
        text: " cue ".into(),
    });
    let rendered = text_events::render(raw.into(), &source, &Some(document.clone())).unwrap();
    assert!(rendered.starts_with(raw));
    assert!(rendered.contains("0 = E \" cue \""));
    document.source_sha256 = "0".repeat(64);
    assert!(text_events::render(raw.into(), &source, &Some(document)).is_err());
}

#[test]
fn note_proposals_refuse_to_discard_text_edits_and_old_backups_omit_the_optional_field() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    let original = store.load().unwrap();
    let json = serde_json::to_value(&original).unwrap();
    assert!(json["content"].get("text_events").is_none());
    edit(&store, put(None, Scope::Global, 0, "music_start")).unwrap();
    assert!(
        chart_engine::collaboration::Proposal::between(
            &original,
            &store.load().unwrap(),
            "notes".into(),
            "Notes".into(),
            String::new(),
            Actor {
                kind: ActorKind::Human,
                name: "You".into()
            },
        )
        .is_err()
    );
}

#[test]
fn chart_export_keeps_local_scope_and_undo_while_midi_conversion_refuses_unknown_local_text() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    edit(&store, put(None, local("ExpertSingle"), 480, "local cue")).unwrap();
    let output = root.path().join("chart-export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    assert!(
        fs::read_to_string(output.join("notes.chart"))
            .unwrap()
            .contains("480 = E \"local cue\"")
    );
    let chart = Chart::parse(text(&store)).unwrap();
    // The conversion has no representation for a per-difficulty arbitrary cue.
    // Its established refusal is preferable to silently widening its scope.
    assert!(chart_engine::game_export::chart_to_midi(&chart).is_err());
    apply(&store, Action::Undo).unwrap();
    assert!(!text(&store).contains("local cue"));
}

#[test]
fn deleting_and_recreating_an_identical_event_cannot_reuse_a_stale_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = support::fixture(root.path());
    edit(&store, put(None, Scope::Global, 480, "cue")).unwrap();
    let old = events(&store).pop().unwrap();
    // An earlier client materialized new:1 without advancing the counter.
    let mut legacy = store.load().unwrap();
    legacy.next_id = 1;
    support::save(&store, &legacy);
    edit(&store, Edit::Remove { event: old.clone() }).unwrap();
    edit(&store, put(None, Scope::Global, 480, "cue")).unwrap();
    let fresh = events(&store).pop().unwrap();
    assert_ne!(fresh.id, old.id);
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(edit(&store, Edit::Remove { event: old }).is_err());
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    apply(&store, Action::Undo).unwrap();
    apply(&store, Action::Undo).unwrap();
    assert_ne!(store.load().unwrap().next_id, 1);
    assert!(events(&store).iter().all(|event| event.id != fresh.id));
    apply(&store, Action::Undo).unwrap();
    edit(&store, put(None, Scope::Global, 480, "cue")).unwrap();
    assert_ne!(events(&store).pop().unwrap().id, fresh.id);
}
