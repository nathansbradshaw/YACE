use chart_engine::{
    timing::TempoMap,
    venue::{self, Edit, Encoding},
};
use midly::{
    Format, Header, MetaMessage as M, MidiMessage as N, Smf, Timing, TrackEvent,
    TrackEventKind as K,
};
fn timing() -> TempoMap {
    TempoMap::new(480, 0., vec![]).unwrap()
}
fn track(name: &'static [u8], mut events: Vec<(u64, K<'static>)>) -> Vec<TrackEvent<'static>> {
    events.push((0, K::Meta(M::TrackName(name))));
    events.push((4000, K::Meta(M::EndOfTrack)));
    events.sort_by_key(|e| e.0);
    let mut before = 0;
    events
        .into_iter()
        .map(|(at, kind)| {
            let delta = at - before;
            before = at;
            TrackEvent {
                delta: (delta as u32).into(),
                kind,
            }
        })
        .collect()
}
fn encode(tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
    let midi = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(480.into()),
        },
        tracks,
    };
    let mut bytes = Vec::new();
    midi.write_std(&mut bytes).unwrap();
    bytes
}
fn note(key: u8, on: bool, zero: bool) -> K<'static> {
    K::Midi {
        channel: 4.into(),
        message: if on {
            N::NoteOn {
                key: key.into(),
                vel: 89.into(),
            }
        } else if zero {
            N::NoteOn {
                key: key.into(),
                vel: 0.into(),
            }
        } else {
            N::NoteOff {
                key: key.into(),
                vel: 19.into(),
            }
        },
    }
}
fn fixture() -> Vec<u8> {
    encode(vec![
        track(
            b"TEMPO",
            vec![
                (0, K::Meta(M::Tempo(500000.into()))),
                (1920, K::Meta(M::Tempo(666667.into()))),
            ],
        ),
        track(
            b"PART GUITAR",
            vec![
                (120, note(96, true, false)),
                (360, note(96, false, false)),
                (240, K::Meta(M::Text(b"opaque guitar"))),
            ],
        ),
        track(
            b"VENUE",
            vec![
                (100, K::Meta(M::Text(b" [lighting (verse)] "))),
                (100, K::Meta(M::Text(b"[lighting (verse)]"))),
                (110, K::Meta(M::Marker(b"opaque meta marker"))),
                (111, K::Meta(M::Text(b"unknown stage command"))),
                (112, K::SysEx(b"\x01\x02\x03")),
                (113, K::Meta(M::Text(b"\xff opaque"))),
                (120, note(39, true, false)),
                (180, note(39, false, false)),
                (240, note(85, true, false)),
                (360, note(85, false, true)),
                (300, note(15, true, false)),
                (450, note(15, false, false)),
            ],
        ),
    ])
}
fn absolute(bytes: &[u8], index: usize) -> Vec<(u64, String)> {
    let midi = Smf::parse(bytes).unwrap();
    let mut tick = 0;
    midi.tracks[index]
        .iter()
        .map(|e| {
            tick += u64::from(e.delta.as_int());
            (tick, format!("{:?}", e.kind))
        })
        .collect()
}
fn put(code: &str, tick: u64, end: Option<u64>) -> Edit {
    Edit::Put {
        previous: None,
        cue: code.into(),
        tick,
        end,
    }
}
fn apply(
    source: &venue::Source,
    current: &venue::Document,
    edit: &Edit,
    n: u64,
) -> venue::Document {
    venue::edited(source, current, edit, n, n - 1, &timing(), 12.)
        .unwrap()
        .0
}
#[test]
fn venue_retains_duplicate_text_note_attributes_and_opaque_events() {
    let bytes = fixture();
    let source = venue::imported(&bytes).unwrap();
    assert_eq!(source.document.events.len(), 4);
    assert_ne!(source.document.events[0].id, source.document.events[1].id);
    assert_eq!(
        source.document.events[2]
            .attributes
            .as_ref()
            .unwrap()
            .off_velocity,
        19
    );
    assert!(
        source.document.events[3]
            .attributes
            .as_ref()
            .unwrap()
            .zero_note_on
    );
    assert_eq!(
        venue::rewrite(&bytes, &source, &source.document).unwrap(),
        bytes
    );
    let after = apply(&source, &source.document, &put("post_bloom", 480, None), 1);
    let result = venue::rewrite(&bytes, &source, &after).unwrap();
    assert_eq!(absolute(&bytes, 0), absolute(&result, 0));
    assert_eq!(absolute(&bytes, 1), absolute(&result, 1));
    let original = absolute(&bytes, 2);
    let mut actual = absolute(&result, 2);
    let added = actual.iter().position(|(t, _)| *t == 480).unwrap();
    assert_eq!(
        actual.remove(added).1,
        format!("{:?}", K::Meta(M::Text(b"[bloom.pp]")))
    );
    assert_eq!(original, actual);
    assert_eq!(
        venue::values(&venue::imported(&result).unwrap().document),
        venue::values(&after)
    );
}
#[test]
fn venue_creates_real_track_and_all_five_cue_categories() {
    let bytes = encode(vec![track(b"TEMPO", vec![]), track(b"PART GUITAR", vec![])]);
    let source = venue::imported(&bytes).unwrap();
    assert!(!source.present);
    let mut after = source.document.clone();
    for (i, code) in [
        "light_chorus",
        "post_negative",
        "camera_guitar",
        "fog_on",
        "spot_guitar",
    ]
    .iter()
    .enumerate()
    {
        after = apply(
            &source,
            &after,
            &put(
                code,
                480 * (i as u64 + 1),
                if i == 4 { Some(2880) } else { None },
            ),
            i as u64 + 1,
        )
    }
    let result = venue::rewrite(&bytes, &source, &after).unwrap();
    assert_eq!(Smf::parse(&result).unwrap().tracks.len(), 3);
    let imported = venue::imported(&result).unwrap();
    assert!(imported.present);
    assert_eq!(venue::values(&imported.document), venue::values(&after));
    assert_eq!(absolute(&result, 0), absolute(&bytes, 0));
    assert_eq!(absolute(&result, 1), absolute(&bytes, 1));
}
#[test]
fn venue_moves_resizes_removes_and_refuses_stale_or_out_of_range() {
    let bytes = fixture();
    let source = venue::imported(&bytes).unwrap();
    let e = source.document.events[2].clone();
    let edit = Edit::Put {
        previous: Some(e.clone()),
        cue: "spot_bass".into(),
        tick: 600,
        end: Some(960),
    };
    let moved = apply(&source, &source.document, &edit, 1);
    assert!(venue::edited(&source, &moved, &edit, 2, 1, &timing(), 12.).is_err());
    let result = venue::rewrite(&bytes, &source, &moved).unwrap();
    let imported = venue::imported(&result).unwrap();
    let bass = imported
        .document
        .events
        .iter()
        .find(|e| e.cue == "spot_bass")
        .unwrap();
    assert_eq!(bass.attributes, e.attributes);
    assert_eq!(bass.end, Some(960));
    let removed = apply(
        &source,
        &moved,
        &Edit::Remove {
            event: moved
                .events
                .iter()
                .find(|e| e.cue == "spot_bass")
                .unwrap()
                .clone(),
        },
        2,
    );
    assert_eq!(removed.events.len(), 3);
    assert!(
        venue::edited(
            &source,
            &removed,
            &put("light_chorus", 100_000, None),
            3,
            2,
            &timing(),
            12.
        )
        .is_err()
    );
    assert!(venue::rewrite(&bytes, &source, &removed).is_ok());
}
#[test]
fn venue_rejects_ambiguous_sources_bindings_and_note_collisions() {
    let bytes = fixture();
    let mut smf = Smf::parse(&bytes).unwrap();
    smf.tracks.push(smf.tracks[2].clone());
    let mut duplicate = Vec::new();
    smf.write_std(&mut duplicate).unwrap();
    assert!(venue::imported(&duplicate).is_err());
    let source = venue::imported(&bytes).unwrap();
    let mut wrong = source.document.clone();
    wrong.source_sha256 = "0".repeat(64);
    assert!(venue::rewrite(&bytes, &source, &wrong).is_err());
    let collision = Edit::Put {
        previous: None,
        cue: "spot_guitar".into(),
        tick: 130,
        end: Some(150),
    };
    let mut after = source.document.clone();
    let mut e = source.document.events[2].clone();
    e.id = "new:1".into();
    e.tick = 130;
    e.end = Some(150);
    after.events.push(e);
    assert!(venue::validate_shape(&after).is_err());
    assert!(
        venue::edited(&source, &source.document, &collision, 1, 0, &timing(), 12.).is_ok(),
        "New channel0 cue is separate from source channel4"
    );
    let unclosed = encode(vec![track(b"VENUE", vec![(120, note(39, true, false))])]);
    assert!(venue::imported(&unclosed).is_err());
}
#[test]
fn venue_preview_latches_point_cues_and_bounds_performers() {
    let bytes = fixture();
    let source = venue::imported(&bytes).unwrap();
    let mut doc = source.document.clone();
    for (i, (code, tick)) in [
        ("light_chorus", 480),
        ("post_sepia", 600),
        ("camera_guitar", 720),
        ("fog_on", 800),
        ("bonus", 900),
        ("fog_off", 1600),
    ]
    .iter()
    .enumerate()
    {
        doc = apply(&source, &doc, &put(code, *tick, None), i as u64 + 1)
    }
    let p = venue::preview(&doc, 130, 480);
    assert_eq!(p.lighting, "Verse");
    assert_eq!(p.performers, ["Guitar spotlight"]);
    assert!(venue::preview(&doc, 180, 480).performers.is_empty());
    let p = venue::preview(&doc, 1000, 480);
    assert_eq!(p.lighting, "Chorus");
    assert_eq!(p.post, "Sepia");
    assert_eq!(p.camera, "Directed guitar");
    assert!(p.fog && p.bonus);
    assert!(!venue::preview(&doc, 1600, 480).fog);
    assert!(!venue::preview(&doc, 1600, 480).bonus);
    assert!(
        venue::CUES
            .iter()
            .any(|c| matches!(c.encoding, Encoding::Note(87)))
    );
}
#[test]
fn venue_new_identity_is_not_reused_after_undo_or_legacy_counter() {
    let source = venue::imported(&fixture()).unwrap();
    let (first, next) = venue::edited(
        &source,
        &source.document,
        &put("fog_on", 500, None),
        0,
        7,
        &timing(),
        12.,
    )
    .unwrap();
    let added = first
        .events
        .iter()
        .find(|e| e.id.starts_with("new:"))
        .unwrap();
    assert_eq!(added.id, "new:8");
    let (second, _) = venue::edited(
        &source,
        &source.document,
        &put("fog_off", 600, None),
        0,
        9,
        &timing(),
        12.,
    )
    .unwrap();
    assert_eq!(next, 9);
    assert!(second.events.iter().any(|e| e.id == "new:10"));
}
#[test]
fn venue_touching_performer_ranges_emit_note_off_before_reentry() {
    let bytes = encode(vec![
        track(b"TEMPO", vec![]),
        track(
            b"VENUE",
            vec![
                (120, note(39, true, false)),
                (180, note(39, false, false)),
                (240, note(39, true, false)),
                (360, note(39, false, false)),
            ],
        ),
    ]);
    let source = venue::imported(&bytes).unwrap();
    let after = apply(
        &source,
        &source.document,
        &Edit::Put {
            previous: Some(source.document.events[0].clone()),
            cue: "spot_guitar".into(),
            tick: 360,
            end: Some(480),
        },
        1,
    );
    let output = venue::rewrite(&bytes, &source, &after).unwrap();
    let parsed = venue::imported(&output)
        .expect("Touching ranges must remain valid MIDI, with off before new on");
    assert_eq!(venue::values(&parsed.document), venue::values(&after));
}

