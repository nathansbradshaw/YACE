//! Shared song import, source-copy and sidecar layout rules.
use crate::{chart::Chart, models, song_layout, song_metadata::song_ini, storage as fs};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
#[path = "song_files/game_midi.rs"]
pub mod game_midi;
#[path = "song_files/source_choice.rs"]
pub mod source_choice;
#[path = "song_files/timing.rs"]
pub mod timing;
/// Prepare an unpublished editing snapshot using the same rules on both clients.
pub fn prepare_editable_source(song: &Path, dest: &Path) -> Result<()> {
    prepare_editable_source_with_choice(song, dest, None)
}
pub fn prepare_editable_source_with_choice(
    song: &Path,
    dest: &Path,
    choice: Option<source_choice::Source>,
) -> Result<()> {
    source_choice::selected_file(song, choice)?;
    let multiple = source_choice::ambiguous(song);
    copy_song_source(song, dest)?;
    organize_song_stage(dest)?;
    if multiple {
        source_choice::retain_alternate(dest, choice.context("Choose the source to edit")?)?;
    }
    crate::stem_review::game_audio::restore_recording(dest)?;
    prepare_copied_source(dest)
}
fn prepare_copied_source(song: &Path) -> Result<()> {
    validate_existing(song)?;
    let selected = chart_file(song)?;
    let working = song_layout::resolve(song, "chart-engine-working");
    if selected.extension().is_some_and(|e| e == "mid") && !fs::exists(working.join("binding.json"))
    {
        ensure!(
            !fs::exists(working.join("notes.chart")),
            "Incomplete YACE editable-master binding; recover the original project rather than replacing its edits"
        );
        game_midi::prepare(song)?;
        return Ok(());
    }
    let master = editable_chart_file(song)?;
    let ini = song_ini(song)?;
    let legacy_chart = master == song.join("notes.chart") && timing::legacy_delay(&ini)?.is_some();
    let original = legacy_chart
        .then(|| fs::read_to_string(&master))
        .transpose()?;
    let normalized = original
        .as_ref()
        .map(|raw| timing::apply_ini(Chart::parse(raw.clone())?, &ini).map(|chart| chart.raw))
        .transpose()?;
    if master != song.join("notes.chart") {
        fs::rename(
            song.join("notes.mid"),
            song_layout::path(song, "chart-engine-working").join("game-notes.mid"),
        )?;
        fs::copy(&master, song.join("notes.chart"))?;
        fs::write(
            song.join("song.ini"),
            crate::game_export::ini_with_delay(&song_ini(song)?, 0),
        )?;
    }
    if let Some(raw) = original {
        timing::retain_original(song, &raw, &ini)?;
        fs::write(
            song.join("notes.chart"),
            normalized.context("Missing normalized chart")?,
        )?;
        fs::write(
            song.join("song.ini"),
            crate::game_export::ini_with_delay(&ini, 0),
        )?;
    }
    Ok(())
}

pub fn chart_file(folder: &Path) -> Result<PathBuf> {
    source_choice::selected_file(folder, None)
}

/// A generated MIDI song has one authoritative game MIDI and a separately
/// archived editable master. Refuse mismatched pairs rather than edit stale data.
pub fn editable_chart_file(folder: &Path) -> Result<PathBuf> {
    let chart = chart_file(folder)?;
    if chart.extension().is_some_and(|e| e == "chart") {
        return Ok(chart);
    }
    let master = song_layout::resolve(folder, "chart-engine-working").join("notes.chart");
    let binding: serde_json::Value = serde_json::from_slice(
        &fs::read(song_layout::resolve(folder, "chart-engine-working").join("binding.json"))
            .context("MIDI-only songs require a YACE editable-master binding")?,
    )?;
    ensure!(
        binding["version"] == 1
            && binding["master_sha256"].as_str() == Some(&models::hash_file(&master)?)
            && binding["midi_sha256"].as_str() == Some(&models::hash_file(&chart)?),
        "Editable master and game MIDI no longer match; open the original YACE project to preserve edits"
    );
    Ok(master)
}

pub fn validate_existing(folder: &Path) -> Result<()> {
    source_choice::validate_retained(folder)?;
    let delay = timing::legacy_delay(&song_ini(folder)?)?.unwrap_or(0.0);
    let binding = song_layout::resolve(folder, "chart-engine-working").join("binding.json");
    if fs::is_file(folder.join("notes.mid")) && fs::exists(&binding) {
        // Verify the pair before trusting its original chart offset. Unbound
        // legacy MIDI delays still have no editable timing witness.
        let master = editable_chart_file(folder)?;
        let chart = Chart::parse(fs::read_to_string(master)?)?;
        ensure!(
            (delay * 1000.0 - crate::game_export::chart_delay(&chart)? as f64).abs() < 0.000001,
            "Game MIDI delay does not match its bound editable master"
        );
    }
    Ok(())
}

pub fn copy_song_source(source: &Path, dest: &Path) -> Result<()> {
    fn copy(source: &Path, dest: &Path, private: bool) -> Result<()> {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            let text = name.to_string_lossy();
            if private
                && (text.starts_with(".tmp")
                    || text.starts_with(".yace-project-")
                    || matches!(
                        text.as_ref(),
                        "project" | "projects" | "jobs" | "cache" | "logs"
                    ))
            {
                continue;
            }
            let kind = entry.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Song source contains a symlink: {}",
                entry.path().display()
            );
            let output = dest.join(&name);
            if kind.is_dir() {
                copy(&entry.path(), &output, text == song_layout::DIRECTORY)?;
            } else {
                ensure!(kind.is_file(), "Unsupported song source entry");
                fs::copy(entry.path(), output)?;
            }
        }
        Ok(())
    }
    copy(source, dest, false)
}

/// Consolidate copied legacy YACE files in an unpublished song staging folder.
/// Existing source folders are never changed by this operation.
pub fn organize_song_stage(stage: &Path) -> Result<()> {
    song_layout::prepare(stage)?;
    let entries = fs::read_dir(stage)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = match name.as_str() {
            "stems" => PathBuf::from(".yace/audio/stems"),
            ".yace-audio" => PathBuf::from(".yace/originals/audio"),
            value if value == "editor-source" || value.starts_with("editor-source-") => {
                Path::new(".yace/history").join(value)
            }
            _ => song_layout::relative(&name),
        };
        if relative == Path::new(&name) {
            continue;
        }
        let mut target = stage.join(relative);
        fs::create_dir_all(target.parent().context("Sidecar needs a parent")?)?;
        if fs::is_dir(&target) && fs::read_dir(&target)?.next().is_none() {
            fs::remove_dir(&target)?;
        }
        if fs::exists(&target) {
            if fs::is_file(&target)
                && entry.file_type()?.is_file()
                && models::hash_file(&target)? == models::hash_file(&entry.path())?
            {
                fs::remove_file(entry.path())?;
                continue;
            }
            let history = stage.join(".yace/history/legacy");
            fs::create_dir_all(&history)?;
            target = history.join(&name);
            let mut number = 1;
            while fs::exists(&target) {
                target = history.join(format!("{name}-{number}"));
                number += 1;
            }
        }
        fs::rename(entry.path(), target)?;
    }
    Ok(())
}
