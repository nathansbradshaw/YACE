use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use midly::{
    Format, Header, MetaMessage as Meta, Timing, TrackEvent,
    num::{u15, u28},
};
fn event(on: bool, key: u8, channel: u8, velocity: u8) -> Event<'static> {
    Event::Midi {
        channel: u4::new(channel),
        message: if on {
            Note::NoteOn {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        } else {
            Note::NoteOff {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        },
    }
}
fn track(name: &'static [u8], mut events: Vec<(u64, Event<'static>)>) -> Vec<TrackEvent<'static>> {
    events.push((0, Event::Meta(Meta::TrackName(name))));
    events.push((8000, Event::Meta(Meta::EndOfTrack)));
    events.sort_by_key(|e| (e.0, matches!(e.1, Event::Meta(Meta::EndOfTrack))));
    let mut last = 0;
    events
        .into_iter()
        .map(|(t, kind)| {
            let delta = u28::new((t - last) as u32);
            last = t;
            TrackEvent { delta, kind }
        })
        .collect()
}
fn fixture() -> Vec<u8> {
    let mut events = vec![
        (0, event(true, 96, 1, 99)),
        (120, event(false, 96, 1, 17)),
        (42, Event::Meta(Meta::Text(b"Original drum author text"))),
        (480, event(true, 126, 7, 91)),
        (960, event(false, 126, 7, 24)),
        (1440, event(true, 127, 6, 92)),
        (1680, event(false, 127, 6, 25)),
    ];
    for key in 120..=124 {
        events.push((1920, event(true, key, key - 120, 80 + key - 120)));
        events.push((
            2400,
            if key == 124 {
                event(true, key, key - 120, 0)
            } else {
                event(false, key, key - 120, 10 + key - 120)
            },
        ));
    }
    let mut bytes = vec![];
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(480))),
        tracks: vec![
            track(b"PART DRUMS", events),
            track(
                b"PART GUITAR",
                vec![(0, event(true, 96, 2, 88)), (120, event(false, 96, 2, 23))],
            ),
            track(
                b"VENUE",
                vec![(42, Event::Meta(Meta::Text(b"Original venue")))],
            ),
        ],
    }
    .write_std(&mut bytes)
    .unwrap();
    bytes
}
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Drum attribute regression".into(),
    }
}
fn action(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: actor(),
        action,
    })?;
    Ok(())
}
fn edit(store: &Store, edit: Edit) -> Result<()> {
    action(store, Action::EditDrumRange { edit })
}
fn project() -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    std::fs::create_dir(&song).unwrap();
    std::fs::write(song.join("notes.mid"), fixture()).unwrap();
    std::fs::write(
        song.join("song.ini"),
        "[song]\nname = Drum metadata regression\nsong_length = 12000\npro_drums = true\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn export(store: &Store, path: &Path, selected: Option<BTreeSet<String>>) -> Vec<u8> {
    store
        .export_with_options(
            &store.load().unwrap(),
            path,
            &actor(),
            &crate::export_options::Options {
                include_audio: false,
                tracks: selected,
                ..Default::default()
            },
        )
        .unwrap();
    std::fs::read(path.join("notes.mid")).unwrap()
}
fn move_range(store: &Store, kind: Kind, start: u64, end: u64) {
    let doc = store.load().unwrap();
    let before = drum_ranges::view(
        &doc,
        &store.source_drum_ranges(&doc).unwrap(),
        "ExpertDrums",
    )
    .unwrap()
    .into_iter()
    .find(|r| r.kind == kind)
    .unwrap();
    edit(
        store,
        Edit::Put {
            track: "ExpertDrums".into(),
            previous: Some(before),
            range: Range { start, end, kind },
        },
    )
    .unwrap();
}
#[test]
fn drum_attributes_move_all_marker_kinds_history_reopen_and_clear_preserve_source_attrs() {
    let (root, store) = project();
    let original = fixture();
    let before = groups(&original).unwrap();
    assert_eq!(
        export(&store, &root.path().join("unchanged"), None),
        original
    );
    for (kind, start, end) in [
        (Kind::FillActivation, 2880, 3360),
        (Kind::SingleRoll, 720, 1080),
        (Kind::DoubleRoll, 3840, 4080),
    ] {
        move_range(&store, kind, start, end);
    }
    let output = root.path().join("export");
    let bytes = export(&store, &output, None);
    let after = groups(&bytes).unwrap();
    for old in &before {
        assert_eq!(
            after
                .iter()
                .find(|g| g.range.kind == old.range.kind)
                .unwrap()
                .attributes,
            old.attributes
        );
    }
    let unchanged = |bytes: &[u8]| {
        let smf = Smf::parse(bytes).unwrap();
        merge::absolute(&smf.tracks[0]).into_iter().filter(|(_,e)|!matches!(e,Event::Midi{message:Note::NoteOn{key,..}|Note::NoteOff{key,..},..}if(120..=124).contains(&key.as_int())||matches!(key.as_int(),126|127))).map(|(t,e)|(t,format!("{e:?}"))).collect::<Vec<_>>()
    };
    assert_eq!(unchanged(&original), unchanged(&bytes));
    let (_, a) = merge::chunks(&original, 3).unwrap();
    let (_, b) = merge::chunks(&bytes, 3).unwrap();
    assert_eq!(a[1..], b[1..]);
    action(&store, Action::Undo).unwrap();
    let undo = export(&store, &root.path().join("undo"), None);
    assert_eq!(
        groups(&undo)
            .unwrap()
            .iter()
            .find(|g| g.range.kind == Kind::DoubleRoll)
            .unwrap()
            .range
            .start,
        1440
    );
    action(&store, Action::Redo).unwrap();
    assert_eq!(export(&store, &root.path().join("redo"), None), bytes);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(
        reopened
            .load()
            .unwrap()
            .content
            .midi_drum_attributes
            .is_some()
    );
    assert_eq!(export(&reopened, &root.path().join("again"), None), bytes);
    move_range(&reopened, Kind::FillActivation, 4320, 4800);
    let moved = export(&reopened, &root.path().join("moved-again"), None);
    assert_eq!(
        groups(&moved)
            .unwrap()
            .iter()
            .find(|g| g.range.kind == Kind::FillActivation)
            .unwrap()
            .attributes,
        before
            .iter()
            .find(|g| g.range.kind == Kind::FillActivation)
            .unwrap()
            .attributes
    );
    for group in groups(&moved).unwrap() {
        edit(
            &reopened,
            Edit::Remove {
                track: "ExpertDrums".into(),
                range: group.range,
            },
        )
        .unwrap();
    }
    let clear = root.path().join("clear");
    let cleared = export(&reopened, &clear, None);
    assert!(groups(&cleared).unwrap().is_empty());
    let reopened = Store::create(&clear, None, &root.path().join("clear-reopened")).unwrap();
    assert_eq!(
        export(&reopened, &root.path().join("clear-again"), None),
        cleared
    );
    edit(
        &reopened,
        Edit::Put {
            track: "ExpertDrums".into(),
            previous: None,
            range: Range {
                kind: Kind::SingleRoll,
                start: 6000,
                end: 6240,
            },
        },
    )
    .unwrap();
    let fresh = groups(&export(&reopened, &root.path().join("fresh"), None)).unwrap();
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].attributes[0].is_default());
}
#[test]
fn drum_attributes_scoped_omission_reauthoring_and_changed_source_guard() {
    let (root, store) = project();
    move_range(&store, Kind::FillActivation, 2880, 3360);
    let output = root.path().join("omit");
    export(&store, &output, Some(["ExpertSingle".into()].into()));
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(
        reopened
            .load()
            .unwrap()
            .content
            .midi_drum_attributes
            .as_ref()
            .unwrap()
            .groups
            .is_empty()
    );
    action(
        &reopened,
        Action::CreateTrack {
            track: "ExpertDrums".into(),
        },
    )
    .unwrap();
    edit(
        &reopened,
        Edit::Put {
            track: "ExpertDrums".into(),
            previous: None,
            range: Range {
                kind: Kind::SingleRoll,
                start: 6000,
                end: 6240,
            },
        },
    )
    .unwrap();
    let bytes = export(&reopened, &root.path().join("fresh"), None);
    let current = groups(&bytes).unwrap();
    assert_eq!(current.len(), 1);
    assert!(current[0].attributes[0].is_default());
    let mut state = store.load().unwrap().content.midi_drum_attributes.unwrap();
    state.source_sha256 = "0".repeat(64);
    assert!(
        super::export(
            &store.root.join("source"),
            Some(&state),
            &bytes,
            &["ExpertDrums".into()].into()
        )
        .is_err()
    );
}
