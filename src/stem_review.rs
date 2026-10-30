//! Attach stems to their recording and play them on one sample clock.
//! Keep alignment statements as source records; they do not prove separation
//! quality or correct notes.
use crate::models;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

pub const SAMPLE_RATE: u32 = 44_100;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum StemId {
    Drums,
    Bass,
    Vocals,
    Guitar,
    Piano,
    Other,
}
impl StemId {
    pub const ALL: [Self; 6] = [
        Self::Drums,
        Self::Bass,
        Self::Vocals,
        Self::Guitar,
        Self::Piano,
        Self::Other,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drums => "drums",
            Self::Bass => "bass",
            Self::Vocals => "vocals",
            Self::Guitar => "guitar",
            Self::Piano => "piano",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AlignmentEvidence {
    pub method: String,
    pub details: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StemAsset {
    pub id: StemId,
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SeparationIdentity {
    pub model_id: String,
    pub model_sha256: String,
    pub backend: String,
    pub parameters: BTreeMap<String, String>,
    pub cache_key: String,
}
impl SeparationIdentity {
    pub fn calculated_cache_key(&self, source_hash: &str) -> Result<String> {
        ensure!(
            valid_hash(source_hash) && valid_hash(&self.model_sha256),
            "Separation identity requires source/model SHA-256 hashes"
        );
        ensure!(
            !self.model_id.trim().is_empty()
                && self.model_id.len() <= 200
                && !self.backend.trim().is_empty()
                && self.backend.len() <= 500,
            "Separation model and backend identity are required"
        );
        ensure!(
            self.parameters.len() <= 100
                && self
                    .parameters
                    .iter()
                    .all(|(key, value)| !key.trim().is_empty()
                        && key.len() <= 200
                        && value.len() <= 10000),
            "Invalid separation parameters"
        );
        let bytes = serde_json::to_vec(&(
            "chart-engine-separation-v1",
            source_hash.to_ascii_lowercase(),
            &self.model_id,
            self.model_sha256.to_ascii_lowercase(),
            &self.backend,
            &self.parameters,
            SAMPLE_RATE,
            2u16,
        ))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn validate(&self, source_hash: &str) -> Result<()> {
        ensure!(
            self.cache_key == self.calculated_cache_key(source_hash)?,
            "Separation cache key does not match source/model/backend/parameters"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StemManifest {
    pub version: u32,
    pub source_audio_sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub alignment: AlignmentEvidence,
    pub stems: Vec<StemAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<SeparationIdentity>,
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
impl StemManifest {
    /// Build source-bound metadata for existing files. The supplied evidence
    /// must describe the real provenance; no alignment is inferred from length.
    pub fn build(
        source: &Path,
        stems: BTreeMap<StemId, PathBuf>,
        alignment: AlignmentEvidence,
    ) -> Result<Self> {
        let source_audio_sha256 = models::hash_file(source)?;
        let original = models::decode(source, SAMPLE_RATE, 2)?;
        let frames = (original.len() / 2) as u64;
        drop(original);
        let stems = stems
            .into_iter()
            .map(|(id, path)| {
                let path = path.canonicalize()?;
                let sha256 = models::hash_file(&path)?;
                Ok(StemAsset { id, path, sha256 })
            })
            .collect::<Result<Vec<_>>>()?;
        let manifest = Self {
            version: 1,
            source_audio_sha256,
            sample_rate: SAMPLE_RATE,
            channels: 2,
            frames,
            alignment,
            stems,
            generation: None,
        };
        manifest.load(source)?;
        Ok(manifest)
    }
    /// Resolve paths relative to this manifest, never to an application's cwd.
    pub fn read(path: &Path) -> Result<Self> {
        let path = path.canonicalize().context("Stem manifest is missing")?;
        let mut manifest: Self = serde_json::from_slice(&fs::read(&path)?)?;
        for stem in &mut manifest.stems {
            if stem.path.is_relative() {
                stem.path = path
                    .parent()
                    .context("Manifest has no parent")?
                    .join(&stem.path);
            }
            stem.path = stem.path.canonicalize().context("Stem file is missing")?;
        }
        Ok(manifest)
    }
    pub fn ids(&self) -> BTreeSet<StemId> {
        self.stems.iter().map(|asset| asset.id).collect()
    }
    /// Relative assets survive atomic directory publication without staging paths.
    pub fn write_relative(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("Stem manifest needs a parent directory")?
            .canonicalize()?;
        let mut manifest = self.clone();
        for stem in &mut manifest.stems {
            stem.path = stem
                .path
                .canonicalize()?
                .strip_prefix(&parent)
                .context("Relative manifest assets must be inside its directory")?
                .to_owned();
        }
        let mut file = tempfile::NamedTempFile::new_in(&parent)?;
        serde_json::to_writer_pretty(&mut file, &manifest)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    }
    /// Lightweight shape/provenance check, safe for every project load.
    pub fn validate_binding(&self, source_hash: &str, duration: f64) -> Result<()> {
        ensure!(self.version == 1, "Unsupported stem manifest version");
        if let Some(identity) = &self.generation {
            identity.validate(source_hash)?;
        }
        ensure!(
            valid_hash(&self.source_audio_sha256)
                && self.source_audio_sha256.eq_ignore_ascii_case(source_hash),
            "Stem manifest belongs to another recording"
        );
        ensure!(
            self.sample_rate == SAMPLE_RATE && self.channels == 2 && self.frames > 0,
            "Review stems require full-length 44100 Hz stereo PCM coordinates"
        );
        ensure!(
            duration.is_finite()
                && (self.frames as f64 / self.sample_rate as f64 - duration).abs() <= 0.10,
            "Stem manifest duration differs from the recording"
        );
        ensure!(
            !self.alignment.method.trim().is_empty()
                && self.alignment.method.len() <= 200
                && !self.alignment.details.trim().is_empty()
                && self.alignment.details.len() <= 10_000,
            "Stem alignment provenance is required; matching duration alone does not prove alignment"
        );
        ensure!(
            (1..=6).contains(&self.stems.len()) && self.ids().len() == self.stems.len(),
            "Provide one to six unique instrument stems"
        );
        let mut paths = BTreeSet::new();
        for stem in &self.stems {
            ensure!(
                stem.path.is_absolute() && valid_hash(&stem.sha256),
                "Stem paths must be resolved and hashes must be SHA-256"
            );
            ensure!(
                paths.insert(&stem.path),
                "A stem file cannot serve multiple instrument IDs"
            );
        }
        Ok(())
    }
    /// Check actual file hashes, native stem format and decoded sample counts.
    /// This verifies engineering consistency, not the supplied phase alignment.
    pub fn validate_source(
        &self,
        source: &Path,
        expected_hash: &str,
        duration: f64,
    ) -> Result<Self> {
        self.validate_binding(expected_hash, duration)?;
        self.load(source)?;
        Ok(self.clone())
    }
    pub fn load(&self, source: &Path) -> Result<LoadedStems> {
        self.validate_binding(
            &self.source_audio_sha256,
            self.frames as f64 / SAMPLE_RATE as f64,
        )?;
        ensure!(
            models::hash_file(source)?.eq_ignore_ascii_case(&self.source_audio_sha256),
            "Source audio changed; refusing stem playback"
        );
        let original = Arc::new(models::decode(source, SAMPLE_RATE, 2)?);
        ensure!(
            original.len() as u64
                == self
                    .frames
                    .checked_mul(2)
                    .context("Stem frame count overflow")?,
            "Manifest frame count does not match decoded source audio"
        );
        let source_path = source.canonicalize()?;
        let mut stems = BTreeMap::new();
        let mut resolved = BTreeSet::new();
        for asset in &self.stems {
            let path = asset
                .path
                .canonicalize()
                .context("Attached stem is missing")?;
            ensure!(
                path != source_path && resolved.insert(path.clone()),
                "Stem files must be distinct from the original and one another"
            );
            ensure!(
                models::hash_file(&path)?.eq_ignore_ascii_case(&asset.sha256),
                "Attached {} stem changed",
                asset.id.as_str()
            );
            validate_stereo_format(&path)?;
            let pcm = models::decode(&path, SAMPLE_RATE, 2)?;
            ensure!(
                pcm.len() == original.len(),
                "{} stem must have exactly the source's decoded frame count",
                asset.id.as_str()
            );
            ensure!(
                models::hash_file(&path)?.eq_ignore_ascii_case(&asset.sha256),
                "Attached stem changed while decoding"
            );
            stems.insert(asset.id, Arc::new(pcm));
        }
        ensure!(
            models::hash_file(source)?.eq_ignore_ascii_case(&self.source_audio_sha256),
            "Source changed while loading stems"
        );
        LoadedStems::from_pcm(SAMPLE_RATE, original, stems)
    }
}

fn validate_stereo_format(path: &Path) -> Result<()> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,channels",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;
    ensure!(
        output.status.success(),
        "Could not inspect attached stem format"
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let stream = value["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .context("Stem contains no audio")?;
    ensure!(
        stream["sample_rate"].as_str() == Some("44100") && stream["channels"].as_u64() == Some(2),
        "Attached stems must already be 44100 Hz stereo; implicit resampling is not alignment evidence"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MixMode {
    #[default]
    Original,
    Stems,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StemChannel {
    pub gain: f32,
    pub mute: bool,
    pub solo: bool,
}
impl Default for StemChannel {
    fn default() -> Self {
        Self {
            gain: 1.0,
            mute: false,
            solo: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StemMix {
    pub mode: MixMode,
    pub channels: BTreeMap<StemId, StemChannel>,
}
/// Frozen weights: prepare once per UI action, then mix without allocation.
#[derive(Clone, Debug)]
pub struct PreparedMix {
    pub original: bool,
    pub gains: BTreeMap<StemId, f32>,
}
impl StemMix {
    pub fn prepare(&self, available: impl IntoIterator<Item = StemId>) -> Result<PreparedMix> {
        let available: BTreeSet<_> = available.into_iter().collect();
        ensure!(
            self.mode == MixMode::Original || !available.is_empty(),
            "Attach stems before selecting instrument playback"
        );
        for (id, channel) in &self.channels {
            ensure!(
                available.contains(id),
                "Mix setting refers to an unavailable stem"
            );
            ensure!(
                channel.gain.is_finite() && (0.0..=2.0).contains(&channel.gain),
                "Stem gain must be finite and between 0 and 2"
            );
        }
        let any_solo = self.channels.values().any(|channel| channel.solo);
        let gains = available
            .into_iter()
            .map(|id| {
                let channel = self.channels.get(&id).copied().unwrap_or_default();
                (
                    id,
                    if channel.mute || (any_solo && !channel.solo) {
                        0.0
                    } else {
                        channel.gain
                    },
                )
            })
            .collect();
        Ok(PreparedMix {
            original: self.mode == MixMode::Original,
            gains,
        })
    }
    pub fn effective_gains(
        &self,
        available: impl IntoIterator<Item = StemId>,
    ) -> Result<BTreeMap<StemId, f32>> {
        Ok(self.prepare(available)?.gains)
    }
}

#[derive(Clone, Debug)]
pub struct LoadedStems {
    pub sample_rate: u32,
    pub original: Arc<Vec<f32>>,
    pub stems: BTreeMap<StemId, Arc<Vec<f32>>>,
}
impl LoadedStems {
    /// Accepts already validated/tempo-transformed buffers, never file provenance.
    pub fn from_pcm(
        sample_rate: u32,
        original: Arc<Vec<f32>>,
        stems: BTreeMap<StemId, Arc<Vec<f32>>>,
    ) -> Result<Self> {
        ensure!(
            sample_rate > 0
                && !original.is_empty()
                && original.len().is_multiple_of(2)
                && original.iter().all(|sample| sample.is_finite()),
            "Invalid original stereo PCM"
        );
        for pcm in stems.values() {
            ensure!(
                pcm.len() == original.len() && pcm.iter().all(|sample| sample.is_finite()),
                "All review buffers must have identical finite stereo frame counts"
            );
        }
        Ok(Self {
            sample_rate,
            original,
            stems,
        })
    }
    pub fn frame_count(&self) -> usize {
        self.original.len() / 2
    }
    pub fn mix_frame(&self, frame: usize, mix: &StemMix) -> Result<[f32; 2]> {
        ensure!(frame < self.frame_count(), "Frame is outside the recording");
        Ok(self.frame_with_mix(frame, &mix.prepare(self.stems.keys().copied())?))
    }
    /// Real-time path: no allocation, locking, decoding or validation per sample.
    /// Out-of-range reads return silence so an exhausted source cannot panic.
    pub fn frame_with_mix(&self, frame: usize, mix: &PreparedMix) -> [f32; 2] {
        if frame >= self.frame_count() {
            return [0.0; 2];
        }
        let offset = frame * 2;
        let mut output = [0.0f64; 2];
        if mix.original {
            output = [
                self.original[offset] as f64,
                self.original[offset + 1] as f64,
            ];
        } else {
            for (id, gain) in &mix.gains {
                if let Some(pcm) = self.stems.get(id) {
                    output[0] += pcm[offset] as f64 * *gain as f64;
                    output[1] += pcm[offset + 1] as f64 * *gain as f64;
                }
            }
        }
        // One common stereo peak guard; no independent stem/channel normalization.
        let divisor = output[0].abs().max(output[1].abs()).max(1.0);
        [(output[0] / divisor) as f32, (output[1] / divisor) as f32]
    }
    pub fn render(&self, mix: &StemMix) -> Result<Vec<f32>> {
        let prepared = mix.prepare(self.stems.keys().copied())?;
        Ok((0..self.frame_count())
            .flat_map(|frame| self.frame_with_mix(frame, &prepared))
            .collect())
    }
}

/// The same frame position addresses every channel/stem; seek never restarts
/// instruments independently. Ranges are half-open source PCM frame positions.
#[derive(Clone, Debug)]
pub struct FrameCursor {
    frame: usize,
    frames: usize,
    loop_range: Option<Range<usize>>,
}
impl FrameCursor {
    pub fn new(frames: usize) -> Self {
        Self {
            frame: 0,
            frames,
            loop_range: None,
        }
    }
    pub fn position(&self) -> usize {
        self.frame
    }
    pub fn seek(&mut self, frame: usize) -> Result<()> {
        ensure!(frame <= self.frames, "Seek is outside PCM frames");
        self.frame = frame;
        Ok(())
    }
    pub fn set_loop(&mut self, range: Option<Range<usize>>) -> Result<()> {
        if let Some(range) = &range {
            ensure!(
                range.start < range.end && range.end <= self.frames,
                "Invalid PCM loop range"
            );
            if !range.contains(&self.frame) {
                self.frame = range.start;
            }
        }
        self.loop_range = range;
        Ok(())
    }
    pub fn next_frame(&mut self) -> Option<usize> {
        if let Some(range) = &self.loop_range
            && self.frame >= range.end
        {
            self.frame = range.start;
        }
        if self.frame >= self.frames {
            return None;
        }
        let frame = self.frame;
        self.frame += 1;
        Some(frame)
    }
}
