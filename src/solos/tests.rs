use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use std::fs;
fn range(start: u64, end: u64) -> Phrase {
    Phrase { start, end }
}
fn chart(body: &str) -> Chart {
    Chart::parse(format!("[Song]\n{{\nResolution = 480\n}}\n[SyncTrack]\n{{\n0 = B 120000\n}}\n[ExpertSingle]\n{{\n{body}\n}}\n")).unwrap()
}
fn project(midi: bool) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    let raw = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n120 = E solo\n720 = E soloend\n}\n[HardSingle]\n{\n480 = N 1 0\n}\n[ExpertProKeys]\n{\n240 = N 60 120\n}\n[EasyProKeys]\n{\n240 = N 58 120\n}\n";
    if midi {
        fs::write(
            song.join("notes.mid"),
            crate::game_export::song(&Chart::parse(raw.into()).unwrap(), None)
                .unwrap()
                .0,
        )
        .unwrap();
    } else {
        fs::write(song.join("notes.chart"), raw).unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Independent solo regression\nsong_length = 10000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn apply(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Solo regression".into(),
        },
        action,
    })?;
    Ok(())
}
fn put(store: &Store, track: &str, previous: Option<Phrase>, phrase: Phrase) -> Result<()> {
    apply(
        store,
        Action::EditSolo {
            edit: Edit::Put {
                track: track.into(),
                previous,
                phrase,
            },
        },
    )
}
fn current_ranges(store: &Store, track: &str) -> Vec<Phrase> {
    let doc = store.load().unwrap();
    current(
        &doc,
        &Chart::parse(doc.chart_text(store).unwrap()).unwrap(),
        track,
    )
    .unwrap()
}
#[test]
fn solo_render_retains_non_solo_text_and_other_tracks_with_crlf_and_adjacent_ranges() {
    let raw = chart("720 = E \"soloend\"\n120 = E \"solo\"\n720 = E solo\n960 = E soloend\n240 = N 0 120\n3 = E \"solo lighting\"\n# keep exact annotation").raw.replace('\n', "\r\n");
    let source = Chart::parse(raw.clone()).unwrap();
    assert_eq!(
        imported(&source, "ExpertSingle").unwrap(),
        vec![range(120, 720), range(720, 960)]
    );
    assert_eq!(render(raw.clone(), &Tracks::new()).unwrap(), raw);
    let output = render(
        raw.clone(),
        &Tracks::from([("ExpertSingle".into(), vec![range(180, 840)])]),
    )
    .unwrap();
    assert!(output.contains("3 = E \"solo lighting\"\r\n# keep exact annotation\r\n"));
    assert!(output.contains("240 = N 0 120\r\n"));
    assert_eq!(
        imported(&Chart::parse(output).unwrap(), "ExpertSingle").unwrap(),
        vec![range(180, 840)]
    );
    let cleared = render(raw, &Tracks::from([("ExpertSingle".into(), vec![])])).unwrap();
    assert!(
        imported(&Chart::parse(cleared.clone()).unwrap(), "ExpertSingle")
            .unwrap()
            .is_empty()
    );
    assert!(cleared.contains("solo lighting"));
}
#[test]
fn solo_malformed_source_is_retained_and_never_silently_replaced() {
    for body in [
        "0 = E solo",
        "120 = E soloend",
        "0 = E solo\n0 = E soloend",
        "0 = E solo\n1 = E solo\n2 = E soloend",
        "bad = E solo\n2 = E soloend",
    ] {
        let source = chart(body);
        assert!(imported(&source, "ExpertSingle").is_err());
        assert_eq!(
            render(source.raw.clone(), &Tracks::new()).unwrap(),
            source.raw
        );
        assert!(
            render(
                source.raw.clone(),
                &Tracks::from([("ExpertSingle".into(), vec![])])
            )
            .is_err()
        );
    }
    let mut source = chart("0 = E solo\n480 = E soloend");
    source.raw.push_str("[ExpertSingle]\n{\n}\n");
    assert!(Chart::parse(source.raw).is_err());
}
#[test]
fn solo_edits_are_witnessed_undoable_and_leave_chart_difficulties_independent() {
    let (root, store) = project(false);
    assert!(
        serde_json::to_value(&store.load().unwrap().content)
            .unwrap()
            .get("solos")
            .is_none()
    );
    put(
        &store,
        "ExpertSingle",
        Some(range(120, 720)),
        range(240, 960),
    )
    .unwrap();
    assert!(current_ranges(&store, "HardSingle").is_empty());
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        put(
            &store,
            "ExpertSingle",
            Some(range(120, 720)),
            range(300, 1000)
        )
        .is_err()
    );
    assert!(put(&store, "ExpertSingle", None, range(800, 1200)).is_err());
    assert!(put(&store, "ExpertSingle", None, range(2000, 1000000)).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(120, 720)]
    );
    apply(&store, Action::Redo).unwrap();
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(240, 960)]
    );
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        current_ranges(&reopened, "ExpertSingle"),
        vec![range(240, 960)]
    );
    assert!(current_ranges(&reopened, "HardSingle").is_empty());
    apply(
        &store,
        Action::EditSolo {
            edit: Edit::Remove {
                track: "ExpertSingle".into(),
                phrase: range(240, 960),
            },
        },
    )
    .unwrap();
    assert!(current_ranges(&store, "ExpertSingle").is_empty());
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(240, 960)]
    );
    assert_eq!(
        fs::read(root.path().join("song/notes.chart")).unwrap(),
        fs::read(store.root.join("source/notes.chart")).unwrap()
    );
}
#[test]
fn solo_midi_shared_difficulties_do_not_resurrect_removed_ranges_and_pro_keys_stay_independent() {
    let (root, store) = project(true);
    assert_eq!(current_ranges(&store, "HardSingle"), vec![range(120, 720)]);
    put(
        &store,
        "ExpertSingle",
        Some(range(120, 720)),
        range(240, 960),
    )
    .unwrap();
    assert_eq!(current_ranges(&store, "HardSingle"), vec![range(240, 960)]);
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        put(
            &store,
            "HardSingle",
            Some(range(120, 720)),
            range(300, 1000)
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    put(&store, "ExpertProKeys", None, range(0, 480)).unwrap();
    assert!(current_ranges(&store, "EasyProKeys").is_empty());
    apply(
        &store,
        Action::EditSolo {
            edit: Edit::Remove {
                track: "HardSingle".into(),
                phrase: range(240, 960),
            },
        },
    )
    .unwrap();
    assert!(current_ranges(&store, "ExpertSingle").is_empty());
    let output = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(current_ranges(&reopened, "ExpertSingle").is_empty());
    assert!(current_ranges(&reopened, "HardSingle").is_empty());
    assert_eq!(
        current_ranges(&reopened, "ExpertProKeys"),
        vec![range(0, 480)]
    );
    assert!(current_ranges(&reopened, "EasyProKeys").is_empty());
    apply(&store, Action::Undo).unwrap();
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(240, 960)]
    );
}
#[test]
fn solo_note_selection_covers_taps_and_sustains_and_refuses_stale_ids() {
    let (_root, store) = project(false);
    let doc = store.load().unwrap();
    let id = doc.content.tracks["ExpertSingle"][0].id.clone();
    let mut selection = crate::note_groups::Selection {
        track: "ExpertSingle".into(),
        ids: [id].into(),
        ..Default::default()
    };
    assert_eq!(selected(&doc, &selection, 240).unwrap(), range(240, 480));
    selection.ids.insert("stale-id".into());
    assert!(selected(&doc, &selection, 240).is_err());
}

