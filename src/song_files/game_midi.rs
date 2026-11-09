//! Ordinary game MIDI import with immutable originals and managed-event edits.
//! Unsupported tracks/events remain in the original MIDI rather than being
//! represented as invented chart rows.
use crate::{
    chart::Chart, editor::EditorNote, models, song_layout, song_metadata::song_ini, storage as fs,
    vocals::VocalChart,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[path = "game_midi/decode.rs"]
mod decode;
#[path = "game_midi/merge.rs"]
mod merge;
pub(crate) use merge::{chunks as midi_chunks, validate_complete as validate_midi_complete};
#[cfg(test)]
#[path = "game_midi/tests.rs"]
mod tests;

const DESCRIPTOR: &str = "chart-engine-midi-import.json";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAttributes {
    pub channel: u8,
    pub velocity: u8,
    pub off_velocity: u8,
    pub off_as_note_on: bool,
    pub zero_length: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talkie_pitch: Option<u8>,
}
impl NoteAttributes {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.channel < 16 && (1..=127).contains(&self.velocity) && self.off_velocity <= 127,
            "Invalid preserved MIDI note attributes"
        );
        ensure!(
            !self.off_as_note_on || self.off_velocity == 0,
            "A zero-velocity note-off must have velocity zero"
        );
        ensure!(
            self.talkie_pitch.is_none_or(|p| (36..=84).contains(&p)),
            "Invalid preserved talkie pitch"
        );
        Ok(())
    }
    pub(crate) fn is_default(&self) -> bool {
        self.channel == 0
            && self.velocity == 100
            && self.off_velocity == 0
            && !self.off_as_note_on
            && !self.zero_length
            && self.talkie_pitch.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributeRecord {
    track: String,
    note: crate::chart::Note,
    attributes: NoteAttributes,
}
pub type TrackAttributes = BTreeMap<String, BTreeMap<(u64, u8), NoteAttributes>>;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Imported {
    version: u32,
    archive_index: u32,
    midi_sha256: String,
    chart_sha256: String,
    baseline_attributes_sha256: String,
    baseline_vocals_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_vocal_controls: Option<bool>,
    attributes_chart_sha256: String,
    attributes: Vec<AttributeRecord>,
    warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retained_extra_parts: Option<std::collections::BTreeSet<String>>,
}

fn archive(song: &Path, index: u32) -> PathBuf {
    song.join(".yace/originals").join(if index == 0 {
        "game-midi".into()
    } else {
        format!("game-midi-{index}")
    })
}

pub fn exists(song: &Path) -> bool {
    fs::is_file(song_layout::resolve(song, DESCRIPTOR))
}

fn extra_key(track: &[midly::TrackEvent<'_>], index: usize) -> Option<String> {
    let name = decode::label(track);
    track
        .iter()
        .any(|event| {
            matches!(
                event.kind,
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. },
                    ..
                }
            )
        })
        .then(|| {
            format!(
                "MIDI: {}",
                name.unwrap_or_else(|| format!("Unnamed source track {}", index + 1))
            )
        })
}

/// Offer original parts that have no editable chart view, without transcribing them.
pub fn extra_parts(song: &Path) -> Result<std::collections::BTreeSet<String>> {
    original_extra_parts(song, true)
}

/// Deleting the last pitched syllable does not deselect source percussion,
/// overdrive or author controls. Older exported omissions lack the editable
/// sidecar; treat those conservatively rather than restoring omitted content.
pub fn retained_vocal_controls(song: &Path) -> Result<bool> {
    Ok(source(song)?.is_some_and(|(imported, _)| {
        imported.baseline_vocals_sha256.is_some()
            && imported.retained_vocal_controls.unwrap_or_else(|| {
                fs::exists(song_layout::resolve(song, "chart-engine-vocals.json"))
            })
    }))
}
fn original_extra_parts(
    song: &Path,
    honor_scope: bool,
) -> Result<std::collections::BTreeSet<String>> {
    let Some((imported, directory)) = source(song)? else {
        return Ok(Default::default());
    };
    let baseline = Chart::parse(fs::read_to_string(directory.join("notes.chart"))?)?;
    let bytes = fs::read(directory.join("notes.mid"))?;
    let midi = midly::Smf::parse(&bytes)?;
    Ok(midi
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, track)| {
            let name = decode::label(track);
            let managed = if name.as_deref() == Some("PART VOCALS") {
                imported.baseline_vocals_sha256.is_some()
            } else {
                name.as_deref()
                    .and_then(decode::part)
                    .is_some_and(|(suffix, difficulty)| {
                        ["Easy", "Medium", "Hard", "Expert"]
                            .iter()
                            .enumerate()
                            .any(|(d, tier)| {
                                difficulty.is_none_or(|target| target == d)
                                    && baseline
                                        .sections
                                        .iter()
                                        .any(|s| s.name == format!("{tier}{suffix}"))
                            })
                    })
            };
            if managed { None } else { extra_key(track, i) }
        })
        .filter(|key| {
            !honor_scope
                || imported
                    .retained_extra_parts
                    .as_ref()
                    .is_none_or(|parts| parts.contains(key))
        })
        .collect())
}

