use crate::{
    chart::{self, Chart},
    mapping::{self, Transcription},
    midi, models,
    timing::TempoMap,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Serialize)]
pub struct Inventory {
    pub path: PathBuf,
    pub chart: Option<String>,
    pub tracks: Vec<(String, usize)>,
    pub missing: Vec<String>,
    pub error: Option<String>,
}

pub fn chart_file(folder: &Path) -> Result<PathBuf> {
    let chart = folder.join("notes.chart");
    let mid = folder.join("notes.mid");
    ensure!(
        !(chart.exists() && mid.exists()),
        "Both notes.chart and notes.mid exist; choose one in a working copy before enhancement"
    );
    if chart.is_file() {
        Ok(chart)
    } else if mid.is_file() {
        Ok(mid)
    } else {
        bail!("No notes.chart or notes.mid in {}", folder.display())
    }
}

/// A generated Pro Keys song has one authoritative game MIDI and a separately
/// archived editable master. Refuse mismatched pairs rather than edit stale data.
pub fn editable_chart_file(folder: &Path) -> Result<PathBuf> {
    let chart = chart_file(folder)?;
    if chart.extension().is_some_and(|e| e == "chart") {
        return Ok(chart);
    }
    let master = folder.join("chart-engine-working/notes.chart");
    let binding: serde_json::Value = serde_json::from_slice(
        &fs::read(folder.join("chart-engine-working/binding.json"))
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

pub fn song_ini(folder: &Path) -> Result<String> {
    match fs::read_to_string(folder.join("song.ini")) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).context(
            "Cannot read song.ini; preserving source rather than replacing unreadable metadata",
        ),
    }
}

pub fn ini_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut song = false;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') {
            song = line.eq_ignore_ascii_case("[song]");
        } else if song && let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    fields
}

pub fn validate_existing(folder: &Path) -> Result<()> {
    let fields = ini_fields(&song_ini(folder)?);
    if let Some(delay) = fields.get("delay") {
        ensure!(
            delay.parse::<f64>().context("Invalid legacy delay")? == 0.0,
            "Nonzero legacy song.ini delay is not yet supported; refusing to misalign notes"
        );
    }
    Ok(())
}

