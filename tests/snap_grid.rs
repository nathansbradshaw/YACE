use chart_engine::{
    snap_grid::Grid,
    timing::{Tempo, TempoMap},
};

#[test]
fn integer_repair_policy_uses_the_musical_grid_and_keeps_distant_notes() {
    use chart_engine::snap_grid::RepairPolicy;
    let grid = Grid::new(7, 3).unwrap();
    assert_eq!(
        (0..=7)
            .map(|t| grid.snap_tick(t).unwrap())
            .collect::<Vec<_>>(),
        vec![0, 0, 2, 2, 5, 5, 7, 7]
    );
    let policy = RepairPolicy {
        divisions: 4,
        maximum_shift: Some(2),
    };
    let grid = policy.grid(480).unwrap();
    assert_eq!(policy.destination(grid, 242).unwrap(), 240);
    assert_eq!(policy.destination(grid, 243).unwrap(), 243);
    assert_eq!(
        RepairPolicy {
            maximum_shift: None,
            ..policy
        }
        .destination(grid, 243)
        .unwrap(),
        240
    );
    assert_eq!(
        Grid::new(480, 4).unwrap().snap_tick(9_007_199_254_740_993),
        Some(9_007_199_254_740_960)
    );
}

#[test]
fn dance_repair_previews_policy_preserves_hold_lengths_and_other_difficulties_with_exact_undo() {
    use yace_stepmania::{
        Action, Difficulty, NoteKind, Profile, Project, Request, project::SelectionEdit,
    };
    let apply = |p: &mut Project, action| {
        p.apply(Request {
            expected_revision: p.revision,
            actor: "Repair regression".into(),
            action,
        })
    };
    let mut p = Project::blank(
        "Repair".into(),
        "Fixture".into(),
        String::new(),
        120.,
        0.,
        Profile::Pad,
    )
    .unwrap();
    for description in ["First", "Other"] {
        apply(
            &mut p,
            Action::AddChart {
                difficulty: Difficulty::Edit,
                meter: 1,
                description: description.into(),
            },
        )
        .unwrap();
    }
    for (row, lane, kind, end_row) in [
        (13, 0, NoteKind::Hold, Some(20)),
        (18, 1, NoteKind::Mine, None),
        (48, 2, NoteKind::Tap, None),
    ] {
        apply(
            &mut p,
            Action::AddNote {
                chart_id: "added-1".into(),
                row,
                lane,
                kind,
                end_row,
            },
        )
        .unwrap();
    }
    let ids = p.content.document.charts[0]
        .notes
        .iter()
        .map(|n| n.id.clone())
        .collect::<Vec<_>>();
    let before = serde_json::to_vec(&p).unwrap();
    let old_content = serde_json::to_value(&p.content).unwrap();
    let policy = chart_engine::snap_grid::RepairPolicy {
        divisions: 4,
        maximum_shift: Some(2),
    };
    let report = p.preview_timing_repair("added-1", &ids, policy).unwrap();
    assert_eq!(
        (report.changed, report.already_on_grid, report.kept_off_grid),
        (1, 1, 1)
    );
    assert_eq!(serde_json::to_vec(&p).unwrap(), before);
    apply(
        &mut p,
        Action::EditNotes {
            chart_id: "added-1".into(),
            note_ids: ids.clone(),
            edit: SelectionEdit::Repair { policy },
        },
    )
    .unwrap();
    let repaired = serde_json::to_value(&p.content).unwrap();
    let hold = p.content.document.charts[0]
        .notes
        .iter()
        .find(|n| n.lane == 0)
        .unwrap();
    assert_eq!((hold.row, hold.end_row), (12, Some(19)));
    assert_eq!(
        repaired["document"]["charts"][1],
        old_content["document"]["charts"][1]
    );
    apply(&mut p, Action::Undo).unwrap();
    assert_eq!(serde_json::to_value(&p.content).unwrap(), old_content);
    apply(&mut p, Action::Redo).unwrap();
    assert_eq!(serde_json::to_value(&p.content).unwrap(), repaired);
    let stable = serde_json::to_vec(&p).unwrap();
    assert_eq!(
        p.preview_timing_repair("added-1", &ids, policy)
            .unwrap()
            .changed,
        0
    );
    assert!(
        apply(
            &mut p,
            Action::EditNotes {
                chart_id: "added-1".into(),
                note_ids: ids,
                edit: SelectionEdit::Repair { policy }
            }
        )
        .is_err()
    );
    assert_eq!(serde_json::to_vec(&p).unwrap(), stable);
}