/// Keep untouched MIDI chunks byte-identical. Managed difficulty notes have
/// already been rewritten from the selected chart by the normal MIDI merger.
pub fn select_export(
    song: &Path,
    stage: &Path,
    bytes: Vec<u8>,
    selected: &std::collections::BTreeSet<String>,
) -> Result<Vec<u8>> {
    let original_extras = original_extra_parts(song, false)?;
    let parsed = midly::Smf::parse(&bytes)?;
    let (header, chunks) = merge::chunks(&bytes, parsed.tracks.len())?;
    let mut kept = Vec::new();
    for (index, (track, chunk)) in parsed.tracks.iter().zip(chunks).enumerate() {
        let name = decode::label(track);
        let raw_key = extra_key(track, index);
        let keep = if let Some(key) = raw_key
            .as_ref()
            .filter(|key| original_extras.contains(*key))
        {
            selected.contains(key)
        } else if let Some((suffix, difficulty)) = name.as_deref().and_then(decode::part) {
            ["Easy", "Medium", "Hard", "Expert"]
                .iter()
                .enumerate()
                .any(|(i, tier)| {
                    difficulty.is_none_or(|d| d == i)
                        && selected.contains(&format!("{tier}{suffix}"))
                })
        } else if name.as_deref() == Some("PART VOCALS") {
            selected.contains("Vocals")
        } else if let Some(key) = extra_key(track, index) {
            let generated_suffix = match name.as_deref() {
                Some("PART RHYTHM") => Some("DoubleRhythm"),
                Some("PART GUITAR COOP") => Some("DoubleGuitar"),
                Some("PART GUITAR GHL") => Some("GHLGuitar"),
                Some("PART BASS GHL") => Some("GHLBass"),
                Some("PART RHYTHM GHL") => Some("GHLRhythm"),
                Some("PART GUITAR COOP GHL") => Some("GHLCoop"),
                Some("PART KEYS GHL") => Some("GHLKeys"),
                _ => None,
            };
            selected.contains(&key)
                || generated_suffix.is_some_and(|suffix| {
                    ["Easy", "Medium", "Hard", "Expert"]
                        .iter()
                        .any(|tier| selected.contains(&format!("{tier}{suffix}")))
                })
        } else {
            true
        };
        let metadata_only = !track.iter().any(|event| {
            matches!(
                event.kind,
                midly::TrackEventKind::Midi {
                    message: midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. },
                    ..
                }
            )
        });
        if keep || metadata_only {
            kept.push(chunk);
        } else {
            ensure!(
                !track.iter().any(|event| matches!(
                    event.kind,
                    midly::TrackEventKind::Meta(
                        midly::MetaMessage::Tempo(_) | midly::MetaMessage::TimeSignature(..)
                    )
                )),
                "This MIDI stores timing inside an omitted part; retain that part to preserve the song clock"
            );
        }
    }
    let mut output = bytes[..header].to_vec();
    output[10..12].copy_from_slice(&u16::try_from(kept.len())?.to_be_bytes());
    for chunk in kept {
        output.extend_from_slice(chunk);
    }
    midly::Smf::parse(&output)?;
    if exists(song) {
        let path = song_layout::resolve(stage, DESCRIPTOR);
        let mut imported: Imported = serde_json::from_slice(&fs::read(&path)?)?;
        imported.retained_extra_parts = Some(
            selected
                .iter()
                .filter(|t| t.starts_with("MIDI: "))
                .cloned()
                .collect(),
        );
        imported.retained_vocal_controls =
            Some(retained_vocal_controls(song)? && selected.contains("Vocals"));
        fs::write(path, serde_json::to_vec_pretty(&imported)?)?;
    }
    Ok(output)
}

fn source(song: &Path) -> Result<Option<(Imported, PathBuf)>> {
    let path = song_layout::resolve(song, DESCRIPTOR);
    if !fs::exists(&path) {
        return Ok(None);
    }
    let imported: Imported = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        imported.version == 1,
        "Unsupported game MIDI import version"
    );
    let directory = archive(song, imported.archive_index);
    let vocal_path = directory.join("vocals.json");
    ensure!(
        match &imported.baseline_vocals_sha256 {
            Some(expected) => models::hash_file(&vocal_path)? == *expected,
            None => !fs::exists(vocal_path),
        },
        "Imported vocal baseline changed; recover the original project before export"
    );
    ensure!(
        models::hash_file(&directory.join("notes.mid"))? == imported.midi_sha256
            && models::hash_file(&directory.join("notes.chart"))? == imported.chart_sha256
            && models::hash_file(&directory.join("attributes.json"))?
                == imported.baseline_attributes_sha256,
        "Imported MIDI or baseline master changed; recover the original project before export"
    );
    Ok(Some((imported, directory)))
}

