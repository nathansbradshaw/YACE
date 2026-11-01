//! Shared source-time bounds for read-only diagnostics.
use anyhow::{Result, ensure};
pub fn validate_chart_events(
    chart: &crate::chart::Chart,
    events: Option<&crate::mapping::Transcription>,
    duration: f64,
) -> Result<()> {
    ensure!(
        duration.is_finite() && duration > 0.0,
        "Invalid audit duration"
    );
    for section in &chart.sections {
        if !["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
            ["Single", "DoubleBass", "Drums"]
                .iter()
                .any(|part| section.name == format!("{tier}{part}"))
        }) {
            continue;
        }
        for note in chart.notes(&section.name)? {
            let end = note
                .tick
                .checked_add(note.sustain)
                .ok_or_else(|| anyhow::anyhow!("Audit source note tick overflow"))?;
            let playable = note.lane <= 4
                || if section.name.ends_with("Drums") {
                    note.lane == 32
                } else {
                    note.lane == 7
                };
            ensure!(
                !playable
                    || (chart.timing.seconds(end).is_finite()
                        && chart.timing.seconds(end) <= duration + 0.1),
                "Audit source note extends beyond recording"
            );
        }
    }
    if let Some(events) = events {
        events.validate()?;
        ensure!(
            events.drums.iter().all(|n| n.time <= duration + 0.1)
                && events
                    .bass
                    .iter()
                    .chain(&events.guitar)
                    .all(|n| n.end <= duration + 0.1),
            "Audit transcription event extends beyond recording"
        );
    }
    Ok(())
}
