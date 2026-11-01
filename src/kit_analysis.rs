//! Saved kit validation and native separation.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumPartFile {
    pub id: String,
    pub label: String,
    pub filename: String,
    pub sha256: String,
    pub frames: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrumPartSet {
    pub source_key: String,
    pub original_audio_sha256: String,
    pub drum_audio_sha256: String,
    pub directory: PathBuf,
    pub parts: Vec<DrumPartFile>,
    pub receipt: crate::larsnet::SeparationReceipt,
}
/// A persisted listening preference, independent of chart notes and approvals.
pub use crate::kit_audition::KitAudition;
pub struct KitClip {
    pub part: String,
    pub start: f64,
    pub end: f64,
    pub wav: Vec<u8>,
}

/// Audio analysis can survive note edits. Never reuse it for changed stems or
/// another recording merely because the song title or duration happens to match.
pub use crate::kit_audition::source_key as kit_source_key;
fn project_drums(store: &Store, project: &Project) -> Result<(PathBuf, String)> {
    let original = store.audio_path(project)?;
    ensure!(
        models::hash_file(&original)? == project.audio_sha256,
        "Project recording changed"
    );
    let manifest = project
        .content
        .stems
        .as_ref()
        .context("Attach aligned drum audio before splitting the kit")?;
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    let asset = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Drums)
        .context("No drum stem is attached")?;
    ensure!(
        models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
        "Attached drum audio changed"
    );
    Ok((asset.path.clone(), asset.sha256.clone()))
}

const KIT_JSON_LIMIT: u64 = 256 * 1024;
fn read_kit_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= KIT_JSON_LIMIT,
        "Invalid or oversized kit cache JSON"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(KIT_JSON_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= KIT_JSON_LIMIT,
        "Kit cache JSON grew while reading"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_cached_kit(
    set: &DrumPartSet,
    cache: &Path,
    project: &Project,
    source_key: &str,
    drum_hash: &str,
) -> Result<()> {
    use crate::larsnet;
    let frames = project.content.stems.as_ref().context("No stems")?.frames;
    ensure!(
        set.source_key == source_key
            && set.original_audio_sha256 == project.audio_sha256
            && set.drum_audio_sha256 == drum_hash
            && set.receipt.source_audio_sha256.as_deref() == Some(drum_hash),
        "Stale kit cache source"
    );
    let receipt = &set.receipt;
    ensure!(
        receipt.version == 1
            && receipt.algorithm == larsnet::ALGORITHM
            && larsnet::supported_implementation(&receipt.implementation_sha256)
            && receipt.sample_rate == 44100
            && receipt.channels == 2
            && receipt.frames == frames
            && receipt.weight_license == larsnet::WEIGHT_LICENSE,
        "Unsupported kit cache receipt"
    );
    let expected: BTreeMap<String, String> = larsnet::MODELS
        .into_iter()
        .map(|(id, hash)| (id.into(), hash.into()))
        .collect();
    ensure!(
        receipt.models == expected && set.parts.len() == 5 && receipt.outputs.len() == 5,
        "Incomplete kit cache or model identities"
    );
    let directory = cache.join("parts");
    ensure!(
        fs::symlink_metadata(&directory)?.is_dir()
            && fs::canonicalize(&set.directory)? == fs::canonicalize(&directory)?,
        "Kit cache points outside its component folder"
    );
    let native: larsnet::SeparationReceipt =
        read_kit_json(&directory.join(larsnet::RECEIPT_FILENAME))?;
    ensure!(
        serde_json::to_value(&native)? == serde_json::to_value(receipt)?,
        "Kit cache/native receipt mismatch"
    );
    let valid_hash = |hash: &str| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        valid_hash(&receipt.source_pcm_sha256),
        "Invalid kit source PCM identity"
    );
    for (id, label) in [
        ("kick", "Kick"),
        ("snare", "Snare"),
        ("toms", "Toms"),
        ("hihat", "Hi-hat"),
        ("cymbals", "Cymbals"),
    ] {
        ensure!(
            set.parts.iter().filter(|p| p.id == id).count() == 1
                && receipt.outputs.iter().filter(|p| p.part == id).count() == 1,
            "Duplicate or missing kit component"
        );
        let part = set
            .parts
            .iter()
            .find(|p| p.id == id)
            .context("Missing kit part")?;
        let output = receipt
            .outputs
            .iter()
            .find(|p| p.part == id)
            .context("Missing native part receipt")?;
        ensure!(
            part.label == label
                && part.filename == format!("{id}.wav")
                && output.filename == part.filename
                && part.frames == frames
                && output.frames == frames
                && output.wav_sha256.as_ref() == Some(&part.sha256)
                && valid_hash(&part.sha256)
                && valid_hash(&output.pcm_sha256),
            "Invalid cached component metadata"
        );
        let path = directory.join(&part.filename);
        let metadata = fs::symlink_metadata(&path)?;
        // Native output has a 44-byte IEEE-float RIFF header and eight bytes
        // per stereo frame. Check the frame count without decoding the audio.
        let data_bytes = u32::try_from(frames.checked_mul(8).context("Kit frame count overflow")?)?;
        let riff_bytes = data_bytes
            .checked_add(36)
            .context("Kit RIFF size overflow")?;
        ensure!(
            metadata.is_file() && metadata.len() == u64::from(data_bytes) + 44,
            "Cached component has another frame count"
        );
        let mut header = [0u8; 44];
        fs::File::open(&path)?.read_exact(&mut header)?;
        ensure!(
            &header[0..4] == b"RIFF"
                && &header[8..16] == b"WAVEfmt "
                && u32::from_le_bytes(header[4..8].try_into()?) == riff_bytes
                && u32::from_le_bytes(header[16..20].try_into()?) == 16
                && u16::from_le_bytes(header[20..22].try_into()?) == 3
                && u16::from_le_bytes(header[22..24].try_into()?) == 2
                && u32::from_le_bytes(header[24..28].try_into()?) == 44100
                && u32::from_le_bytes(header[28..32].try_into()?) == 44100 * 8
                && u16::from_le_bytes(header[32..34].try_into()?) == 8
                && u16::from_le_bytes(header[34..36].try_into()?) == 32
                && &header[36..40] == b"data"
                && u32::from_le_bytes(header[40..44].try_into()?) == data_bytes,
            "Invalid native kit audio header/clock"
        );
        ensure!(
            models::hash_file(&path)? == part.sha256,
            "Cached kit audio changed"
        );
    }
    Ok(())
}

