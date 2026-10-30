use chart_engine::benchmark::{self, Instrument, Request};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
    num::{u4, u7, u15, u24, u28},
};
use std::{fs, path::Path};

fn midi(
    label: &'static [u8],
    notes: &[(u64, u64, u8, &'static str)],
    tempo_change: bool,
) -> Vec<u8> {
    let mut absolute = vec![
        (0, TrackEventKind::Meta(MetaMessage::TrackName(label))),
        (
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500000))),
        ),
    ];
    if tempo_change {
        absolute.push((
            480,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(1000000))),
        ));
    }
    for (start, end, key, lyric) in notes {
        if !lyric.is_empty() {
            absolute.push((
                *start,
                TrackEventKind::Meta(MetaMessage::Lyric(lyric.as_bytes())),
            ));
        }
        absolute.push((
            *start,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(*key),
                    vel: u7::new(100),
                },
            },
        ));
        absolute.push((
            *end,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key: u7::new(*key),
                    vel: u7::new(0),
                },
            },
        ));
    }
    if label == b"PART VOCALS" && !notes.iter().any(|note| matches!(note.2, 105 | 106)) {
        let end = notes.iter().map(|note| note.1).max().unwrap_or(1);
        absolute.push((
            0,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(105),
                    vel: u7::new(100),
                },
            },
        ));
        absolute.push((
            end,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key: u7::new(105),
                    vel: u7::new(0),
                },
            },
        ));
    }
    absolute.sort_by_key(|(tick, kind)| {
        (
            *tick,
            matches!(
                kind,
                TrackEventKind::Midi {
                    message: MidiMessage::NoteOn { .. },
                    ..
                }
            ),
        )
    });
    let mut previous = 0;
    let mut track: Vec<_> = absolute
        .into_iter()
        .map(|(tick, kind)| {
            let event = TrackEvent {
                delta: u28::new((tick - previous) as u32),
                kind,
            };
            previous = tick;
            event
        })
        .collect();
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let mut encoded = Vec::new();
    Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(480))),
        tracks: vec![track],
    }
    .write_std(&mut encoded)
    .unwrap();
    encoded
}

#[test]
fn onset_matching_is_one_to_one_and_optimizes_cardinality_before_error() {
    let matches = benchmark::match_attacks(&[0., 0.1], &[0.09, 0.11], 0.1).unwrap();
    assert_eq!(matches, vec![(0, 0), (1, 1)]);
    let matches = benchmark::match_attacks(&[0., 0.02], &[0.019], 0.02).unwrap();
    assert_eq!(
        matches,
        vec![(1, 0)],
        "Equal cardinality chooses the smaller error"
    );
    assert_eq!(
        benchmark::match_attacks(&[1.], &[0.99, 1., 1.01], 0.03).unwrap(),
        vec![(0, 1)]
    );
    assert!(benchmark::match_attacks(&[1., 1.], &[1.], 0.01).is_err());
    assert!(benchmark::match_attacks(&[f64::NAN], &[], 0.01).is_err());
}

#[test]
fn reference_targets_preserve_tempo_delay_chords_and_exclude_other_tiers() {
    let bytes = midi(
        b"PART BASS",
        &[
            (0, 120, 60, ""),
            (240, 360, 95, ""),
            (240, 360, 99, ""),
            (240, 360, 101, ""),
            (720, 840, 98, ""),
        ],
        true,
    );
    let parsed = benchmark::parse(&bytes, Instrument::Bass, 50.).unwrap();
    assert_eq!(parsed.notes.len(), 2);
    assert_eq!(parsed.excluded_note_onsets, 3);
    let attacks = benchmark::attack_times(&parsed);
    assert!((attacks[0] - 0.3).abs() < 1e-12);
    assert!((attacks[1] - 1.05).abs() < 1e-12);
    assert!(
        parsed.notes.iter().all(|note| note.pitch.is_none()),
        "Fret colors cannot become acoustic pitch ground truth"
    );
    assert!(benchmark::parse(&bytes, Instrument::Vocals, 0.).is_err());
}

