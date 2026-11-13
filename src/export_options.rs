//! Export a chosen playback copy while keeping the editing project complete.
use crate::{
    chart::Chart,
    editor::{Project, Store},
    storage as fs,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    #[default]
    Automatic,
    Chart,
    Midi,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    #[default]
    Preserve,
    Yarg,
    CloneHero,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub target: Target,
    pub format: Format,
    /// None preserves all parts; Some is an explicit export selection.
    pub tracks: Option<BTreeSet<String>>,
    pub include_audio: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            target: Target::Preserve,
            format: Format::Automatic,
            tracks: None,
            include_audio: true,
        }
    }
}
pub fn is_track(name: &str) -> bool {
    ["Easy", "Medium", "Hard", "Expert"]
        .iter()
        .any(|tier| name.starts_with(tier))
}
pub fn available(store: &Store, doc: &Project) -> Result<BTreeSet<String>> {
    let chart = Chart::parse(doc.chart_text(store)?)?;
    let mut tracks: BTreeSet<_> = chart
        .sections
        .iter()
        .filter(|s| is_track(&s.name))
        .map(|s| s.name.clone())
        .collect();
    // Local mix text can own a difficulty even when it has no playable notes.
    // Keep that source control selectable so the default export cannot omit it.
    if let Ok(source) = store.source_drum_mix(doc)
        && source.midi
        && source.error.is_none()
        && let Some(state) = doc
            .content
            .midi_drum_mix
            .as_ref()
            .map(|s| &s.state)
            .or(source.physical.as_ref())
    {
        tracks.extend(
            state
                .events
                .iter()
                .map(|e| crate::drum_ranges::TRACKS[usize::from(e.mix.difficulty)].to_owned()),
        );
    }
    if doc.content.vocals.is_some()
        || crate::song_files::game_midi::retained_vocal_controls(&store.root.join("source"))?
    {
        tracks.insert("Vocals".into());
    }
    if doc.content.harmonies.is_some()
        || crate::song_files::game_midi::retained_harmony_controls(&store.root.join("source"))?
    {
        tracks.insert("Harmonies".into());
    }
    let mut owners = crate::song_files::game_midi::percussion_owners(
        &store.root.join("source"),
        doc.content.vocal_percussion.as_ref(),
    )?;
    if let Some(state) = &doc.content.vocal_overdrive {
        owners.extend(state.tracks.keys().copied());
    }
    if owners.contains(&crate::vocal_percussion::Part::Lead) {
        tracks.insert("Vocals".into());
    }
    if owners.contains(&crate::vocal_percussion::Part::Harm1) {
        tracks.insert("Harmonies".into());
    }
    tracks.extend(
        crate::song_files::game_midi::extra_parts(&store.root.join("source"))?
            .into_iter()
            .filter(|key| {
                crate::song_files::game_midi::logical_vocal_owner(
                    key.strip_prefix("MIDI: "),
                    &owners,
                )
                .is_none()
            }),
    );
    Ok(tracks)
}
impl Options {
    pub fn contains(&self, track: &str) -> bool {
        self.tracks
            .as_ref()
            .is_none_or(|tracks| tracks.contains(track))
    }
    pub fn selected(&self, store: &Store, doc: &Project) -> Result<BTreeSet<String>> {
        self.validate_for(
            doc,
            &available(store, doc)?,
            crate::song_files::game_midi::exists(&store.root.join("source")),
        )
    }
    pub fn validate_for(
        &self,
        doc: &Project,
        available: &BTreeSet<String>,
        imported_midi: bool,
    ) -> Result<BTreeSet<String>> {
        ensure!(
            self.format != Format::Chart || doc.content.midi_lane_ranges.is_none(),
            "Physical MIDI lane markers require MIDI export; use a project backup for editable history"
        );
        let selected = self.validate(available, imported_midi)?;
        ensure!(
            self.format != Format::Chart || !crate::bre::midi_required(doc, &selected),
            "Guitar/keys BRE requires MIDI; choose Automatic or MIDI"
        );
        Ok(selected)
    }
    pub fn validate(
        &self,
        available: &BTreeSet<String>,
        imported_midi: bool,
    ) -> Result<BTreeSet<String>> {
        let selected = self.tracks.clone().unwrap_or_else(|| available.clone());
        ensure!(
            !selected.is_empty(),
            "Choose at least one track or vocal part"
        );
        ensure!(
            selected.is_subset(available),
            "The available parts changed; reopen Export"
        );
        if self.target == Target::CloneHero {
            ensure!(
                !selected
                    .iter()
                    .any(|t| matches!(t.as_str(), "Vocals" | "Harmonies")
                        || crate::keys::is_pro(t)
                        || crate::pro_guitar::is_track(t)
                        || t.starts_with("MIDI: ")),
                "For Clone Hero, deselect vocals, harmonies, Pro Keys, Pro Guitar, Pro Bass and other original MIDI parts, or choose Preserve all parts"
            );
        }
        if self.format == Format::Chart {
            ensure!(
                !selected
                    .iter()
                    .any(|t| matches!(t.as_str(), "Vocals" | "Harmonies")
                        || crate::keys::is_pro(t)
                        || crate::pro_guitar::is_track(t)),
                "Vocals, harmonies, Pro Keys, Pro Guitar and Pro Bass require MIDI; choose Automatic or MIDI"
            );
            ensure!(
                !imported_midi,
                "Imported MIDI can contain events that .chart cannot preserve; choose Automatic or MIDI"
            );
        }
        Ok(selected)
    }
    pub fn midi(&self, store: &Store, doc: &Project) -> Result<bool> {
        let selected = self.selected(store, doc)?;
        Ok(crate::bre::midi_required(doc, &selected)
            || self.use_midi(
                &selected,
                crate::song_files::game_midi::exists(&store.root.join("source")),
            ))
    }
    pub fn use_midi(&self, selected: &BTreeSet<String>, imported_midi: bool) -> bool {
        match self.format {
            Format::Chart => false,
            Format::Midi => true,
            Format::Automatic => {
                selected.iter().any(|t| {
                    matches!(t.as_str(), "Vocals" | "Harmonies")
                        || crate::keys::is_pro(t)
                        || crate::pro_guitar::is_track(t)
                }) || imported_midi
            }
        }
    }
    pub fn chart(&self, text: String) -> Result<String> {
        let chart = Chart::parse(text)?;
        let mut output = chart.raw.clone();
        for section in chart.sections.iter().rev() {
            if is_track(&section.name) && !self.contains(&section.name) {
                let start = chart.raw[..section.body_start]
                    .rfind(&format!("[{}]", section.name))
                    .expect("parsed section header");
                let end = chart.raw[section.body_end..]
                    .find('\n')
                    .map_or(chart.raw.len(), |n| section.body_end + n + 1);
                output.replace_range(start..end, "");
            }
        }
        Ok(output)
    }
}

/// Use lengths only: estimating a download must not decode or copy recordings.
pub fn source_size(store: &Store, doc: &Project, audio: bool) -> Result<u64> {
    fn size(path: &std::path::Path, audio: bool) -> Result<u64> {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            ensure!(!kind.is_symlink(), "Export source contains a symbolic link");
            if kind.is_dir() {
                total += size(&entry.path(), audio)?;
            } else if audio || !is_audio(&entry.path()) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    total += entry.metadata()?.len();
                }
                #[cfg(target_arch = "wasm32")]
                {
                    total += fs::shared(entry.path())?.len() as u64;
                }
            }
        }
        Ok(total)
    }
    let mut total = size(&store.root.join("source"), audio)?;
    if audio && doc.has_recording() && doc.recording.is_some() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            total += fs::metadata(store.audio_path(doc)?)?.len();
        }
        #[cfg(target_arch = "wasm32")]
        {
            total += fs::shared(store.audio_path(doc)?)?.len() as u64;
        }
    }
    Ok(total)
}
pub fn is_audio(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        [
            "wav", "ogg", "opus", "mp3", "flac", "m4a", "aac", "aiff", "aif", "wma",
        ]
        .contains(&e.to_ascii_lowercase().as_str())
    })
}
