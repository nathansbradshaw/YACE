use super::*;
use midly::{
    Header, Timing,
    num::{u4, u7, u15, u24, u28},
};
type Event = (u32, K<'static>);
fn note(channel: u8, key: u8, velocity: u8, on: bool) -> K<'static> {
    let key = u7::new(key);
    let vel = u7::new(velocity);
    K::Midi {
        channel: u4::new(channel),
        message: if on {
            N::NoteOn { key, vel }
        } else {
            N::NoteOff { key, vel }
        },
    }
}
fn percussion(label: &'static str) -> Vec<Event> {
    vec![
        (0, K::Meta(M::TrackName(label.as_bytes()))),
        (0, note(7, 105, 90, true)),
        (960, note(7, 105, 7, false)),
        (240, note(3, 96, 86, true)),
        (260, note(3, 96, 9, false)),
        (480, note(6, 97, 87, true)),
        (480, note(6, 97, 0, true)),
        (1920, K::Meta(M::EndOfTrack)),
    ]
}
fn singing(label: &'static str, start: u32, end: u32) -> Vec<Event> {
    vec![
        (0, K::Meta(M::TrackName(label.as_bytes()))),
        (start, note(4, 60, 88, true)),
        (end, note(4, 60, 5, false)),
        (1920, K::Meta(M::EndOfTrack)),
    ]
}
fn encode(mut tracks: Vec<Vec<Event>>) -> Vec<u8> {
    tracks.insert(
        0,
        vec![
            (0, K::Meta(M::TrackName(b"TEMPO"))),
            (0, K::Meta(M::Tempo(u24::new(500000)))),
            (1920, K::Meta(M::EndOfTrack)),
        ],
    );
    let tracks = tracks
        .into_iter()
        .map(|mut track| {
            track.sort_by_key(|event| (event.0, matches!(event.1, K::Meta(M::EndOfTrack))));
            let mut last = 0;
            track
                .into_iter()
                .map(|(tick, kind)| {
                    let delta = u28::new(tick - last);
                    last = tick;
                    TrackEvent { delta, kind }
                })
                .collect()
        })
        .collect();
    let mut output = Vec::new();
    Smf {
        header: Header::new(Format::Parallel, Timing::Metrical(u15::new(480))),
        tracks,
    }
    .write_std(&mut output)
    .unwrap();
    output
}
#[test]
fn dedicated_source_hits_keep_hidden_zero_length_channels_velocities_and_phrase_attributes() {
    let chart = import_midi(&encode(vec![percussion("PART VOCALS")]))
        .unwrap()
        .unwrap();
    let track = &chart.tracks[&Part::Lead];
    assert_eq!(chart.resolution, 480);
    assert_eq!(track.phrases.len(), 1);
    let phrase = &track.phrases[0];
    assert_eq!(
        (phrase.range.start, phrase.range.end, phrase.range.player),
        (0, 960, 1)
    );
    assert_eq!(phrase.midi.as_ref().unwrap().channel, 7);
    assert_eq!(phrase.midi.as_ref().unwrap().off_velocity, 7);
    assert_eq!(track.hits.len(), 2);
    let hit = &track.hits[0];
    assert_eq!((hit.start, hit.end, hit.key()), (240, 260, 96));
    let attrs = hit.midi.as_ref().unwrap();
    assert_eq!(
        (attrs.channel, attrs.velocity, attrs.off_velocity),
        (3, 86, 9)
    );
    let hidden = &track.hits[1];
    assert_eq!((hidden.start, hidden.end, hidden.key()), (480, 480, 97));
    let attrs = hidden.midi.as_ref().unwrap();
    assert_eq!(
        (attrs.channel, attrs.velocity, attrs.off_velocity),
        (6, 87, 0)
    );
    assert!(attrs.zero_length && attrs.off_as_note_on);
}
#[test]
fn duplicate_aliases_and_ambiguous_vocal_names_refuse_instead_of_combining_parts() {
    let duplicate = encode(vec![percussion("HARM1"), percussion("PART HARM1")]);
    assert!(
        import_midi(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("Duplicate")
    );
    let mut ambiguous = percussion("PART VOCALS");
    ambiguous.push((0, K::Meta(M::TrackName(b"HARM1"))));
    assert!(
        import_midi(&encode(vec![ambiguous]))
            .unwrap_err()
            .to_string()
            .contains("Ambiguous")
    );
}
#[test]
fn singing_and_lyrics_cannot_be_reclassified_as_a_percussion_phrase() {
    let mut mixed = percussion("PART VOCALS");
    mixed.extend([(720, note(2, 60, 87, true)), (840, note(2, 60, 9, false))]);
    assert!(
        import_midi(&encode(vec![mixed]))
            .unwrap_err()
            .to_string()
            .contains("singing")
    );
    let mut lyrics_only = percussion("PART VOCALS");
    lyrics_only.push((720, K::Meta(M::Lyric(b"Keep this word"))));
    assert!(
        import_midi(&encode(vec![lyrics_only]))
            .unwrap_err()
            .to_string()
            .contains("Lyrics")
    );
}
#[test]
fn harm1_percussion_respects_other_singers_even_when_harm1_has_no_pitched_notes() {
    let mut first = percussion("PART HARM1");
    first.extend([
        (960, note(7, 105, 90, true)),
        (1920, note(7, 105, 7, false)),
    ]);
    let valid = import_midi(&encode(vec![first.clone(), singing("HARM3", 1200, 1440)]))
        .unwrap()
        .unwrap();
    assert_eq!(valid.tracks[&Part::Harm1].phrases.len(), 1);
    let mixed = encode(vec![first.clone(), singing("PART HARM2", 600, 720)]);
    assert!(
        import_midi(&mixed)
            .unwrap_err()
            .to_string()
            .contains("singing")
    );
    let point_at_start = encode(vec![first.clone(), singing("HARM3", 0, 0)]);
    assert!(import_midi(&point_at_start).is_err());
    let point_at_end = encode(vec![first, singing("HARM3", 960, 960)]);
    import_midi(&point_at_end).unwrap().unwrap();
    for key in [96, 97] {
        let shared_hits = vec![
            (0, K::Meta(M::TrackName(b"HARM2"))),
            (600, note(4, key, 88, true)),
            (620, note(4, key, 5, false)),
            (1920, K::Meta(M::EndOfTrack)),
        ];
        let source = encode(vec![percussion("HARM1"), shared_hits]);
        assert!(
            import_midi(&source)
                .unwrap_err()
                .to_string()
                .contains("unmanaged harmony percussion")
        );
    }
}
#[test]
fn uncovered_hits_and_unterminated_or_overlapping_source_keys_refuse_without_guessing() {
    let mut uncovered = percussion("PART VOCALS");
    uncovered.iter_mut().find(|event| event.0 == 260).unwrap().0 = 1000;
    assert!(import_midi(&encode(vec![uncovered])).is_err());
    let mut unmatched = percussion("PART VOCALS");
    unmatched.retain(|event| event.0 != 260);
    assert!(
        import_midi(&encode(vec![unmatched]))
            .unwrap_err()
            .to_string()
            .contains("Unterminated")
    );
    let mut overlapping = percussion("PART VOCALS");
    overlapping.push((200, note(3, 96, 88, true)));
    assert!(
        import_midi(&encode(vec![overlapping]))
            .unwrap_err()
            .to_string()
            .contains("Overlapping")
    );
}
#[test]
fn a_singing_only_source_creates_no_percussion_document() {
    let mut lead = singing("PART VOCALS", 240, 480);
    lead.extend([(0, note(0, 105, 100, true)), (960, note(0, 105, 0, false))]);
    assert!(import_midi(&encode(vec![lead])).unwrap().is_none());
}

#[test]
fn append_round_trips_independent_parts_and_retains_all_original_chunks_exactly() {
    let original = encode(vec![
        singing("PART GUITAR", 240, 480),
        vec![
            (0, K::Meta(M::TrackName(b"VENUE"))),
            (123, K::Meta(M::Text(b"Unknown stage event stays intact"))),
            (1920, K::Meta(M::EndOfTrack)),
        ],
    ]);
    let mut chart = crate::vocal_percussion::tests::fixture();
    chart.tracks.get_mut(&Part::Lead).unwrap().hits.push(Hit {
        start: 480,
        end: 480,
        displayed: false,
        midi: Some(NoteAttributes {
            channel: 6,
            velocity: 87,
            off_velocity: 0,
            off_as_note_on: true,
            zero_length: true,
            talkie_pitch: None,
        }),
    });
    let bytes = append_midi(&original, &chart).unwrap();
    let imported = import_midi(&bytes).unwrap().unwrap();
    for part in [Part::Lead, Part::Harm1] {
        assert_eq!(imported.tracks[&part].hits, chart.tracks[&part].hits);
        assert_eq!(imported.tracks[&part].phrases, chart.tracks[&part].phrases);
    }
    let before = Smf::parse(&original).unwrap();
    let after = Smf::parse(&bytes).unwrap();
    let before_chunks = midi_chunks(&original, before.tracks.len()).unwrap().1;
    let after_chunks = midi_chunks(&bytes, after.tracks.len()).unwrap().1;
    assert_eq!(after.tracks.len(), before.tracks.len() + 2);
    assert_eq!(before_chunks, after_chunks[..before.tracks.len()]);
}
#[test]
fn append_refuses_existing_vocal_owners_shared_harmony_sources_and_clock_changes() {
    let chart = crate::vocal_percussion::tests::fixture();
    for source in [
        vec![percussion("PART VOCALS")],
        vec![percussion("PART HARM1")],
        vec![singing("HARM3", 1200, 1440)],
    ] {
        assert!(
            append_midi(&encode(source), &chart)
                .unwrap_err()
                .to_string()
                .contains("preservation-aware")
        );
    }
    let original = encode(vec![singing("PART GUITAR", 240, 480)]);
    let mut other_clock = chart;
    other_clock.resolution = 960;
    assert!(
        append_midi(&original, &other_clock)
            .unwrap_err()
            .to_string()
            .contains("resolutions")
    );
}
#[test]
fn appended_zero_length_hit_at_phrase_start_has_a_paired_on_and_off() {
    let original = encode(Vec::new());
    let mut chart = crate::vocal_percussion::tests::fixture();
    chart.tracks.remove(&Part::Harm1);
    let hit = &mut chart.tracks.get_mut(&Part::Lead).unwrap().hits[0];
    hit.start = 0;
    hit.end = 0;
    let bytes = append_midi(&original, &chart).unwrap();
    let imported = import_midi(&bytes).unwrap().unwrap();
    let hit = &imported.tracks[&Part::Lead].hits[0];
    assert_eq!((hit.start, hit.end, hit.key()), (0, 0, 96));
    let attributes = hit.midi.as_ref().unwrap();
    assert!(attributes.zero_length);
    assert!(!attributes.off_as_note_on);
    assert_eq!(attributes.off_velocity, 0);
}

fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn raw_track(bytes: &[u8], label: &str) -> Vec<u8> {
    let source = Smf::parse(bytes).unwrap();
    let chunks = midi_chunks(bytes, source.tracks.len()).unwrap().1;
    source
        .tracks
        .iter()
        .zip(chunks)
        .find(|(events, _)| name(events).unwrap() == Some(label))
        .unwrap()
        .1
        .to_vec()
}
fn absolute<'a>(bytes: &'a [u8], label: &str) -> Vec<(u64, K<'a>)> {
    let source = Smf::parse(bytes).unwrap();
    let track = source
        .tracks
        .iter()
        .find(|events| name(events).unwrap() == Some(label))
        .unwrap();
    let mut tick = 0;
    track
        .iter()
        .map(|event| {
            tick += u64::from(event.delta.as_int());
            (tick, event.kind)
        })
        .collect()
}
fn retained_lead(bytes: &[u8], percussion_channel: u8) -> Vec<(u64, K<'_>)> {
    absolute(bytes, "PART VOCALS")
        .into_iter()
        .filter(|(_, kind)| {
            !matches!(kind, K::Midi { channel, message: N::NoteOn {key,..} | N::NoteOff {key,..} }
            if matches!(key.as_int(),96|97) || (key.as_int()==105 && channel.as_int()==percussion_channel))
        })
        .collect()
}
fn source_with_singing_and_controls() -> Vec<u8> {
    let mut lead = percussion("PART VOCALS");
    lead.extend([
        (960, note(2, 106, 83, true)),
        (1920, note(2, 106, 19, false)),
        (1200, note(4, 60, 88, true)),
        (1440, note(4, 60, 5, false)),
        (1200, K::Meta(M::Lyric(b"Later"))),
        (0, note(1, 116, 100, true)),
        (1920, note(1, 116, 13, false)),
        (
            111,
            K::Midi {
                channel: u4::new(2),
                message: N::Controller {
                    controller: u7::new(10),
                    value: u7::new(42),
                },
            },
        ),
        (0, K::Meta(M::Text(b"[mood_play]"))),
    ]);
    let mut harmony = percussion("PART HARM1");
    harmony.extend([
        (960, note(0, 105, 100, true)),
        (1920, note(0, 105, 0, false)),
    ]);
    let mut second = singing("HARM2", 1200, 1440);
    second.extend([
        (960, note(0, 105, 100, true)),
        (1920, note(0, 105, 0, false)),
    ]);
    encode(vec![
        lead,
        harmony,
        second,
        singing("PART GUITAR", 240, 480),
        vec![
            (0, K::Meta(M::TrackName(b"VENUE"))),
            (123, K::Meta(M::Text(b"Keep venue control"))),
            (1920, K::Meta(M::EndOfTrack)),
        ],
    ])
}
#[test]
fn selective_hit_rewrite_keeps_other_owners_raw_and_all_unmanaged_lead_events_exact() {
    let original = source_with_singing_and_controls();
    let before = import_midi(&original).unwrap().unwrap();
    let mut after = before.clone();
    let hit = &mut after.tracks.get_mut(&Part::Lead).unwrap().hits[0];
    hit.start += 1;
    hit.end += 1;
    let bytes = rewrite_midi(
        &original,
        &fingerprint(&original),
        Some(&before),
        Some(&after),
    )
    .unwrap();
    assert!(same_chart(
        import_midi(&bytes).unwrap().as_ref(),
        Some(&after)
    ));
    assert_eq!(retained_lead(&original, 7), retained_lead(&bytes, 7));
    for label in ["TEMPO", "PART HARM1", "HARM2", "PART GUITAR", "VENUE"] {
        assert_eq!(raw_track(&original, label), raw_track(&bytes, label));
    }
}
#[test]
fn phrase_resize_retains_hit_attributes_and_places_new_start_before_same_tick_hit() {
    let original = source_with_singing_and_controls();
    let before = import_midi(&original).unwrap().unwrap();
    let mut after = before.clone();
    let phrase = &mut after.tracks.get_mut(&Part::Lead).unwrap().phrases[0];
    phrase.range.start = 240;
    phrase.range.end = 600;
    let bytes = rewrite_midi(
        &original,
        &fingerprint(&original),
        Some(&before),
        Some(&after),
    )
    .unwrap();
    let imported = import_midi(&bytes).unwrap().unwrap();
    assert_eq!(
        imported.tracks[&Part::Lead].hits,
        before.tracks[&Part::Lead].hits
    );
    assert_eq!(
        imported.tracks[&Part::Lead].phrases,
        after.tracks[&Part::Lead].phrases
    );
    let events = absolute(&bytes, "PART VOCALS");
    let marker = events
        .iter()
        .position(|(tick, kind)| {
            *tick == 240
                && matches!(kind,
        K::Midi{message:N::NoteOn{key,vel},..} if key.as_int()==105 && vel.as_int()>0)
        })
        .unwrap();
    let hit = events
        .iter()
        .position(|(tick, kind)| {
            *tick == 240
                && matches!(kind,
        K::Midi{message:N::NoteOn{key,vel},..} if key.as_int()==96 && vel.as_int()>0)
        })
        .unwrap();
    assert!(marker < hit);
    assert_eq!(retained_lead(&original, 7), retained_lead(&bytes, 7));
}
#[test]
fn clearing_percussion_keeps_singing_phrases_power_author_controls_and_other_raw_tracks() {
    let original = source_with_singing_and_controls();
    let before = import_midi(&original).unwrap().unwrap();
    let bytes = rewrite_midi(&original, &fingerprint(&original), Some(&before), None).unwrap();
    assert!(import_midi(&bytes).unwrap().is_none());
    assert_eq!(retained_lead(&original, 7), retained_lead(&bytes, 7));
    for label in ["TEMPO", "HARM2", "PART GUITAR", "VENUE"] {
        assert_eq!(raw_track(&original, label), raw_track(&bytes, label));
    }
    assert_eq!(
        rewrite_midi(&bytes, &fingerprint(&bytes), None, None).unwrap(),
        bytes
    );
}
#[test]
fn verified_source_hash_stale_view_and_provenance_only_noop_are_distinct() {
    let original = source_with_singing_and_controls();
    let before = import_midi(&original).unwrap().unwrap();
    let mut provenance = before.clone();
    provenance.source = "Reviewed percussion provenance".into();
    assert_eq!(
        rewrite_midi(
            &original,
            &fingerprint(&original),
            Some(&before),
            Some(&provenance)
        )
        .unwrap(),
        original
    );
    let mut stale = before.clone();
    stale.tracks.get_mut(&Part::Lead).unwrap().hits[0].start += 1;
    assert!(
        rewrite_midi(&original, &fingerprint(&original), Some(&stale), None)
            .unwrap_err()
            .to_string()
            .contains("baseline changed")
    );
    let mut changed = original.clone();
    let index = changed
        .windows(4)
        .position(|bytes| bytes == b"Keep")
        .unwrap();
    changed[index] = b'k';
    assert_eq!(import_midi(&changed).unwrap().unwrap(), before);
    assert!(
        rewrite_midi(&changed, &fingerprint(&original), Some(&before), None)
            .unwrap_err()
            .to_string()
            .contains("source MIDI changed")
    );
    let mut other_clock = before.clone();
    other_clock.resolution = 960;
    assert!(
        rewrite_midi(
            &original,
            &fingerprint(&original),
            Some(&before),
            Some(&other_clock)
        )
        .unwrap_err()
        .to_string()
        .contains("resolutions")
    );
}
#[test]
fn new_percussion_can_join_an_existing_singing_owner_without_replacing_its_notes() {
    let mut lead = singing("PART VOCALS", 1200, 1440);
    lead.extend([
        (960, note(2, 106, 83, true)),
        (1920, note(2, 106, 19, false)),
        (1200, K::Meta(M::Lyric(b"Keep"))),
    ]);
    let original = encode(vec![lead]);
    assert!(import_midi(&original).unwrap().is_none());
    let mut after = crate::vocal_percussion::tests::fixture();
    after.tracks.remove(&Part::Harm1);
    let bytes = rewrite_midi(&original, &fingerprint(&original), None, Some(&after)).unwrap();
    assert!(same_chart(
        import_midi(&bytes).unwrap().as_ref(),
        Some(&after)
    ));
    assert_eq!(retained_lead(&original, 0), retained_lead(&bytes, 0));
}
#[test]
fn expanded_phrases_cannot_consume_retained_singing_or_secondary_percussion() {
    let original = source_with_singing_and_controls();
    let before = import_midi(&original).unwrap().unwrap();
    let mut after = before.clone();
    after.tracks.get_mut(&Part::Lead).unwrap().phrases[0]
        .range
        .end = 1600;
    assert!(
        rewrite_midi(
            &original,
            &fingerprint(&original),
            Some(&before),
            Some(&after)
        )
        .is_err()
    );
    let second = vec![
        (0, K::Meta(M::TrackName(b"HARM3"))),
        (1200, note(4, 97, 88, true)),
        (1220, note(4, 97, 5, false)),
        (1920, K::Meta(M::EndOfTrack)),
    ];
    let source = encode(vec![percussion("HARM1"), second]);
    let before = import_midi(&source).unwrap().unwrap();
    let mut after = before.clone();
    after.tracks.get_mut(&Part::Harm1).unwrap().phrases[0]
        .range
        .end = 1600;
    assert!(
        rewrite_midi(&source, &fingerprint(&source), Some(&before), Some(&after))
            .unwrap_err()
            .to_string()
            .contains("unmanaged harmony percussion")
    );
}