#[test]
fn enhanced_open_notes_require_the_midi_dialect_marker() {
    let bytes = midi(b"PART BASS", &[(0, 120, 95, ""), (240, 360, 96, "")], false);
    assert_eq!(
        benchmark::parse(&bytes, Instrument::Bass, 0.)
            .unwrap()
            .notes
            .len(),
        1
    );
    let mut file = Smf::parse(&bytes).unwrap();
    file.tracks[0].insert(
        0,
        TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::Text(b"[ENHANCED_OPENS]")),
        },
    );
    let mut marked = Vec::new();
    file.write_std(&mut marked).unwrap();
    assert_eq!(
        benchmark::parse(&marked, Instrument::Bass, 0.)
            .unwrap()
            .notes
            .len(),
        2
    );
    file.tracks[0][0].kind = TrackEventKind::Meta(MetaMessage::Text(b"ENHANCED_OPENS"));
    let mut bare = Vec::new();
    file.write_std(&mut bare).unwrap();
    assert_eq!(
        benchmark::parse(&bare, Instrument::Bass, 0.)
            .unwrap()
            .notes
            .len(),
        2
    );
}

#[test]
fn unbracketed_vocal_text_events_are_lyrics_in_the_supported_game_dialect() {
    let bytes = midi(
        b"PART VOCALS",
        &[
            (0, 120, 60, "Sing-"),
            (120, 240, 62, "ing"),
            (240, 360, 60, "soft*$"),
        ],
        false,
    );
    let mut file = Smf::parse(&bytes).unwrap();
    for event in &mut file.tracks[0] {
        if let TrackEventKind::Meta(MetaMessage::Lyric(text)) = event.kind {
            event.kind = TrackEventKind::Meta(MetaMessage::Text(text));
        }
    }
    let mut encoded = Vec::new();
    file.write_std(&mut encoded).unwrap();
    assert_eq!(
        benchmark::lyric_source(&encoded).unwrap().text,
        "Singing soft"
    );
    let track = benchmark::parse(&encoded, Instrument::Vocals, 0.).unwrap();
    assert_eq!(track.notes.len(), 3);
    assert!(track.notes[2].pitch.is_none());
}

#[test]
fn talkie_marker_runs_exclude_all_unpitched_lyric_variants() {
    let bytes = midi(
        b"PART VOCALS",
        &[
            (0, 120, 60, "one#%"),
            (120, 240, 60, "two^/"),
            (240, 360, 60, "three*$"),
            (360, 480, 60, "four"),
        ],
        false,
    );
    let track = benchmark::parse(&bytes, Instrument::Vocals, 0.).unwrap();
    assert!(track.notes[..3].iter().all(|note| note.pitch.is_none()));
    assert_eq!(track.notes[3].pitch, Some(60));
}

#[test]
fn vocal_targets_follow_phrase_membership_and_consume_early_lyrics() {
    let bytes = midi(
        b"PART VOCALS",
        &[(0, 120, 105, ""), (240, 360, 60, "outside")],
        false,
    );
    let track = benchmark::parse(&bytes, Instrument::Vocals, 0.).unwrap();
    assert!(track.notes.is_empty());
    assert_eq!(track.excluded_unphrased_vocal_notes, 1);
    let bytes = midi(
        b"PART VOCALS",
        &[(240, 360, 60, "early#%"), (360, 480, 60, "pitched")],
        false,
    );
    let mut file = Smf::parse(&bytes).unwrap();
    let at = file.tracks[0]
        .iter()
        .position(|event| matches!(event.kind, TrackEventKind::Meta(MetaMessage::Lyric(_))))
        .unwrap();
    assert_eq!(file.tracks[0][at].delta.as_int(), 240);
    file.tracks[0][at].delta = u28::new(220);
    file.tracks[0][at + 1].delta = u28::new(20);
    let mut early = Vec::new();
    file.write_std(&mut early).unwrap();
    let track = benchmark::parse(&early, Instrument::Vocals, 0.).unwrap();
    assert_eq!(track.notes[0].start, 0.25);
    assert!(track.notes[0].pitch.is_none());
    assert_eq!(
        track.notes[1].pitch,
        Some(60),
        "A consumed talkie marker must not leak to the next pitched word"
    );
}

