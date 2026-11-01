//! Retain source-bound stems and route complete sets to game instrument channels.
use super::{StemId, StemManifest};
use crate::storage as fs;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const RECEIPT: &str = "yace-audio.json";
const GAME_NAMES: [&str; 15] = [
    "song", "guitar", "bass", "rhythm", "keys", "vocals", "vocals_1", "vocals_2", "drums",
    "drums_1", "drums_2", "drums_3", "drums_4", "crowd", "piano",
];
const EXTENSIONS: [&str; 8] = ["wav", "ogg", "opus", "mp3", "aiff", "flac", "m4a", "aac"];

#[derive(Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub mode: String,
    pub detail: String,
    pub files: BTreeMap<String, String>,
    pub original_audio: Option<PathBuf>,
    pub original_sha256: String,
}

/// Content-addressed copies preserve attachments used by undo history as well.
pub fn retain(project_root: &Path, manifest: &StemManifest) -> Result<StemManifest> {
    retain_in(&project_root.join("source"), manifest)
}

pub fn retain_in(source_folder: &Path, manifest: &StemManifest) -> Result<StemManifest> {
    let mut retained = manifest.clone();
    for asset in &mut retained.stems {
        let extension = asset
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .context("Stem needs an extension")?;
        ensure!(
            extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) && extension.len() <= 10,
            "Unsupported stem extension"
        );
        ensure!(
            crate::models::hash_file(&asset.path)? == asset.sha256,
            "{} stem changed before retention",
            asset.id.as_str()
        );
        let folder = source_folder.join("stems").join(&asset.sha256);
        fs::create_dir_all(&folder)?;
        let target = folder.join(format!(
            "{}.{}",
            asset.id.as_str(),
            extension.to_ascii_lowercase()
        ));
        if !fs::exists(&target) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut temporary = tempfile::NamedTempFile::new_in(&folder)?;
                std::io::copy(&mut fs::File::open(&asset.path)?, temporary.as_file_mut())?;
                temporary.as_file().sync_all()?;
                ensure!(
                    crate::models::hash_file(temporary.path())? == asset.sha256,
                    "Stem changed while retaining it"
                );
                temporary
                    .persist_noclobber(&target)
                    .map_err(|error| error.error)?;
            }
            #[cfg(target_arch = "wasm32")]
            fs::copy(&asset.path, &target)?;
        }
        ensure!(
            crate::models::hash_file(&target)? == asset.sha256,
            "Retained stem changed"
        );
        asset.path = fs::canonicalize(target)?;
    }
    Ok(retained)
}

pub fn filename(id: StemId) -> &'static str {
    match id {
        StemId::Drums => "drums.wav",
        StemId::Bass => "bass.wav",
        StemId::Vocals => "vocals.wav",
        StemId::Guitar => "guitar.wav",
        StemId::Piano => "keys.wav",
        StemId::Other => "song.wav",
    }
}