/// Restore the newest intact source-bound five-part cache without loading a
/// model, decoding full audio, changing notes, or writing a review decision.
/// Invalid/stale candidates are skipped; a changed current recording or stem
/// remains an error instead of masquerading as an absent cache.
pub fn load_project_drum_parts(store: &Store, project: &Project) -> Result<Option<DrumPartSet>> {
    if project
        .content
        .stems
        .as_ref()
        .is_none_or(|m| !m.stems.iter().any(|s| s.id == StemId::Drums))
    {
        return Ok(None);
    }
    let (_, drum_hash) = project_drums(store, project)?;
    let source_key = kit_source_key(project)?;
    let analysis = store.root.join("analysis");
    let entries = match fs::read_dir(&analysis) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if !entry.file_name().to_string_lossy().starts_with("kit-")
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let path = entry.path().join("kit-parts.json");
        if let Ok(meta) = fs::symlink_metadata(&path) {
            candidates.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), path));
        }
    }
    // Keep an explicitly saved listening choice when several intact runs exist.
    let preferred = project.kit_audition.as_ref().map(|selected| &selected.kit);
    candidates.sort_by(|a, b| {
        (Some(&b.1) == preferred)
            .cmp(&(Some(&a.1) == preferred))
            .then(b.0.cmp(&a.0))
            .then(b.1.cmp(&a.1))
    });
    let mut selected = None;
    for (_, path) in candidates {
        let Ok(set) = read_kit_json::<DrumPartSet>(&path) else {
            continue;
        };
        if validate_cached_kit(
            &set,
            path.parent().context("Missing kit cache folder")?,
            project,
            &source_key,
            &drum_hash,
        )
        .is_ok()
        {
            selected = Some(set);
            break;
        }
    }
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed while loading kit cache"
    );
    Ok(selected)
}

/// Select a complete current cache. Call on a worker: all five WAV hashes are
/// checked, and the descriptor never admits an arbitrary caller-provided file.
pub fn select_kit_audition(store: &Store, project: &Project, part: &str) -> Result<KitAudition> {
    let set = load_project_drum_parts(store, project)?.context("Split the drum kit first")?;
    let file = set
        .parts
        .iter()
        .find(|file| file.id == part)
        .context("Unknown kit part")?;
    Ok(KitAudition {
        source_key: set.source_key.clone(),
        kit: set
            .directory
            .parent()
            .context("Missing kit cache directory")?
            .join("kit-parts.json"),
        part: file.id.clone(),
        label: file.label.clone(),
        audio_sha256: file.sha256.clone(),
        frames: file.frames,
    })
}