#[test]
fn solo_midi_new_difficulties_inherit_authored_ranges_and_explicit_clears() {
    let (_root, store) = project(true);
    put(
        &store,
        "ExpertSingle",
        Some(range(120, 720)),
        range(240, 960),
    )
    .unwrap();
    apply(
        &store,
        Action::CreateTrack {
            track: "MediumSingle".into(),
        },
    )
    .unwrap();
    assert_eq!(
        current_ranges(&store, "MediumSingle"),
        vec![range(240, 960)]
    );
    apply(
        &store,
        Action::EditSolo {
            edit: Edit::Remove {
                track: "MediumSingle".into(),
                phrase: range(240, 960),
            },
        },
    )
    .unwrap();
    apply(
        &store,
        Action::CreateTrack {
            track: "EasySingle".into(),
        },
    )
    .unwrap();
    for t in ["ExpertSingle", "HardSingle", "MediumSingle", "EasySingle"] {
        assert!(
            current_ranges(&store, t).is_empty(),
            "Source solo resurrected on {t}"
        );
        assert_eq!(store.load().unwrap().content.solos.get(t), Some(&vec![]));
    }
    apply(&store, Action::Undo).unwrap();
    assert!(
        !store
            .load()
            .unwrap()
            .content
            .tracks
            .contains_key("EasySingle")
    );
    apply(&store, Action::Undo).unwrap();
    for t in ["ExpertSingle", "HardSingle", "MediumSingle"] {
        assert_eq!(current_ranges(&store, t), vec![range(240, 960)]);
    }
    apply(&store, Action::Redo).unwrap();
    apply(&store, Action::Redo).unwrap();
    assert_eq!(
        store.load().unwrap().content.solos.get("EasySingle"),
        Some(&vec![])
    );
}

