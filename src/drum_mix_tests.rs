use super::*;
fn event(tick: u64, text: &str) -> Event {
    Event {
        tick,
        mix: parse(text).unwrap().unwrap(),
        text: text.into(),
    }
}
fn fixture() -> Vec<Event> {
    vec![
        event(120, "mix_2_drums3d"),
        event(240, "[mix 3 drums3d]"),
        event(480, "[mix 3 drums3]"),
        event(600, "mix_2_drums3"),
        event(1920, "mix_1_drums3dnoflip"),
    ]
}
#[test]
fn drum_mix_parses_exact_difficulty_configuration_and_known_flags() {
    for difficulty in 0..4 {
        for configuration in 0..6 {
            for setting in [
                Setting::Clear,
                Setting::Disco,
                Setting::DiscoNoFlip,
                Setting::Easy,
                Setting::EasyNoKick,
            ] {
                let mix = Mix {
                    difficulty,
                    configuration,
                    setting,
                };
                assert_eq!(parse(&mix.text()).unwrap(), Some(mix));
            }
        }
    }
    assert_eq!(parse("mix_2_drums_4_d").unwrap().unwrap().configuration, 4);
    for text in ["solo", "mixture custom", "[section Verse]"] {
        assert_eq!(parse(text).unwrap(), None);
    }
    for text in [
        "[mix 3 drums0d",
        "mix 4 drums0d",
        "mix 3 drums6d",
        "mix 3 drums0new",
        "mix 3 drums10d",
        "mix 3 drums0d]",
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}
#[test]
fn drum_mix_range_edits_retain_text_difficulty_stem_layout_and_other_settings() {
    let before = fixture();
    let previous = ranges(&before, 3).unwrap()[0].clone();
    let after = edited(
        &before,
        3,
        &Edit::Put {
            previous: Some(previous.clone()),
            start: 720,
            end: 960,
            configuration: 3,
        },
    )
    .unwrap();
    let range = &ranges(&after, 3).unwrap()[0];
    assert_eq!((range.start(), range.end()), (720, 960));
    assert_eq!(range.on.text, previous.on.text);
    assert_eq!(range.off.text, previous.off.text);
    assert_eq!(
        before
            .iter()
            .filter(|e| e.mix.difficulty != 3)
            .collect::<Vec<_>>(),
        after
            .iter()
            .filter(|e| e.mix.difficulty != 3)
            .collect::<Vec<_>>()
    );
    assert!(edited(&after, 3, &Edit::Remove { range: previous }).is_err());
    let clear = edited(
        &after,
        3,
        &Edit::Remove {
            range: range.clone(),
        },
    )
    .unwrap();
    assert!(ranges(&clear, 3).unwrap().is_empty());
    assert_eq!(ranges(&clear, 2).unwrap().len(), 1);
}
#[test]
fn drum_mix_refuses_ambiguous_open_overlapping_and_routing_changes_atomically() {
    let before = fixture();
    let saved = before.clone();
    for (start, end, configuration) in [
        (0, 240, 3),
        (300, 400, 3),
        (480, 720, 3),
        (720, 960, 0),
        (720, 720, 3),
    ] {
        assert!(
            edited(
                &before,
                3,
                &Edit::Put {
                    previous: None,
                    start,
                    end,
                    configuration
                }
            )
            .is_err()
        );
        assert_eq!(before, saved);
    }
    assert!(
        edited(
            &before,
            1,
            &Edit::Put {
                previous: None,
                start: 2000,
                end: 2400,
                configuration: 3
            }
        )
        .is_err()
    );
    assert!(ranges(&[event(0, "mix 3 drums0d")], 3).is_err());
    assert!(
        ranges(
            &[event(0, "mix 3 drums0d"), event(480, "mix 3 drums0dnoflip")],
            3
        )
        .is_err()
    );
    let mut duplicate = before.clone();
    duplicate.push(duplicate.last().unwrap().clone());
    assert!(validate(&duplicate).is_err());
    let mut mismatch = before.clone();
    mismatch[0].mix.configuration = 4;
    assert!(validate(&mismatch).is_err());
}
#[test]
fn drum_mix_first_ranges_have_explicit_resets_and_independent_tiers() {
    let first = edited(
        &[],
        3,
        &Edit::Put {
            previous: None,
            start: 240,
            end: 480,
            configuration: 5,
        },
    )
    .unwrap();
    let both = edited(
        &first,
        2,
        &Edit::Put {
            previous: None,
            start: 240,
            end: 960,
            configuration: 5,
        },
    )
    .unwrap();
    assert_eq!(ranges(&both, 3).unwrap()[0].end(), 480);
    assert_eq!(ranges(&both, 2).unwrap()[0].end(), 960);
    let original = first.clone();
    let range = ranges(&first, 3).unwrap()[0].clone();
    assert!(
        edited(
            &first,
            3,
            &Edit::Put {
                previous: None,
                start: 480,
                end: 960,
                configuration: 5
            }
        )
        .is_err()
    );
    assert_eq!(first, original);
    let clear = edited(&first, 3, &Edit::Remove { range }).unwrap();
    assert!(ranges(&clear, 3).unwrap().is_empty());
    assert!(
        clear
            .iter()
            .all(|e| e.mix.configuration == 5 && e.mix.setting == Setting::Clear)
    );
}

#[test]
fn drum_mix_removal_keeps_initial_routing_and_refuses_nontrivial_preceding_flags() {
    let original = vec![event(0, "mix_3_drums3")];
    let authored = edited(
        &original,
        3,
        &Edit::Put {
            previous: None,
            start: 0,
            end: 480,
            configuration: 3,
        },
    )
    .unwrap();
    let range = ranges(&authored, 3).unwrap()[0].clone();
    let cleared = edited(&authored, 3, &Edit::Remove { range }).unwrap();
    assert_eq!(cleared[0].tick, 0);
    assert_eq!(cleared[0].mix, original[0].mix);
    assert!(ranges(&cleared, 3).unwrap().is_empty());
    let routing = vec![
        event(0, "mix 3 drums3dnoflip"),
        event(240, "mix 3 drums3d"),
        event(480, "mix 3 drums3"),
    ];
    assert!(ranges(&routing, 3).is_err());
}

#[test]
fn drum_mix_resize_covers_selected_boundaries_without_losing_outside_resets() {
    let original = vec![
        event(0, "mix_3_drums3"),
        event(240, "mix_3_drums3d"),
        event(480, "mix_3_drums3"),
    ];
    let previous = ranges(&original, 3).unwrap()[0].clone();
    let longer = edited(
        &original,
        3,
        &Edit::Put {
            previous: Some(previous),
            start: 120,
            end: 720,
            configuration: 3,
        },
    )
    .unwrap();
    assert_eq!(longer[0], original[0]);
    let range = ranges(&longer, 3).unwrap()[0].clone();
    assert_eq!((range.start(), range.end()), (120, 720));
    let clear = edited(&longer, 3, &Edit::Remove { range }).unwrap();
    assert!(ranges(&clear, 3).unwrap().is_empty());
    assert!(clear.iter().all(|e| e.mix.configuration == 3));
}
