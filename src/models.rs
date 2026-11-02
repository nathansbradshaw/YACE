use crate::mapping::{DrumHit, PitchCandidateEvidence, PitchNote};
pub mod postprocessing;
use anyhow::{Context, Result, ensure};
use ort::{session::Session, value::Tensor};
pub use postprocessing::{
    BassPitchInference, GuitarPitchInference, ShortPitchInference, decode_pitch,
    decode_pitch_with_evidence, drum_peaks, pitch_frame_seconds, supported_drum_peaks,
};
use postprocessing::{PitchActivations, clip_pitch_candidates, recover_short_bass};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelAsset {
    pub id: String,
    pub filename: String,
    pub url: String,
    pub sha256: String,
    pub license: String,
}

pub fn assets() -> Vec<ModelAsset> {
    serde_json::from_str(include_str!("../models/manifest.json")).expect("valid embedded manifest")
}

pub(crate) fn user_model_cache() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join("Library/Application Support/YACE/models"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join("YACE/models"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|home| !home.is_empty())
                    .map(|home| PathBuf::from(home).join(".cache"))
            })
            .map(|cache| cache.join("YACE/models"))
    }
}

pub(crate) fn bundle_model_directory(path: &Path) -> bool {
    let matches = |path: &Path| {
        let mut parts = Vec::new();
        for part in path.components() {
            match part {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    parts.pop();
                }
                part => parts.push(part.as_os_str()),
            }
        }
        parts.windows(4).any(|parts| {
            Path::new(parts[0])
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                && parts[1] == "Contents"
                && parts[2] == "Resources"
                && parts[3] == "models"
        })
    };
    matches(path) || path.canonicalize().ok().is_some_and(|path| matches(&path))
}

pub(crate) fn optional_model_directory(
    root: &Path,
    nested: &Path,
    files: &[&str],
) -> Result<PathBuf> {
    optional_directory_for(root, nested, files, user_model_cache().as_deref())
}