#[test]
fn unusual_ppq_keeps_lines_placement_and_navigation_on_the_same_grid() {
    for resolution in [1, 7, 47, 100, 480, 960] {
        for divisions in [1, 3, 5, 7, 48, 64, 96, 192, 1024] {
            let grid = Grid::new(resolution, divisions).unwrap();
            let ticks = grid.visible(0, resolution as u64 * 4, 5000);
            assert!(!ticks.is_empty());
            for &tick in &ticks {
                assert_eq!(grid.snap(tick as f64), Some(tick));
            }
            for pair in ticks.windows(2) {
                assert_eq!(grid.adjacent(pair[0], true), Some(pair[1]));
                assert_eq!(grid.adjacent(pair[1], false), Some(pair[0]));
            }
        }
    }
}

#[test]
fn fractional_pointer_chooses_musical_grid_before_tick_rounding() {
    let grid = Grid::new(7, 3).unwrap();
    // Rounding 1.2 to tick 1 first would incorrectly choose tick 0.
    assert_eq!(grid.snap(1.2), Some(2));
    assert_eq!(grid.visible(0, 7, 100), vec![0, 2, 5, 7]);
    let map = TempoMap::new(
        7,
        0.25,
        vec![Tempo { tick: 0, bpm: 120. }, Tempo { tick: 7, bpm: 60. }],
    )
    .unwrap();
    assert_eq!(
        grid.snap(map.tick_position(0.25 + 1.2 / 14.).unwrap()),
        Some(2)
    );
    assert_eq!(
        grid.snap(map.tick_position(0.75 + 1.2 / 7.).unwrap()),
        Some(9)
    );
    assert!(map.tick_position(0.24).is_none());
}

#[test]
fn invalid_and_extreme_grids_are_bounded_without_overflow_or_zero_step() {
    assert!(Grid::new(0, 4).is_err());
    assert!(Grid::new(480, 0).is_err());
    assert!(Grid::new(480, 1025).is_err());
    let grid = Grid::new(1, 1024).unwrap();
    assert_eq!(grid.adjacent(4, true), Some(5));
    assert_eq!(grid.adjacent(4, false), Some(3));
    assert!(grid.adjacent(0, false).is_none());
    assert!(grid.snap(f64::NAN).is_none());
    assert!(grid.snap(-1.).is_none());
    assert!(grid.visible(0, u64::MAX, 2048).len() <= 2048);
    assert!(grid.visible(0, 100, 0).is_empty());
}

#[test]
fn dance_quantize_uses_the_same_rounded_grid_and_preserves_atomic_history() {
    use yace_stepmania::{
        Action, Difficulty, Format, NoteKind, Profile, Project, Request, project::SelectionEdit,
    };
    let apply = |p: &mut Project, action| {
        p.apply(Request {
            expected_revision: p.revision,
            actor: "grid regression".into(),
            action,
        })
    };
    let mut original = Project::blank(
        "Grid".into(),
        "Test".into(),
        "song.ogg".into(),
        120.,
        0.,
        Profile::Pad,
    )
    .unwrap();
    apply(
        &mut original,
        Action::AddChart {
            difficulty: Difficulty::Edit,
            meter: 1,
            description: "Grid".into(),
        },
    )
    .unwrap();
    apply(
        &mut original,
        Action::AddNote {
            chart_id: "added-1".into(),
            row: 10,
            lane: 0,
            kind: NoteKind::Hold,
            end_row: Some(20),
        },
    )
    .unwrap();
    let id = original.content.document.charts[0].notes[0].id.clone();
    for divisions in [5, 7, 48, 64, 192, 1024] {
        let mut p = original.clone();
        let before = p.content.clone();
        apply(
            &mut p,
            Action::EditNotes {
                chart_id: "added-1".into(),
                note_ids: vec![id.clone()],
                edit: SelectionEdit::QuantizeDivisions { divisions },
            },
        )
        .unwrap();
        let grid = Grid::new(48, divisions).unwrap();
        let note = &p.content.document.charts[0].notes[0];
        assert_eq!(note.row, grid.snap(10.).unwrap() as i64);
        assert_eq!(note.end_row, Some(grid.snap(20.).unwrap() as i64));
        assert_eq!(note.id, id);
        let accepted = p.content.clone();
        let raw = p.content.document.export(Format::Sm).unwrap();
        let reopened = yace_stepmania::Document::parse(raw, Format::Sm).unwrap();
        assert_eq!(reopened.charts[0].notes[0].row, note.row);
        assert_eq!(reopened.charts[0].notes[0].end_row, note.end_row);
        apply(&mut p, Action::Undo).unwrap();
        assert_eq!(p.content, before);
        apply(&mut p, Action::Redo).unwrap();
        assert_eq!(p.content, accepted);
    }
    for divisions in [0, 1, 1025] {
        let mut p = original.clone();
        let before = p.encode().unwrap();
        // One step per beat collapses this short hold; refuse atomically.
        assert!(
            apply(
                &mut p,
                Action::EditNotes {
                    chart_id: "added-1".into(),
                    note_ids: vec![id.clone()],
                    edit: SelectionEdit::QuantizeDivisions { divisions }
                }
            )
            .is_err()
        );
        assert_eq!(p.encode().unwrap(), before);
    }
}
