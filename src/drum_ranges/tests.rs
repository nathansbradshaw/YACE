use super::*;
use crate::editor::{Action, Actor, ActorKind, Request, Store};
use std::fs;
fn range(start: u64, end: u64, kind: Kind) -> Range {
    Range { start, end, kind }
}
const BODY: &str = "0 = N 0 120\n960 = N 2 120\n0 = S 64 480\n720 = S 65 240\n1200 = S 66 240\n";
fn chart(body: &str) -> Chart {
    Chart::parse(format!("[Song]\n{{\nResolution = 480\n}}\n[SyncTrack]\n{{\n0 = B 120000\n}}\n[ExpertDrums]\n{{\n{body}}}\n[HardDrums]\n{{\n240 = N 1 120\n}}\n")).unwrap()
}
fn project(midi: bool, opaque: bool) -> (tempfile::TempDir, Store) {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    let source = chart(BODY);
    if midi {
        let mut bytes = crate::game_export::song(&source, None).unwrap().0;
        if opaque {
            use midly::{MidiMessage as N, TrackEventKind as K};
            let mut smf = midly::Smf::parse(&bytes).unwrap();
            for track in &mut smf.tracks {
                let mut tick = 0;
                let mut last = 0;
                let mut kept = Vec::new();
                for event in track.iter() {
                    tick += event.delta.as_int();
                    if matches!(event.kind,K::Midi {message:N::NoteOn {key,..}|N::NoteOff {key,..},..} if (121..=124).contains(&key.as_int()))
                    {
                        continue;
                    }
                    let mut event = *event;
                    event.delta = midly::num::u28::new(tick - last);
                    last = tick;
                    kept.push(event);
                }
                *track = kept;
            }
            let mut changed = Vec::new();
            smf.write_std(&mut changed).unwrap();
            bytes = changed;
        }
        fs::write(song.join("notes.mid"), bytes).unwrap();
    } else {
        fs::write(song.join("notes.chart"), source.raw).unwrap();
    }
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Drum range regression\nsong_length = 10000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    (root, store)
}
fn apply(store: &Store, edit: Edit) -> Result<()> {
    action(store, Action::EditDrumRange { edit })
}
fn action(store: &Store, action: Action) -> Result<()> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Drum range regression".into(),
        },
        action,
    })?;
    Ok(())
}
fn current_ranges(store: &Store, track: &str) -> Vec<Range> {
    let doc = store.load().unwrap();
    current(
        &doc,
        &Chart::parse(doc.chart_text(store).unwrap()).unwrap(),
        track,
    )
    .unwrap()
}
fn physical(bytes: &[u8], wanted: u8) -> Vec<(u64, u64)> {
    use midly::{MidiMessage as N, TrackEventKind as K};
    let smf = midly::Smf::parse(bytes).unwrap();
    let mut out = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0;
        let mut active = BTreeMap::new();
        for e in track {
            tick += u64::from(e.delta.as_int());
            if let K::Midi { channel, message } = e.kind {
                match message {
                    N::NoteOn { key, vel } if key.as_int() == wanted && vel.as_int() > 0 => {
                        assert!(active.insert(channel.as_int(), tick).is_none());
                    }
                    N::NoteOff { key, .. } | N::NoteOn { key, .. } if key.as_int() == wanted => {
                        out.push((active.remove(&channel.as_int()).unwrap(), tick));
                    }
                    _ => (),
                }
            }
        }
        assert!(active.is_empty());
    }
    out.sort();
    out
}
#[test]
fn drum_ranges_render_only_managed_rows_and_preserve_independent_kind_overlaps() {
    let raw=chart("0 = S 64 480\n240 = S 65 480\n300 = S 66 120\n0 = S 2 960\n12 = E solo\n600 = E soloend\n13 = S 99 20\n0 = N 0 120\n").raw.replace('\n',"\r\n");
    let source = Chart::parse(raw.clone()).unwrap();
    assert_eq!(imported(&source, "ExpertDrums").unwrap().len(), 3);
    assert_eq!(render(raw.clone(), &Tracks::new()).unwrap(), raw);
    let cleared = render(raw, &Tracks::from([("ExpertDrums".into(), vec![])])).unwrap();
    for line in [
        "0 = S 2 960\r\n",
        "12 = E solo\r\n",
        "600 = E soloend\r\n",
        "13 = S 99 20\r\n",
        "0 = N 0 120\r\n",
    ] {
        assert!(cleared.contains(line));
    }
    assert!(
        imported(&Chart::parse(cleared).unwrap(), "ExpertDrums")
            .unwrap()
            .is_empty()
    );
}
#[test]
fn drum_ranges_malformed_source_stays_read_only_and_cannot_be_cleared_by_rendering() {
    for body in [
        "0 = S 64 0\n",
        "0 = S 65 20 extra\n",
        "18446744073709551615 = S 66 1\n",
        "0 = S 65 480\n240 = S 65 480\n",
    ] {
        let source = chart(body);
        assert!(imported(&source, "ExpertDrums").is_err());
        assert_eq!(
            render(source.raw.clone(), &Tracks::new()).unwrap(),
            source.raw
        );
        assert!(render(source.raw, &Tracks::from([("ExpertDrums".into(), vec![])])).is_err());
    }
}
#[test]
fn drum_clipboard_is_relative_bounded_resolution_checked_and_pasted_atomically() {
    let (_root, store) = project(false, false);
    let ranges = [
        range(720, 960, Kind::SingleRoll),
        range(1200, 1440, Kind::DoubleRoll),
    ];
    let text = encode_clipboard(&ranges, 480).unwrap();
    let clipboard = decode_clipboard(&text).unwrap();
    assert_eq!(clipboard.ranges[0].start, 0);
    apply(
        &store,
        Edit::Paste {
            track: "ExpertDrums".into(),
            at: 1920,
            clipboard: clipboard.clone(),
        },
    )
    .unwrap();
    let current = current_ranges(&store, "ExpertDrums");
    assert!(current.contains(&range(1920, 2160, Kind::SingleRoll)));
    assert!(current.contains(&range(2400, 2640, Kind::DoubleRoll)));
    assert!(current_ranges(&store, "HardDrums").is_empty());
    let before = fs::read(store.root.join("project.json")).unwrap();
    let mut wrong = clipboard.clone();
    wrong.resolution = 960;
    for (at, clipboard) in [
        (1920, wrong),
        (u64::MAX, clipboard.clone()),
        (720, clipboard),
    ] {
        assert!(
            apply(
                &store,
                Edit::Paste {
                    track: "ExpertDrums".into(),
                    at,
                    clipboard
                }
            )
            .is_err()
        );
    }
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    assert!(decode_clipboard("YACE notes v1\n{}").is_err());
    assert!(decode_clipboard(&"x".repeat(256 * 1024 + 1)).is_err());
    assert!(decode_clipboard(&text.replace("\"version\":1", "\"version\":2")).is_err());
    action(&store, Action::Undo).unwrap();
    assert_eq!(current_ranges(&store, "ExpertDrums").len(), 3);
    action(&store, Action::Redo).unwrap();
    assert_eq!(current_ranges(&store, "ExpertDrums").len(), 5);
}
#[test]
fn drum_range_witnesses_preserve_drafts_and_invalid_edits_do_not_save() {
    let (_root, store) = project(false, false);
    assert!(
        serde_json::to_value(&store.load().unwrap().content)
            .unwrap()
            .get("drum_ranges")
            .is_none()
    );
    let old = range(0, 480, Kind::FillActivation);
    let next = range(120, 600, Kind::FillActivation);
    apply(
        &store,
        Edit::Put {
            track: "ExpertDrums".into(),
            previous: Some(old.clone()),
            range: next.clone(),
        },
    )
    .unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        apply(
            &store,
            Edit::Put {
                track: "ExpertDrums".into(),
                previous: Some(old),
                range: range(0, 600, Kind::FillActivation)
            }
        )
        .is_err()
    );
    assert!(
        apply(
            &store,
            Edit::Put {
                track: "ExpertDrums".into(),
                previous: None,
                range: range(500, 1000, Kind::FillActivation)
            }
        )
        .is_err()
    );
    assert!(
        apply(
            &store,
            Edit::Put {
                track: "ExpertDrums".into(),
                previous: None,
                range: range(12000, 20000, Kind::DoubleRoll)
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    action(&store, Action::Undo).unwrap();
    assert!(current_ranges(&store, "ExpertDrums").contains(&range(0, 480, Kind::FillActivation)));
    action(&store, Action::Redo).unwrap();
    apply(
        &store,
        Edit::Remove {
            track: "ExpertDrums".into(),
            range: next,
        },
    )
    .unwrap();
    assert_eq!(current_ranges(&store, "ExpertDrums").len(), 2);
}
#[test]
fn drum_midi_shared_difficulties_do_not_resurrect_removed_activation_ranges() {
    let (root, store) = project(true, false);
    assert!(store.drum_activation_editable());
    assert_eq!(
        store.source_drum_ranges(&store.load().unwrap()).unwrap()["ExpertDrums"]
            .as_ref()
            .unwrap(),
        &current_ranges(&store, "ExpertDrums")
    );
    let old = range(0, 480, Kind::FillActivation);
    assert!(current_ranges(&store, "HardDrums").contains(&old));
    apply(
        &store,
        Edit::Remove {
            track: "ExpertDrums".into(),
            range: old.clone(),
        },
    )
    .unwrap();
    assert!(!current_ranges(&store, "HardDrums").contains(&old));
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    for key in 120..=124 {
        assert!(physical(&bytes, key).is_empty());
    }
    assert_eq!(physical(&bytes, 126), vec![(720, 960)]);
    assert_eq!(physical(&bytes, 127), vec![(1200, 1440)]);
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(current_ranges(&reopened, "ExpertDrums").len(), 2);
    assert_eq!(current_ranges(&reopened, "HardDrums").len(), 2);
    action(&store, Action::Undo).unwrap();
    assert!(current_ranges(&store, "HardDrums").contains(&old));
}
#[test]
fn opaque_asymmetric_midi_activation_refuses_before_save_but_rolls_preserve_it() {
    let (root, store) = project(true, true);
    assert!(!store.drum_activation_editable());
    assert!(
        store.source_drum_ranges(&store.load().unwrap()).unwrap()["ExpertDrums"]
            .as_ref()
            .unwrap()
            .iter()
            .any(|range| range.kind == Kind::SingleRoll)
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    let activation = range(0, 480, Kind::FillActivation);
    assert!(
        apply(
            &store,
            Edit::Put {
                track: "ExpertDrums".into(),
                previous: None,
                range: activation.clone()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("not fully represented")
    );
    let clip = decode_clipboard(&encode_clipboard(&[activation], 480).unwrap()).unwrap();
    assert!(
        apply(
            &store,
            Edit::Paste {
                track: "HardDrums".into(),
                at: 1920,
                clipboard: clip
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
    apply(
        &store,
        Edit::Put {
            track: "ExpertDrums".into(),
            previous: None,
            range: range(1920, 2160, Kind::SingleRoll),
        },
    )
    .unwrap();
    let output = root.path().join("export");
    action(
        &store,
        Action::Export {
            output: output.clone(),
        },
    )
    .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    assert_eq!(physical(&bytes, 120), vec![(0, 480)]);
    for key in 121..=124 {
        assert!(physical(&bytes, key).is_empty());
    }
    assert_eq!(physical(&bytes, 126), vec![(720, 960), (1920, 2160)]);
    assert_eq!(
        physical(&fs::read(root.path().join("song/notes.mid")).unwrap(), 120),
        vec![(0, 480)]
    );
}
#[test]
fn drum_range_changes_cannot_be_silently_dropped_from_note_proposals() {
    let (_root, store) = project(false, false);
    let base = store.load().unwrap();
    let mut edited = base.clone();
    edited.content.tracks.get_mut("ExpertDrums").unwrap()[0]
        .note
        .tick += 120;
    let author = Actor {
        kind: ActorKind::Ai,
        name: "Local regression".into(),
    };
    let legacy = crate::collaboration::Proposal::between(
        &base,
        &edited,
        "before".into(),
        "Notes".into(),
        "Independent note change".into(),
        author.clone(),
    )
    .unwrap();
    edited.content.drum_ranges.insert(
        "ExpertDrums".into(),
        vec![range(0, 480, Kind::FillActivation)],
    );
    assert!(
        crate::collaboration::Proposal::between(
            &base,
            &edited,
            "refuse".into(),
            "Notes".into(),
            "Independent note change".into(),
            author.clone()
        )
        .is_err()
    );
    let mut bound = base.clone();
    bound.content.drum_ranges = edited.content.drum_ranges.clone();
    let next = crate::collaboration::Proposal::between(
        &bound,
        &edited,
        "after".into(),
        "Notes".into(),
        "Independent note change".into(),
        author,
    )
    .unwrap();
    assert_ne!(legacy.fingerprint().unwrap(), next.fingerprint().unwrap());
}