fn optional_directory_for(
    root: &Path,
    nested: &Path,
    files: &[&str],
    cache: Option<&Path>,
) -> Result<PathBuf> {
    let requested = root.join(nested);
    let present = files
        .iter()
        .map(|file| requested.join(file).try_exists())
        .collect::<std::io::Result<Vec<_>>>()?;
    if !bundle_model_directory(root) || present.iter().all(|present| *present) {
        return Ok(requested);
    }
    // Read complete bundled assets in place. Missing optional assets belong
    // in the user cache so a download cannot change the app's signature.
    Ok(cache
        .context("Cannot locate a writable model cache; choose a custom model directory")?
        .join(nested))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn verified(dir: &Path, id: &str) -> Result<PathBuf> {
    let asset = assets()
        .into_iter()
        .find(|a| a.id == id)
        .context("Unknown model")?;
    let path = dir.join(&asset.filename);
    ensure!(
        path.is_file(),
        "Model missing: {}. Run models fetch first",
        path.display()
    );
    ensure!(
        hash_file(&path)? == asset.sha256,
        "Model checksum mismatch: {}",
        path.display()
    );
    Ok(path)
}

/// Identify a six-stem run by its audio, model and implementation.
/// Leave paths out so moving the same files does not invalidate the cache.
pub fn separation_identity(model: &Path) -> Result<crate::stem_review::SeparationIdentity> {
    use std::collections::BTreeMap;
    // Include native implementation bytes as well as the checkpoint so changes
    // to chunk stitching, model loading or export cannot reuse an older result.
    let mut implementation = Sha256::new();
    let sources: &[&[u8]] = &[
        include_bytes!("models.rs"),
        include_bytes!("../vendor/demucs-core/src/lib.rs"),
        include_bytes!("../vendor/demucs-core/src/dsp/stft.rs"),
        include_bytes!("../vendor/demucs-core/src/dsp/spectrogram.rs"),
        include_bytes!("../vendor/demucs-core/src/dsp/resample.rs"),
        include_bytes!("../vendor/demucs-core/src/dsp/cac.rs"),
        include_bytes!("../vendor/demucs-core/src/model/htdemucs.rs"),
        include_bytes!("../vendor/demucs-core/src/model/metadata.rs"),
        include_bytes!("../vendor/demucs-core/src/model/conv.rs"),
        include_bytes!("../vendor/demucs-core/src/model/transformer.rs"),
        include_bytes!("../vendor/demucs-core/src/weights/load.rs"),
        include_bytes!("../vendor/demucs-core/src/weights/tensor_store.rs"),
    ];
    for source in sources {
        implementation.update((source.len() as u64).to_le_bytes());
        implementation.update(source);
    }
    let decoder = Command::new("ffmpeg").arg("-version").output()?;
    ensure!(decoder.status.success(), "Cannot identify FFmpeg decoder");
    let decoder = String::from_utf8_lossy(&decoder.stdout)
        .lines()
        .next()
        .context("FFmpeg version is missing")?
        .to_owned();
    Ok(crate::stem_review::SeparationIdentity {
        model_id: "separation".into(),
        model_sha256: hash_file(model)?,
        backend: "demucs-rs-six-stem-wgpu-v1".into(),
        parameters: BTreeMap::from([
            ("variant".into(), "htdemucs_6s".into()),
            ("sample_rate".into(), "44100".into()),
            ("channels".into(), "2".into()),
            ("input".into(), "entire_source_no_trim_or_shift".into()),
            ("export".into(), "pcm_f32le_wav_no_normalization".into()),
            ("burn".into(), "0.20.1".into()),
            ("decoder".into(), decoder),
            (
                "native_code_sha256".into(),
                format!("{:x}", implementation.finalize()),
            ),
        ]),
        cache_key: String::new(),
    })
}

pub fn fetch(dir: &Path) -> Result<()> {
    fetch_required(
        dir,
        &assets().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
    )
}

/// Fetch only the assets needed by the selected generation instruments.
pub fn fetch_required(dir: &Path, wanted: &[&str]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(900))
        .user_agent("chart-engine/0.1")
        .build()?;
    for asset in assets() {
        if !wanted.contains(&asset.id.as_str()) {
            continue;
        }
        let target = dir.join(&asset.filename);
        crate::generation_progress::detail(format!("Verifying {} model", asset.id));
        if target.exists() {
            ensure!(
                hash_file(&target)? == asset.sha256,
                "Existing model has unexpected checksum: {}",
                target.display()
            );
            eprintln!("Verified {}", asset.id);
            continue;
        }
        eprintln!("Downloading {} ({})", asset.id, asset.license);
        crate::generation_progress::detail(format!("Downloading {} model", asset.id));
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        let mut response = client.get(&asset.url).send()?.error_for_status()?;
        let total = response
            .content_length()
            .and_then(|size| usize::try_from(size).ok());
        let mut copied = 0usize;
        let mut reported = 0usize;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            temp.write_all(&buffer[..count])?;
            copied += count;
            if copied.saturating_sub(reported) >= 4 * 1024 * 1024 {
                crate::generation_progress::units(
                    format!(
                        "Downloading {} · {} MiB{}",
                        asset.id,
                        copied / (1024 * 1024),
                        total
                            .map(|bytes| format!(" / {} MiB", bytes / (1024 * 1024)))
                            .unwrap_or_default()
                    ),
                    copied,
                    total.unwrap_or(0),
                );
                reported = copied;
            }
        }
        crate::generation_progress::detail(format!("Checking downloaded {} model", asset.id));
        temp.flush()?;
        ensure!(
            hash_file(temp.path())? == asset.sha256,
            "Downloaded checksum mismatch for {}",
            asset.id
        );
        temp.persist_noclobber(&target).map_err(|e| e.error)?;
    }
    Ok(())
}

