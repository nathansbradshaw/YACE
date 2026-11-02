//! Explicit selection of one game-chart source, with both originals retained.
use crate::{models, song_layout, storage as fs};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RECEIPT: &str = "chart-engine-source-choice.json";
#[cfg(test)]
#[path = "source_choice/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Chart,
    Midi,
}
impl Source {
    pub fn filename(self) -> &'static str {
        match self {
            Self::Chart => "notes.chart",
            Self::Midi => "notes.mid",
        }
    }
}
impl std::str::FromStr for Source {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "chart" => Ok(Self::Chart),
            "midi" => Ok(Self::Midi),
            _ => Err("Choose chart or midi".into()),
        }
    }
}
pub fn ambiguous(song: &Path) -> bool {
    fs::is_file(song.join(Source::Chart.filename()))
        && fs::is_file(song.join(Source::Midi.filename()))
}
pub fn selected_file(song: &Path, source: Option<Source>) -> Result<PathBuf> {
    if let Some(source) = source {
        let path = song.join(source.filename());
        ensure!(
            fs::is_file(&path),
            "Chosen {} is missing",
            source.filename()
        );
        return Ok(path);
    }
    ensure!(
        !ambiguous(song),
        "Both notes.chart and notes.mid exist; choose the source to edit"
    );
    for source in [Source::Chart, Source::Midi] {
        let path = song.join(source.filename());
        if fs::is_file(&path) {
            return Ok(path);
        }
    }
    anyhow::bail!("No notes.chart or notes.mid in {}", song.display())
}

/// Include alternate-source provenance in browser recovery identity. A song
/// gaining a second chart must not silently reopen a pre-choice editing copy.
pub fn recovery_identity(song: &Path, source: Option<Source>) -> Result<Option<String>> {
    if ambiguous(song) {
        let selected = source.context("Choose the source to edit")?;
        return Ok(Some(format!(
            "chart-source-choice-v1:{}:{}:{}",
            selected.filename(),
            models::hash_file(&song.join("notes.chart"))?,
            models::hash_file(&song.join("notes.mid"))?
        )));
    }
    let receipt = song_layout::resolve(song, RECEIPT);
    if fs::exists(&receipt) {
        validate_retained(song)?;
        return Ok(Some(format!(
            "retained-chart-source-choice-v1:{}",
            models::hash_file(&receipt)?
        )));
    }
    Ok(None)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    selected: Source,
    archive_index: u32,
    chart_sha256: String,
    midi_sha256: String,
}
fn archive(song: &Path, index: u32) -> PathBuf {
    song.join(".yace/originals").join(if index == 0 {
        "chart-source".into()
    } else {
        format!("chart-source-{index}")
    })
}
pub fn validate_retained(song: &Path) -> Result<()> {
    let path = song_layout::resolve(song, RECEIPT);
    if !fs::exists(&path) {
        return Ok(());
    }
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        receipt.version == 1,
        "Unsupported chart source selection version"
    );
    let original = archive(song, receipt.archive_index);
    ensure!(
        models::hash_file(&original.join("notes.chart"))? == receipt.chart_sha256
            && models::hash_file(&original.join("notes.mid"))? == receipt.midi_sha256,
        "Retained chart/MIDI originals changed; recover the original project before exporting"
    );
    Ok(())
}
pub fn recorded_source(song: &Path) -> Result<Option<Source>> {
    validate_retained(song)?;
    let path = song_layout::resolve(song, RECEIPT);
    if !fs::exists(&path) {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice::<Receipt>(&fs::read(path)?)?.selected,
    ))
}

/// Only mutate an unpublished copied song. The user-selected source remains at
/// the game-standard root name; the alternate cannot collide on later reopen.
pub(super) fn retain_alternate(song: &Path, selected: Source) -> Result<()> {
    ensure!(
        ambiguous(song),
        "Source selection requires both chart formats"
    );
    validate_retained(song)?;
    let mut index = 0u32;
    while fs::exists(archive(song, index)) {
        index = index
            .checked_add(1)
            .context("Too many source-choice archives")?;
    }
    let directory = archive(song, index);
    fs::create_dir_all(&directory)?;
    let receipt_path = song_layout::resolve(song, RECEIPT);
    if fs::exists(&receipt_path) {
        fs::rename(receipt_path, directory.join("previous-choice.json"))?;
    }
    for source in [Source::Chart, Source::Midi] {
        let path = song.join(source.filename());
        let original = directory.join(source.filename());
        if source == selected {
            fs::copy(path, original)?;
        } else {
            fs::rename(path, original)?;
        }
    }
    if fs::is_file(song.join("song.ini")) {
        fs::copy(song.join("song.ini"), directory.join("song.ini"))?;
    }
    let receipt = Receipt {
        version: 1,
        selected,
        archive_index: index,
        chart_sha256: models::hash_file(&directory.join("notes.chart"))?,
        midi_sha256: models::hash_file(&directory.join("notes.mid"))?,
    };
    fs::write(
        song_layout::path(song, RECEIPT),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}