pub fn inspect(folder: &Path) -> Result<Inventory> {
    let path = chart_file(folder)?;
    let tracks = if path.extension().is_some_and(|e| e == "chart") {
        let chart = Chart::parse(fs::read_to_string(&path)?)?;
        chart
            .sections
            .iter()
            .filter(|s| {
                ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"]
                    .iter()
                    .any(|suffix| s.name.ends_with(suffix))
            })
            .map(|s| {
                Ok((
                    s.name.clone(),
                    chart
                        .notes(&s.name)?
                        .iter()
                        .filter(|n| crate::editor::playable(&s.name, n.lane))
                        .count(),
                ))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        midi::inventory(&fs::read(&path)?)?
    };
    let missing = [
        ("guitar", "Single", "PART GUITAR"),
        ("bass", "DoubleBass", "PART BASS"),
        ("drums", "Drums", "PART DRUMS"),
    ]
    .into_iter()
    .filter(|(_, suffix, midi)| {
        !tracks
            .iter()
            .any(|(n, count)| *count > 0 && (n.ends_with(suffix) || n == midi))
    })
    .map(|(n, _, _)| n.to_owned())
    .collect();
    Ok(Inventory {
        path: folder.to_owned(),
        chart: Some(path.file_name().unwrap().to_string_lossy().into_owned()),
        tracks,
        missing,
        error: None,
    })
}

pub fn scan(root: &Path) -> Result<Vec<Inventory>> {
    ensure!(root.is_dir(), "Not a folder: {}", root.display());
    let mut out = Vec::new();
    let mut queue = vec![root.to_owned()];
    while let Some(dir) = queue.pop() {
        let entries: Vec<_> = fs::read_dir(&dir)?.collect::<std::io::Result<_>>()?;
        if entries
            .iter()
            .any(|e| e.file_name() == "notes.chart" || e.file_name() == "notes.mid")
        {
            out.push(inspect(&dir).unwrap_or_else(|e| Inventory {
                path: dir.clone(),
                chart: None,
                tracks: vec![],
                missing: vec![],
                error: Some(e.to_string()),
            }));
            continue;
        }
        for entry in entries {
            let ft = entry.file_type()?;
            if ft.is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
                && !entry.file_name().to_string_lossy().ends_with(".app")
            {
                queue.push(entry.path());
            } else if ft.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|e| e == "sng" || e == "srb")
            {
                out.push(Inventory { path:entry.path(),chart:None,tracks:vec![],missing:vec![],error:Some("Packed/encrypted song: unpack supported containers before chart enhancement".into()) });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn destination(out: &Path, source: Option<&Path>) -> Result<()> {
    ensure!(
        !out.exists(),
        "Output already exists; refusing to overwrite {}",
        out.display()
    );
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(
        parent.is_dir(),
        "Output parent does not exist: {}",
        parent.display()
    );
    if let Some(source) = source {
        let source = source.canonicalize()?;
        let destination = parent
            .canonicalize()?
            .join(out.file_name().context("Missing output name")?);
        ensure!(
            !destination.starts_with(source),
            "Output must be outside the source song folder"
        );
    }
    Ok(())
}

/// Publish a complete staged directory without replacing a concurrent output.
/// macOS/Linux expose atomic no-replace rename; errors preserve the stage.
pub fn publish(stage: &Path, output: &Path) -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from = CString::new(stage.as_os_str().as_bytes())?;
        let to = CString::new(output.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let status = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let status = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Cannot publish output without overwriting an existing path");
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        ensure!(
            !output.exists(),
            "Output appeared during processing; refusing to overwrite"
        );
        fs::rename(stage, output).context("Cannot publish output folder")
    }
}

pub fn audio_file(folder: &Path) -> Result<PathBuf> {
    for name in ["song", "guitar"] {
        for extension in ["ogg", "opus", "mp3", "flac", "wav", "m4a"] {
            let path = folder.join(format!("{name}.{extension}"));
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    bail!(
        "No full-song audio found in {}; supply --audio",
        folder.display()
    )
}

pub const STEM_MANIFEST: &str = "stem-manifest.json";

fn isolation_limits(manifest: &crate::stem_review::StemManifest) -> Vec<String> {
    let mut warnings = vec!["Source-bound stems were validated for hashes, format and full-song sample counts. Isolation can still leak other instruments, omit notes or shift transients; scores remain model outputs, not probabilities of correct playable notes. Review weak attacks, bass pitch and candidate scores; no listening approval is implied.".into()];
    if manifest.generation.is_none() {
        warnings.push("Supplied isolation is an explicit alignment attestation, not a contemporaneous model-bound native separation receipt. Its actual source/model history and phase alignment are not independently established.".into());
    }
    warnings
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IsolationOptions<'a> {
    pub guitar_source: crate::mapping::GuitarSource,
    /// Compatibility folder option; it must contain stem-manifest.json.
    pub stems: Option<&'a Path>,
    pub manifest: Option<&'a Path>,
    /// Defaults to model_dir/stem-cache, shared by successive generation jobs.
    pub cache_dir: Option<&'a Path>,
    /// Explicit lower-isolation fallback, restricted to drums-only requests.
    pub full_mix_drums: bool,
    /// Keyboard timbre selects the detector; arrangement remains independent.
    pub keyboard_sound: crate::keyboard_detection::KeyboardSound,
    pub piano_quality: crate::keyboard_detection::PianoQuality,
    /// A separate authoritative six-stem Piano analysis for mixed jobs.
    pub keyboard_analysis: Option<&'a Path>,
}

/// Same-filesystem transaction shared by native separation and deterministic
/// backend tests. The callback must write all six untrimmed stereo WAV files.
/// A failed/cancelled callback or changed source/model never publishes a cache.
pub fn isolation_cached_with(
    audio: &Path,
    model: &Path,
    cache_root: &Path,
    mut identity: crate::stem_review::SeparationIdentity,
    separate: impl FnOnce(&Path, &Path, &Path) -> Result<()>,
) -> Result<crate::stem_review::StemManifest> {
    let source_hash = models::hash_file(audio)?;
    ensure!(
        models::hash_file(model)? == identity.model_sha256,
        "Separation model does not match its identity"
    );
    identity.cache_key = identity.calculated_cache_key(&source_hash)?;
    fs::create_dir_all(cache_root)?;
    let output = cache_root.join(&identity.cache_key);
    if output.exists() {
        let manifest = read_isolation(audio, &output.join(STEM_MANIFEST))?;
        ensure!(
            manifest.generation.as_ref() == Some(&identity),
            "Cached separator identity does not match this operation"
        );
        ensure!(
            manifest.ids() == crate::stem_review::StemId::ALL.into_iter().collect(),
            "Generated cache is missing one or more of its six stems"
        );
        return Ok(manifest);
    }
    let stage = tempfile::Builder::new()
        .prefix(".separation-")
        .tempdir_in(cache_root)?;
    separate(audio, model, stage.path())?;
    ensure!(
        models::hash_file(audio)? == source_hash,
        "Source changed during separation; refusing to publish"
    );
    ensure!(
        models::hash_file(model)? == identity.model_sha256,
        "Model changed during separation; refusing to publish"
    );
    let mut manifest = crate::stem_review::StemManifest::build(
        audio,
        crate::stem_review::StemId::ALL.into_iter().map(|id| (id, stage.path().join(format!("{}.wav", id.as_str())))).collect(),
        crate::stem_review::AlignmentEvidence {
            method: "native_full_song_separation".into(),
            details: "Generated from the entire source decoded to 44100 Hz stereo, with no caller trim, time shift, gain normalization or rescale. All six outputs have exact decoded source frame counts and hash-bound source/model/parameters. This proves engineering provenance, not phase-perfect separation, instrument purity or musical correctness.".into(),
        },
    )?;
    // The source could have changed while full stem validation was running.
    ensure!(
        manifest.source_audio_sha256 == source_hash && models::hash_file(audio)? == source_hash,
        "Source changed during separation validation; refusing to publish"
    );
    ensure!(
        models::hash_file(model)? == identity.model_sha256,
        "Model changed during separation validation; refusing to publish"
    );
    manifest.generation = Some(identity);
    manifest.validate_binding(&source_hash, manifest.frames as f64 / 44100.0)?;
    manifest.write_relative(&stage.path().join(STEM_MANIFEST))?;
    // Keep a cache another worker finished first.
    // Report a corrupt existing cache instead of replacing it silently.
    publish(stage.path(), &output)?;
    read_isolation(audio, &output.join(STEM_MANIFEST))
}

pub fn cached_separation(
    audio: &Path,
    model_dir: &Path,
    cache_root: &Path,
) -> Result<crate::stem_review::StemManifest> {
    let model = models::verified(model_dir, "separation")?;
    isolation_cached_with(
        audio,
        &model,
        cache_root,
        models::separation_identity(&model)?,
        models::separate,
    )
}

pub fn cached_keyboard_separation(
    audio: &Path,
    model_dir: &Path,
    cache_root: &Path,
) -> Result<crate::stem_review::StemManifest> {
    let model = crate::keyboard_separation::verified(model_dir)?;
    isolation_cached_with(
        audio,
        &model,
        cache_root,
        crate::keyboard_separation::identity(&model)?,
        crate::keyboard_separation::separate,
    )
}

pub struct IsolationPlan {
    pub primary: crate::stem_review::StemManifest,
    pub keyboard_analysis: Option<crate::stem_review::StemManifest>,
}

fn higher_quality_manifest(manifest: &crate::stem_review::StemManifest) -> bool {
    manifest.generation.as_ref().is_some_and(|identity| {
        identity.model_id == crate::keyboard_separation::MODEL_ID
            && identity.model_sha256 == crate::keyboard_separation::asset().sha256
    })
}

/// Keep existing instrument inputs intact. A Piano-only high-quality job does
/// not need the standard separator; mixed jobs retain two complete manifests.
pub fn prepare_keyboard_isolation(
    audio: &Path,
    model_dir: &Path,
    cache_root: &Path,
    instruments: &[String],
    sound: crate::keyboard_detection::KeyboardSound,
    quality: crate::keyboard_detection::PianoQuality,
) -> Result<IsolationPlan> {
    let hq = crate::keyboard_detection::high_quality_piano(instruments, sound, quality);
    let piano_only = instruments
        .iter()
        .all(|s| matches!(s.as_str(), "piano" | "pro_piano"));
    let primary = if hq && piano_only {
        cached_keyboard_separation(audio, model_dir, cache_root)?
    } else {
        cached_separation(audio, model_dir, cache_root)?
    };
    let keyboard_analysis = if hq && !piano_only {
        Some(cached_keyboard_separation(audio, model_dir, cache_root)?)
    } else {
        None
    };
    Ok(IsolationPlan {
        primary,
        keyboard_analysis,
    })
}

/// Read the same source-bound manifest used by the editor, checking actual
/// native format, exact frame count, finite samples and every current file hash.
pub fn read_isolation(audio: &Path, path: &Path) -> Result<crate::stem_review::StemManifest> {
    let manifest = crate::stem_review::StemManifest::read(path)?;
    manifest.validate_source(audio, &models::hash_file(audio)?, metadata(audio)?.duration)
}

/// Publish a portable copy of a complete cache for sharing or editor attachment.
pub fn export_isolation(
    audio: &Path,
    manifest: &crate::stem_review::StemManifest,
    output: &Path,
) -> Result<()> {
    destination(output, None)?;
    manifest.validate_source(audio, &models::hash_file(audio)?, metadata(audio)?.duration)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".stem-export-")
        .tempdir_in(parent)?;
    let mut portable = manifest.clone();
    for asset in &mut portable.stems {
        let extension = asset
            .path
            .extension()
            .context("Stem file has no extension")?
            .to_string_lossy();
        let target = stage
            .path()
            .join(format!("{}.{extension}", asset.id.as_str()));
        fs::copy(&asset.path, &target)?;
        ensure!(
            models::hash_file(&target)? == asset.sha256,
            "Stem changed during export"
        );
        asset.path = target.canonicalize()?;
    }
    portable.write_relative(&stage.path().join(STEM_MANIFEST))?;
    ensure!(
        models::hash_file(audio)? == manifest.source_audio_sha256,
        "Source changed during stem export"
    );
    publish(stage.path(), output)
}

/// Explicit import of existing six-stem files. This records the caller's real
/// provenance statement; equal length never upgrades an imported cache to a
/// verified native separation job or establishes actual instrument purity.
pub fn import_isolation(
    audio: &Path,
    stems: &Path,
    output: &Path,
    alignment: crate::stem_review::AlignmentEvidence,
) -> Result<crate::stem_review::StemManifest> {
    let mut files = BTreeMap::new();
    for id in crate::stem_review::StemId::ALL {
        let candidates: Vec<_> = ["wav", "flac"]
            .into_iter()
            .map(|ext| stems.join(format!("{}.{ext}", id.as_str())))
            .filter(|path| path.is_file())
            .collect();
        ensure!(
            candidates.len() == 1,
            "Import requires exactly one native 44100 Hz stereo WAV/FLAC file for {}",
            id.as_str()
        );
        files.insert(id, candidates[0].clone());
    }
    let manifest = crate::stem_review::StemManifest::build(audio, files, alignment)?;
    // Imported manifests can reference existing files outside their directory;
    // their absolute paths and hashes record exactly what the user attested.
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut stage, &manifest)?;
    stage.as_file().sync_all()?;
    stage.persist_noclobber(output).map_err(|e| e.error)?;
    Ok(manifest)
}

pub fn transcribe(
    audio: &Path,
    model_dir: &Path,
    stems: Option<&Path>,
    wanted: &[String],
) -> Result<Transcription> {
    transcribe_with_isolation(
        audio,
        model_dir,
        IsolationOptions {
            stems,
            ..Default::default()
        },
        wanted,
    )
}

pub fn transcribe_with_isolation(
    audio: &Path,
    model_dir: &Path,
    options: IsolationOptions<'_>,
    wanted: &[String],
) -> Result<Transcription> {
    ensure!(audio.is_file(), "Audio missing: {}", audio.display());
    ensure!(
        !wanted.is_empty()
            && wanted
                .iter()
                .all(|name| ["drums", "bass", "guitar", "piano", "pro_piano"]
                    .contains(&name.as_str())),
        "Request drums, guitar, bass, piano and/or pro_piano instruments"
    );
    ensure!(
        !(options.stems.is_some() && options.manifest.is_some()),
        "Choose --stems or --stem-manifest"
    );
    let hq = crate::keyboard_detection::high_quality_piano(
        wanted,
        options.keyboard_sound,
        options.piano_quality,
    );
    ensure!(
        options.keyboard_analysis.is_none() || hq,
        "Separate Piano analysis requires higher-quality Piano detection"
    );
    ensure!(
        !options.full_mix_drums
            || (wanted.iter().all(|n| n == "drums")
                && options.stems.is_none()
                && options.manifest.is_none()
                && options.keyboard_analysis.is_none()),
        "Full-mix fallback is only available for drums-only without supplied stems"
    );
    let mut out = Transcription::default();
    let needs_pitch = wanted.iter().any(|n| n == "guitar" || n == "bass");
    let default_cache = model_dir.join("stem-cache");
    let (manifest, mut keyboard_analysis) = if let Some(path) = options.manifest {
        (Some(read_isolation(audio, path)?), None)
    } else if let Some(dir) = options.stems {
        let path = dir.join(STEM_MANIFEST);
        ensure!(
            path.is_file(),
            "Supplied stems require {}; duration alone does not bind them to this recording. Run import-stems with actual alignment provenance, or supply --stem-manifest",
            path.display()
        );
        (Some(read_isolation(audio, &path)?), None)
    } else if options.full_mix_drums {
        (None, None)
    } else {
        let plan = prepare_keyboard_isolation(
            audio,
            model_dir,
            options.cache_dir.unwrap_or(&default_cache),
            wanted,
            options.keyboard_sound,
            options.piano_quality,
        )?;
        (Some(plan.primary), plan.keyboard_analysis)
    };
    if let Some(path) = options.keyboard_analysis {
        keyboard_analysis = Some(read_isolation(audio, path)?);
    } else if hq
        && keyboard_analysis.is_none()
        && !manifest.as_ref().is_some_and(higher_quality_manifest)
    {
        keyboard_analysis = Some(cached_keyboard_separation(
            audio,
            model_dir,
            options.cache_dir.unwrap_or(&default_cache),
        )?);
    }
    if let Some(analysis) = &keyboard_analysis {
        ensure!(
            higher_quality_manifest(analysis),
            "Higher-quality Piano analysis has a different separator model identity"
        );
    }
    if let Some(manifest) = &manifest {
        for name in wanted {
            let name = if name == "guitar" {
                options.guitar_source.stem().as_str()
            } else if name == "pro_piano" {
                "piano"
            } else {
                name.as_str()
            };
            let requested_manifest = if name == "piano"
                && options.keyboard_sound == crate::keyboard_detection::KeyboardSound::Piano
            {
                keyboard_analysis.as_ref().unwrap_or(manifest)
            } else {
                manifest
            };
            ensure!(
                requested_manifest
                    .stems
                    .iter()
                    .any(|stem| stem.id.as_str() == name),
                "The bound isolation manifest has no {name} stem"
            );
        }
    }
    let find_stem = |name: &str| -> Result<PathBuf> {
        manifest
            .as_ref()
            .and_then(|m| m.stems.iter().find(|s| s.id.as_str() == name))
            .map(|s| s.path.clone())
            .with_context(|| format!("The bound isolation manifest has no {name} stem"))
    };
    // Set the source identity before analyzers add evidence.
    // Recheck the full manifest after inference to catch changed input files.
    out.isolation = manifest.clone();
    if wanted.iter().any(|n| n == "drums") {
        let drum_audio = if manifest.is_some() {
            find_stem("drums")?
        } else {
            audio.to_owned()
        };
        let (hits, recovered) =
            models::drums_with_recovery(&drum_audio, &models::verified(model_dir, "drums")?)?;
        out.drums = hits;
        out.drum_recovered_hits = recovered;
        if let Some(manifest) = &manifest {
            out.drum_articulations = crate::drum_articulation::analyze(
                &models::decode(&drum_audio, 22050, 1)?,
                &out.drums,
            )?;
            let detail_model = model_dir.join("oaf_drums.onnx");
            match fs::symlink_metadata(&detail_model) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_file() && !metadata.file_type().is_symlink(),
                        "Detailed drum model must be a regular non-symlink file"
                    );
                    crate::drum_articulation::infer_cymbals(
                        &drum_audio,
                        &detail_model,
                        &out.drums,
                        &mut out.drum_articulations,
                    )?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            out.drum_articulation_source =
                Some(crate::drum_articulation::Source::from_manifest(manifest)?);
        }
        out.warnings.push("Dense-percussion recovery admits additional strong local model peaks only with an independent source attack within 25 ms. Recovered attacks retain model timing and five-class identity; this does not identify güira, bongos or congas. Listening review is required.".into());
        out.warnings.push("ADTOF's raw five-class hits and clocks are retained. An installed detailed drum model can propose ride/crash on existing source-bound Expert cymbal hits; ambiguous/colliding votes retain the generic lane. Tom-register diagnostics do not change notes. All articulation decisions need review; model scores are not accuracy probabilities.".into());
    }
    if wanted.iter().any(|n| n == "guitar") {
        out.guitar_source = options.guitar_source;
        let inference = models::guitar_pitch_with_evidence_for_source(
            &find_stem(options.guitar_source.stem().as_str())?,
            &models::verified(model_dir, "pitch")?,
            options.guitar_source,
        )?;
        out.guitar = inference
            .candidates
            .iter()
            .map(|c| c.candidate.clone())
            .collect();
        out.guitar_candidates = inference.candidates;
        out.guitar_short_candidates = inference.short_candidates;
        out.guitar_polyphonic_notes = inference.polyphonic_notes;
        out.guitar_polyphonic_diagnostics = inference.diagnostics;
        if !out.guitar_polyphonic_notes.is_empty() {
            out.warnings.push("Polyphonic guitar recovery: additional network hypotheses have owned harmonic support and a nearby stem attack; model scores are uncalibrated and these notes need listening review.".into());
        }
        out.warnings.push("Short guitar recovery requires a strong model onset, a separate stem attack and matching pitch within the note's own audio window. Polyphonic, quiet and ambiguous short figures can remain missing. Original hypotheses and rejection reasons are retained; recovered notes need listening review.".into());
        if options.guitar_source != crate::mapping::GuitarSource::Guitar {
            out.warnings.push(format!("Five-fret Guitar intentionally follows the selected {} stem. This is a melodic arrangement, not proof of a guitar performance; separation can mix multiple instruments. Review phrase choice and source identity.", options.guitar_source.stem().as_str()));
        }
    }
    if wanted.iter().any(|n| n == "bass") {
        crate::bass_analysis::infer(
            &mut out,
            &find_stem("bass")?,
            &models::verified(model_dir, "pitch")?,
        )?;
    }
    if wanted.iter().any(|n| n == "piano" || n == "pro_piano") {
        let evidence = crate::keyboard_detection::detect(
            audio,
            keyboard_analysis
                .as_ref()
                .or(manifest.as_ref())
                .context("Keyboard detection requires source-bound stems")?,
            model_dir,
            options.keyboard_sound,
        )?;
        let notes = evidence.raw_notes.clone();
        if evidence.sound == crate::keyboard_detection::KeyboardSound::Piano
            && evidence.version == 1
            && !notes.iter().any(|n| n.confidence >= 0.65)
        {
            out.warnings.push("Piano recovery was skipped because the separated view has no strong attacks. This can mean no piano is present or that separation missed it. Listen to the original recording before approving an empty or sparse keyboard chart.".into());
        }
        out.keyboard_detection = Some(evidence);
        out.keyboard_analysis = keyboard_analysis.clone();
        if hq {
            out.warnings.push("Higher-quality Piano uses its own source-bound six-stem analysis. For mixed jobs, guitar, bass, drums and vocals keep their original isolation inputs. The actual Piano analysis manifest is retained independently for review; separation quality and instrument identity still require listening.".into());
        }
        if wanted.iter().any(|n| n == "piano") {
            out.piano = notes.clone();
        }
        if wanted.iter().any(|n| n == "pro_piano") {
            let outside = notes
                .iter()
                .filter(|n| !(crate::keys::MIN_PITCH..=crate::keys::MAX_PITCH).contains(&n.pitch))
                .count();
            if outside > 0 {
                out.warnings.push(format!("Pro Keys: {outside} source candidates need octave/register adaptation for the game keyboard. Original pitches remain in transcription evidence; the arrangement receipt records every change."));
            }
            out.pro_piano = notes;
        }
        out.warnings.push(match options.keyboard_sound {
            crate::keyboard_detection::KeyboardSound::Piano => "Piano uses a source-bound Kong estimate with corroborated recovery from the original recording's harmonic residual when strong piano attacks are present. When recovery runs, both raw views and recovery decisions are retained. Phantom notes and missed attacks remain possible. Onset scores are uncalibrated, not correctness probabilities. Pedal events remain separate from key releases.",
            crate::keyboard_detection::KeyboardSound::Synth => "Experimental Synth uses MuScriptor on the original full mix with advisory lead conditioning. Drums and low bass are excluded; upper bass-labelled voices require coherent melodic-phrase evidence. Other melodic voices remain even when labeled guitar, brass or strings. Instrument identity is uncertain. Raw notes, tokens, exclusions and release evidence are retained. Arrangement priority1 is nominal, not a model confidence.",
        }.into());
    }
    if needs_pitch {
        out.warnings.push("Basic Pitch uses onset-first decoding with Melodia recovery; pitch bends are not charted. Five-fret mapping and difficulty reductions are automatic drafts.".into());
    }
    if let Some(manifest) = manifest {
        // Detect source/stem changes that occurred after the initial validation
        // and while the individual transcribers were reading their inputs.
        manifest.validate_source(audio, &models::hash_file(audio)?, metadata(audio)?.duration)?;
        out.warnings.extend(isolation_limits(&manifest));
        out.isolation = Some(manifest);
    } else {
        out.warnings.push("Explicit full-mix drums fallback: no isolated drum stem was used. Other instruments can affect attack detection; separation provenance is absent.".into());
    }
    if let Some(analysis) = &out.keyboard_analysis {
        analysis.validate_source(audio, &models::hash_file(audio)?, metadata(audio)?.duration)?;
    }
    for requested in wanted {
        let empty = match requested.as_str() {
            "drums" => out.drums.is_empty(),
            "guitar" => out.guitar.is_empty(),
            "bass" => out.bass.is_empty(),
            "piano" => out.piano.is_empty(),
            "pro_piano" => out.pro_piano.is_empty(),
            _ => true,
        };
        if empty {
            out.warnings.push(format!(
                "No {requested} notes detected; no {requested} track was generated."
            ));
        }
    }
    out.validate()?;
    Ok(out)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub duration: f64,
}

pub fn metadata(audio: &Path) -> Result<Metadata> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:format_tags",
            "-of",
            "json",
        ])
        .arg(audio)
        .output()
        .context("ffprobe is required on PATH")?;
    ensure!(output.status.success(), "Cannot read audio metadata");
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let tags: BTreeMap<String, String> = data["format"]["tags"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.to_ascii_lowercase(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let get = |key: &str| tags.get(key).cloned().unwrap_or_default();
    let duration = data["format"]["duration"]
        .as_str()
        .context("Audio has no duration")?
        .parse::<f64>()?;
    ensure!(
        duration.is_finite() && duration > 0.0,
        "Invalid audio duration"
    );
    Ok(Metadata {
        name: get("title"),
        artist: get("artist"),
        album: get("album"),
        year: get("date").chars().take(4).collect(),
        genre: get("genre"),
        duration,
    })
}

pub(crate) fn copy_folder(source: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dest.join(entry.file_name());
        ensure!(
            !ty.is_symlink(),
            "Source contains a symlink: {}; make an ordinary working copy first",
            entry.path().display()
        );
        if ty.is_dir() {
            fs::create_dir(&target)?;
            copy_folder(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!("Unsupported source file type");
        }
    }
    Ok(())
}

/// Archive copied generation sidecars before a new additive pass. This only
/// mutates the unpublished staging copy; source history is never overwritten.
fn archive_generation_sidecars(stage: &Path, source: Option<&Path>) -> Result<Option<PathBuf>> {
    let mut files = BTreeMap::new();
    for name in [
        "chart-engine-report.json",
        "chart-engine-transcription.json",
        "chart-engine-lyrics.json",
        "chart-engine-review.json",
        "chart-engine-isolation.json",
        "chart-engine-keyboard-isolation.json",
        "chart-engine-pro-keys-arrangement.json",
    ] {
        let path = stage.join(name);
        if path.exists() {
            ensure!(path.is_file(), "Generation sidecar must be a file: {name}");
            files.insert(name, models::hash_file(&path)?);
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&files)?));
    let relative = PathBuf::from("chart-engine-history").join(id);
    let history = stage.join(&relative);
    fs::create_dir_all(history.parent().context("History parent missing")?)?;
    // Reject history-name collisions, even when the main files match.
    // An existing entry may contain extra recovery files.
    fs::create_dir(&history).context("Generation history already contains this receipt set")?;
    for name in files.keys() {
        fs::rename(stage.join(name), history.join(name))?;
    }
    fs::write(
        history.join("archive-sha256.json"),
        serde_json::to_vec_pretty(&files)?,
    )?;
    fs::write(
        history.join("archive-origin.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source_folder":source.map(Path::canonicalize).transpose()?,
            "relative_asset_base":"Original source_folder; archived sidecars retain their exact original bytes and are historical evidence, not active manifests",
        }))?,
    )?;
    Ok(Some(relative))
}

