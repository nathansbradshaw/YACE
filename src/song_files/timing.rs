//! Normalize legacy song delay once into the editable chart clock.
use crate::{chart::Chart, song_metadata::ini_fields, storage as fs};
use anyhow::{Context, Result, ensure};
use std::path::Path;

#[cfg(test)]
#[path = "timing/tests.rs"]
mod tests;

/// Match YARG SongMetadata/IniModifierCollection: nonzero millisecond delay
/// takes precedence, then nonzero delay_seconds, then the chart's own Offset.
/// Reject malformed or out-of-range input instead of guessing an audio clock.
pub fn legacy_delay(ini: &str) -> Result<Option<f64>> {
    let fields = ini_fields(ini);
    let millis = fields
        .get("delay")
        .map(|s| s.parse::<f64>().context("Invalid legacy delay"))
        .transpose()?
        .unwrap_or(0.0);
    ensure!(
        millis.is_finite() && millis.fract() == 0.0 && millis.abs() <= i32::MAX as f64,
        "Legacy delay must be a finite integer number of milliseconds within the supported range"
    );
    let seconds = fields
        .get("delay_seconds")
        .map(|s| s.parse::<f64>().context("Invalid legacy delay_seconds"))
        .transpose()?
        .unwrap_or(0.0);
    ensure!(
        seconds.is_finite() && (seconds * 1000.0).abs() <= i32::MAX as f64,
        "Legacy delay_seconds must be finite and within the supported range"
    );
    Ok(if millis != 0.0 {
        Some(millis / 1000.0)
    } else if seconds != 0.0 {
        Some(seconds)
    } else {
        None
    })
}

pub fn apply_ini(chart: Chart, ini: &str) -> Result<Chart> {
    let Some(offset) = legacy_delay(ini)? else {
        return Ok(chart);
    };
    with_offset(chart, offset)
}

/// Replace the chart origin without changing any event tick or other section.
pub fn with_offset(chart: Chart, offset: f64) -> Result<Chart> {
    ensure!(offset.is_finite(), "Offset must be finite");
    if offset == chart.timing.offset {
        return Ok(chart);
    }
    let section = chart
        .sections
        .iter()
        .find(|s| s.name == "Song")
        .context("Missing Song section")?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut body = String::new();
    let mut found = false;
    for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
        if let Some((key, _)) = line.split_once('=')
            && key.trim() == "Offset"
        {
            found = true;
            body.push_str(&format!("{key}= {offset}"));
            if line.ends_with('\n') {
                body.push_str(newline);
            }
        } else {
            body.push_str(line);
        }
    }
    if !found {
        body.push_str(&format!("  Offset = {offset}{newline}"));
    }
    let mut raw = chart.raw.clone();
    raw.replace_range(section.body_start..section.body_end, &body);
    Chart::parse(raw)
}

/// Only unpublished copies are normalized; retain the exact original timing
/// witnesses there as well, without replacing earlier recovery files.
pub fn retain_original(stage: &Path, chart: &str, ini: &str) -> Result<()> {
    let parent = stage.join(".yace/originals");
    fs::create_dir_all(&parent)?;
    let mut folder = parent.join("legacy-timing");
    let mut index = 1u64;
    while fs::exists(&folder) {
        folder = parent.join(format!("legacy-timing-{index}"));
        index = index.checked_add(1).context("Too many timing archives")?;
    }
    fs::create_dir(&folder)?;
    fs::write(folder.join("notes.chart"), chart)?;
    fs::write(folder.join("song.ini"), ini)?;
    Ok(())
}