use chart_engine::editor::{Action, Actor, ActorKind, Request, Store};
use std::{fs, path::Path};
fn transact(store: &Store, action: Action) -> anyhow::Result<()> {
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
fn export(store: &Store, path: &Path) -> Vec<u8> {
    transact(
        store,
        Action::Export {
            output: path.into(),
        },
    )
    .unwrap();
    fs::read(path.join("notes.mid")).unwrap()
}
#[test]
fn venue_store_exports_reopens_undoes_and_composes_with_ordinary_text() {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), fixture()).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Venue fixture\nsong_length = 10000\ncustom = retain\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let before = store.load().unwrap();
    // The first saved request representation also lives in operation
    // history. Reopening it must not depend on a newer editor's enum tags.
    let request:Request=serde_json::from_str(r#"{"expected_revision":0,"actor":{"kind":"human","name":"You"},"action":{"type":"edit_venue","edit":{"kind":"put","previous":null,"cue":"fog_on","tick":480,"end":null}}}"#).unwrap();
    store.apply(request).unwrap();
    assert_eq!(Store::new(&store.root).load().unwrap().revision, 1);
    let selected = store.venue_events(&store.load().unwrap()).unwrap().events[2].clone();
    transact(
        &store,
        Action::EditVenue {
            edit: Edit::Put {
                previous: Some(selected),
                cue: "spot_guitar".into(),
                tick: 720,
                end: Some(960),
            },
        },
    )
    .unwrap();
    transact(
        &store,
        Action::EditTextEvent {
            edit: chart_engine::text_events::Edit::Put {
                previous: None,
                scope: chart_engine::text_events::Scope::Global,
                tick: 600,
                text: "other editor cue".into(),
            },
        },
    )
    .unwrap();
    let after = store.load().unwrap();
    assert_eq!(after.content.tracks, before.content.tracks);
    assert_eq!(after.undo.len(), 3);
    assert!(
        chart_engine::collaboration::Proposal::between(
            &before,
            &after,
            "id".into(),
            "Notes".into(),
            String::new(),
            Actor {
                kind: ActorKind::Human,
                name: "You".into()
            }
        )
        .is_err()
    );
    let output = root.path().join("edited");
    let bytes = export(&store, &output);
    let state = after.content.venue_events.as_ref().unwrap();
    assert_eq!(
        venue::values(&venue::imported(&bytes).unwrap().document),
        venue::values(state)
    );
    assert_eq!(absolute(&bytes, 0), absolute(&fixture(), 0));
    assert_eq!(absolute(&bytes, 1), absolute(&fixture(), 1));
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        reopened.venue_events(&reopened.load().unwrap()).unwrap(),
        *state
    );
    let mut legacy = reopened.load().unwrap();
    legacy.content.venue_events = None;
    fs::write(
        reopened.root.join("project.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    assert_eq!(export(&reopened, &root.path().join("inherited")), bytes);
    let cue = reopened
        .venue_events(&reopened.load().unwrap())
        .unwrap()
        .events
        .into_iter()
        .find(|e| e.cue == "fog_on")
        .unwrap();
    let id = cue.id.clone();
    transact(
        &reopened,
        Action::EditVenue {
            edit: Edit::Put {
                previous: Some(cue),
                cue: "fog_off".into(),
                tick: 1200,
                end: None,
            },
        },
    )
    .unwrap();
    assert!(
        reopened
            .venue_events(&reopened.load().unwrap())
            .unwrap()
            .events
            .iter()
            .any(|e| e.id == id && e.cue == "fog_off")
    );
    let next = export(&reopened, &root.path().join("reopened-edit"));
    assert_ne!(next, bytes);
    transact(&reopened, Action::Undo).unwrap();
    assert_eq!(export(&reopened, &root.path().join("reopened-undo")), bytes);
    for _ in 0..3 {
        transact(&store, Action::Undo).unwrap();
    }
    assert!(store.load().unwrap().content.venue_events.is_none());
    assert_eq!(export(&store, &root.path().join("original")), fixture());
    for _ in 0..3 {
        transact(&store, Action::Redo).unwrap();
    }
    assert_eq!(export(&store, &root.path().join("redo")), bytes);
}
#[test]
fn venue_refusals_preserve_revision_and_source_for_bad_drafts_and_chart_conversion() {
    let root = tempfile::tempdir().unwrap();
    let song = root.path().join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.mid"), fixture()).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Venue refusal\nsong_length = 10000\n",
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("project")).unwrap();
    let before = store.load().unwrap();
    assert!(
        transact(
            &store,
            Action::EditVenue {
                edit: put("spot_keys", 480, None)
            }
        )
        .is_err()
    );
    assert_eq!(store.load().unwrap().revision, before.revision);
    assert!(
        transact(
            &store,
            Action::EditVenue {
                edit: put("fog_on", 100_000, None)
            }
        )
        .is_err()
    );
    assert_eq!(store.load().unwrap().revision, before.revision);
    transact(
        &store,
        Action::EditVenue {
            edit: put("fog_on", 480, None),
        },
    )
    .unwrap();
    let project = store.load().unwrap();
    let staging = root.path().join("chart-export");
    fs::create_dir(&staging).unwrap();
    let options = chart_engine::export_options::Options {
        format: chart_engine::export_options::Format::Chart,
        ..Default::default()
    };
    assert!(
        store
            .write_export_with_options(
                &project,
                &staging,
                &Actor {
                    kind: ActorKind::Human,
                    name: "You".into()
                },
                &options
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    assert_eq!(
        fs::read(
            store
                .root
                .join("source/.yace/originals/game-midi/notes.mid")
        )
        .unwrap(),
        fixture()
    );
}
