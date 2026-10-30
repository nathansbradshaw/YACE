//! One horizontal coordinate system for waveform, notes, review flags and seeks.
//!
//! Start coordinates after the label gutter. Leave results unclamped so the
//! caller can clip partial notes and set its own seek limits.
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug)]
pub struct TimelineTransform {
    start_seconds: f64,
    span_seconds: f64,
    left: f64,
    width: f64,
}

impl TimelineTransform {
    pub fn new(start_seconds: f64, span_seconds: f64, left: f32, width: f32) -> Result<Self> {
        ensure!(start_seconds.is_finite(), "Timeline start must be finite");
        ensure!(
            span_seconds.is_finite() && span_seconds > 0.0,
            "Timeline span must be finite and positive"
        );
        ensure!(left.is_finite(), "Timeline left edge must be finite");
        ensure!(
            width.is_finite() && width > 0.0,
            "Timeline width must be finite and positive"
        );
        let end_seconds = start_seconds + span_seconds;
        ensure!(
            end_seconds.is_finite() && end_seconds > start_seconds,
            "Timeline time range is not representable"
        );
        let right = (f64::from(left) + f64::from(width)) as f32;
        ensure!(
            right.is_finite() && right > left,
            "Timeline pixel range is not representable"
        );
        Ok(Self {
            start_seconds,
            span_seconds,
            left: f64::from(left),
            width: f64::from(width),
        })
    }

    /// Convert recording seconds into the shared drawable area's x coordinate.
    pub fn x(&self, seconds: f64) -> f32 {
        (self.left + (seconds - self.start_seconds) / self.span_seconds * self.width) as f32
    }

    /// Convert a pointer's x coordinate back into recording seconds.
    pub fn seconds(&self, x: f32) -> f64 {
        self.start_seconds + (f64::from(x) - self.left) / self.width * self.span_seconds
    }
}

/// Editable row order for the editor's currently supported instrument tracks.
///
/// Guitar and bass have a distinct open-note row (lane 7); drums have five rows.
/// Marker lanes and double-kick lane 32 are not editable rows in this editor.
/// Rows run from top to bottom; Pro Keys puts higher pitches first.
pub fn lane_rows(track: &str) -> Vec<u8> {
    if crate::keys::is_pro(track) {
        (crate::keys::MIN_PITCH..=crate::keys::MAX_PITCH)
            .rev()
            .collect()
    } else if track.ends_with("Drums") || track.ends_with("Keyboard") {
        vec![0, 1, 2, 3, 4]
    } else {
        vec![0, 1, 2, 3, 4, 7]
    }
}