#[test]
fn supplied_reference_lyrics_are_text_only_and_join_syllables() {
    let bytes = midi(
        b"PART VOCALS",
        &[
            (0, 120, 60, "Sing-"),
            (120, 240, 62, "ing"),
            (240, 360, 62, "+"),
            (360, 480, 60, "soft#"),
        ],
        false,
    );
    let lyrics = benchmark::lyric_source(&bytes).unwrap();
    assert_eq!(lyrics.text, "Singing soft");
    assert!(lyrics.synced_lyrics.is_none());
    assert!(lyrics.source.contains("text only"));
    let parsed = benchmark::parse(&bytes, Instrument::Vocals, 0.).unwrap();
    assert_eq!(parsed.notes.len(), 4);
    assert!(parsed.notes[3].pitch.is_none());
    assert_eq!(parsed.phrase_markers, 1);
}

#[test]
fn bass_chart_input_retains_embedded_offset_and_ignores_force_markers() {
    let timing = chart_engine::timing::TempoMap::new(
        192,
        -0.1,
        vec![chart_engine::timing::Tempo { tick: 0, bpm: 120. }],
    )
    .unwrap();
    let notes = vec![
        chart_engine::chart::Note {
            tick: 192,
            lane: 7,
            sustain: 0,
        },
        chart_engine::chart::Note {
            tick: 192,
            lane: 5,
            sustain: 0,
        },
        chart_engine::chart::Note {
            tick: 384,
            lane: 2,
            sustain: 96,
        },
    ];
    let tracks = [("ExpertDoubleBass".into(), notes)].into_iter().collect();
    let text = chart_engine::chart::new_chart("Fixed source", "Controlled", &timing, &tracks);
    let parsed = benchmark::parse(text.as_bytes(), Instrument::Bass, 50.).unwrap();
    assert_eq!(parsed.notes.len(), 2);
    assert_eq!(parsed.excluded_note_onsets, 1);
    let attacks = benchmark::attack_times(&parsed);
    assert!((attacks[0] - 0.45).abs() < 1e-12);
    assert!((attacks[1] - 0.95).abs() < 1e-12);
    assert!(benchmark::parse(text.as_bytes(), Instrument::Vocals, 0.).is_err());
}

fn compare(reference: &[u8], generated: &[u8], root: &Path) -> benchmark::Report {
    compare_instrument(reference, generated, root, Instrument::Vocals)
}
fn compare_instrument(
    reference: &[u8],
    generated: &[u8],
    root: &Path,
    instrument: Instrument,
) -> benchmark::Report {
    let reference_path = root.join("reference.mid");
    let generated_path = root.join("generated.mid");
    let audio = root.join("same-audio.bin");
    fs::write(&reference_path, reference).unwrap();
    fs::write(&generated_path, generated).unwrap();
    fs::write(&audio, b"fixed source bytes").unwrap();
    benchmark::compare(Request {
        reference_midi: &reference_path,
        generated_midi: &generated_path,
        reference_audio: &audio,
        generated_audio: &audio,
        reference_delay_ms: 0.,
        generated_delay_ms: 0.,
        instrument,
    })
    .unwrap()
}

fn compare_result(
    reference: &[u8],
    generated: &[u8],
    root: &Path,
    instrument: Instrument,
) -> anyhow::Result<benchmark::Report> {
    let reference_path = root.join("reference.mid");
    let generated_path = root.join("generated.mid");
    let audio = root.join("same-audio.bin");
    fs::write(&reference_path, reference)?;
    fs::write(&generated_path, generated)?;
    fs::write(&audio, b"fixed source bytes")?;
    benchmark::compare(Request {
        reference_midi: &reference_path,
        generated_midi: &generated_path,
        reference_audio: &audio,
        generated_audio: &audio,
        reference_delay_ms: 0.,
        generated_delay_ms: 0.,
        instrument,
    })
}

