use super::{decode, merge, source};
use crate::{
    storage as fs,
    vocal_harmonies::{self, HarmonyChart, Part},
};
use anyhow::{Result, ensure};
use midly::{MetaMessage as M, Smf, TrackEventKind as K};
use std::path::Path;
pub(crate) fn harmony_percussion_ranges(
    track: &[midly::TrackEvent<'_>],
) -> Result<Vec<(u64, u64)>> {
    Ok(decode::spans(track, |key| matches!(key, 96 | 97))?
        .into_iter()
        .map(|hit| (hit.start, hit.end))
        .collect())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetainedHarmonyPhrases {
    pub scoring: Vec<crate::vocals::VocalPhrase>,
    pub lyrics: Vec<crate::vocals::VocalPhrase>,
}
/// Show retained control ranges after clearing the voices, without recovering
/// any original singer notes or reviving an explicitly omitted part.
pub fn retained_harmony_phrases(song: &Path) -> Result<Option<RetainedHarmonyPhrases>> {
    let Some((imported, directory)) = source(song)? else {
        return Ok(None);
    };
    if imported.baseline_harmonies_sha256.is_none()
        || !imported.retained_harmony_controls.unwrap_or(true)
    {
        return Ok(None);
    }
    let chart: HarmonyChart = serde_json::from_slice(&fs::read(directory.join("harmonies.json"))?)?;
    Ok(Some(RetainedHarmonyPhrases {
        scoring: chart.phrases,
        lyrics: chart.lyric_phrases,
    }))
}

pub fn retained_harmony_controls(song: &Path) -> Result<bool> {
    Ok(source(song)?.is_some_and(|(imported, _)| {
        imported.baseline_harmonies_sha256.is_some()
            && imported.retained_harmony_controls.unwrap_or(true)
    }))
}
pub fn validate_harmony_edit(song: &Path, after: Option<&HarmonyChart>) -> Result<()> {
    let Some(after) = after else { return Ok(()) };
    let Some((imported, directory)) = source(song)? else {
        return Ok(());
    };
    if imported.baseline_harmonies_sha256.is_some()
        && imported.retained_harmony_controls.unwrap_or(true)
    {
        let before: HarmonyChart =
            serde_json::from_slice(&fs::read(directory.join("harmonies.json"))?)?;
        if before.phrases != after.phrases {
            let bytes = fs::read(directory.join("notes.mid"))?;
            vocal_harmonies::validate_percussion_scoring(&Smf::parse(&bytes)?, &before, after)?;
        }
    }
    Ok(())
}
fn omit(bytes: &[u8]) -> Result<Vec<u8>> {
    let midi = Smf::parse(bytes)?;
    let (header, chunks) = merge::chunks(bytes, midi.tracks.len())?;
    let mut kept = Vec::new();
    for (track, raw) in midi.tracks.iter().zip(chunks) {
        if decode::label(track)
            .as_deref()
            .and_then(Part::from_midi)
            .is_some()
        {
            ensure!(
                !track
                    .iter()
                    .any(|event| matches!(event.kind, K::Meta(M::Tempo(_) | M::TimeSignature(..)))),
                "An omitted harmony part contains timing; retain that part to preserve the song clock"
            );
        } else {
            kept.push(raw);
        }
    }
    let mut output = bytes[..header].to_vec();
    output[10..12].copy_from_slice(&u16::try_from(kept.len())?.to_be_bytes());
    for track in kept {
        output.extend(track);
    }
    Ok(output)
}
/// Apply harmony editing after the ordinary MIDI merger has retained its raw
/// source tracks. Explicit omissions never revive archived controls later.
pub fn export_harmonies(
    song: &Path,
    bytes: Vec<u8>,
    chart: Option<&HarmonyChart>,
    selected: bool,
) -> Result<Vec<u8>> {
    let imported = source(song)?;
    let baseline = imported
        .as_ref()
        .filter(|(i, _)| i.baseline_harmonies_sha256.is_some());
    if !selected {
        return if baseline.is_some() {
            omit(&bytes)
        } else {
            Ok(bytes)
        };
    }
    if let Some((imported, directory)) = baseline
        && imported.retained_harmony_controls.unwrap_or(true)
    {
        let before: HarmonyChart =
            serde_json::from_slice(&fs::read(directory.join("harmonies.json"))?)?;
        return match chart {
            Some(after) => vocal_harmonies::rewrite_midi(&bytes, &before, after),
            None => vocal_harmonies::clear_midi(&bytes, &before),
        };
    }
    let bytes = if baseline.is_some() {
        omit(&bytes)?
    } else {
        bytes
    };
    match chart {
        Some(chart) => vocal_harmonies::append_midi(&bytes, chart),
        None => Ok(bytes),
    }
}