pub fn warnings(song: &Path) -> Result<Vec<String>> {
    Ok(source(song)?.map_or_else(Vec::new, |(s, _)| s.warnings))
}

/// A standard MIDI part shares phrase events across its difficulty pitches.
/// Keep the editing views in agreement so removing a phrase cannot be undone
/// accidentally by an untouched difficulty when the MIDI is regenerated.
pub fn edit_star_power(
    project: &mut crate::editor::Project,
    source: &Chart,
    edit: &crate::star_power::Edit,
) -> Result<()> {
    use crate::star_power::{self, Edit};
    let (Edit::Put { track, .. } | Edit::Remove { track, .. }) = edit;
    let suffix = ["Easy", "Medium", "Hard", "Expert"]
        .iter()
        .find_map(|tier| track.strip_prefix(tier));
    let Some(suffix) = suffix.filter(|s| *s != "ProKeys") else {
        return star_power::apply(project, source, edit);
    };
    let tracks: Vec<_> = ["Easy", "Medium", "Hard", "Expert"]
        .iter()
        .map(|tier| format!("{tier}{suffix}"))
        .filter(|name| project.content.tracks.contains_key(name))
        .collect();
    let mut phrases = Vec::new();
    for name in &tracks {
        phrases.extend(star_power::current(project, source, name)?);
    }
    phrases.sort_by_key(|p| (p.start, p.end));
    phrases.dedup();
    project.content.star_power.insert(track.clone(), phrases);
    star_power::apply(project, source, edit)?;
    let phrases = project.content.star_power[track].clone();
    for name in tracks {
        project.content.star_power.insert(name, phrases.clone());
    }
    Ok(())
}

