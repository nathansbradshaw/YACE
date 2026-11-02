use yace_stepmania::{
    project::{Foot, footwork},
    timing::{AudioPosition, Boundary},
    *,
};
fn sm() -> String {
    "// untouched comment\r\n#TITLE:Example\\: song;\r\n#ARTIST:Author;\r\n#MUSIC:song.ogg;\r\n#CUSTOM:keep me;\r\n#OFFSET:0.25;\r\n#BPMS:0=120,8=180;\r\n#NOTES:dance-single:First:Hard:7:0,0,0,0,0:\r\n2000\r\n0000\r\n3000\r\n0100\r\n;\r\n#NOTES:dance-single:Second:Hard:8:0,0,0,0,0:\r\n0010\r\n0001\r\n0000\r\n0000\r\n;\r\n".into()
}
fn edit(p: &mut Project, action: Action) -> anyhow::Result<()> {
    p.apply(Request {
        expected_revision: p.revision,
        actor: "test".into(),
        action,
    })
}
#[test]
fn unchanged_sm_is_byte_exact() {
    let raw = sm();
    let doc = Document::parse(raw.clone(), Format::Sm).unwrap();
    assert_eq!(doc.charts.len(), 2);
    assert_ne!(doc.charts[0].id, doc.charts[1].id);
    assert_eq!(doc.metadata["TITLE"], "Example: song");
    assert_eq!(doc.export(Format::Sm).unwrap(), raw);
}
#[test]
fn changed_chart_preserves_other_chart_and_source() {
    let raw = sm();
    let mut p = Project::import(raw.clone(), Format::Sm, Profile::Pad).unwrap();
    edit(
        &mut p,
        Action::AddNote {
            chart_id: "chart-1".into(),
            row: 180,
            lane: 3,
            kind: NoteKind::Lift,
            end_row: None,
        },
    )
    .unwrap();
    let text = p.content.document.export(Format::Sm).unwrap();
    assert!(text.contains("// untouched comment\r\n"));
    assert!(text.contains("#CUSTOM:keep me;\r\n"));
    assert!(
        text.ends_with(
            raw.split("#NOTES:dance-single:Second")
                .last()
                .map(|v| format!("#NOTES:dance-single:Second{v}"))
                .unwrap()
                .as_str()
        )
    );
    assert_eq!(p.content.document.raw, raw);
    let imported = Document::parse(text, Format::Sm).unwrap();
    assert!(
        imported.charts[0]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Lift)
    );
}
#[test]
fn all_note_kinds_round_trip() {
    let mut p = Project::blank(
        "All".into(),
        "Test".into(),
        "song.ogg".into(),
        120.,
        0.,
        Profile::Pad,
    )
    .unwrap();
    edit(
        &mut p,
        Action::AddChart {
            difficulty: Difficulty::Edit,
            meter: 7,
            description: "custom".into(),
        },
    )
    .unwrap();
    for (i, kind) in NoteKind::ALL.into_iter().enumerate() {
        let row = i as i64 * 96;
        edit(
            &mut p,
            Action::AddNote {
                chart_id: "added-1".into(),
                row,
                lane: 0,
                kind,
                end_row: kind.sustained().then_some(row + 48),
            },
        )
        .unwrap();
    }
    let doc =
        Document::parse(p.content.document.export(Format::Ssc).unwrap(), Format::Ssc).unwrap();
    assert_eq!(
        doc.charts[0]
            .notes
            .iter()
            .map(|n| n.kind)
            .collect::<Vec<_>>(),
        NoteKind::ALL
    );
}
#[test]
fn malformed_holds_are_rejected() {
    for text in [
        "2000\n0000\n0000\n0000",
        "3000\n0000\n0000\n0000",
        "2000\n1000\n3000\n0000",
    ] {
        assert!(
            Document::parse(
                format!("#BPMS:0=120;#NOTES:dance-single::Hard:7::\n{text};"),
                Format::Sm
            )
            .is_err()
        );
    }
}
#[test]
fn unsupported_charts_and_symbols_survive() {
    let raw = "#BPMS:0=120;#NOTEDATA:;#STEPSTYPE:pump-single;#DIFFICULTY:Hard;#METER:8;#NOTES:10000\n00000\n00000\n00000;#NOTEDATA:;#STEPSTYPE:dance-single;#DIFFICULTY:Hard;#METER:7;#NOTES:K000\n0000\n0000\n0000;";
    let doc = Document::parse(raw.into(), Format::Ssc).unwrap();
    assert!(doc.charts.iter().all(|c| c.read_only_reason.is_some()));
    assert_eq!(doc.export(Format::Ssc).unwrap(), raw);
}
#[test]
fn split_timing_is_complete_override() {
    let raw = "#OFFSET:0.1;#BPMS:0=120;#STOPS:2=1;#NOTEDATA:;#STEPSTYPE:dance-single;#DIFFICULTY:Edit;#METER:1;#BPMS:0=60;#NOTES:1000\n0000\n0000\n0000;";
    let doc = Document::parse(raw.into(), Format::Ssc).unwrap();
    let timing = doc.effective_timing("chart-1").unwrap();
    assert_eq!(timing.events.len(), 1);
    assert!((timing.seconds(4., Boundary::Note) - 3.9).abs() < 1e-8);
    assert!(doc.export(Format::Sm).is_err());
}
#[test]
fn offset_stop_delay_warp_boundaries() {
    let mut t = Timing {
        offset: 0.25,
        ..Default::default()
    };
    t.events.extend([
        Event::value(TimingKind::Delays, 2., 0.3),
        Event::value(TimingKind::Stops, 2., 0.7),
        Event::value(TimingKind::Warps, 4., 4.),
        Event::value(TimingKind::Bpms, 6., 240.),
    ]);
    assert!((t.seconds(0., Boundary::Note) + 0.25).abs() < 1e-8);
    assert!((t.seconds(2., Boundary::Before) - 0.75).abs() < 1e-8);
    assert!((t.seconds(2., Boundary::Note) - 1.05).abs() < 1e-8);
    assert!((t.seconds(2., Boundary::After) - 1.75).abs() < 1e-8);
    assert!((t.seconds(4., Boundary::Note) - 2.75).abs() < 1e-8);
    assert!((t.seconds(8., Boundary::Note) - 2.75).abs() < 1e-8);
    assert!((t.seconds(9., Boundary::Note) - 3.).abs() < 1e-8);
    assert!(!t.judgable(4.));
    assert!(!t.judgable(7.));
    assert!(t.judgable(8.));
    assert!(matches!(t.position(1.), AudioPosition::Delay { beat: 2. }));
    assert!(matches!(t.position(1.5), AudioPosition::Stop { beat: 2. }));
    assert!(matches!(
        t.position(2.75),
        AudioPosition::Warp { from: 4., to: 8. }
    ));
}
#[test]
fn overlapping_warps_and_stop_inside_warp() {
    let mut t = Timing::default();
    t.events.extend([
        Event::value(TimingKind::Warps, 2., 4.),
        Event::value(TimingKind::Warps, 4., 4.),
        Event::value(TimingKind::Stops, 5., 1.),
    ]);
    assert_eq!(t.seconds(8., Boundary::Note), 2.);
    assert!(t.judgable(5.));
    assert!(!t.judgable(6.));
}
#[test]
fn fake_scroll_and_speed_clocks() {
    let mut t = Timing::default();
    t.events.extend([
        Event::value(TimingKind::Fakes, 2., 2.),
        Event::value(TimingKind::Scrolls, 2., -1.),
        Event {
            kind: TimingKind::Speeds,
            beat: 0.,
            values: vec![2., 4., 0.],
            text: String::new(),
        },
    ]);
    assert!(!t.judgable(3.));
    assert!(t.judgable(4.));
    assert_eq!(t.display(4., 0., false), 0.);
    assert_eq!(t.display(4., 0., true), 4.);
    assert_eq!(t.display(1., 0.5, false), 0.5625);
}
#[test]
fn typed_edits_are_atomic_revision_bound_and_recoverable() {
    let mut p = Project::import(sm(), Format::Sm, Profile::Pad).unwrap();
    let before = p.encode().unwrap();
    assert!(
        edit(
            &mut p,
            Action::AddNote {
                chart_id: "chart-1".into(),
                row: 48,
                lane: 0,
                kind: NoteKind::Tap,
                end_row: None
            }
        )
        .is_err()
    );
    assert_eq!(p.encode().unwrap(), before);
    edit(
        &mut p,
        Action::Review {
            chart_id: "chart-1".into(),
            comment: "reviewed".into(),
        },
    )
    .unwrap();
    assert!(p.reviewed("chart-1"));
    edit(
        &mut p,
        Action::AddNote {
            chart_id: "chart-1".into(),
            row: 168,
            lane: 2,
            kind: NoteKind::Fake,
            end_row: None,
        },
    )
    .unwrap();
    assert!(!p.reviewed("chart-1"));
    assert!(
        p.apply(Request {
            expected_revision: 0,
            actor: "stale".into(),
            action: Action::Undo
        })
        .is_err()
    );
    edit(&mut p, Action::Undo).unwrap();
    assert!(p.reviewed("chart-1"));
    edit(&mut p, Action::Redo).unwrap();
    assert!(!p.reviewed("chart-1"));
    let restored = Project::decode(&p.encode().unwrap()).unwrap();
    assert_eq!(restored.content, p.content);
    assert_eq!(restored.revision, p.revision);
}
#[test]
fn unsupported_metadata_cannot_bypass_commands() {
    let mut p = Project::import(sm(), Format::Sm, Profile::Pad).unwrap();
    for key in ["NOTES", "OFFSET", "BPMS", "STEPSTYPE"] {
        assert!(
            edit(
                &mut p,
                Action::SetMetadata {
                    chart_id: None,
                    key: key.into(),
                    value: "oops".into()
                }
            )
            .is_err()
        );
    }
}
fn clicks() -> Vec<f32> {
    let mut pcm = vec![0.; 11025 * 16];
    for beat in 0..30 {
        let start = beat * 11025 / 2;
        for i in 0..100 {
            pcm[start + i] = (i as f32 * 0.73).sin() * (1. - i as f32 / 100.);
        }
    }
    pcm
}
#[test]
fn both_profiles_are_deterministic_and_cancel_without_edits() {
    for profile in [Profile::Pad, Profile::Keyboard] {
        let mut p = Project::blank(
            "Click".into(),
            "Test".into(),
            "song.wav".into(),
            120.,
            0.,
            profile,
        )
        .unwrap();
        let request = GenerationRequest {
            profile,
            ..Default::default()
        };
        let before = p.encode().unwrap();
        assert!(
            p.generate(0, "test".into(), &request, &clicks(), 11025, || true)
                .is_err()
        );
        assert_eq!(p.encode().unwrap(), before);
        let mut copy = p.clone();
        p.generate(0, "test".into(), &request, &clicks(), 11025, || false)
            .unwrap();
        copy.generate(0, "test".into(), &request, &clicks(), 11025, || false)
            .unwrap();
        assert_eq!(p.content, copy.content);
        assert_eq!(p.content.document.charts.len(), 5);
        for c in &p.content.document.charts {
            assert!(!c.notes.is_empty());
            c.validate().unwrap();
            if profile == Profile::Pad {
                assert!(
                    !footwork(c, &p.content.document.timing, &Default::default())
                        .warnings
                        .iter()
                        .any(|w| w.message.contains("two feet"))
                );
            }
        }
        Document::parse(p.content.document.export(Format::Ssc).unwrap(), Format::Ssc).unwrap();
    }
}
#[test]
fn foot_overrides_detect_three_foot_chords() {
    let raw = "#BPMS:0=120;#NOTES:dance-single::Hard:9::1110\n0000\n0000\n0000;";
    let p = Project::import(raw.into(), Format::Sm, Profile::Pad).unwrap();
    let chart = &p.content.document.charts[0];
    assert!(
        !footwork(chart, &p.content.document.timing, &Default::default())
            .warnings
            .is_empty()
    );
    let mut p = Project::import(sm(), Format::Sm, Profile::Pad).unwrap();
    edit(
        &mut p,
        Action::SetFoot {
            note_id: "chart-1-n1".into(),
            foot: Some(Foot::Right),
        },
    )
    .unwrap();
    assert_eq!(
        footwork(
            &p.content.document.charts[0],
            &p.content.document.timing,
            &p.content.foot_overrides
        )
        .assignments["chart-1-n1"],
        Foot::Right
    );
}
#[test]
fn backup_binds_assets_and_exports_both_profiles_separately() {
    let mut p = Project::blank(
        "Bundle".into(),
        "Test".into(),
        "song.wav".into(),
        120.,
        0.,
        Profile::Pad,
    )
    .unwrap();
    edit(
        &mut p,
        Action::AddChart {
            difficulty: Difficulty::Easy,
            meter: 2,
            description: "Pad".into(),
        },
    )
    .unwrap();
    edit(
        &mut p,
        Action::SetProfile {
            profile: Profile::Keyboard,
        },
    )
    .unwrap();
    edit(
        &mut p,
        Action::AddChart {
            difficulty: Difficulty::Easy,
            meter: 3,
            description: "Keyboard".into(),
        },
    )
    .unwrap();
    let source = bundle::Files::from([
        ("song.wav".into(), vec![1, 2, 3]),
        ("jacket.png".into(), vec![4, 5]),
    ]);
    let (restored, assets) = bundle::restore(bundle::backup(&p, &source).unwrap()).unwrap();
    assert_eq!(assets, source);
    assert_eq!(restored.assets_sha256.len(), 2);
    let files = bundle::export(&restored, &assets, Format::Ssc).unwrap();
    for (profile, description) in [("pad", "Pad"), ("keyboard", "Keyboard")] {
        let doc = Document::parse(
            String::from_utf8(files[&format!("YACE-{profile}/notes.ssc")].clone()).unwrap(),
            Format::Ssc,
        )
        .unwrap();
        assert_eq!(doc.charts.len(), 1);
        assert_eq!(doc.charts[0].description, description);
    }
    let mut changed = assets;
    changed.insert("song.wav".into(), vec![9]);
    assert!(bundle::export(&restored, &changed, Format::Ssc).is_err());
}
#[test]
fn advanced_timing_and_unchanged_unknown_tags_survive_edits() {
    let raw = "#VERSION:0.83;\n#TITLE:Timing;\n#BPMS:0=120;\n#SPEEDS:0=1=0=0,4=-1=2=1;\n#SCROLLS:0=1,8=0;\n#TIMESIGNATURES:0=3=4;\n#LABELS:0=Verse;\n#TICKCOUNTS:0=4;\n#COMBOS:0=1=2;\n#FAKES:8=2;\n#XUNKNOWN:exact\\:value;\n#NOTEDATA:;\n#STEPSTYPE:dance-single;\n#DIFFICULTY:Edit;\n#METER:5;\n#CREDIT:Keep me;\n#NOTES:1000\n0000\n0000\n0000;";
    let mut p = Project::import(raw.into(), Format::Ssc, Profile::Pad).unwrap();
    edit(
        &mut p,
        Action::AddNote {
            chart_id: "chart-1".into(),
            row: 48,
            lane: 1,
            kind: NoteKind::Tap,
            end_row: None,
        },
    )
    .unwrap();
    let text = p.content.document.export(Format::Ssc).unwrap();
    assert!(text.contains("#XUNKNOWN:exact\\:value;"));
    assert!(text.contains("#SPEEDS:0=1=0=0,4=-1=2=1;"));
    assert!(text.contains("#CREDIT:Keep me;"));
    let doc = Document::parse(text, Format::Ssc).unwrap();
    assert_eq!(doc.timing, p.content.document.timing);
    assert!(doc.export(Format::Sm).is_err());
}
#[test]
fn revision_and_hold_failures_never_change_source_or_history() {
    let mut p = Project::import(sm(), Format::Sm, Profile::Pad).unwrap();
    let old = p.encode().unwrap();
    let result = edit(
        &mut p,
        Action::SetTiming {
            chart_id: None,
            timing: Some(Timing {
                offset: f64::NAN,
                ..Default::default()
            }),
        },
    );
    assert!(result.is_err());
    assert_eq!(old, p.encode().unwrap());
    let result = edit(
        &mut p,
        Action::SetSong {
            title: "Changed".into(),
            artist: "Changed".into(),
            timing: Timing {
                offset: f64::INFINITY,
                ..Default::default()
            },
        },
    );
    assert!(result.is_err());
    assert_eq!(old, p.encode().unwrap());
}
#[test]
fn passage_editing_preserves_tails_ids_and_fails_atomically_on_collisions() {
    use yace_stepmania::project::SelectionEdit;
    let mut p = Project::import(sm(), Format::Sm, Profile::Pad).unwrap();
    let notes = p.content.document.charts[0].notes.clone();
    let ids = notes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
    edit(
        &mut p,
        Action::EditNotes {
            chart_id: "chart-1".into(),
            note_ids: ids.clone(),
            edit: SelectionEdit::Mirror,
        },
    )
    .unwrap();
    assert_eq!(p.content.document.charts[0].notes[0].lane, 3);
    assert_eq!(p.content.document.charts[0].notes[0].end_row, Some(96));
    edit(
        &mut p,
        Action::EditNotes {
            chart_id: "chart-1".into(),
            note_ids: ids,
            edit: SelectionEdit::Move { rows: 192 },
        },
    )
    .unwrap();
    assert_eq!(p.content.document.charts[0].notes[0].row, 192);
    assert_eq!(p.content.document.charts[0].notes[0].end_row, Some(288));
    edit(
        &mut p,
        Action::PasteNotes {
            chart_id: "chart-1".into(),
            notes: notes.clone(),
            row: 0,
        },
    )
    .unwrap();
    assert_eq!(p.content.document.charts[0].notes.len(), 4);
    assert!(
        p.content.document.charts[0]
            .notes
            .iter()
            .any(|n| n.id.starts_with("paste-"))
    );
    let before = p.encode().unwrap();
    assert!(
        edit(
            &mut p,
            Action::PasteNotes {
                chart_id: "chart-1".into(),
                notes,
                row: 0
            }
        )
        .is_err()
    );
    assert_eq!(before, p.encode().unwrap());
    edit(&mut p, Action::Undo).unwrap();
    assert_eq!(p.content.document.charts[0].notes.len(), 2);
}
#[test]
fn constant_scroll_uses_audio_clock_during_stops() {
    let mut t = Timing::default();
    t.events.push(Event::value(TimingKind::Stops, 2., 1.));
    assert_eq!(t.display_at(4., 2., 1.25, true), 3.5);
    assert_eq!(t.display_at(4., 2., 1.75, true), 2.5);
    assert_eq!(t.display_at(4., 2., 1.25, false), 2.);
    t.events.push(Event {
        kind: TimingKind::Speeds,
        beat: 2.,
        values: vec![2., 1., 1.],
        text: String::new(),
    });
    assert_eq!(t.display_at(4., 2., 1.25, false), 2.5);
    assert_eq!(t.display_at(4., 2., 1.75, false), 3.5);
}
#[test]
fn archive_entry_keeps_combined_projects_and_wrapped_backups_together() {
    use yace_stepmania::bundle::{self, Files, MANIFEST};
    let mut files = Files::from([
        ("Pack/Song/notes.SSC".into(), vec![]),
        (format!("Pack/Song/yace-history/{MANIFEST}"), vec![]),
    ]);
    assert_eq!(
        bundle::entry(&files).as_deref(),
        Some("Pack/Song/notes.SSC")
    );
    files.insert("Pack/Song/notes.chart".into(), vec![]);
    assert_eq!(bundle::entry(&files), None);
    let files = Files::from([
        (format!("Backup/{MANIFEST}"), vec![]),
        ("Backup/source/original.ssc".into(), vec![]),
    ]);
    assert_eq!(bundle::entry(&files), Some(format!("Backup/{MANIFEST}")));
    let files = Files::from([
        ("Backup/project.json".into(), vec![]),
        (format!("Backup/dance/{MANIFEST}"), vec![]),
    ]);
    assert_eq!(bundle::entry(&files), None);
}