pub fn decode(path: &Path, rate: u32, channels: u32) -> Result<Vec<f32>> {
    decode_at_tempo(path, rate, channels, 1.0)
}
/// Practice audio changes tempo without changing pitch. Returned sample time is
/// recording time divided by tempo; all chart coordinates remain source time.
pub fn decode_at_tempo(path: &Path, rate: u32, channels: u32, tempo: f64) -> Result<Vec<f32>> {
    ensure!(
        tempo.is_finite() && (0.5..=1.5).contains(&tempo),
        "Invalid audio tempo"
    );
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error", "-nostdin", "-i"]).arg(path);
    if tempo != 1.0 {
        command.args(["-af", &format!("atempo={tempo:.8}")]);
    }
    let output = command
        .args([
            "-map",
            "0:a:0",
            "-f",
            "f32le",
            "-ar",
            &rate.to_string(),
            "-ac",
            &channels.to_string(),
            "pipe:1",
        ])
        .output()
        .context("ffmpeg is required on PATH")?;
    ensure!(
        output.status.success(),
        "Audio decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(output.stdout.len() % 4 == 0, "Invalid decoded sample bytes");
    let samples: Vec<f32> = output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    ensure!(
        !samples.is_empty() && samples.iter().all(|x| x.is_finite()),
        "Empty or invalid audio"
    );
    Ok(samples)
}

pub fn write_audio(path: &Path, left: &[f32], right: &[f32]) -> Result<()> {
    ensure!(left.len() == right.len(), "Unequal stereo channels");
    ensure!(
        !left.is_empty() && left.iter().chain(right).all(|sample| sample.is_finite()),
        "Cannot export empty or nonfinite audio"
    );
    ensure!(
        !path.exists(),
        "Audio output already exists: {}",
        path.display()
    );
    let mut child = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "f32le",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-i",
            "pipe:0",
            "-c:a",
            "pcm_f32le",
            "-n",
        ])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    // Buffer stereo interleaving: writing each four-byte sample directly to
    // the pipe causes millions of syscalls per song and stalls stem export.
    let mut input = BufWriter::with_capacity(
        256 * 1024,
        child.stdin.take().context("Missing encoder input")?,
    );
    for (l, r) in left.iter().zip(right) {
        input.write_all(&l.to_le_bytes())?;
        input.write_all(&r.to_le_bytes())?;
    }
    input.flush()?;
    drop(input);
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "Audio encode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        path.is_file() && path.metadata()?.len() > 44,
        "Encoder did not produce a complete audio file"
    );
    Ok(())
}

fn session(path: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_intra_threads(4)?
        .commit_from_file(path)?)
}

pub fn drums(audio: &Path, model: &Path) -> Result<Vec<DrumHit>> {
    Ok(drums_with_recovery(audio, model)?.0)
}

pub fn drums_with_recovery(audio: &Path, model: &Path) -> Result<(Vec<DrumHit>, Vec<DrumHit>)> {
    let samples = decode(audio, 44100, 1)?;
    let mut model = session(model)?;
    let count = samples.len().div_ceil(441);
    let mut activations = vec![0.0; count * 5];
    // 20-second central regions, three seconds of context on each side.
    // Crop context outputs before peak picking, so seams are not double hits.
    for core in (0..samples.len()).step_by(20 * 44100) {
        let start = core.saturating_sub(3 * 44100);
        let end = (core + 23 * 44100).min(samples.len());
        let mut chunk = samples[start..end].to_vec();
        chunk.resize(chunk.len().max(44100), 0.0);
        let tensor = Tensor::from_array(([1, chunk.len()], chunk.into_boxed_slice()))?;
        let outputs = model.run(ort::inputs!["waveform" => tensor])?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.len() == 3 && shape[0] == 1 && shape[2] == 5,
            "Unexpected ADTOF output shape: {shape:?}"
        );
        let local_start = (core - start) / 441;
        let global_start = core / 441;
        let frames = (20 * 100).min(count - global_start);
        ensure!(
            (local_start + frames) * 5 <= data.len(),
            "ADTOF output too short"
        );
        activations[global_start * 5..(global_start + frames) * 5]
            .copy_from_slice(&data[local_start * 5..(local_start + frames) * 5]);
        eprintln!(
            "Drums: {:.1}/{:.1} seconds",
            (core + 20 * 44100).min(samples.len()) as f64 / 44100.0,
            samples.len() as f64 / 44100.0
        );
        crate::generation_progress::units(
            format!(
                "Transcribing drums · {:.0}/{:.0} seconds",
                (core + 20 * 44100).min(samples.len()) as f64 / 44100.0,
                samples.len() as f64 / 44100.0
            ),
            (core + 20 * 44100).min(samples.len()),
            samples.len(),
        );
    }
    Ok(supported_drum_peaks(
        &activations,
        &decode(audio, 22050, 1)?,
    ))
}