#[test]
fn solo_and_drum_range_histories_preserve_each_other_after_main_integration() {
    let (_root, store) = project(true);
    apply(
        &store,
        Action::CreateTrack {
            track: "ExpertDrums".into(),
        },
    )
    .unwrap();
    apply(
        &store,
        Action::EditDrumRange {
            edit: crate::drum_ranges::Edit::Put {
                track: "ExpertDrums".into(),
                previous: None,
                range: crate::drum_ranges::Range {
                    start: 1440,
                    end: 1920,
                    kind: crate::drum_ranges::Kind::DoubleRoll,
                },
            },
        },
    )
    .unwrap();
    let drums = store.load().unwrap().content.drum_ranges;
    put(
        &store,
        "ExpertSingle",
        Some(range(120, 720)),
        range(240, 960),
    )
    .unwrap();
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.drum_ranges, drums);
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(120, 720)]
    );
    apply(&store, Action::Redo).unwrap();
    assert_eq!(
        current_ranges(&store, "ExpertSingle"),
        vec![range(240, 960)]
    );
    assert_eq!(store.load().unwrap().content.drum_ranges, drums);
    let output = store.root.parent().unwrap().join("joint-export");
    apply(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let reopened = Store::create(
        &output,
        None,
        &store.root.parent().unwrap().join("joint-reopened"),
    )
    .unwrap();
    assert_eq!(
        current_ranges(&reopened, "ExpertSingle"),
        vec![range(240, 960)]
    );
    let doc = reopened.load().unwrap();
    let source = Chart::parse(doc.chart_text(&reopened).unwrap()).unwrap();
    assert_eq!(
        crate::drum_ranges::current(&doc, &source, "ExpertDrums").unwrap(),
        drums["ExpertDrums"]
    );
}

#[test]
fn solo_edits_cannot_be_dropped_from_note_proposals_and_bind_their_fingerprint() {
    use crate::collaboration::Proposal;
    let (_root, store) = project(false);
    let base = store.load().unwrap();
    let mut edited = base.clone();
    edited.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 120;
    let author = Actor {
        kind: ActorKind::Human,
        name: "Independent review".into(),
    };
    let legacy = Proposal::between(
        &base,
        &edited,
        "before".into(),
        "Notes".into(),
        "Independent note change".into(),
        author.clone(),
    )
    .unwrap();
    edited
        .content
        .solos
        .insert("ExpertSingle".into(), vec![range(120, 960)]);
    assert!(
        Proposal::between(
            &base,
            &edited,
            "refuse".into(),
            "Notes".into(),
            "Independent note change".into(),
            author.clone()
        )
        .is_err()
    );
    let mut same_ranges = base.clone();
    same_ranges.content.solos = edited.content.solos.clone();
    let bound = Proposal::between(
        &same_ranges,
        &edited,
        "after".into(),
        "Notes".into(),
        "Independent note change".into(),
        author,
    )
    .unwrap();
    assert_ne!(legacy.fingerprint().unwrap(), bound.fingerprint().unwrap());
    assert!(legacy.star_power_sha256.is_none());
    assert!(bound.star_power_sha256.is_some());
}