#[test]
fn editing_refuses_non_final_owner_end_events_instead_of_activating_unread_controls() {
    let mut malformed = percussion("PART VOCALS");
    malformed
        .iter_mut()
        .find(|(_, kind)| matches!(kind, K::Meta(M::EndOfTrack)))
        .unwrap()
        .0 = 100;
    assert!(
        import_midi(&encode(vec![malformed]))
            .unwrap_err()
            .to_string()
            .contains("end-of-track")
    );
    let mut singing_only = singing("PART VOCALS", 1200, 1440);
    singing_only
        .iter_mut()
        .find(|(_, kind)| matches!(kind, K::Meta(M::EndOfTrack)))
        .unwrap()
        .0 = 100;
    let original = encode(vec![singing_only]);
    assert!(import_midi(&original).unwrap().is_none());
    let mut after = crate::vocal_percussion::tests::fixture();
    after.tracks.remove(&Part::Harm1);
    assert!(
        rewrite_midi(&original, &fingerprint(&original), None, Some(&after))
            .unwrap_err()
            .to_string()
            .contains("end-of-track")
    );
}

#[test]
fn adding_and_clearing_one_owner_retains_the_other_raw_and_does_not_invent_harmony_scoring() {
    let original = encode(vec![percussion("PART VOCALS")]);
    let before = import_midi(&original).unwrap().unwrap();
    let mut after = before.clone();
    let harmony = crate::vocal_percussion::tests::fixture()
        .tracks
        .remove(&Part::Harm1)
        .unwrap();
    after.tracks.insert(Part::Harm1, harmony.clone());
    let bytes = rewrite_midi(
        &original,
        &fingerprint(&original),
        Some(&before),
        Some(&after),
    )
    .unwrap();
    assert_eq!(
        raw_track(&original, "PART VOCALS"),
        raw_track(&bytes, "PART VOCALS")
    );
    let baseline = import_midi(&bytes).unwrap().unwrap();
    let mut keep = baseline.clone();
    keep.tracks.remove(&Part::Lead);
    let cleared = rewrite_midi(&bytes, &fingerprint(&bytes), Some(&baseline), Some(&keep)).unwrap();
    assert!(same_chart(
        import_midi(&cleared).unwrap().as_ref(),
        Some(&keep)
    ));
    assert_eq!(raw_track(&bytes, "HARM1"), raw_track(&cleared, "HARM1"));
    let orphan = encode(vec![
        percussion("PART VOCALS"),
        singing("HARM3", 1200, 1440),
    ]);
    let before = import_midi(&orphan).unwrap().unwrap();
    let mut after = before.clone();
    after.tracks.insert(Part::Harm1, harmony);
    assert!(
        rewrite_midi(&orphan, &fingerprint(&orphan), Some(&before), Some(&after))
            .unwrap_err()
            .to_string()
            .contains("original shared scoring owner")
    );
}