/// Port of Basic Pitch's overlapping inference windows and onset-first decoder.
/// Includes upstream's Melodia energy-recovery pass. Pitch bends are omitted.
pub fn pitch(audio: &Path, model: &Path) -> Result<Vec<PitchNote>> {
    Ok(pitch_with_evidence(audio, model)?
        .into_iter()
        .map(|evidence| evidence.candidate)
        .collect())
}

pub fn pitch_with_evidence(audio: &Path, model: &Path) -> Result<Vec<PitchCandidateEvidence>> {
    let activations = pitch_activations(audio, model)?;
    Ok(clip_pitch_candidates(
        decode_pitch_with_evidence(&activations.frames, &activations.onsets),
        activations.audio.len() as f64 / 22050.,
    ))
}

/// Short picked guitar notes must have both an actual source attack and a
/// stable owned pitch window. The generic vocal decoder remains unchanged.
pub fn guitar_pitch_with_evidence(audio: &Path, model: &Path) -> Result<GuitarPitchInference> {
    guitar_pitch_with_evidence_for_source(audio, model, crate::mapping::GuitarSource::Guitar)
}

pub(crate) fn guitar_pitch_with_evidence_for_source(
    audio: &Path,
    model: &Path,
    source: crate::mapping::GuitarSource,
) -> Result<GuitarPitchInference> {
    let activations = pitch_activations(audio, model)?;
    Ok(postprocessing::guitar_from_activations(
        activations,
        &decode(audio, 11025, 1)?,
        source,
    ))
}

/// Bass-only recovery, independent of the generic guitar/vocal decoder. Long
/// candidates come from the unchanged pass; shorter network hypotheses need
/// both source-attack and owned acoustic pitch evidence before joining them.
pub fn bass_pitch_with_evidence(audio: &Path, model: &Path) -> Result<BassPitchInference> {
    let activations = pitch_activations(audio, model)?;
    let pitch_audio = decode(audio, 11025, 1)?;
    Ok(recover_short_bass(&activations, &pitch_audio))
}

pub(crate) fn pitch_activations(audio: &Path, model: &Path) -> Result<PitchActivations> {
    const WINDOW: usize = 43844;
    const HOP: usize = WINDOW - 30 * 256;
    let audio = decode(audio, 22050, 1)?;
    let original = audio.len();
    let mut padded = vec![0.0; 15 * 256];
    padded.extend_from_slice(&audio);
    let mut model = session(model)?;
    let mut notes = Vec::new();
    let mut onsets = Vec::new();
    for start in (0..padded.len()).step_by(HOP) {
        let mut chunk = padded[start..(start + WINDOW).min(padded.len())].to_vec();
        chunk.resize(WINDOW, 0.0);
        let tensor = Tensor::from_array(([1, WINDOW, 1], chunk.into_boxed_slice()))?;
        let outputs = model.run(ort::inputs!["serving_default_input_2:0" => tensor])?;
        for (label, target) in [
            ("StatefulPartitionedCall:1", &mut notes),
            ("StatefulPartitionedCall:2", &mut onsets),
        ] {
            let (shape, data) = outputs[label].try_extract_tensor::<f32>()?;
            ensure!(
                shape.len() == 3 && shape[1] == 172 && shape[2] == 88,
                "Unexpected Basic Pitch output shape: {shape:?}"
            );
            target.extend_from_slice(&data[15 * 88..(172 - 15) * 88]);
        }
        eprintln!(
            "Pitch: {:.1}/{:.1} seconds",
            (start + HOP).min(original) as f64 / 22050.0,
            original as f64 / 22050.0
        );
        crate::generation_progress::units(
            format!(
                "Detecting instrument pitches · {:.0}/{:.0} seconds",
                (start + HOP).min(original) as f64 / 22050.0,
                original as f64 / 22050.0
            ),
            (start + HOP).min(original),
            original,
        );
    }
    let duration = original as f64 / 22050.0;
    let expected = (0..notes.len() / 88)
        .take_while(|frame| pitch_frame_seconds(*frame) < duration)
        .count();
    notes.truncate(expected * 88);
    onsets.truncate(expected * 88);
    Ok(PitchActivations {
        frames: notes,
        onsets,
        audio,
    })
}

