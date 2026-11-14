//! Portable simfile authoring. Audio, files and UI belong to the host client.
pub mod codec;
pub mod generation;
pub mod grid;
pub mod project;
pub mod timing;
pub use codec::{Chart, Difficulty, Document, Format, Note, NoteKind};
pub use generation::{GenerationRequest, Profile};
pub use project::{Action, Project, Request};
pub use timing::{Event, Timing, TimingKind};
pub const ROWS_PER_BEAT: i64 = 48;
pub const MAX_ROW: i64 = 100_000_000;
pub mod bundle;
/// Match the engine's note-row precision, rejecting nonfinite or negative
/// input before a float-to-integer cast can turn it into beat zero.
pub fn row_from_beat(beat: f64) -> anyhow::Result<i64> {
    anyhow::ensure!(
        beat.is_finite() && (0. ..=MAX_ROW as f64 / 48.).contains(&beat),
        "Note beat is outside the chart"
    );
    Ok((beat * 48.).round() as i64)
}
