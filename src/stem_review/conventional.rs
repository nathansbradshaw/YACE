//! Supplied game audio channels, separate from model-produced separation.
use super::{AlignmentEvidence, SAMPLE_RATE, StemAsset, StemId, StemManifest};
use crate::{models, song_layout, storage as fs};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

pub const PREVIEW: &str = ".yace/audio/conventional/complete.wav";
pub fn valid_audio_filename(name: &str) -> bool {
    name == PREVIEW
        || crate::project_recording::attached_filename(name)
        || (Path::new(name).components().count() == 1
            && matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            ))
}
const RECEIPT: &str = "chart-engine-conventional-audio.json";
const EXTENSIONS: [&str; 8] = ["ogg", "opus", "mp3", "flac", "wav", "m4a", "aiff", "aac"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Gain(u32);
impl Gain {
    pub fn value(self) -> f32 {
        f32::from_bits(self.0)
    }
}
impl TryFrom<f32> for Gain {
    type Error = String;
    fn try_from(value: f32) -> std::result::Result<Self, Self::Error> {
        if value.is_finite() && value > 0. && value <= 1. {
            Ok(Self(value.to_bits()))
        } else {
            Err("Invalid common audition headroom gain".into())
        }
    }
}
impl From<Gain> for f32 {
    fn from(value: Gain) -> Self {
        value.value()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clock {
    pub version: u32,
    pub headroom: Gain,
}
impl Clock {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported game-channel clock");
        Gain::try_from(self.headroom.value()).map_err(anyhow::Error::msg)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub id: StemId,
    pub filename: String,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    cache_key: String,
    inputs: Vec<Input>,
    manifest: StemManifest,
}

fn role(name: &str) -> Option<StemId> {
    Some(match name {
        "song" => StemId::Backing,
        "guitar" => StemId::Guitar,
        "bass" => StemId::Bass,
        "rhythm" => StemId::Rhythm,
        "keys" => StemId::Piano,
        "vocals" | "vocals_1" | "vocals_2" => StemId::Vocals,
        "drums" | "drums_1" | "drums_2" | "drums_3" | "drums_4" => StemId::Drums,
        "crowd" => StemId::Crowd,
        _ => return None,
    })
}
pub fn inputs(song: &Path) -> Result<Vec<Input>> {
    let mut selected = BTreeMap::<String, (usize, Input)>::new();
    for entry in fs::read_dir(song)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(id) = role(&stem) else {
            continue;
        };
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(priority) = EXTENSIONS.iter().position(|e| *e == extension) else {
            continue;
        };
        let input = Input {
            id,
            filename: entry.file_name().to_string_lossy().into(),
            sha256: models::hash_file(&path)?,
        };
        if let Some((old, previous)) = selected.get(&stem) {
            ensure!(
                *old != priority,
                "Ambiguous game-channel filenames: {} and {}",
                previous.filename,
                input.filename
            );
        }
        if selected.get(&stem).is_none_or(|(old, _)| priority < *old) {
            selected.insert(stem, (priority, input));
        }
    }
    Ok(selected.into_values().map(|(_, input)| input).collect())
}
pub fn warnings(song: &Path) -> Result<Vec<String>> {
    if read(song)?.is_none() {
        return Ok(Vec::new());
    }
    let selected = inputs(song)?;
    let names: BTreeSet<_> = selected.iter().map(|i| i.filename.as_str()).collect();
    let mut skipped = Vec::new();
    for entry in fs::read_dir(song)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !names.contains(name)
            && role(
                &path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            )
            .is_some()
            && EXTENSIONS.contains(
                &path
                    .extension()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
            )
        {
            skipped.push(name.to_owned());
        }
    }
    skipped.sort();
    Ok(if skipped.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "Alternative encodings retained but not mixed twice: {}. Audition uses one encoding per game channel (OGG, Opus, MP3, FLAC, WAV, M4A, AIFF, AAC preference).",
            skipped.join(", ")
        )]
    })
}
fn read(song: &Path) -> Result<Option<Receipt>> {
    let path = song_layout::resolve(song, RECEIPT);
    if !fs::exists(&path) {
        return Ok(None);
    }
    #[cfg(not(target_arch = "wasm32"))]
    ensure!(
        fs::metadata(&path)?.len() <= 65536,
        "Conventional audio receipt exceeds 64 KiB"
    );
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() <= 65536,
        "Conventional audio receipt exceeds 64 KiB"
    );
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.version == 1 && (1..=14).contains(&receipt.inputs.len()),
        "Invalid conventional audio receipt"
    );
    for input in &receipt.inputs {
        ensure!(
            Path::new(&input.filename).components().count() == 1
                && matches!(
                    Path::new(&input.filename).components().next(),
                    Some(Component::Normal(_))
                ),
            "Invalid game-channel filename"
        );
        ensure!(
            role(
                Path::new(&input.filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
            ) == Some(input.id),
            "Game-channel role does not match its filename"
        );
    }
    ensure!(
        receipt
            .inputs
            .iter()
            .map(|i| &i.filename)
            .collect::<BTreeSet<_>>()
            .len()
            == receipt.inputs.len()
            && receipt.manifest.ids() == receipt.inputs.iter().map(|i| i.id).collect(),
        "Conventional channel manifest differs from its input roles"
    );
    let expected = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "game-channels-v1",
            SAMPLE_RATE,
            &receipt.inputs
        ))?)
    );
    ensure!(
        expected == receipt.cache_key
            && receipt.manifest.conventional.is_some()
            && receipt.manifest.generation.is_none(),
        "Conventional audio identity changed"
    );
    Ok(Some(receipt))
}
pub fn preview(song: &Path) -> Result<Option<PathBuf>> {
    if read(song)?.is_none() {
        return Ok(None);
    }
    let path = song.join(PREVIEW);
    ensure!(
        fs::is_file(&path),
        "Complete game-channel preview is missing; recover its source snapshot"
    );
    Ok(Some(path))
}
pub fn manifest(song: &Path) -> Result<Option<StemManifest>> {
    let Some(receipt) = read(song)? else {
        return Ok(None);
    };
    let mut manifest = receipt.manifest;
    for asset in &mut manifest.stems {
        ensure!(
            asset
                .path
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "Invalid conventional channel cache path"
        );
        asset.path = fs::canonicalize(song.join(&asset.path))?;
    }
    manifest.validate_binding(
        &manifest.source_audio_sha256,
        manifest.frames as f64 / SAMPLE_RATE as f64,
    )?;
    Ok(Some(manifest))
}
pub fn validate(song: &Path) -> Result<bool> {
    let Some(receipt) = read(song)? else {
        return Ok(false);
    };
    ensure!(
        inputs(song)? == receipt.inputs,
        "Supplied game channels changed; import their current source as a new project"
    );
    ensure!(
        models::hash_file(&song.join(PREVIEW))? == receipt.manifest.source_audio_sha256,
        "Complete game-channel preview changed"
    );
    for input in &receipt.inputs {
        ensure!(
            models::hash_file(&song.join(&input.filename))? == input.sha256,
            "Supplied game channel changed: {}",
            input.filename
        );
    }
    for asset in &receipt.manifest.stems {
        ensure!(
            asset
                .path
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "Invalid cached channel path"
        );
        ensure!(
            models::hash_file(&song.join(&asset.path))? == asset.sha256,
            "Game-channel audition cache changed"
        );
    }
    Ok(true)
}
pub fn audio_identity(song: &Path) -> Result<Option<String>> {
    if !should_prepare(song)? {
        return Ok(None);
    }
    let inputs = inputs(song)?;
    if inputs.is_empty()
        || (inputs.len() == 1 && matches!(inputs[0].id, StemId::Backing | StemId::Guitar))
    {
        return Ok(None);
    }
    Ok(Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("game-channels-v1", inputs))?)
    )))
}
pub fn decode_inputs(song: &Path) -> Result<Vec<PathBuf>> {
    if read(song)?.is_some() || !should_prepare(song)? {
        return Ok(Vec::new());
    }
    Ok(inputs(song)?
        .into_iter()
        .map(|i| song.join(i.filename))
        .collect())
}
pub fn should_prepare(song: &Path) -> Result<bool> {
    if read(song)?.is_some() {
        return Ok(true);
    }
    // Explicit separation sources keep their original recording semantics.
    if fs::exists(song_layout::resolve(song, "chart-engine-isolation.json"))
        || fs::exists(song_layout::resolve(
            song,
            "chart-engine-transcription.json",
        ))
    {
        return Ok(false);
    }
    for marker in [
        "chart-engine-review.json",
        "chart-engine-melody-source.json",
    ] {
        if fs::exists(song_layout::resolve(song, marker)) {
            return Ok(false);
        }
    }
    let receipt = song_layout::resolve(song, "yace-audio.json");
    if fs::is_file(&receipt) {
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(receipt)?)?;
        if !saved["original_audio"].is_null() {
            return Ok(false);
        }
    }
    let input = inputs(song)?;
    Ok(!input.is_empty()
        && !(input.len() == 1 && matches!(input[0].id, StemId::Backing | StemId::Guitar)))
}
/// Called only in an unpublished copied source. The browser supplies decoded
/// inputs on its existing worker; native callers use the local decoder.
pub fn prepare(song: &Path) -> Result<bool> {
    if !should_prepare(song)? {
        return Ok(false);
    }
    if read(song)?.is_some() {
        validate(song)?;
        return Ok(true);
    }
    let input = inputs(song)?;
    let key = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "game-channels-v1",
            SAMPLE_RATE,
            &input
        ))?)
    );
    let cache = song.join(".yace/audio/conventional").join(&key);
    fs::create_dir_all(&cache)?;
    let mut grouped = BTreeMap::<StemId, (Vec<f32>, Vec<&Input>)>::new();
    for asset in &input {
        let path = song.join(&asset.filename);
        let pcm = models::decode(&path, SAMPLE_RATE, 2)?;
        ensure!(
            !pcm.is_empty() && pcm.len().is_multiple_of(2) && pcm.iter().all(|x| x.is_finite()),
            "Invalid game channel PCM"
        );
        ensure!(
            models::hash_file(&path)? == asset.sha256,
            "Game channel changed during decoding"
        );
        let (sum, members) = grouped.entry(asset.id).or_default();
        sum.resize(sum.len().max(pcm.len()), 0.);
        for (out, value) in sum.iter_mut().zip(pcm) {
            *out += value;
            ensure!(out.is_finite(), "Game channel sum overflow");
        }
        members.push(asset);
        #[cfg(target_arch = "wasm32")]
        ensure!(
            grouped
                .values()
                .map(|(pcm, _)| pcm.len() * 4)
                .sum::<usize>()
                <= 1024 * 1024 * 1024,
            "Conventional audition exceeds the browser decoded-audio budget"
        );
    }
    let samples = grouped
        .values()
        .map(|(pcm, _)| pcm.len())
        .max()
        .context("No game channels")?;
    let mut complete = vec![0f64; samples];
    let mut peak = 1f64;
    for (pcm, _) in grouped.values() {
        for (out, value) in complete.iter_mut().zip(pcm) {
            *out += f64::from(*value);
            peak = peak.max(f64::from(value.abs()));
        }
    }
    for value in &complete {
        peak = peak.max(value.abs());
    }
    let target = 1. / peak;
    let mut gain = target as f32;
    if f64::from(gain) > target {
        gain = gain.next_down();
    }
    let gain = Gain::try_from(gain).map_err(anyhow::Error::msg)?;
    let preview: Vec<f32> = complete
        .into_iter()
        .map(|v| (v * f64::from(gain.value())) as f32)
        .collect();
    write_stereo(&song.join(PREVIEW), &preview)?;
    let mut stems = Vec::new();
    for (id, (mut pcm, members)) in grouped {
        let raw = song.join(&members[0].filename);
        let path = if members.len() == 1
            && pcm.len() == samples
            && super::validate_stereo_format(&raw).is_ok()
        {
            song.join(&members[0].filename)
        } else {
            pcm.resize(samples, 0.);
            let path = cache.join(format!("{}.wav", id.as_str()));
            write_stereo(&path, &pcm)?;
            path
        };
        stems.push(StemAsset {
            id,
            sha256: models::hash_file(&path)?,
            path: path.strip_prefix(song)?.to_owned(),
        });
    }
    let alignment = AlignmentEvidence {
        method: "Conventional game-channel clock".into(),
        details: concat!(
            "Supplied game files share song time zero. Shorter tails are silent; ",
            "a common audition gain preserves channel and stereo balance. ",
            "This records the file convention, not measured phase alignment."
        )
        .into(),
    };
    let manifest = StemManifest {
        version: 1,
        source_audio_sha256: models::hash_file(&song.join(PREVIEW))?,
        sample_rate: SAMPLE_RATE,
        channels: 2,
        frames: (samples / 2) as u64,
        alignment,
        stems,
        generation: None,
        conventional: Some(Clock {
            version: 1,
            headroom: gain,
        }),
    };
    let receipt = Receipt {
        version: 1,
        cache_key: key,
        inputs: input,
        manifest,
    };
    fs::write(
        song_layout::path(song, RECEIPT),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    validate(song)?;
    Ok(true)
}
fn write_stereo(path: &Path, pcm: &[f32]) -> Result<()> {
    fs::create_dir_all(path.parent().context("Audio cache needs a directory")?)?;
    let left: Vec<_> = pcm.chunks_exact(2).map(|f| f[0]).collect();
    let right: Vec<_> = pcm.chunks_exact(2).map(|f| f[1]).collect();
    models::write_audio(path, &left, &right)?;
    Ok(())
}
pub fn routing(song: &Path) -> Result<Option<BTreeMap<String, String>>> {
    if !validate(song)? {
        return Ok(None);
    }
    Ok(Some(
        read(song)?
            .unwrap()
            .inputs
            .into_iter()
            .map(|i| (i.filename.clone(), i.filename))
            .collect(),
    ))
}

#[cfg(test)]
#[path = "conventional/tests.rs"]
mod tests;