#[cfg(feature = "separation")]
pub fn separate(audio: &Path, model: &Path, dir: &Path) -> Result<()> {
    use burn::backend::Wgpu;
    use demucs_core::listener::{ForwardEvent, ForwardListener};
    use demucs_core::{Demucs, ModelOptions};
    struct Progress;
    impl ForwardListener for Progress {
        fn on_event(&mut self, event: ForwardEvent) {
            match event {
                ForwardEvent::ChunkStarted { index, total } => {
                    eprintln!("Separation: chunk {}/{total}", index + 1);
                    crate::generation_progress::units(
                        format!("Separating instruments · chunk {} of {total}", index + 1),
                        index,
                        total,
                    );
                }
                ForwardEvent::ChunkDone { index, total } => {
                    eprintln!("Separation: completed {}/{total}", index + 1);
                    crate::generation_progress::units(
                        format!("Separated {} of {total} chunks", index + 1),
                        index + 1,
                        total,
                    );
                }
                _ => {}
            }
        }
    }
    let data = decode(audio, 44100, 2)?;
    let left: Vec<_> = data.iter().step_by(2).copied().collect();
    let right: Vec<_> = data.iter().skip(1).step_by(2).copied().collect();
    eprintln!("Separating six stems with demucs-rs");
    crate::generation_progress::detail("Loading the six-part separation model onto the GPU");
    let bytes = fs::read(model)?;
    let demucs = Demucs::<Wgpu>::from_bytes(ModelOptions::SixStem, &bytes, Default::default())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let stems =
        pollster::block_on(demucs.separate_with_listener(&left, &right, 44100, &mut Progress))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    let ids: std::collections::BTreeSet<_> = stems.iter().map(|stem| stem.id.as_str()).collect();
    ensure!(
        ids == ["drums", "bass", "other", "vocals", "guitar", "piano"]
            .into_iter()
            .collect(),
        "Separator did not return all six unique stems"
    );
    ensure!(
        stems.iter().all(|stem| stem.left.len() == left.len()
            && stem.right.len() == right.len()
            && stem.left.iter().chain(&stem.right).all(|s| s.is_finite())),
        "Separator returned truncated or nonfinite audio; no complete cache can be published"
    );
    fs::create_dir_all(dir)?;
    crate::generation_progress::detail("Saving all six separated instrument recordings");
    for stem in stems {
        write_audio(
            &dir.join(format!("{}.wav", stem.id.as_str())),
            &stem.left,
            &stem.right,
        )?;
    }
    Ok(())
}

#[cfg(not(feature = "separation"))]
pub fn separate(_: &Path, _: &Path, _: &Path) -> Result<()> {
    anyhow::bail!("Built without separation; supply existing stems or build with default features")
}

#[cfg(test)]
mod optional_directory_tests {
    use super::*;
    #[test]
    fn missing_optional_assets_use_cache_and_existing_bundle_assets_stay_put() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("YACE.app/Contents/Resources/models");
        let cache = temp.path().join("cache");
        let nested = Path::new("vocal-alignment/en");
        fs::create_dir_all(root.join(nested)).unwrap();
        let select = || {
            optional_directory_for(
                &root,
                nested,
                &["config.json", "model.safetensors"],
                Some(&cache),
            )
            .unwrap()
        };
        fs::write(root.join(nested).join("config.json"), b"retained").unwrap();
        assert_eq!(select(), cache.join(nested));
        assert!(!cache.exists());
        fs::write(
            root.join(nested).join("model.safetensors"),
            b"location fixture",
        )
        .unwrap();
        assert_eq!(select(), root.join(nested));
        assert_eq!(
            fs::read(root.join(nested).join("config.json")).unwrap(),
            b"retained"
        );
    }
    #[test]
    fn custom_paths_stay_exact_and_bundle_symlinks_cannot_redirect_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("custom");
        let cache = temp.path().join("cache");
        assert_eq!(
            optional_directory_for(&custom, Path::new(""), &["model"], Some(&cache)).unwrap(),
            custom
        );
        #[cfg(unix)]
        {
            let bundled = temp.path().join("YACE.app/Contents/Resources/models");
            fs::create_dir_all(&bundled).unwrap();
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(&bundled, &alias).unwrap();
            assert_eq!(
                optional_directory_for(&alias, Path::new(""), &["model"], Some(&cache)).unwrap(),
                cache
            );
            assert!(optional_directory_for(&alias, Path::new(""), &["model"], None).is_err());
        }
    }
}