#[test]
fn absent_generated_midi_parts_are_abstentions_but_reference_and_inspect_remain_strict() {
    let root = tempfile::tempdir().unwrap();
    let empty = midi(b"TEMPO", &[], false);
    for (instrument, label, key) in [
        (Instrument::Guitar, b"PART GUITAR".as_slice(), 96),
        (Instrument::Bass, b"PART BASS".as_slice(), 96),
        (Instrument::Drums, b"PART DRUMS".as_slice(), 96),
        (Instrument::ProDrums, b"PART DRUMS".as_slice(), 96),
        (Instrument::Vocals, b"PART VOCALS".as_slice(), 60),
    ] {
        let reference = midi(label, &[(0, 480, key, "word")], false);
        let report = compare_instrument(&reference, &empty, root.path(), instrument);
        assert_eq!(report.reference_gems, 1);
        assert_eq!(report.generated_gems, 0);
        for metrics in report.attacks {
            assert_eq!(metrics.reference_attacks, 1);
            assert_eq!(metrics.generated_attacks, 0);
            assert_eq!(metrics.matched, 0);
            assert_eq!(metrics.recall, Some(0.));
            assert_eq!(metrics.precision, None);
        }
        assert!(benchmark::parse(&empty, instrument, 0.).is_err());
        assert!(compare_result(&empty, &reference, root.path(), instrument).is_err());
    }
}

#[test]
fn missing_generated_part_does_not_excuse_damaged_midi_or_duplicate_requested_tracks() {
    let root = tempfile::tempdir().unwrap();
    let reference = midi(b"PART BASS", &[(0, 480, 96, "")], false);
    let empty = midi(b"TEMPO", &[], false);
    let mut wrong_track_count = empty.clone();
    wrong_track_count[10..12].copy_from_slice(&2u16.to_be_bytes());
    let mut malformed_event = empty.clone();
    // Keep exact declared chunk framing but put an invalid event after EOT.
    let length = u32::from_be_bytes(malformed_event[18..22].try_into().unwrap());
    malformed_event[18..22].copy_from_slice(&(length + 1).to_be_bytes());
    malformed_event.push(0xff);
    for damaged in [
        empty[..empty.len() - 1].to_vec(),
        empty[..14].to_vec(),
        wrong_track_count,
        malformed_event,
    ] {
        assert!(compare_result(&reference, &damaged, root.path(), Instrument::Bass).is_err());
    }
    let mut duplicate = Smf::parse(&reference).unwrap();
    duplicate.header.format = Format::Parallel;
    duplicate.tracks.push(duplicate.tracks[0].clone());
    let mut encoded = vec![];
    duplicate.write_std(&mut encoded).unwrap();
    assert!(compare_result(&reference, &encoded, root.path(), Instrument::Bass).is_err());
}

#[test]
fn present_unphrased_generated_vocals_still_abstain_from_scored_notes() {
    let root = tempfile::tempdir().unwrap();
    let reference = midi(b"PART VOCALS", &[(0, 480, 60, "word")], false);
    let mut unphrased = Smf::parse(&reference).unwrap();
    unphrased.tracks[0].retain(|event| !matches!(event.kind,
        TrackEventKind::Midi { message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. }, .. } if key.as_int() == 105));
    let mut encoded = vec![];
    unphrased.write_std(&mut encoded).unwrap();
    let parsed = benchmark::parse(&encoded, Instrument::Vocals, 0.).unwrap();
    assert_eq!(parsed.excluded_unphrased_vocal_notes, 1);
    let report = compare(&reference, &encoded, root.path());
    assert_eq!(report.generated_gems, 0);
    assert_eq!(report.attacks[1].recall, Some(0.));
}

