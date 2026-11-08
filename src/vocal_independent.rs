//! Explicit desktop multi-model analysis. Shared Rust decoding and reports are
//! used for review on both clients; missing native resources never select a
//! cheaper profile silently. Model outputs are reusable source-bound caches.
use crate::{clean_recognition::Control, models, vocal_generation::Generation};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
pub const DIRECTORY: &str = "vocal-independent";
pub const HELPERS: [&str; 5] = [
    "yace-singing-pitch-probe",
    "yace-rosvot-convert",
    "vocal_fcpe_probe",
    "vocal_swift_probe",
    "vocal_posterior_replay",
];
pub struct Asset {
    pub source: &'static str,
    pub filename: &'static str,
    pub sha256: &'static str,
}
pub const ASSETS: [Asset; 5] = [
    Asset {
        source: "full.onnx",
        filename: "full.onnx",
        sha256: "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610",
    },
    Asset {
        source: "fcpe-deiteris.onnx",
        filename: "fcpe.onnx",
        sha256: "0467e0a627c4dd7e774c2d3a05fb5ca1830af7491451ca0158e62cb2621e0908",
    },
    Asset {
        source: "rosvot/onnx-v2/encoder.onnx",
        filename: "rosvot/encoder.onnx",
        sha256: crate::vocal_note_consensus::ENCODER_SHA,
    },
    Asset {
        source: "rosvot/onnx-v2/decoder.onnx",
        filename: "rosvot/decoder.onnx",
        sha256: crate::vocal_note_consensus::DECODER_SHA,
    },
    Asset {
        source: "rosvot/rwbd-onnx-v2/encoder.onnx",
        filename: "rwbd/encoder.onnx",
        sha256: crate::vocal_note_consensus::RWBD_SHA,
    },
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperReceipt {
    pub name: String,
    pub sha256: String,
}
fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
pub fn helpers() -> Result<PathBuf> {
    let override_dir = std::env::var_os("YACE_VOCAL_TOOLS").map(PathBuf::from);
    let executable = std::env::current_exe()?;
    let executable_dir = executable.parent().context("Executable directory")?;
    let dir = override_dir
        .clone()
        .unwrap_or_else(|| executable_dir.join("vocal-tools"));
    // MacOS is a code directory: a JSON receipt there is interpreted as unsigned
    // nested code by codesign. Keep the receipt in the bundle's resource seal.
    let receipt = if override_dir.is_some() {
        dir.join("manifest.json")
    } else {
        executable_dir
            .parent()
            .context("Application contents")?
            .join("Resources/vocal-tools.json")
    };
    validate_helpers(&dir, &receipt)?;
    Ok(dir)
}
fn validate_helpers(dir: &Path, receipt: &Path) -> Result<()> {
    let receipts:Vec<HelperReceipt>=serde_json::from_slice(&fs::read(receipt).context("Multi-model pitch tools are not installed. Install the desktop vocal model pack; another analysis mode was not substituted.")?)?;
    ensure!(
        receipts.len() == HELPERS.len(),
        "Incomplete multi-model tool receipt"
    );
    for name in HELPERS {
        let receipt = receipts
            .iter()
            .find(|r| r.name == name)
            .context("Missing pitch tool receipt")?;
        ensure!(
            models::hash_file(&dir.join(executable(name)))? == receipt.sha256,
            "Multi-model tool changed: {name}"
        );
    }
    Ok(())
}
pub fn model_directory(root: &Path) -> Result<PathBuf> {
    let names: Vec<_> = ASSETS.iter().map(|a| a.filename).collect();
    let dir = models::optional_model_directory(root, Path::new(DIRECTORY), &names)?;
    for a in ASSETS {
        ensure!(
            models::hash_file(&dir.join(a.filename)).with_context(|| format!(
                "Multi-model pitch model pack is missing {}. Another mode was not substituted.",
                a.filename
            ))? == a.sha256,
            "Independent pitch model checksum mismatch: {}",
            a.filename
        );
    }
    Ok(dir)
}
pub fn cache_key(report: &Generation) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &report.audio_sha256,
            &report.vocal_stem_sha256,
            report.duration_seconds,
            crate::vocal_rmvpe::implementation_hash(),
            crate::vocal_pitch::implementation_hash(),
            crate::vocal_note_recipe::implementation_hash(),
            crate::vocal_posterior::spectral_probe_implementation_hash(),
            crate::vocal_posterior::fcpe_implementation_hash(),
        ))?)
    ))
}
fn child(command: &mut Command, control: &Arc<Control>, label: &str) -> Result<()> {
    control.check()?;
    control.update(label);
    command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut process = command.spawn().with_context(|| format!("Start {label}"))?;
    let stderr = process.stderr.take().context("Pitch tool diagnostics")?;
    let tail = Arc::new(Mutex::new(String::new()));
    let capture = tail.clone();
    let progress = control.clone();
    let name = label.to_owned();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::result::Result::ok)
        {
            let line: String = line.chars().take(2000).collect();
            progress.update(format!("{name}: {line}"));
            if let Ok(mut last) = capture.lock() {
                *last = line;
            }
        }
    });
    let outcome = loop {
        if control.is_cancelled() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(process.id() as i32), libc::SIGTERM);
            }
            let _ = process.kill();
            let _ = process.wait();
            break None;
        }
        match process.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(process.id() as i32), libc::SIGTERM);
                }
                let _ = process.kill();
                let _ = process.wait();
                let _ = reader.join();
                return Err(error.into());
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = reader.join();
    ensure!(!control.is_cancelled(), "Vocal generation cancelled");
    ensure!(
        outcome.is_some_and(|s| s.success()),
        "{label} failed: {}",
        tail.lock().map(|s| s.clone()).unwrap_or_default()
    );
    Ok(())
}
fn raw_valid(path: &Path, report: &Generation, model: &str) -> bool {
    let check = || -> Result<()> {
        ensure!(
            fs::metadata(path)?.len() <= 128 * 1024 * 1024,
            "Raw evidence budget"
        );
        let v: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let source = v
            .get("source_sha256")
            .or_else(|| v["source"].get("sha256"))
            .and_then(|s| s.as_str());
        ensure!(
            source == Some(&report.vocal_stem_sha256) && v["model_sha256"] == model,
            "Raw evidence source/model"
        );
        let implementation = if v["model_id"] == "fcpe-deiteris-onnx" {
            crate::vocal_posterior::fcpe_implementation_hash()
        } else {
            crate::vocal_posterior::spectral_probe_implementation_hash()
        };
        ensure!(
            v["implementation_sha256"] == implementation
                && v["pcm_sha256"].as_str()
                    == Some(
                        &report
                            .rmvpe_pitch
                            .as_ref()
                            .context("Primary pitch")?
                            .pcm_sha256
                    ),
            "Cached pitch recipe/PCM differs"
        );
        let blob = path
            .parent()
            .unwrap()
            .join(v["salience_file"].as_str().context("Salience path")?);
        ensure!(
            models::hash_file(&blob)? == v["salience_sha256"].as_str().context("Salience SHA")?,
            "Raw evidence checksum"
        );
        Ok(())
    };
    check().is_ok()
}
fn publish_raw(temp: &Path, destination: &Path) -> Result<()> {
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(temp)?)?;
    let old = PathBuf::from(v["salience_file"].as_str().context("Raw blob")?);
    let source = temp.parent().unwrap().join(&old);
    let basename = old.file_name().context("Raw filename")?;
    fs::create_dir_all(destination.parent().context("Cache parent")?)?;
    let target = destination.parent().unwrap().join(basename);
    if target.exists() {
        fs::remove_file(&target)?;
    }
    fs::rename(source, &target)?;
    // Relocation changes storage coordinates only, never strength values.
    v["salience_file"] = basename.to_string_lossy().into_owned().into();
    crate::generation::write_json(destination, &v)?;
    Ok(())
}
pub fn analyze(
    baseline: &Generation,
    audio: &Path,
    stem: &Path,
    model_root: &Path,
    cache_root: &Path,
    control: Arc<Control>,
) -> Result<Generation> {
    ensure!(
        baseline.policy.analysis_mode == crate::vocal_generation::AnalysisMode::HighAccuracy,
        "Multi-model pitch requires acoustic High accuracy boundaries"
    );
    let tools = helpers()?;
    let pack = model_directory(model_root)?;
    let swift = models::verified(model_root, "vocal-pitch")?;
    let rmvpe = crate::vocal_rmvpe::prepare(model_root, false, &|| control.is_cancelled())?;
    ensure!(
        models::hash_file(audio)? == baseline.audio_sha256
            && models::hash_file(stem)? == baseline.vocal_stem_sha256,
        "Independent pitch source changed"
    );
    let cache = cache_root.join(cache_key(baseline)?);
    fs::create_dir_all(&cache)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(cache.join("analysis.lock"))?;
    lock.try_lock()
        .context("These independent pitch models are already analyzing this recording")?;
    let baseline_file = cache.join("baseline.json");
    crate::generation::write_json(&baseline_file, baseline)?;
    for (kind, name, model) in [
        ("rmvpe", "raw-rmvpe-source", rmvpe),
        ("full", "fresh-full-source", pack.join("full.onnx")),
    ] {
        let destination = cache.join(name).join("evidence.json");
        let sha = if kind == "rmvpe" {
            crate::vocal_rmvpe::SHA256
        } else {
            ASSETS[0].sha256
        };
        if !raw_valid(&destination, baseline, sha) {
            let temp = tempfile::tempdir_in(&cache)?;
            let output = temp.path().join("raw");
            child(
                Command::new(tools.join(executable(HELPERS[0])))
                    .args(["--model", kind, "--model-file"])
                    .arg(model)
                    .arg("--input")
                    .arg(stem)
                    .arg("--output")
                    .arg(&output),
                &control,
                if kind == "rmvpe" {
                    "Comparing RMVPE pitches"
                } else {
                    "Comparing CREPE full pitches"
                },
            )?;
            publish_raw(&output.join("evidence.json"), &destination)?;
        }
    }
    let fcpe = cache.join("source-fcpe-posterior.json");
    if !raw_valid(&fcpe, baseline, ASSETS[1].sha256) {
        let temp = tempfile::tempdir_in(&cache)?;
        let file = temp.path().join("source-fcpe-posterior.json");
        child(
            Command::new(tools.join(executable(HELPERS[2])))
                .arg(stem)
                .arg(pack.join("fcpe.onnx"))
                .arg(&file),
            &control,
            "Comparing FCPE pitches",
        )?;
        publish_raw(&file, &fcpe)?;
    }
    let swift_file = cache.join("source-swift.json");
    let valid_swift = fs::read(&swift_file)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::vocal_pitch::Evidence>(&b).ok())
        .is_some_and(|e| {
            crate::vocal_generation::validate_vocal_frame_evidence(
                &e,
                &baseline.audio_sha256,
                &baseline.vocal_stem_sha256,
                baseline.duration_seconds,
            )
            .is_ok()
        });
    if !valid_swift {
        let temp = tempfile::tempdir_in(&cache)?;
        let file = temp.path().join("swift.json");
        child(
            Command::new(tools.join(executable(HELPERS[3])))
                .arg(audio)
                .arg(stem)
                .arg(&baseline_file)
                .arg(swift)
                .arg(&file),
            &control,
            "Comparing SwiftF0 pitches",
        )?;
        if swift_file.exists() {
            fs::remove_file(&swift_file)?;
        }
        fs::rename(file, &swift_file)?;
    }
    let note_file = cache.join("source.notes.json");
    let valid_notes = fs::read(&note_file)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::vocal_note_consensus::Evidence>(&b).ok())
        .is_some_and(|e| {
            crate::vocal_note_consensus::validate(
                &e,
                &baseline.vocal_stem_sha256,
                baseline.duration_seconds,
            )
            .is_ok()
        });
    if !valid_notes {
        let temp = tempfile::tempdir_in(&cache)?;
        let file = temp.path().join("notes.json");
        child(
            Command::new(tools.join(executable(HELPERS[1])))
                .arg("infer")
                .arg(stem)
                .arg(&baseline_file)
                .arg(pack.join("rosvot"))
                .arg(pack.join("rwbd"))
                .arg(&file),
            &control,
            "Comparing ROSVOT acoustic note centres",
        )?;
        if note_file.exists() {
            fs::remove_file(&note_file)?;
        }
        fs::rename(file, &note_file)?;
    }
    let corpus = crate::vocal_benchmark::Corpus {
        version: 1,
        recordings: vec![crate::vocal_benchmark::Recording {
            id: "source".into(),
            artist: "Current recording".into(),
            language: "und".into(),
            split: "held_out".into(),
            categories: vec![],
            audio: audio.into(),
            audio_sha256: baseline.audio_sha256.clone(),
            reference: None,
            baseline: Some(baseline_file.clone()),
            candidate: Some(baseline_file),
        }],
    };
    let corpus_file = cache.join("input.json");
    crate::generation::write_json(&corpus_file, &corpus)?;
    let temp = tempfile::tempdir_in(&cache)?;
    let output = temp.path().join("decoded");
    child(
        Command::new(tools.join(executable(HELPERS[4])))
            .arg(&corpus_file)
            .arg(&cache)
            .arg(&cache)
            .arg(&output)
            .args(["--notes-root"])
            .arg(&cache)
            .args(["--swift-root"])
            .arg(&cache)
            .args([
                "--predict-only",
                "--predict-recovery",
                "--predict-energy",
                "--held-out",
                "--crepe-prefix",
                "fresh-full-",
            ]),
        &control,
        "Combining independent pitch evidence",
    )?;
    let result: Generation =
        serde_json::from_slice(&fs::read(output.join("source.generation.json"))?)?;
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("frozen-profile.json"))?)?;
    ensure!(
        receipt["contour_implementation_sha256"]
            == format!("{:x}", Sha256::digest(include_bytes!("vocal_posterior.rs")))
            && receipt["target_values_read"] == false,
        "Pitch decoder resource differs from the installed app"
    );
    ensure!(
        result.algorithm == crate::vocal_posterior::FROZEN_ENERGY_PROFILE
            && result.audio_sha256 == baseline.audio_sha256
            && result.vocal_stem_sha256 == baseline.vocal_stem_sha256
            && result.timing == baseline.timing,
        "Independent model result binding differs"
    );
    ensure!(
        result.note_evidence.iter().all(|n| n.models.len() == 5),
        "Pitch decoder did not retain the five requested model observations"
    );
    if result.chart.is_some() {
        crate::vocal_review::VocalReviewMap::from_generation(&result)?;
    }
    control.check()?;
    ensure!(
        models::hash_file(audio)? == baseline.audio_sha256
            && models::hash_file(stem)? == baseline.vocal_stem_sha256,
        "Sources changed during independent analysis"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_receipts_reject_missing_replaced_and_duplicate_resources() {
        let temp = tempfile::tempdir().unwrap();
        let receipt = temp.path().join("manifest.json");
        assert!(validate_helpers(temp.path(), &receipt).is_err());
        let mut receipts = vec![];
        for name in HELPERS {
            let path = temp.path().join(executable(name));
            fs::write(&path, name).unwrap();
            receipts.push(HelperReceipt {
                name: name.into(),
                sha256: models::hash_file(&path).unwrap(),
            });
        }
        crate::generation::write_json(&temp.path().join("manifest.json"), &receipts).unwrap();
        validate_helpers(temp.path(), &receipt).unwrap();
        fs::write(temp.path().join(executable(HELPERS[0])), b"changed").unwrap();
        assert!(validate_helpers(temp.path(), &receipt).is_err());
        receipts[0].name = HELPERS[1].into();
        crate::generation::write_json(&temp.path().join("manifest.json"), &receipts).unwrap();
        assert!(validate_helpers(temp.path(), &receipt).is_err());
    }
    #[test]
    fn cached_salience_relocation_preserves_values_and_uses_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input");
        let output = temp.path().join("output");
        fs::create_dir(&input).unwrap();
        let raw = input.join("data.f32le");
        fs::write(&raw, [0u8; 16]).unwrap();
        let json = input.join("evidence.json");
        crate::generation::write_json(&json,&serde_json::json!({"salience_file":raw,"salience_sha256":models::hash_file(&raw).unwrap()})).unwrap();
        let target = output.join("evidence.json");
        publish_raw(&json, &target).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&fs::read(target).unwrap()).unwrap();
        assert_eq!(v["salience_file"], "data.f32le");
        assert_eq!(
            models::hash_file(&output.join("data.f32le")).unwrap(),
            v["salience_sha256"]
        );
    }
    #[cfg(unix)]
    #[test]
    fn cancellation_stops_the_owned_worker() {
        let control = Arc::new(Control::default());
        let stop = control.clone();
        let signal = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            stop.cancel();
        });
        let start = std::time::Instant::now();
        let error = child(
            Command::new("/bin/sleep").arg("20"),
            &control,
            "Test pitch worker",
        )
        .unwrap_err();
        signal.join().unwrap();
        assert!(error.to_string().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