pub(crate) fn update_ini(original: &str, updates: &BTreeMap<String, String>) -> String {
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut result = String::new();
    let mut in_song = false;
    let mut found = false;
    for line in original.lines() {
        if line.trim().starts_with('[') {
            if in_song {
                for (k, v) in updates {
                    if seen.insert(k.clone()) {
                        result.push_str(&format!("{k} = {v}{newline}"));
                    }
                }
            }
            in_song = line.trim().eq_ignore_ascii_case("[song]");
            found |= in_song;
        }
        if in_song && let Some((key, _)) = line.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            if let Some(value) = updates.get(&key) {
                if seen.insert(key.clone()) {
                    result.push_str(&format!("{key} = {value}{newline}"));
                }
                continue;
            }
        }
        result.push_str(line);
        result.push_str(newline);
    }
    if !found {
        result.push_str(&format!("[song]{newline}"));
    }
    for (k, v) in updates {
        if seen.insert(k.clone()) {
            result.push_str(&format!("{k} = {v}{newline}"));
        }
    }
    result
}

pub struct Build<'a> {
    pub audio: &'a Path,
    pub output: &'a Path,
    pub existing: Option<&'a Path>,
    pub timing: Option<TempoMap>,
    pub transcription: &'a Transcription,
    pub meta: Metadata,
    pub pro: bool,
    pub lookup_lyrics: bool,
}