pub fn attributes(song: &Path, chart: &Chart) -> Result<TrackAttributes> {
    let Some((imported, _)) = source(song)? else {
        return Ok(BTreeMap::new());
    };
    ensure!(
        imported.attributes_chart_sha256 == hash(chart.raw.as_bytes()),
        "MIDI note metadata does not match its editable master"
    );
    let tracks: std::collections::BTreeSet<_> = imported
        .attributes
        .iter()
        .map(|a| a.track.clone())
        .collect();
    let available = tracks
        .into_iter()
        .map(|track| {
            let notes = chart
                .notes(&track)?
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            Ok((track, notes))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut result = TrackAttributes::new();
    for record in imported.attributes {
        record.attributes.validate()?;
        ensure!(
            available[&record.track].contains(&record.note),
            "Preserved MIDI note metadata has no matching note"
        );
        ensure!(
            result
                .entry(record.track)
                .or_default()
                .insert((record.note.tick, record.note.lane), record.attributes)
                .is_none(),
            "Duplicate MIDI note metadata"
        );
    }
    Ok(result)
}

/// Run only inside the unpublished copied song, after source layout preparation.
pub fn prepare(song: &Path) -> Result<()> {
    ensure!(
        !exists(song),
        "An existing MIDI import receipt must not be replaced"
    );
    ensure!(
        !fs::exists(song_layout::resolve(song, "chart-engine-vocals.json")),
        "Unbound MIDI has an existing vocal sidecar; recover its editable-master binding before importing"
    );
    let bytes = fs::read(song.join("notes.mid"))?;
    let ini = song_ini(song)?;
    let decoded = decode::import(&bytes, &ini)?;
    let mut index = 0u32;
    while fs::exists(archive(song, index)) {
        index = index
            .checked_add(1)
            .context("Too many MIDI import archives")?;
    }
    let directory = archive(song, index);
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("notes.mid"), &bytes)?;
    fs::write(directory.join("song.ini"), &ini)?;
    fs::write(directory.join("notes.chart"), &decoded.chart.raw)?;
    fs::write(
        directory.join("attributes.json"),
        serde_json::to_vec(&decoded.attributes)?,
    )?;
    if let Some(vocals) = &decoded.vocals {
        let value = serde_json::to_vec_pretty(vocals)?;
        fs::write(directory.join("vocals.json"), &value)?;
        fs::write(song_layout::path(song, "chart-engine-vocals.json"), value)?;
    }
    let imported = Imported {
        version: 1,
        archive_index: index,
        midi_sha256: models::hash_file(&directory.join("notes.mid"))?,
        chart_sha256: models::hash_file(&directory.join("notes.chart"))?,
        baseline_attributes_sha256: models::hash_file(&directory.join("attributes.json"))?,
        baseline_vocals_sha256: decoded
            .vocals
            .as_ref()
            .map(|_| models::hash_file(&directory.join("vocals.json")))
            .transpose()?,
        retained_vocal_controls: None,
        attributes_chart_sha256: hash(decoded.chart.raw.as_bytes()),
        attributes: decoded.attributes,
        warnings: decoded.warnings,
        retained_extra_parts: None,
    };
    fs::write(
        song_layout::path(song, DESCRIPTOR),
        serde_json::to_vec_pretty(&imported)?,
    )?;
    fs::write(song.join("notes.chart"), decoded.chart.raw)?;
    fs::write(
        song.join("song.ini"),
        crate::game_export::ini_with_delay(&ini, 0),
    )?;
    fs::remove_file(song.join("notes.mid"))?;
    Ok(())
}

pub fn export(
    song: &Path,
    stage: &Path,
    chart: &Chart,
    vocals: Option<&VocalChart>,
    tracks: &BTreeMap<String, Vec<EditorNote>>,
    generated: &[u8],
) -> Result<Option<Vec<u8>>> {
    let Some((mut imported, directory)) = source(song)? else {
        let records: Vec<_> = tracks
            .iter()
            .filter(|(track, _)| crate::pro_guitar::is_track(track))
            .flat_map(|(track, notes)| {
                notes.iter().filter_map(|n| {
                    n.midi.as_ref().map(|attributes| AttributeRecord {
                        track: track.clone(),
                        note: n.note.clone(),
                        attributes: attributes.clone(),
                    })
                })
            })
            .collect();
        return if records.is_empty() {
            Ok(None)
        } else {
            Ok(Some(merge::authored_attributes(generated, &records)?))
        };
    };
    let original = fs::read(directory.join("notes.mid"))?;
    let retain_vocal_controls = retained_vocal_controls(song)?;
    let original = if imported.baseline_vocals_sha256.is_some() && !retain_vocal_controls {
        omit_original_vocals(&original)?
    } else {
        original
    };
    let baseline = Chart::parse(fs::read_to_string(directory.join("notes.chart"))?)?;
    let baseline_attributes: Vec<AttributeRecord> =
        serde_json::from_slice(&fs::read(directory.join("attributes.json"))?)?;
    let vocal_path = directory.join("vocals.json");
    let baseline_vocals: Option<VocalChart> = if retain_vocal_controls && fs::is_file(&vocal_path) {
        Some(serde_json::from_slice(&fs::read(vocal_path)?)?)
    } else {
        None
    };
    ensure!(
        chart.timing.resolution == baseline.timing.resolution,
        "An imported MIDI's resolution must remain unchanged"
    );
    let attributes: Vec<_> = tracks
        .iter()
        .flat_map(|(track, notes)| {
            notes.iter().filter_map(|note| {
                note.midi.as_ref().map(|attributes| AttributeRecord {
                    track: track.clone(),
                    note: note.note.clone(),
                    attributes: attributes.clone(),
                })
            })
        })
        .collect();
    let output = merge::rewrite(
        &original,
        generated,
        merge::State {
            chart: &baseline,
            vocals: baseline_vocals.as_ref(),
            attributes: &baseline_attributes,
        },
        merge::State {
            chart,
            vocals,
            attributes: &attributes,
        },
    )?;
    imported.attributes_chart_sha256 = hash(chart.raw.as_bytes());
    imported.attributes = attributes;
    fs::write(
        song_layout::path(stage, DESCRIPTOR),
        serde_json::to_vec_pretty(&imported)?,
    )?;
    Ok(Some(output))
}

fn omit_original_vocals(bytes: &[u8]) -> Result<Vec<u8>> {
    let midi = midly::Smf::parse(bytes)?;
    let (header, chunks) = merge::chunks(bytes, midi.tracks.len())?;
    let mut kept = Vec::new();
    for (events, chunk) in midi.tracks.iter().zip(chunks) {
        if decode::label(events).as_deref() == Some("PART VOCALS") {
            ensure!(
                !events.iter().any(|event| matches!(
                    event.kind,
                    midly::TrackEventKind::Meta(
                        midly::MetaMessage::Tempo(_) | midly::MetaMessage::TimeSignature(..)
                    )
                )),
                "An omitted original vocal part contains timing; retain its source before export"
            );
        } else {
            kept.push(chunk);
        }
    }
    let mut output = bytes[..header].to_vec();
    output[10..12].copy_from_slice(&u16::try_from(kept.len())?.to_be_bytes());
    for chunk in kept {
        output.extend_from_slice(chunk);
    }
    Ok(output)
}