/// A partial set cannot replace the recording without dropping instruments.
pub fn write(source: &Path, manifest: Option<&StemManifest>, stage: &Path) -> Result<Receipt> {
    let original_sha256 = crate::models::hash_file(source)?;
    let mut receipt = Receipt {
        version: 1,
        mode: "original".into(),
        detail: "Original song audio retained. Independent game muting requires all six separated parts.".into(),
        files: BTreeMap::new(),
        original_audio: None,
        original_sha256,
    };
    let Some(manifest) =
        manifest.filter(|manifest| manifest.ids() == StemId::ALL.into_iter().collect())
    else {
        fs::write(stage.join(RECEIPT), serde_json::to_vec_pretty(&receipt)?)?;
        return Ok(receipt);
    };
    #[cfg(not(target_arch = "wasm32"))]
    let duration = crate::pipeline::metadata(source)?.duration;
    #[cfg(target_arch = "wasm32")]
    let duration = crate::models::duration(source)?;
    manifest.validate_source(source, &receipt.original_sha256, duration)?;
    let saved = stage.join(".yace-audio");
    fs::create_dir_all(&saved)?;
    let original_name = format!(
        "original.{}",
        source
            .extension()
            .context("Recording needs an extension")?
            .to_string_lossy()
    );
    let original_path = saved.join(&original_name);
    fs::copy(source, &original_path)?;
    ensure!(
        crate::models::hash_file(&original_path)? == receipt.original_sha256,
        "Recording changed during export"
    );
    receipt.original_audio = Some(Path::new(".yace-audio").join(original_name));

    // Move every recognized source channel out of the game's top-level scan.
    // Keeping a full mix beside separated channels would play the music twice.
    let archived = saved.join("source-channels");
    fs::create_dir_all(&archived)?;
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if entry.file_type()?.is_file()
            && GAME_NAMES.contains(&name.as_str())
            && EXTENSIONS.contains(&extension.as_str())
        {
            let target = archived.join(entry.file_name());
            // This is an unpublished staging directory; copied recovery files
            // remain in the previous immutable export/source snapshot.
            if fs::exists(&target) {
                fs::remove_file(&target)?;
            }
            fs::rename(path, target)?;
        }
    }
    for asset in &manifest.stems {
        let name = filename(asset.id);
        let target = stage.join(name);
        if asset
            .path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        {
            fs::copy(&asset.path, &target)?;
            ensure!(
                crate::models::hash_file(&target)? == asset.sha256,
                "Stem changed during game export"
            );
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let result = Command::new("ffmpeg")
                    .args(["-v", "error", "-nostdin", "-n", "-i"])
                    .arg(&asset.path)
                    .args(["-map", "0:a:0", "-c:a", "pcm_f32le"])
                    .arg(&target)
                    .output()?;
                ensure!(
                    result.status.success(),
                    "Could not encode {}: {}",
                    name,
                    String::from_utf8_lossy(&result.stderr)
                );
                ensure!(
                    crate::models::decode(&target, super::SAMPLE_RATE, 2)?.len() as u64
                        == manifest.frames * 2,
                    "Export changed stem duration"
                );
            }
            #[cfg(target_arch = "wasm32")]
            {
                let pcm = crate::models::decode(&asset.path, super::SAMPLE_RATE, 2)?;
                ensure!(
                    pcm.len() as u64 == manifest.frames * 2,
                    "Export changed stem duration"
                );
                fs::write(
                    &target,
                    crate::audio_decode::Pcm {
                        stereo: pcm,
                        rate: super::SAMPLE_RATE,
                        channels: 2,
                    }
                    .float_wave()?,
                )?;
            }
        }
        ensure!(
            crate::models::hash_file(&asset.path)? == asset.sha256,
            "Stem changed during game export"
        );
        receipt.files.insert(asset.id.as_str().into(), name.into());
    }
    receipt.mode = "instrument_stems".into();
    receipt.detail = "Independent guitar, bass, drums, vocals and keys channels; song.wav contains only the other-instruments stem. Original recording is archived. Game mute-on-miss depends on game settings. Separation quality and Rock Band console packaging are unverified.".into();
    fs::write(stage.join(RECEIPT), serde_json::to_vec_pretty(&receipt)?)?;
    Ok(receipt)
}

/// Reopening our game export uses the archived recording for its original clock.
pub fn restore_recording(folder: &Path) -> Result<()> {
    let path = folder.join(RECEIPT);
    if !fs::is_file(&path) {
        return Ok(());
    }
    let receipt: Receipt = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(receipt.version == 1, "Unsupported audio export receipt");
    let Some(original) = receipt.original_audio else {
        return Ok(());
    };
    ensure!(
        original
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "Invalid original audio path"
    );
    let original = folder.join(original);
    ensure!(
        crate::models::hash_file(&original)? == receipt.original_sha256,
        "Archived recording changed"
    );
    // Restore the source channel layout too; a later partial attachment must
    // not export the restored full mix alongside channels from this game copy.
    for filename in receipt.files.values() {
        ensure!(
            Path::new(filename).components().count() == 1
                && StemId::ALL
                    .into_iter()
                    .any(|id| self::filename(id) == filename),
            "Invalid game audio filename"
        );
        let path = folder.join(filename);
        if fs::is_file(&path) {
            fs::remove_file(path)?;
        }
    }
    let channels = folder.join(".yace-audio/source-channels");
    if fs::exists(&channels) {
        for entry in fs::read_dir(channels)? {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_file(),
                "Invalid archived source channel"
            );
            fs::copy(entry.path(), folder.join(entry.file_name()))?;
        }
    }
    for extension in EXTENSIONS {
        let file = folder.join(format!("song.{extension}"));
        if fs::is_file(&file) {
            fs::remove_file(file)?;
        }
    }
    let extension = original
        .extension()
        .context("Archived recording needs an extension")?;
    fs::copy(
        &original,
        folder.join(format!("song.{}", extension.to_string_lossy())),
    )?;
    Ok(())
}