/// Attach generated-rule review prompts to the same timed map used by the
/// native queue and automation. Existing pattern flags are retained once.
pub fn with_generated_rule_reviews(
    map: crate::review_regions::ReviewMap,
    rules: &crate::generated_rules::RuleReport,
) -> Result<crate::review_regions::ReviewMap> {
    use crate::{
        generated_rules::{RuleCode, Severity},
        review_regions::{Issue, Reason, ReviewMap},
    };
    ensure!(
        rules.version == 1,
        "Unsupported generated-rule report version"
    );
    ensure!(
        !rules.has_errors(),
        "Cannot publish review markers for structurally rejected generated notes"
    );
    let generated = map
        .generated_tracks
        .as_ref()
        .context("Generated-rule review requires known generated-track provenance")?;
    let mut issues: Vec<_> = map
        .regions
        .iter()
        .flat_map(|region| region.issues.clone())
        .collect();
    let mut checks = map.checks_performed.clone();
    checks.extend([
        Reason::FastRepeat,
        Reason::HandCrowding,
        Reason::SustainOverlap,
        Reason::ReductionDifference,
    ]);
    for finding in rules
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Review)
    {
        ensure!(
            generated.contains(&finding.track),
            "Generated-rule finding names a preserved or unknown track"
        );
        let reason = match finding.code {
            RuleCode::FastRepeat => Reason::FastRepeat,
            RuleCode::HandCrowding => Reason::HandCrowding,
            RuleCode::SustainRetriggerOverlap => Reason::SustainOverlap,
            RuleCode::LowerTierAttackAbsentFromExpert => Reason::ReductionDifference,
            _ => continue,
        };
        // The existing hand check deliberately collapses a crowded burst to
        // one prompt. Do not reintroduce one marker for every nearby window.
        let tolerance = if reason == Reason::HandCrowding {
            0.020 + 1e-9
        } else {
            1e-6
        };
        if issues.iter().any(|issue| {
            issue.track == finding.track
                && issue.reason == reason
                && (issue.start - finding.seconds).abs() <= tolerance
        }) {
            continue;
        }
        issues.push(Issue::at(
            finding.seconds,
            &finding.track,
            reason,
            &[("generated_rule_tick", finding.tick as f64)],
        ));
    }
    let mut merged = ReviewMap::build(
        map.duration_seconds,
        map.policy,
        map.input_sha256,
        map.generated_tracks,
        checks,
        issues,
    )?;
    merged.coverage_limits = map.coverage_limits;
    merged.coverage_limits.push("Generated-rule sustain/reduction checks cover only newly emitted tracks. Fast patterns and arrangement differences remain review prompts; the generation receipt retains the exact rule codes and explanations.".into());
    // Keep explicit acceptance language from the originating review map.
    merged.musical_acceptance = map.musical_acceptance;
    merged.unflagged_sections = map.unflagged_sections;
    Ok(merged)
}

