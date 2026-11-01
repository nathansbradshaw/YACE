use chart_engine::{
    chart_library::discover,
    editor::{Action, Actor, ActorKind, Request, Store},
    models,
};
use std::fs;

#[test]
fn chart_library_switching_reopens_saved_edits_and_history_without_changing_originals() {
    let temp = tempfile::tempdir().unwrap();
    let chart = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n}\n";
    for name in ["artist-a/a", "artist-b/b"] {
        let song = temp.path().join(name);
        fs::create_dir_all(&song).unwrap();
        fs::write(song.join("notes.chart"), chart).unwrap();
        models::write_audio(&song.join("song.wav"), &vec![0.; 88200], &vec![0.; 88200]).unwrap();
    }
    let library = discover(temp.path()).unwrap();
    let first = library.open(&library.entries[0]).unwrap();
    let store = Store::new(&first);
    let doc = store.load().unwrap();
    store
        .apply(Request {
            expected_revision: doc.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Library test".into(),
            },
            action: Action::AddNote {
                track: "ExpertSingle".into(),
                tick: 960,
                lane: 1,
                sustain: 0,
                cymbal: false,
            },
        })
        .unwrap();
    let edited = store.load().unwrap();
    let direct_song = discover(&library.entries[0].path).unwrap();
    assert_eq!(direct_song.open(&direct_song.entries[0]).unwrap(), first);
    let child = library.entries[0].path.join("references/notes");
    fs::create_dir_all(&child).unwrap();
    let from_child = discover(&child).unwrap();
    assert_eq!(from_child.open(&from_child.entries[0]).unwrap(), first);
    let second = library.open(&library.entries[1]).unwrap();
    assert_ne!(first, second);
    let reopened = discover(temp.path()).unwrap();
    assert_eq!(
        reopened.entries.len(),
        2,
        "Working copies must not appear as duplicate charts"
    );
    assert_eq!(reopened.open(&reopened.entries[0]).unwrap(), first);
    let restored = Store::new(&first).load().unwrap();
    assert_eq!(restored.revision, edited.revision);
    assert_eq!(restored.content.tracks, edited.content.tracks);
    assert_eq!(restored.undo, edited.undo);
    assert_eq!(
        fs::read_to_string(library.entries[0].path.join("notes.chart")).unwrap(),
        chart
    );
    // Relative identities retain the editing copy when the whole collection moves.
    let moved = temp.path().with_extension("moved-library");
    fs::rename(temp.path(), &moved).unwrap();
    let moved_library = discover(&moved).unwrap();
    let moved_project = moved_library.open(&moved_library.entries[0]).unwrap();
    assert_eq!(
        Store::new(&moved_project).load().unwrap().revision,
        edited.revision
    );
    fs::remove_dir_all(moved).unwrap();
}
