use chart_engine::timeline::{TimelineTransform, lane_rows};

#[test]
fn audio_notes_flags_and_seeks_share_the_same_gutter_and_zoom() {
    // These independent drawing surfaces reserve the same label gutter.
    for (panel_left, gutter, width) in [(0.0, 62.0, 900.0), (278.0, 90.0, 517.5)] {
        for (start, span) in [(-0.5, 4.0), (0.0, 12.0), (83.25, 32.0), (215.0, 0.125)] {
            let audio = TimelineTransform::new(start, span, panel_left + gutter, width).unwrap();
            let notes = TimelineTransform::new(start, span, panel_left + gutter, width).unwrap();
            let flags = TimelineTransform::new(start, span, panel_left + gutter, width).unwrap();
            for fraction in [0.0, 0.125, 0.5, 0.875, 1.0] {
                let seconds = start + fraction * span;
                let expected_x = panel_left + gutter + fraction as f32 * width;
                assert!((audio.x(seconds) - expected_x).abs() <= 0.0002);
                assert_eq!(audio.x(seconds), notes.x(seconds));
                assert_eq!(audio.x(seconds), flags.x(seconds));
                assert!((audio.seconds(notes.x(seconds)) - seconds).abs() < span * 0.000_001);
            }
        }
    }
}

#[test]
fn inverse_is_unclamped_for_partially_visible_notes_and_pointer_positions() {
    let transform = TimelineTransform::new(40.0, 8.0, 62.0, 800.0).unwrap();
    assert_eq!(transform.x(39.0), -38.0);
    assert_eq!(transform.x(49.0), 962.0);
    assert_eq!(transform.seconds(-38.0), 39.0);
    assert_eq!(transform.seconds(962.0), 49.0);
}

#[test]
fn rejects_nonfinite_nonpositive_and_unrepresentable_coordinate_ranges() {
    for start in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(TimelineTransform::new(start, 12.0, 62.0, 800.0).is_err());
    }
    for span in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(TimelineTransform::new(0.0, span, 62.0, 800.0).is_err());
    }
    for left in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(TimelineTransform::new(0.0, 12.0, left, 800.0).is_err());
    }
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(TimelineTransform::new(0.0, 12.0, 62.0, width).is_err());
    }
    assert!(TimelineTransform::new(f64::MAX, f64::MAX, 0.0, 800.0).is_err());
    assert!(TimelineTransform::new(f64::MAX, 1.0, 0.0, 800.0).is_err());
    assert!(TimelineTransform::new(0.0, 12.0, f32::MAX, f32::MAX).is_err());
    assert!(TimelineTransform::new(0.0, 12.0, f32::MAX, 1.0).is_err());
}

#[test]
fn drum_rows_and_open_guitar_bass_rows_remain_distinct() {
    for difficulty in ["Easy", "Medium", "Hard", "Expert"] {
        assert_eq!(lane_rows(&format!("{difficulty}Drums")), [0, 1, 2, 3, 4]);
        for instrument in ["Single", "DoubleBass"] {
            let rows = lane_rows(&format!("{difficulty}{instrument}"));
            assert_eq!(rows, [0, 1, 2, 3, 4, 7]);
            assert_ne!(
                rows.iter().position(|lane| *lane == 4),
                rows.iter().position(|lane| *lane == 7)
            );
        }
    }
}