/// Validate and decode one full-length component on the recording clock. The
/// transport performs rate conversion separately; no offset or trim is applied.
pub fn validated_kit_audition_path(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
) -> Result<PathBuf> {
    ensure!(
        selected.source_key == kit_source_key(project)?,
        "Kit audition belongs to different source audio"
    );
    let (_, drum_hash) = project_drums(store, project)?;
    let kit = fs::canonicalize(&selected.kit)?;
    let cache = kit.parent().context("Missing kit cache")?;
    let analysis = fs::canonicalize(store.root.join("analysis"))?;
    ensure!(
        kit.file_name().is_some_and(|name| name == "kit-parts.json")
            && cache.parent() == Some(analysis.as_path())
            && cache
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("kit-")),
        "Kit audition must use a saved cache in this project"
    );
    let set: DrumPartSet = read_kit_json(&kit)?;
    validate_cached_kit(&set, cache, project, &selected.source_key, &drum_hash)?;
    let file = set
        .parts
        .iter()
        .find(|file| file.id == selected.part)
        .context("Unknown kit part")?;
    ensure!(
        file.label == selected.label
            && file.sha256 == selected.audio_sha256
            && file.frames == selected.frames,
        "Kit audition descriptor changed"
    );
    let path = set.directory.join(&file.filename);
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed while validating kit audition"
    );
    Ok(path)
}

pub fn load_kit_audition(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
) -> Result<(PathBuf, Vec<f32>)> {
    let path = validated_kit_audition_path(store, project, selected)?;
    let (_, drum_hash) = project_drums(store, project)?;
    let pcm = models::decode(&path, 44100, 2)?;
    ensure!(
        pcm.iter().all(|sample| sample.is_finite()),
        "Kit audition contains nonfinite PCM"
    );
    ensure!(
        pcm.len() as u64 == selected.frames * 2,
        "Kit audition has another sample clock"
    );
    ensure!(
        models::hash_file(&path)? == selected.audio_sha256,
        "Kit component changed during decode"
    );
    ensure!(
        project_drums(store, project)?.1 == drum_hash,
        "Drum source changed during kit decode"
    );
    Ok((path, pcm))
}