#[test]
fn expert_guitar_midi_ignores_force_tap_and_other_difficulty_markers() {
    let bytes = midi(
        b"PART GUITAR",
        &[
            (0, 1, 60, ""),
            (240, 241, 95, ""),
            (240, 480, 96, ""),
            (240, 480, 100, ""),
            (240, 241, 101, ""),
            (240, 241, 102, ""),
            (240, 241, 104, ""),
            (240, 241, 116, ""),
            (240, 241, 103, ""),
        ],
        false,
    );
    let parsed = benchmark::parse(&bytes, Instrument::Guitar, 25.).unwrap();
    assert_eq!(parsed.notes.len(), 2);
    assert_eq!(parsed.excluded_note_onsets, 7);
    assert_eq!(benchmark::attack_times(&parsed), [0.275]);
    assert!(parsed.notes.iter().all(|n| n.pitch.is_none()));
    for marker in [b"[ENHANCED_OPENS]".as_slice(), b"ENHANCED_OPENS".as_slice()] {
        let mut file = Smf::parse(&bytes).unwrap();
        file.tracks[0].insert(
            0,
            TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::Text(marker)),
            },
        );
        let mut encoded = vec![];
        file.write_std(&mut encoded).unwrap();
        let parsed = benchmark::parse(&encoded, Instrument::Guitar, 0.).unwrap();
        assert_eq!(parsed.notes.len(), 3);
        assert!(parsed.notes.iter().any(|n| n.lane == Some(7)));
    }
}
#[test]
fn drum_kicks_and_five_lane_gems_are_distinct_from_control_and_plus_notes() {
    let bytes = midi(
        b"PART DRUMS",
        &[
            (0, 1, 95, ""),
            (0, 1, 96, ""),
            (0, 1, 97, ""),
            (240, 241, 101, ""),
            (240, 480, 110, ""),
            (240, 480, 111, ""),
            (240, 480, 112, ""),
            (240, 480, 116, ""),
            (240, 480, 120, ""),
            (240, 241, 72, ""),
        ],
        false,
    );
    let parsed = benchmark::parse(&bytes, Instrument::Drums, 0.).unwrap();
    assert_eq!(parsed.notes.len(), 3);
    assert_eq!(parsed.excluded_note_onsets, 7);
    assert_eq!(parsed.excluded_expert_plus_kicks, Some(1));
    assert_eq!(benchmark::attack_times(&parsed), [0., 0.25]);
    assert_eq!(parsed.notes.iter().filter(|n| n.lane == Some(0)).count(), 1);
    assert!(benchmark::parse(&bytes, Instrument::ProDrums, 0.).is_err());
}
fn chart(notes: &str, label: &str) -> Vec<u8> {
    format!("[Song]\n{{\n Resolution =480\n Offset =0.05\n}}\n[SyncTrack]\n{{\n0 =B 120000\n}}\n[{label}]\n{{\n{notes}\n}}\n").into_bytes()
}
#[test]
fn chart_guitar_open_and_drums_cymbals_follow_different_lane_contracts() {
    let guitar = chart(
        "0 = N 7 0\n0 = N 5 0\n0 = N 6 0\n480 = N 4 240",
        "ExpertSingle",
    );
    let parsed = benchmark::parse(&guitar, Instrument::Guitar, 50.).unwrap();
    assert_eq!(parsed.notes.len(), 2);
    assert_eq!(parsed.excluded_note_onsets, 2);
    for (actual, expected) in benchmark::attack_times(&parsed).iter().zip([0.1, 0.6]) {
        assert!((actual - expected).abs() < 1e-12);
    }
    let drums = chart(
        "0 = N 0 0\n0 = N 32 0\n240 = N 2 0\n240 = N 66 100\n480 = N 2 0\n480 = N 34 0\n480 = N 40 0",
        "ExpertDrums",
    );
    let parsed = benchmark::parse(&drums, Instrument::ProDrums, 0.).unwrap();
    assert_eq!(parsed.notes.len(), 3);
    assert_eq!(parsed.excluded_expert_plus_kicks, Some(1));
    assert_eq!(parsed.notes[1].cymbal, Some(true));
    assert_eq!(
        parsed.notes[2].cymbal,
        Some(false),
        "Chart marker sustain must not create a cymbal span"
    );
    assert_eq!(parsed.excluded_note_onsets, 4);
}
#[test]
fn pro_drum_tom_markers_have_exclusive_ends_and_toggle_default_cymbals() {
    let bytes = midi(
        b"PART DRUMS",
        &[
            (0, 240, 110, ""),
            (0, 1, 98, ""),
            (120, 121, 98, ""),
            (240, 241, 98, ""),
            (240, 241, 97, ""),
            (360, 361, 96, ""),
        ],
        false,
    );
    let parsed = benchmark::parse(&bytes, Instrument::ProDrums, 0.).unwrap();
    assert_eq!(parsed.notes.len(), 5);
    let yellow: Vec<_> = parsed
        .notes
        .iter()
        .filter(|n| n.lane == Some(2))
        .map(|n| n.cymbal)
        .collect();
    assert_eq!(yellow, [Some(false), Some(false), Some(true)]);
    let parsed = benchmark::parse(&bytes, Instrument::Drums, 0.).unwrap();
    assert!(parsed.notes.iter().all(|n| n.cymbal.is_none()));
}
#[test]
fn pro_drum_expert_disco_flip_changes_red_yellow_with_articulation() {
    let bytes = midi(
        b"PART DRUMS",
        &[
            (0, 1, 97, ""),
            (120, 121, 98, ""),
            (240, 241, 97, ""),
            (360, 361, 98, ""),
        ],
        false,
    );
    let mut smf = Smf::parse(&bytes).unwrap();
    smf.tracks[0].insert(
        0,
        TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::Text(b"[mix 3 drums0d]")),
        },
    );
    let mut encoded = vec![];
    smf.write_std(&mut encoded).unwrap();
    let track = benchmark::parse(&encoded, Instrument::ProDrums, 0.).unwrap();
    assert_eq!(track.notes[0].lane, Some(2));
    assert_eq!(track.notes[0].cymbal, Some(true));
    assert_eq!(track.notes[1].lane, Some(1));
    assert_eq!(track.notes[1].cymbal, Some(false));
    let bytes = chart(
        "0 = E \"mix 3 drums0d\"\n0 = N 1 0\n240 = N 2 0\n240 = N 66 0\n480 = E \"mix 3 drums0dnoflip\"\n480 = N 1 0",
        "ExpertDrums",
    );
    let track = benchmark::parse(&bytes, Instrument::ProDrums, 0.).unwrap();
    assert_eq!(track.notes[0].lane, Some(2));
    assert_eq!(track.notes[1].lane, Some(1));
    assert_eq!(track.notes[2].lane, Some(1));
}
#[test]
fn cross_window_matches_keep_each_precision_recall_denominator_local() {
    let root = tempfile::tempdir().unwrap();
    let reference = midi(b"PART GUITAR", &[(11510, 11511, 96, "")], false);
    let generated = midi(b"PART GUITAR", &[(11530, 11531, 100, "")], false);
    let report = compare_instrument(&reference, &generated, root.path(), Instrument::Guitar);
    assert_eq!(report.attacks[1].matched, 1);
    let sections = report.expert_sections.unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].matched_reference_attacks, 1);
    assert_eq!(sections[0].matched_generated_attacks, 0);
    assert_eq!(sections[0].recall_at_50ms, Some(1.));
    assert_eq!(sections[0].precision_at_50ms, None);
    assert_eq!(sections[1].precision_at_50ms, Some(1.));
    assert_eq!(sections[1].recall_at_50ms, None);
}
#[test]
fn extra_guitar_notes_and_sustains_have_symmetric_penalties_without_color_truth() {
    let reference = midi(
        b"PART GUITAR",
        &[(0, 960, 96, ""), (0, 960, 100, "")],
        false,
    );
    let generated = midi(
        b"PART GUITAR",
        &[(0, 1920, 97, ""), (2400, 2401, 98, "")],
        false,
    );
    let a = tempfile::tempdir().unwrap();
    let forward = compare_instrument(&reference, &generated, a.path(), Instrument::Guitar);
    let b = tempfile::tempdir().unwrap();
    let reverse = compare_instrument(&generated, &reference, b.path(), Instrument::Guitar);
    assert_eq!(forward.attacks[1].precision, Some(0.5));
    assert_eq!(forward.attacks[1].recall, Some(1.));
    assert_eq!(reverse.attacks[1].precision, forward.attacks[1].recall);
    assert_eq!(reverse.attacks[1].recall, forward.attacks[1].precision);
    let sustain = forward.guitar_sustain.unwrap();
    assert_eq!(sustain.reference_sustained_seconds, 1.);
    assert_eq!(sustain.generated_sustained_seconds, 2.);
    assert_eq!(sustain.generated_time_precision, Some(0.5));
    assert_eq!(sustain.reference_time_recall, Some(1.));
    let section = &forward.expert_sections.unwrap()[0];
    assert_eq!(
        section.reference.gems_per_attack_histogram.get(&2),
        Some(&1)
    );
}
#[test]
fn drum_pad_attacks_cannot_hide_missing_kicks_or_wrong_pro_articulation() {
    let reference = midi(
        b"PART DRUMS",
        &[(0, 1, 96, ""), (0, 1, 98, ""), (240, 241, 97, "")],
        false,
    );
    let generated = midi(
        b"PART DRUMS",
        &[(0, 1, 98, ""), (0, 240, 110, ""), (240, 241, 97, "")],
        false,
    );
    let root = tempfile::tempdir().unwrap();
    let report = compare_instrument(&reference, &generated, root.path(), Instrument::ProDrums);
    assert_eq!(report.attacks[1].recall, Some(1.));
    let roles = report.drum_roles.unwrap();
    assert_eq!(roles[0].class, "kick");
    assert_eq!(roles[0].attacks[1].recall, Some(0.));
    assert_eq!(roles[0].attacks[1].precision, None);
    let hits = report.pro_drum_hits.unwrap();
    assert_eq!(hits.attacks[1].matched, 1);
    assert_eq!(hits.attacks[1].reference_attacks, 3);
    assert_eq!(hits.attacks[1].generated_attacks, 2);
    assert_eq!(hits.attacks[1].precision, Some(0.5));
    assert_eq!(hits.attacks[1].recall, Some(1. / 3.));
}
#[test]
fn legacy_reports_and_inspection_have_no_new_serialized_fields() {
    for (instrument, label) in [
        (Instrument::Bass, b"PART BASS".as_slice()),
        (Instrument::Vocals, b"PART VOCALS".as_slice()),
    ] {
        let bytes = midi(label, &[(0, 240, 60, "one"), (480, 720, 96, "")], false);
        let root = tempfile::tempdir().unwrap();
        let report = compare_instrument(&bytes, &bytes, root.path(), instrument);
        let value = serde_json::to_value(&report).unwrap();
        assert!(value.get("expert_sections").is_none());
        assert!(value.get("guitar_sustain").is_none());
        assert!(value.get("drum_roles").is_none());
        assert!(value.get("pro_drum_hits").is_none());
        assert!(value.get("excluded_expert_plus_kicks").is_none());
        let track =
            serde_json::to_value(benchmark::parse(&bytes, instrument, 0.).unwrap()).unwrap();
        assert!(track.get("excluded_expert_plus_kicks").is_none());
        assert!(
            track["notes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n.get("lane").is_none() && n.get("cymbal").is_none())
        );
    }
}

#[test]
fn added_nonoverlapping_tones_cannot_inflate_conditional_pitch_accuracy() {
    let root = tempfile::tempdir().unwrap();
    let reference = midi(b"PART VOCALS", &[(0, 960, 60, "one")], false);
    let generated = midi(
        b"PART VOCALS",
        &[(0, 960, 60, "one"), (960, 1920, 72, "extra")],
        false,
    );
    let report = compare(&reference, &generated, root.path());
    let pitch = report.vocal_pitch.unwrap();
    assert_eq!(pitch.reference_coverage, Some(1.));
    assert_eq!(pitch.exact_pitch_agreement, Some(1.));
    assert_eq!(pitch.exact_pitch_precision, Some(0.5));
    assert_eq!(pitch.generated_time_overlap, Some(0.5));
}

#[test]
fn octave_equivalents_are_separate_from_literal_pitch_and_talkies() {
    let root = tempfile::tempdir().unwrap();
    let reference = midi(
        b"PART VOCALS",
        &[(0, 960, 60, "one"), (960, 1440, 60, "talk#")],
        false,
    );
    let generated = midi(b"PART VOCALS", &[(0, 960, 72, "one")], false);
    let pitch = compare(&reference, &generated, root.path())
        .vocal_pitch
        .unwrap();
    assert_eq!(pitch.reference_pitched_seconds, 1.);
    assert_eq!(pitch.exact_pitch_agreement, Some(0.));
    assert_eq!(pitch.octave_folded_pitch_agreement, Some(1.));
}

#[test]
fn arbitrary_recording_substitution_is_rejected_without_offset_guessing() {
    let root = tempfile::tempdir().unwrap();
    let notes = root.path().join("notes.mid");
    fs::write(&notes, midi(b"PART VOCALS", &[(0, 960, 60, "one")], false)).unwrap();
    let a = root.path().join("a.audio");
    let b = root.path().join("b.audio");
    fs::write(&a, b"recording A").unwrap();
    fs::write(&b, b"recording B").unwrap();
    assert!(
        benchmark::compare(Request {
            reference_midi: &notes,
            generated_midi: &notes,
            reference_audio: &a,
            generated_audio: &b,
            reference_delay_ms: 0.,
            generated_delay_ms: 0.,
            instrument: Instrument::Vocals
        })
        .is_err()
    );
}
