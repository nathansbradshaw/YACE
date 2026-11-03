pub use crate::song_files::{
    chart_file, copy_song_source, editable_chart_file, organize_song_stage, validate_existing,
};
pub use crate::song_metadata::{audio_file, ini_fields, song_ini, update_ini};
use crate::{chart::Chart, mapping::Transcription, midi, models};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
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

pub const STEM_MANIFEST: &str = "stem-manifest.json";

#[derive(Clone, Copy, Debug, Default)]
pub struct IsolationOptions<'a> {
    /// Supplied-audio jobs fail rather than invoking either separator.
    pub skip_separation: bool,
    pub detection: crate::detection_settings::Settings,
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
    // The identity already hashes the current model. Compare that same digest
    // with the pinned catalog rather than hashing the full weights twice here.
    let asset = models::assets()
        .into_iter()
        .find(|asset| asset.id == "separation")
        .context("Missing separation model catalog entry")?;
    let model = model_dir.join(asset.filename);
    let identity = models::separation_identity(&model)?;
    ensure!(
        identity.model_sha256 == asset.sha256,
        "Separation model checksum mismatch"
    );
    isolation_cached_with(audio, &model, cache_root, identity, models::separate)
}

pub fn cached_keyboard_separation(
    audio: &Path,
    model_dir: &Path,
    cache_root: &Path,
) -> Result<crate::stem_review::StemManifest> {
    let model = crate::keyboard_separation::resolved_model_path(model_dir)?;
    // identity verifies both pinned byte length and hash before reuse/inference.
    let identity = crate::keyboard_separation::identity(&model)?;
    isolation_cached_with(
        audio,
        &model,
        cache_root,
        identity,
        crate::keyboard_separation::separate,
    )
}

pub struct IsolationPlan {
    pub primary: crate::stem_review::StemManifest,
    pub keyboard_analysis: Option<crate::stem_review::StemManifest>,
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
        !options.skip_separation || options.manifest.is_some() || options.stems.is_some(),
        "Existing-stem generation requires supplied stems; separation is disabled"
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
        ensure!(
            !options.skip_separation,
            "The attached stems do not include higher-quality Piano audio. Use Standard analysis or attach the required stems; separation is disabled."
        );
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
        crate::generation_progress::detail("Transcribing drums and detecting cymbal articulations");
        let drum_audio = if manifest.is_some() {
            find_stem("drums")?
        } else {
            audio.to_owned()
        };
        let (hits, recovered) = models::drums_configured(
            &drum_audio,
            &models::verified(model_dir, "drums")?,
            options.detection,
        )?;
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
        crate::generation_progress::detail("Transcribing guitar pitches and attacks");
        out.guitar_source = options.guitar_source;
        let inference = models::guitar_configured(
            &find_stem(options.guitar_source.stem().as_str())?,
            &models::verified(model_dir, "pitch")?,
            options.guitar_source,
            options.detection,
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
        crate::generation_progress::detail("Transcribing bass and resolving note lengths");
        crate::bass_analysis::infer_configured(
            &mut out,
            &find_stem("bass")?,
            &models::verified(model_dir, "pitch")?,
            options.detection,
        )?;
    }
    if wanted.iter().any(|n| n == "piano" || n == "pro_piano") {
        crate::generation_progress::detail("Transcribing keyboard notes and key releases");
        let evidence = crate::keyboard_detection::detect_configured(
            audio,
            keyboard_analysis
                .as_ref()
                .or(manifest.as_ref())
                .context("Keyboard detection requires source-bound stems")?,
            model_dir,
            options.keyboard_sound,
            options.detection,
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

pub use crate::recording_metadata::Metadata;

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

pub use crate::generation_build::{
    Build, build, build_with_lyrics, lyric_events, with_generated_rule_reviews,
};
use crate::generation_build::{higher_quality_manifest, isolation_limits};

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