/// Prepare the component for the exact already prepared shared transport.
/// Source frames are exact at normal speed; atempo's bounded trailing padding
/// follows the same policy as the ordinary instrument mixer.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_kit_audition(
    store: &Store,
    project: &Project,
    selected: &KitAudition,
    rate: f64,
    transport_samples: usize,
) -> Result<Vec<f32>> {
    ensure!(
        rate.is_finite()
            && (0.5..=1.5).contains(&rate)
            && transport_samples > 0
            && transport_samples.is_multiple_of(2),
        "Invalid kit transport clock"
    );
    let (path, pcm) = load_kit_audition(store, project, selected)?;
    let mut pcm = if rate == 1.0 {
        pcm
    } else {
        models::decode_at_tempo(&path, 44100, 2, rate)?
    };
    ensure!(
        pcm.iter().all(|sample| sample.is_finite()),
        "Practice kit audition contains nonfinite PCM"
    );
    ensure!(
        models::hash_file(&path)? == selected.audio_sha256,
        "Kit audio changed during practice preparation"
    );
    if rate == 1.0 {
        ensure!(
            pcm.len() == transport_samples,
            "Kit source and shared transport have different frame counts"
        );
    } else {
        ensure!(
            pcm.len().abs_diff(transport_samples) <= ((44100.0 * 0.10 / rate) as usize) * 2,
            "Tempo-converted kit duration differs by over 100 ms"
        );
    }
    pcm.resize(transport_samples, 0.0);
    Ok(pcm)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_project_drum_parts(
    store: &Store,
    project: &Project,
    model_dir: &Path,
) -> Result<DrumPartSet> {
    let (drums, hash) = project_drums(store, project)?;
    // Full recording/stem frame validation precedes any expensive model run.
    drop(
        project
            .content
            .stems
            .as_ref()
            .context("No stems")?
            .load(&store.audio_path(project)?)?,
    );
    let analysis = store.root.join("analysis");
    fs::create_dir_all(&analysis)?;
    let analysis = analysis.canonicalize()?;
    let stage = tempfile::Builder::new()
        .prefix("kit-")
        .tempdir_in(&analysis)?;
    let output = stage.path().join("parts");
    let receipt = crate::larsnet::separate_files(&drums, model_dir, &output)?;
    ensure!(
        receipt.source_audio_sha256.as_ref() == Some(&hash),
        "Separated drum receipt belongs to another source"
    );
    ensure!(
        receipt.frames == project.content.stems.as_ref().context("No stems")?.frames,
        "Separated kit changed recording length"
    );
    let parts = receipt
        .outputs
        .iter()
        .map(|part| -> Result<_> {
            let label = match part.part.as_str() {
                "kick" => "Kick",
                "snare" => "Snare",
                "toms" => "Toms",
                "hihat" => "Hi-hat",
                "cymbals" => "Cymbals",
                _ => anyhow::bail!("Unexpected separated kit output"),
            };
            Ok(DrumPartFile {
                id: part.part.clone(),
                label: label.into(),
                filename: part.filename.clone(),
                sha256: part
                    .wav_sha256
                    .clone()
                    .context("Separated audio hash missing")?,
                frames: part.frames,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (_, fresh_hash) = project_drums(store, project)?;
    ensure!(fresh_hash == hash, "Drum source changed during separation");
    let result = DrumPartSet {
        source_key: kit_source_key(project)?,
        original_audio_sha256: project.audio_sha256.clone(),
        drum_audio_sha256: hash,
        directory: output,
        parts,
        receipt,
    };
    fs::write(
        stage.path().join("kit-parts.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    let _durable = stage.keep();
    Ok(result)
}

/// Export a bounded listening clip on the original recording clock. All file
/// reads and conversion must run on a worker; no main-player state is changed.
#[cfg(not(target_arch = "wasm32"))]
pub fn audition_clip(
    store: &Store,
    project: &Project,
    set: &DrumPartSet,
    part_id: &str,
    start: f64,
    end: f64,
) -> Result<KitClip> {
    ensure!(
        start.is_finite()
            && end.is_finite()
            && start >= 0.
            && end > start
            && end <= project.duration
            && end - start <= 20.,
        "Clip must be a valid recording range of at most20seconds"
    );
    ensure!(
        set.source_key == kit_source_key(project)?
            && set.original_audio_sha256 == project.audio_sha256,
        "Separated kit belongs to another recording or stem attachment"
    );
    let (_, hash) = project_drums(store, project)?;
    ensure!(
        set.drum_audio_sha256 == hash && set.receipt.source_audio_sha256.as_ref() == Some(&hash),
        "Separated kit source receipt changed"
    );
    let part = set
        .parts
        .iter()
        .find(|p| p.id == part_id)
        .context("Unknown separated kit part")?;
    ensure!(
        set.receipt.version == 1
            && set.receipt.algorithm == crate::larsnet::ALGORITHM
            && crate::larsnet::supported_implementation(&set.receipt.implementation_sha256)
            && set.receipt.sample_rate == 44100
            && set.receipt.channels == 2,
        "Unsupported separated kit receipt"
    );
    let expected_models: BTreeMap<String, String> = crate::larsnet::MODELS
        .into_iter()
        .map(|(id, hash)| (id.into(), hash.into()))
        .collect();
    ensure!(
        set.receipt.models == expected_models,
        "Separated kit model identities changed"
    );
    let source_frames = project.content.stems.as_ref().context("No stems")?.frames;
    ensure!(
        set.receipt.frames == source_frames && part.frames == source_frames,
        "Separated kit sample count differs from the attached recording clock"
    );
    let output = set
        .receipt
        .outputs
        .iter()
        .find(|p| p.part == part_id)
        .context("Part missing from native separation receipt")?;
    ensure!(
        output.filename == part.filename
            && output.frames == part.frames
            && output.wav_sha256.as_ref() == Some(&part.sha256),
        "Separated part metadata differs from its native receipt"
    );
    ensure!(
        part.filename == format!("{}.wav", part.id)
            && crate::larsnet::MODELS
                .iter()
                .any(|(name, _)| *name == part.id),
        "Invalid kit output filename"
    );
    let path = set.directory.join(&part.filename);
    ensure!(
        models::hash_file(&path)? == part.sha256,
        "Separated kit audio changed"
    );
    let audio = models::decode(&path, 44100, 2)?;
    ensure!(
        audio.len() / 2 == part.frames as usize && part.frames == set.receipt.frames,
        "Separated kit has another sample count"
    );
    let first = (start * 44100.).floor() as usize;
    let last = ((end * 44100.).ceil() as usize).min(audio.len() / 2);
    ensure!(
        last > first && first < audio.len() / 2,
        "Empty audition range"
    );
    let left: Vec<_> = (first..last).map(|i| audio[i * 2]).collect();
    let right: Vec<_> = (first..last).map(|i| audio[i * 2 + 1]).collect();
    let stage = tempfile::tempdir()?;
    let clip = stage.path().join("clip.wav");
    models::write_audio(&clip, &left, &right)?;
    ensure!(
        models::hash_file(&path)? == part.sha256,
        "Separated kit audio changed during audition"
    );
    Ok(KitClip {
        part: part_id.into(),
        start: first as f64 / 44100.,
        end: last as f64 / 44100.,
        wav: fs::read(clip)?,
    })
}
