use super::*;
use crate::song_files::game_midi::{self, import_bre, rewrite_bre};
use midly::{
    Format, Header, MetaMessage as M, MidiMessage as N, Smf, Timing, TrackEvent,
    TrackEventKind as K,
    num::{u4, u7, u15, u28},
};
fn note(on: bool, key: u8, channel: u8, velocity: u8) -> K<'static> {
    K::Midi {
        channel: u4::new(channel),
        message: if on {
            N::NoteOn {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        } else {
            N::NoteOff {
                key: u7::new(key),
                vel: u7::new(velocity),
            }
        },
    }
}
fn track(name: &'static [u8], mut events: Vec<(u64, K<'static>)>) -> Vec<TrackEvent<'static>> {
    events.push((0, K::Meta(M::TrackName(name))));
    events.push((8000, K::Meta(M::EndOfTrack)));
    events.sort_by_key(|e| (e.0, matches!(e.1, K::Meta(M::EndOfTrack))));
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
fn encode(format: Format, tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
    let mut bytes = vec![];
    Smf {
        header: Header::new(format, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut bytes)
    .unwrap();
    bytes
}
fn fixture() -> Vec<u8> {
    let mut events = vec![
        (0, note(true, 96, 2, 89)),
        (120, note(false, 96, 2, 17)),
        (2208, K::Meta(M::Text(b"Original author text"))),
    ];
    for key in 120..=124 {
        events.push((1920, note(true, key, key - 120, 70 + key - 120)));
        events.push((2400, note(false, key, key - 120, 10 + key - 120)));
    }
    encode(
        Format::Parallel,
        vec![
            track(
                b"EVENTS",
                vec![
                    (960, K::Meta(M::Text(b"[coda]"))),
                    (0, K::Meta(M::Text(b"[section Intro]"))),
                ],
            ),
            track(b"PART GUITAR", events),
            track(
                b"PART DRUMS",
                vec![
                    (1920, note(true, 120, 4, 88)),
                    (2400, note(false, 120, 4, 21)),
                ],
            ),
            track(b"VENUE", vec![(42, K::Meta(M::Text(b"Lighting sentinel")))]),
        ],
    )
}
#[test]
fn bre_midi_move_clear_coda_and_raw_owners_preserve_attributes() {
    let bytes = fixture();
    let before = import_bre(&bytes).unwrap();
    let hash = crate::drum_mix::hash(&bytes);
    assert_eq!(rewrite_bre(&bytes, &hash, &before, &before).unwrap(), bytes);
    let old = before.owners["PART GUITAR"][0].clone();
    let after = edited(
        &before,
        &Edit::Put {
            owner: "PART GUITAR".into(),
            previous: Some(old.clone()),
            range: Range {
                start: 2880,
                end: 3360,
            },
        },
    )
    .unwrap();
    let out = rewrite_bre(&bytes, &hash, &before, &after).unwrap();
    let actual = import_bre(&out).unwrap();
    assert_eq!(actual.owners, after.owners);
    assert_eq!(actual.owners["PART GUITAR"][0].lanes, old.lanes);
    let (_, a) = game_midi::midi_chunks(&bytes, 4).unwrap();
    let (_, b) = game_midi::midi_chunks(&out, 4).unwrap();
    assert_eq!(a[0], b[0]);
    assert_eq!(a[2..], b[2..]);
    let retained = |bytes: &[u8]| {
        let smf = Smf::parse(bytes).unwrap();
        game_midi::midi_events(&smf.tracks[1]).into_iter().filter(|(_,kind)|
            !matches!(kind,K::Midi{message:N::NoteOn{key,..}|N::NoteOff{key,..},..}if(120..=124).contains(&key.as_int()))
        ).map(|(tick,kind)|(tick,format!("{kind:?}"))).collect::<Vec<_>>()
    };
    assert_eq!(retained(&bytes), retained(&out));
    let clear = edited(
        &after,
        &Edit::Remove {
            owner: "PART GUITAR".into(),
            phrase: after.owners["PART GUITAR"][0].clone(),
        },
    )
    .unwrap();
    let out2 = rewrite_bre(&out, &crate::drum_mix::hash(&out), &after, &clear).unwrap();
    assert!(import_bre(&out2).unwrap().owners.is_empty());
    let coda = edited(
        &clear,
        &Edit::SetCoda {
            previous: clear.coda.clone(),
            tick: Some(480),
        },
    )
    .unwrap();
    let moved = rewrite_bre(&out2, &crate::drum_mix::hash(&out2), &clear, &coda).unwrap();
    assert_eq!(import_bre(&moved).unwrap().coda.unwrap().tick, 480);
}
#[test]
fn bre_first_owner_and_coda_keep_single_track_source_raw() {
    let bytes = encode(
        Format::SingleTrack,
        vec![track(
            b"PART GUITAR",
            vec![(0, note(true, 96, 2, 89)), (120, note(false, 96, 2, 17))],
        )],
    );
    let before = import_bre(&bytes).unwrap();
    let put = Edit::Put {
        owner: "PART BASS".into(),
        previous: None,
        range: Range {
            start: 1920,
            end: 2400,
        },
    };
    assert!(edited(&before, &put).is_err());
    let coda = edited(
        &before,
        &Edit::SetCoda {
            previous: None,
            tick: Some(1920),
        },
    )
    .unwrap();
    let after = edited(&coda, &put).unwrap();
    let out = rewrite_bre(&bytes, &crate::drum_mix::hash(&bytes), &before, &after).unwrap();
    let (_, a) = game_midi::midi_chunks(&bytes, 1).unwrap();
    let (_, b) = game_midi::midi_chunks(&out, 3).unwrap();
    assert_eq!(a[0], b[0]);
    assert_eq!(Smf::parse(&out).unwrap().header.format, Format::Parallel);
    assert_eq!(import_bre(&out).unwrap().owners, after.owners);
}
#[test]
fn bre_refuses_stale_source_overlap_and_incomplete_lanes() {
    let bytes = fixture();
    let state = import_bre(&bytes).unwrap();
    assert!(
        edited(
            &state,
            &Edit::Put {
                owner: "PART GUITAR".into(),
                previous: None,
                range: Range {
                    start: 2000,
                    end: 2600
                }
            }
        )
        .is_err()
    );
    assert!(
        edited(
            &state,
            &Edit::Put {
                owner: "PART GUITAR".into(),
                previous: None,
                range: Range { start: 0, end: 240 }
            }
        )
        .is_err()
    );
    assert!(
        edited(
            &state,
            &Edit::SetCoda {
                previous: None,
                tick: Some(0)
            }
        )
        .is_err()
    );
    assert!(rewrite_bre(&bytes, &"0".repeat(64), &state, &state).is_err());
    let mut foreign = state.clone();
    foreign.source_sha256 = None;
    assert!(rewrite_bre(&bytes, &crate::drum_mix::hash(&bytes), &state, &foreign).is_err());
    let mut stale = state.owners["PART GUITAR"][0].clone();
    stale.range.end += 1;
    assert!(
        edited(
            &state,
            &Edit::Remove {
                owner: "PART GUITAR".into(),
                phrase: stale
            }
        )
        .is_err()
    );
    let bad = encode(
        Format::Parallel,
        vec![track(
            b"PART BASS",
            vec![
                (1920, note(true, 120, 0, 100)),
                (2400, note(false, 120, 0, 0)),
            ],
        )],
    );
    assert!(import_bre(&bad).is_err());
    let duplicate = encode(
        Format::Parallel,
        vec![track(b"PART BASS", vec![]), track(b"PART BASS", vec![])],
    );
    assert!(import_bre(&duplicate).is_err());
    let codas = encode(
        Format::Parallel,
        vec![track(
            b"EVENTS",
            vec![
                (960, K::Meta(M::Text(b"[coda]"))),
                (1920, K::Meta(M::Text(b"coda"))),
            ],
        )],
    );
    assert!(import_bre(&codas).is_err());
}
#[test]
fn bre_chart_coda_retains_other_rows_and_rejects_guitar_encoding() {
    let source = "[Song]\r\n{\r\nResolution = 480\r\n}\r\n[SyncTrack]\r\n{\r\n0 = B 120000\r\n}\r\n[Events]\r\n{\r\n0 = E \"section Intro\"\r\n}\r\n[ExpertDrums]\r\n{\r\n1920 = S 64 480\r\n}\r\n";
    let chart = Chart::parse(source.into()).unwrap();
    assert!(chart_coda(&chart).unwrap().is_none());
    let state = edited(
        &State::empty(480),
        &Edit::SetCoda {
            previous: None,
            tick: Some(1920),
        },
    )
    .unwrap();
    let output = render_chart(source.into(), Some(&state)).unwrap();
    assert!(output.contains("0 = E \"section Intro\"\r\n"));
    assert!(output.contains("1920 = S 64 480\r\n"));
    assert_eq!(
        chart_coda(&Chart::parse(output.clone()).unwrap()).unwrap(),
        state.coda
    );
    assert_eq!(render_chart(output.clone(), Some(&state)).unwrap(), output);
    let guitar = edited(
        &state,
        &Edit::Put {
            owner: "PART GUITAR".into(),
            previous: None,
            range: Range {
                start: 1920,
                end: 2400,
            },
        },
    )
    .unwrap();
    assert!(render_chart(output, Some(&guitar)).is_err());
}
