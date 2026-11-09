//! Prepare a frozen professional-reference batch without reading MIDI targets.
//! Reuses source-clock stems and pinned raw RMVPE caches. Does not align words.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment,
    models,
    timing::TempoMap,
    vocal_benchmark::{Corpus, Recording},
    vocal_generation as vg, vocal_rmvpe,
};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Parser)]
struct Args {
    selection: PathBuf,
    prepared: PathBuf,
    output: PathBuf,
    root: PathBuf,
    probe: PathBuf,
    model: PathBuf,
    #[arg(long, default_value = "case")]
    prepared_prefix: String,
    #[arg(long)]
    id_prefix: String,
    /// Verify a UVR lead receipt and parent generation beside each prepared stem.
    #[arg(long)]
    derived_lead: bool,
}
fn read(p: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn write(p: &Path, v: &impl serde::Serialize) -> Result<()> {
    ensure!(!p.exists(), "Output exists {}", p.display());
    fs::write(p, serde_json::to_vec(v)?)?;
    Ok(())
}
fn implementation() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("../tools/singing-pitch-probe/src/main.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/src/rmvpe.rs"));
    h.update(include_bytes!("../src/rmvpe_frontend.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.toml"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.lock"));
    format!("{:x}", h.finalize())
}
fn main() -> Result<()> {
    let a = Args::parse();
    let selection = read(&a.selection)?;
    let songs = selection["songs"].as_array().context("Songs")?;
    ensure!(!songs.is_empty() && songs.len() <= 12, "Batch budget");
    ensure!(
        !a.output.join("inputs-corpus.json").exists(),
        "Prepared reports exist"
    );
    ensure!(
        models::hash_file(&a.model)? == vocal_rmvpe::SHA256,
        "Model differs"
    );
    let probe_sha = models::hash_file(&a.probe)?;
    let timing = TempoMap::constant(120.)?;
    let mut recordings = vec![];
    for (i, song) in songs.iter().enumerate() {
        let suffix = format!("{:02}", i + 1);
        let id = format!("{}-{suffix}", a.id_prefix);
        let source = a.prepared.join(format!("{}-{suffix}", a.prepared_prefix));
        let out = a.output.join(format!("case-{suffix}"));
        let receipt = read(&source.join("reference-source.json"))?;
        ensure!(
            receipt["source_archive_sha256"] == song["archive_sha256"],
            "Frozen archive differs"
        );
        let audio = source.join("source.wav");
        let stem = out.join("vocals.wav");
        ensure!(
            stem.exists(),
            "Prepare the explicit zero-padded source-clock vocal stem first"
        );
        let mut binding=StemAlignment{audio_sha256:models::hash_file(&audio)?,stem_sha256:models::hash_file(&stem)?,evidence:"Direct vocal channel from checksum-frozen official archive, zero-padding only on the original recording clock; target notes and timestamps excluded".into()};
        if a.derived_lead {
            let split = read(&out.join("separation.json"))?;
            let parent: vg::Generation =
                serde_json::from_slice(&fs::read(out.join("parent-generation.json"))?)?;
            ensure!(
                parent.audio_sha256 == binding.audio_sha256
                    && split["vocal_stem_sha256"] == parent.vocal_stem_sha256
                    && split["lead_sha256"] == binding.stem_sha256
                    && split["model_sha256"]
                        == "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4"
                    && split["sample_rate"] == 44100
                    && split["channels"] == 2
                    && split["frames"]
                        .as_f64()
                        .is_some_and(
                            |n| (n / 44100. - parent.duration_seconds).abs() <= 1. / 44100.
                        ),
                "Derived lead parent/model/source clock differs"
            );
            binding.evidence = format!(
                "Derived UVR karaoke lead; parent stem {}; separator bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4; original recording clock retained; no lyrics or target pitches",
                parent.vocal_stem_sha256
            );
        }
        let cache = a.root.join(format!("raw-rmvpe-{id}"));
        let raw_path = cache.join("evidence.json");
        if !raw_path.exists() {
            ensure!(!cache.exists(), "Incomplete raw cache exists");
            ensure!(
                Command::new(&a.probe)
                    .args(["--model", "rmvpe", "--model-file"])
                    .arg(&a.model)
                    .arg("--input")
                    .arg(&stem)
                    .arg("--output")
                    .arg(&cache)
                    .status()?
                    .success(),
                "Raw model analysis failed"
            )
        }
        let raw = read(&raw_path)?;
        let pcm = models::decode(&stem, 16000, 1)?;
        let mut ph = Sha256::new();
        for p in &pcm {
            ph.update(p.to_le_bytes())
        }
        let pcm_sha = format!("{:x}", ph.finalize());
        ensure!(
            raw["model_sha256"] == vocal_rmvpe::SHA256
                && raw["source"]["sha256"].as_str() == Some(&binding.stem_sha256)
                && raw["pcm_sha256"] == pcm_sha
                && raw["implementation_sha256"] == implementation()
                && raw["sample_rate"] == 16000
                && raw["hop_samples"] == 160,
            "Raw cache source, model, frontend or PCM differs"
        );
        let blob = cache.join(raw["salience_file"].as_str().context("Blob")?);
        let count = pcm.len() / 160 + 1;
        ensure!(
            raw["salience_shape"] == json!([count, 360])
                && fs::metadata(&blob)?.len() == count as u64 * 360 * 4
                && models::hash_file(&blob)?
                    == raw["salience_sha256"]
                        .as_str()
                        .context("Salience checksum")?,
            "Raw blob shape/checksum differs"
        );
        let bytes = fs::read(blob)?;
        let values: Vec<_> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let frames = vocal_rmvpe::decode_chunk(&pcm, 0, &values)?;
        let mut evidence = vocal_rmvpe::finish_pcm(&pcm, frames)?;
        drop(values);
        drop(bytes);
        drop(pcm);
        let full = models::decode(&audio, 44100, 2)?;
        evidence.duration_seconds = full.len() as f64 / 88200.;
        drop(full);
        evidence.audio_sha256 = binding.audio_sha256.clone();
        evidence.vocal_stem_sha256 = binding.stem_sha256.clone();
        vocal_rmvpe::validate(
            &evidence,
            &binding.audio_sha256,
            &binding.stem_sha256,
            evidence.duration_seconds,
        )?;
        let paired = out.join("paired");
        ensure!(!paired.exists(), "Paired reports exist");
        fs::create_dir(&paired)?;
        for (name, mode, model) in [
            ("standard", vg::AnalysisMode::Standard, None),
            ("high_accuracy", vg::AnalysisMode::HighAccuracy, None),
            (
                "current_high",
                vg::AnalysisMode::HighAccuracy,
                Some(&evidence),
            ),
        ] {
            let report = vg::generate(vg::Request {
                audio: &audio,
                vocal_stem: &stem,
                alignment: &binding,
                timing: &timing,
                words: None,
                lyrics: None,
                policy: &vg::Policy {
                    analysis_mode: mode,
                    ..Default::default()
                },
                neural_pitch: None,
                vocal_pitch: None,
                rmvpe_pitch: model,
            })?;
            write(&paired.join(format!("{name}.json")), &report)?
        }
        let language = match song["vocal_language"].as_str() {
            Some("eng") => "en",
            Some("spa") => "es",
            Some(l) => l,
            None => "und",
        };
        recordings.push(Recording {
            id,
            artist: song["metadata"]["artist"]
                .as_str()
                .context("Artist")?
                .into(),
            language: language.into(),
            split: "held_out".into(),
            categories: vec![],
            audio,
            audio_sha256: binding.audio_sha256,
            reference: None,
            baseline: Some(paired.join("standard.json")),
            candidate: Some(paired.join("current_high.json")),
        });
        eprintln!("{suffix}: source-bound baseline prepared; target MIDI unopened");
    }
    ensure!(
        models::hash_file(&a.probe)? == probe_sha,
        "Probe changed during batch"
    );
    write(
        &a.output.join("inputs-corpus.json"),
        &Corpus {
            version: 1,
            recordings,
        },
    )?;
    write(
        &a.output.join("input-run.json"),
        &json!({"selection_sha256":models::hash_file(&a.selection)?,"probe_sha256":probe_sha,"executable_sha256":models::hash_file(&std::env::current_exe()?)?,"bpm":120,"target_values_read":false,"raw_rmvpe_cache_reused":true}),
    )?;
    Ok(())
}