pub fn build(request: Build<'_>) -> Result<serde_json::Value> {
    build_with_lyrics(request, None)
}

pub fn build_with_lyrics(
    request: Build<'_>,
    supplied_lyrics: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    let Build {
        audio,
        output,
        existing,
        timing,
        transcription,
        mut meta,
        pro,
        lookup_lyrics,
    } = request;
    destination(output, existing)?;
    transcription.validate()?;
    if let Some(isolation) = &transcription.isolation {
        isolation.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        analysis.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(source) = existing {
        validate_existing(source)?;
    }
    let source_chart = existing.map(chart_file).transpose()?;
    let chart_doc = source_chart
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e == "chart"))
        .map(|p| -> Result<Chart> { Chart::parse(fs::read_to_string(p)?) })
        .transpose()?;
    let midi_bytes = source_chart
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e == "mid"))
        .map(fs::read)
        .transpose()?;
    let map = if let Some(chart) = &chart_doc {
        chart.timing.clone()
    } else if let Some(bytes) = &midi_bytes {
        midi::timing(bytes)?
    } else {
        timing.context("New songs require --bpm; automatic beat tracking is not ported yet")?
    };
    let generated = mapping::tracks(transcription, &map, pro)?;
    let authored: crate::chart::Tracks = if let Some(chart) = &chart_doc {
        chart
            .sections
            .iter()
            .filter(|section| {
                ["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
                    ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"]
                        .iter()
                        .any(|instrument| section.name == format!("{tier}{instrument}"))
                })
            })
            .map(|section| Ok((section.name.clone(), chart.notes(&section.name)?)))
            .collect::<Result<_>>()?
    } else {
        // MIDI enhancement preserves whole existing instrument chunks, so no
        // newly emitted lower row can conflict with its authored Expert part.
        Default::default()
    };
    let (generated, difficulty_reduction) =
        crate::difficulty_links::reduce_generated(&generated, &authored, &map, meta.duration)?;
    ensure!(
        generated.values().any(|v| !v.is_empty()),
        "No playable notes detected; no output created"
    );
    let (mut content, chart_name, added) = if let Some(chart) = &chart_doc {
        let (text, added) = chart.enhance(&generated)?;
        (text.into_bytes(), "notes.chart", added)
    } else if let Some(bytes) = &midi_bytes {
        let (bytes, added) = midi::enhance(bytes, &generated)?;
        (bytes, "notes.mid", added)
    } else {
        (
            chart::new_chart(&meta.name, &meta.artist, &map, &generated).into_bytes(),
            "notes.chart",
            generated.keys().cloned().collect(),
        )
    };
    ensure!(
        !added.is_empty(),
        "No missing tracks to add; no output created"
    );
    // MIDI receipts identify whole PART tracks, while the shared review model
    // uses difficulty names. Normalize without changing receipt semantics.
    let review_generated: BTreeSet<String> = generated
        .keys()
        .filter(|name| {
            if chart_name == "notes.chart" {
                added.contains(name)
            } else {
                [
                    ("Single", "PART GUITAR"),
                    ("DoubleBass", "PART BASS"),
                    ("Drums", "PART DRUMS"),
                    ("Keyboard", "PART KEYS"),
                ]
                .iter()
                .any(|(suffix, label)| name.ends_with(suffix) && added.iter().any(|a| a == label))
                    || crate::keys::midi_track(name)
                        .is_some_and(|label| added.iter().any(|a| a == label))
            }
        })
        .cloned()
        .collect();
    let emitted: crate::chart::Tracks = generated
        .iter()
        .filter(|(name, _)| review_generated.contains(*name))
        .map(|(name, notes)| (name.clone(), notes.clone()))
        .collect();
    let difficulty_reduction: BTreeMap<_, _> = difficulty_reduction
        .into_iter()
        .filter(|(track, _)| review_generated.contains(track))
        .collect();
    let rule_report = crate::generated_rules::assess(&emitted, &map, meta.duration);
    ensure!(
        !rule_report.has_errors(),
        "Generated chart violates structural rules: {}",
        serde_json::to_string(&rule_report)?
    );
    // Older saved inference evidence described the old range-dropping mapper.
    // Preserve those source bytes, but describe this newly generated arrangement
    // with the current rule rather than repeating an obsolete export warning.
    let mut warnings: Vec<_> = transcription
        .warnings
        .iter()
        .filter(|warning| {
            !warning.contains("omitted from the playable25-key arrangement")
                && !warning.contains("Pro Keys preserves MIDI48–72")
        })
        .cloned()
        .collect();
    let pro_arrangement = (!transcription.pro_piano.is_empty())
        .then(|| crate::pro_keys::arrange_transcription(transcription, &map))
        .transpose()?;
    let pro_arrangement_summary = pro_arrangement.as_ref().map(|plan| serde_json::json!({
        "rule_id":plan.rule_id,"source_candidates":plan.decisions.len(),"expert_notes":plan.notes.len(),
        "octave_adapted_candidates":plan.octave_adapted_candidates,
        "merged_simultaneous_candidates":plan.merged_simultaneous_candidates,
        "merged_octave_collisions":plan.merged_octave_collisions,
        "shortened_releases":plan.shortened_releases,"out_of_range_omitted":plan.out_of_range_omitted,
        "evidence":"chart-engine-pro-keys-arrangement.json"
    }));
    if let Some(plan) = &pro_arrangement {
        warnings.push(format!("Pro Keys register arrangement {}: {} octave adaptations, {} simultaneous unison merges, {} near-simultaneous octave collisions merged, {} shortened source releases; {} candidates omitted for pitch range. Original detections and every arrangement decision are retained for review.", plan.rule_id,plan.octave_adapted_candidates,plan.merged_simultaneous_candidates,plan.merged_octave_collisions,plan.shortened_releases,plan.out_of_range_omitted));
    }
    let pro_keys_ranges: BTreeMap<_, _> = emitted
        .iter()
        .filter(|(track, _)| crate::keys::is_pro(track))
        .map(|(track, notes)| Ok((track.clone(), crate::key_ranges::plan(notes)?)))
        .collect::<Result<_>>()?;
    for (track, plan) in &pro_keys_ranges {
        if !plan.uncovered.is_empty() {
            warnings.push(format!("{track}: {} sounding pitch groups exceed a single YARG display range. MIDI retains every pitch; explicit Pro Keys range review is required.", plan.uncovered.len()));
        }
    }
    if let Some(isolation) = &transcription.isolation {
        for warning in isolation_limits(isolation) {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".chart-engine-")
        .tempdir_in(parent)?;
    let ini = if let Some(source) = existing {
        copy_folder(source, stage.path())?;
        song_ini(source)?
    } else {
        String::new()
    };
    let previous_generation = archive_generation_sidecars(stage.path(), existing)?;
    if let Some(history) = &previous_generation {
        warnings.push(format!(
            "Prior generation receipts retained in {}; source files were not changed",
            history.display()
        ));
    }
    if !ini.is_empty() {
        for (key, value) in ini_fields(&ini) {
            match key.as_str() {
                "name" if meta.name.is_empty() => meta.name = value,
                "artist" if meta.artist.is_empty() => meta.artist = value,
                _ => {}
            }
        }
    }
    ensure!(
        !meta.name.trim().is_empty() && !meta.artist.trim().is_empty(),
        "Title and artist must come from tags or --title/--artist"
    );
    let mut updates = BTreeMap::new();
    if existing.is_none() {
        for (k, v) in [
            ("name", meta.name.clone()),
            ("artist", meta.artist.clone()),
            ("album", meta.album.clone()),
            ("year", meta.year.clone()),
            ("genre", meta.genre.clone()),
            ("charter", "YACE (automatic draft)".into()),
        ] {
            ensure!(!v.contains(['\r', '\n']), "Metadata contains a newline");
            updates.insert(k.into(), v);
        }
        let extension = audio
            .extension()
            .and_then(|e| e.to_str())
            .context("Audio needs a file extension")?;
        if matches!(
            extension.to_ascii_lowercase().as_str(),
            "ogg" | "opus" | "mp3" | "wav"
        ) {
            fs::copy(
                audio,
                stage
                    .path()
                    .join(format!("song.{}", extension.to_ascii_lowercase())),
            )?;
        } else {
            let encoded = Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(audio)
                .args([
                    "-map",
                    "0:a:0",
                    "-vn",
                    "-c:a",
                    "pcm_s16le",
                    "-ar",
                    "44100",
                    "-ac",
                    "2",
                    "-n",
                ])
                .arg(stage.path().join("song.wav"))
                .output()?;
            ensure!(
                encoded.status.success(),
                "Cannot convert output audio to WAV: {}",
                String::from_utf8_lossy(&encoded.stderr)
            );
            warnings.push("Input audio was converted to song.wav for game compatibility; the input file was preserved.".into());
        }
    }
    for (suffix, part, key) in [
        ("Single", "PART GUITAR", "diff_guitar"),
        ("DoubleBass", "PART BASS", "diff_bass"),
        ("Drums", "PART DRUMS", "diff_drums"),
        ("Keyboard", "PART KEYS", "diff_keys"),
        ("ProKeys", "PART REAL_KEYS_X", "diff_keys_real"),
    ] {
        if added.iter().any(|n| n.ends_with(suffix) || n == part) {
            let count = generated.get(&format!("Expert{suffix}")).map_or(0, |n| {
                n.iter()
                    .filter(|n| crate::editor::playable(&format!("Expert{suffix}"), n.lane))
                    .count()
            });
            let nps = count as f64 / meta.duration.max(1.0);
            let rating = if nps < 1.5 {
                1
            } else if nps < 3.0 {
                2
            } else if nps < 4.5 {
                3
            } else if nps < 6.0 {
                4
            } else if nps < 7.5 {
                5
            } else {
                6
            };
            updates.insert(key.into(), rating.to_string());
            if suffix == "Drums" {
                updates.insert(
                    "pro_drums".into(),
                    if pro { "True" } else { "False" }.into(),
                );
                updates.insert(
                    "diff_drums_real".into(),
                    if pro { rating } else { -1 }.to_string(),
                );
            }
        }
    }
    if lookup_lyrics {
        match supplied_lyrics
            .map(|value| Ok(Some(value.clone())))
            .unwrap_or_else(|| lyrics(&meta))
        {
            Ok(Some(record)) => {
                if let Some(lrc) = record.get("syncedLyrics").and_then(|s| s.as_str())
                    && chart_name == "notes.chart"
                {
                    let raw = String::from_utf8(content.clone())?;
                    let doc = Chart::parse(raw)?;
                    let has_lyrics = doc
                        .sections
                        .iter()
                        .find(|s| s.name == "Events")
                        .is_some_and(|s| doc.raw[s.body_start..s.body_end].contains("lyric "));
                    if !has_lyrics
                        && let Some(section) = doc.sections.iter().find(|s| s.name == "Events")
                    {
                        let lines = lyric_events(lrc, &map, meta.duration);
                        let mut text = doc.raw.clone();
                        text.insert_str(section.body_end, &lines);
                        content = text.into_bytes();
                    }
                }
                fs::write(
                    stage.path().join("chart-engine-lyrics.json"),
                    serde_json::to_vec_pretty(&record)?,
                )?;
                warnings.push("Lyrics are database text/line timings; word-level alignment is not implemented. Existing lyric events are preserved.".into());
            }
            Ok(None) => warnings.push(
                "No exact artist/title/duration lyrics match; existing lyrics were retained."
                    .into(),
            ),
            Err(e) => warnings.push(format!(
                "Lyrics unavailable ({e}); existing lyrics were retained."
            )),
        }
    }
    fs::write(stage.path().join(chart_name), content)?;
    fs::write(stage.path().join("song.ini"), update_ini(&ini, &updates))?;
    fs::write(
        stage.path().join("chart-engine-transcription.json"),
        serde_json::to_vec_pretty(transcription)?,
    )?;
    if added
        .iter()
        .any(|name| name.ends_with("Single") || name == "PART GUITAR")
    {
        fs::write(
            stage.path().join("chart-engine-melody-source.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"audio_sha256":models::hash_file(&audio_file(stage.path())?)?,"guitar_source":transcription.guitar_source}),
            )?,
        )?;
    }
    if let Some(plan) = &pro_arrangement {
        fs::write(
            stage.path().join("chart-engine-pro-keys-arrangement.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"version":1,
                "source_transcription_sha256":models::hash_file(&stage.path().join("chart-engine-transcription.json"))?,
                "arrangement":plan}))?,
        )?;
    }
    if let Some(isolation) = &transcription.isolation {
        fs::write(
            stage.path().join("chart-engine-isolation.json"),
            serde_json::to_vec_pretty(isolation)?,
        )?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        fs::write(
            stage.path().join("chart-engine-keyboard-isolation.json"),
            serde_json::to_vec_pretty(analysis)?,
        )?;
    }
    warnings.push("Instrument intensity is a note-density heuristic. Chart validity is not a listening/playability approval.".into());
    let mut input_hashes = BTreeMap::new();
    input_hashes.insert(audio.display().to_string(), models::hash_file(audio)?);
    if let Some(path) = source_chart {
        input_hashes.insert(path.display().to_string(), models::hash_file(&path)?);
    }
    let pro_keys_output =
        chart_name == "notes.chart" && review_generated.iter().any(|t| crate::keys::is_pro(t));
    let mut report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"status":"automatic_draft","output":output,"added":added,"input_sha256":input_hashes,"metadata":meta,"timing":map,"model_catalog":models::assets(),"isolation":transcription.isolation,"isolation_manifest":transcription.isolation.as_ref().map(|_|"chart-engine-isolation.json"),"difficulty_reduction":difficulty_reduction,"charting_rules":rule_report,"review_markers":"chart-engine-review.json","warnings":warnings,"pro_keys_arrangement":pro_arrangement_summary,"pro_keys_ranges":pro_keys_ranges,"game_chart":if pro_keys_output{"notes.mid"}else{chart_name},"editable_master":if pro_keys_output{Some("chart-engine-working/notes.chart")}else{None}});
    if let Some(analysis) = &transcription.keyboard_analysis {
        report["keyboard_analysis"] = serde_json::to_value(analysis)?;
        report["keyboard_analysis_manifest"] =
            serde_json::json!("chart-engine-keyboard-isolation.json");
    }
    if transcription
        .keyboard_manifest()?
        .is_some_and(higher_quality_manifest)
    {
        report["keyboard_analysis_model"] =
            serde_json::to_value(crate::keyboard_separation::asset())?;
    }
    fs::write(
        stage.path().join("chart-engine-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let review_chart = if chart_name == "notes.chart" {
        Chart::parse(fs::read_to_string(stage.path().join(chart_name))?)?
    } else {
        // This representation is diagnostic only; the exported MIDI remains
        // byte-preserving for authored track chunks. Inspect only newly added parts.
        let added_tracks = generated
            .into_iter()
            .filter(|(name, _)| review_generated.contains(name))
            .collect();
        let mut diagnostic = Chart::parse(chart::new_chart(
            &meta.name,
            &meta.artist,
            &map,
            &added_tracks,
        ))?;
        // Keep MIDI's microsecond tempo precision for diagnostics; .chart text
        // rounds BPM to thousandths and is not the actual exported note file.
        diagnostic.timing = map.clone();
        diagnostic
    };
    let review_inputs = BTreeMap::from([
        (
            "chart".into(),
            models::hash_file(&stage.path().join(chart_name))?,
        ),
        (
            "transcription".into(),
            models::hash_file(&stage.path().join("chart-engine-transcription.json"))?,
        ),
        (
            "generation_receipt".into(),
            models::hash_file(&stage.path().join("chart-engine-report.json"))?,
        ),
        (
            "original_audio".into(),
            models::hash_file(&audio_file(stage.path())?)?,
        ),
    ]);
    let review = crate::review::generation_map(
        &review_chart,
        transcription,
        meta.duration,
        review_inputs,
        review_generated,
    )?;
    let mut review = with_generated_rule_reviews(review, &rule_report)?;
    review.coverage_limits.push("Guitar/bass/drum lower difficulties derive from the effective Expert chart (authored Expert takes precedence). Pro Keys lower tiers select subsets of the fitted Expert register arrangement. Five-lane Piano selects source attacks/chord voices independently. Live Expert-master links are not available for Keys/Pro Keys. These reductions do not establish musical acceptance.".into());
    if let Some(isolation) = &transcription.isolation {
        review.coverage_limits.push(format!("Generation used hash-bound instrument stems with alignment method '{}'. Exact source/stem sample counts establish engineering consistency; they do not establish phase alignment, isolation purity, calibrated note confidence or musical acceptance. Existing attack-support, pitch-disagreement and candidate-score flags remain applicable.", isolation.alignment.method));
        if isolation.generation.is_none() {
            review.coverage_limits.push("Isolation was supplied with an explicit provenance attestation; no contemporaneous native model-bound job receipt is available.".into());
        }
    }
    fs::write(
        stage.path().join("chart-engine-review.json"),
        serde_json::to_vec_pretty(&review)?,
    )?;
    ensure!(
        !output.exists(),
        "Output appeared during processing; refusing to overwrite"
    );
    if let Some(isolation) = &transcription.isolation {
        isolation.validate_binding(&models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        analysis.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if pro_keys_output {
        let master = stage.path().join("notes.chart");
        let chart = Chart::parse(fs::read_to_string(&master)?)?;
        let (midi, delay) = crate::game_export::song(&chart, None)?;
        let working = stage.path().join("chart-engine-working");
        // Existing recovery data must never be overwritten during enhancement.
        ensure!(
            !working.exists(),
            "Existing working-master archive requires opening its YACE project before adding Pro Keys"
        );
        fs::create_dir(&working)?;
        fs::write(stage.path().join("notes.mid"), midi)?;
        fs::rename(&master, working.join("notes.chart"))?;
        fs::write(
            working.join("binding.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"master_sha256":models::hash_file(&working.join("notes.chart"))?,"midi_sha256":models::hash_file(&stage.path().join("notes.mid"))?}),
            )?,
        )?;
        let ini = stage.path().join("song.ini");
        fs::write(
            &ini,
            crate::game_export::ini_with_delay(&fs::read_to_string(&ini)?, delay),
        )?;
    }
    publish(stage.path(), output)?;
    Ok(report)
}

pub fn lyrics(meta: &Metadata) -> Result<Option<serde_json::Value>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("chart-engine/0.1 (local song charting)")
        .build()?;
    let response = client
        .get("https://lrclib.net/api/get")
        .query(&[
            ("track_name", meta.name.as_str()),
            ("artist_name", meta.artist.as_str()),
            ("album_name", meta.album.as_str()),
            ("duration", &meta.duration.round().to_string()),
        ])
        .send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let record: serde_json::Value = response.error_for_status()?.json()?;
    let same = |a: &str, b: &str| a.trim().to_lowercase() == b.trim().to_lowercase();
    let matched = record["artistName"]
        .as_str()
        .is_some_and(|a| same(a, &meta.artist))
        && record["trackName"]
            .as_str()
            .is_some_and(|n| same(n, &meta.name))
        && record["duration"]
            .as_f64()
            .is_some_and(|d| (d - meta.duration).abs() <= 2.0);
    Ok(matched.then_some(record))
}

pub fn lyric_events(lrc: &str, map: &TempoMap, duration: f64) -> String {
    let mut events = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((timestamp, tail)) = after.split_once(']') else {
                break;
            };
            if let Some((minute, second)) = timestamp.split_once(':')
                && let (Ok(minute), Ok(second)) = (minute.parse::<u32>(), second.parse::<f64>())
            {
                let time = minute as f64 * 60.0 + second;
                if (0.0..60.0).contains(&second) && time <= duration {
                    times.push(time);
                }
            }
            rest = tail;
        }
        let body = rest.trim();
        if body.is_empty() {
            continue;
        }
        for time in times {
            if let Some(tick) = map.tick(time) {
                events.push((
                    tick,
                    format!("  {tick} = E \"lyric {}\"\n", chart::quoted(body)),
                ));
            }
        }
    }
    events.sort_by_key(|e| e.0);
    events.into_iter().map(|(_, line)| line).collect()
}
