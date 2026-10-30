//! Staged, source-bound generation. Targets are not read by this runner.
use anyhow::{Context, Result, ensure};
use chart_engine::{chart::Chart, game_export, models::hash_file, timing::TempoMap};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

pub struct Request<'a> {
    pub engine: &'a Path,
    pub case: &'a Path,
    pub oracle: &'a Path,
    pub output: &'a Path,
    pub models: &'a Path,
    pub condition: &'a str,
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn stage(
    engine: &Path,
    output: &Path,
    name: &str,
    args: Vec<String>,
    stages: &mut Vec<Value>,
) -> Result<()> {
    let log = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(format!("{name}.log")))?;
    let start = Instant::now();
    let status = Command::new(engine)
        .args(&args)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .status();
    stages.push(json!({"name":name,"arguments":args,"elapsed_seconds":start.elapsed().as_secs_f64(),
        "success":status.as_ref().is_ok_and(|s|s.success()),"exit_code":status.as_ref().ok().and_then(|s|s.code())}));
    ensure!(status?.success(), "{name} failed; see retained log");
    Ok(())
}

/// Creates a new result directory, keeps failures, and never resumes partial jobs
/// silently. Oracle and ordinary full-mix results have distinct conditions.
pub fn run(request: Request<'_>) -> Result<()> {
    ensure!(
        matches!(
            request.condition,
            "oracle" | "full-mix" | "model-free-vocals"
        ),
        "unknown condition"
    );
    ensure!(
        !request.output.exists(),
        "refusing existing result directory"
    );
    let engine = fs::canonicalize(request.engine)?;
    let case = fs::canonicalize(request.case)?;
    let oracle = fs::canonicalize(request.oracle)?;
    let models = fs::canonicalize(request.models)?;
    let source = case.join("source.wav");
    let mix: Value = serde_json::from_slice(&fs::read(case.join("source.mix.json"))?)?;
    let source_hash = hash_file(&source)?;
    ensure!(mix["output_sha256"] == source_hash, "source mix changed");
    let manifest = oracle.join("stem-manifest.json");
    // Reading the typed manifest validates its source/audio files independently;
    // generation performs its own pipeline validation again.
    let _: chart_engine::stem_review::StemManifest = serde_json::from_slice(&fs::read(&manifest)?)?;
    let engine_hash = hash_file(&engine)?;
    let mut model_hashes = serde_json::Map::new();
    if request.condition != "model-free-vocals" {
        for name in [
            "basic_pitch.onnx",
            "adtof_frame_rnn.onnx",
            "swift-f0-v0.3.0.onnx",
        ] {
            model_hashes.insert(name.into(), json!(hash_file(&models.join(name))?));
        }
        for name in ["config.json", "vocab.json", "model.safetensors"] {
            let relative = format!("vocal-alignment/en/{name}");
            model_hashes.insert(relative.clone(), json!(hash_file(&models.join(relative))?));
        }
        if request.condition == "full-mix" {
            model_hashes.insert(
                "htdemucs_6s.safetensors".into(),
                json!(hash_file(&models.join("htdemucs_6s.safetensors"))?),
            );
        }
    }
    fs::create_dir(request.output)?;
    let output = fs::canonicalize(request.output)?;
    let mut stages = Vec::new();
    let mut artifact = None;
    let mut instrument_outcomes: Vec<Value> = ["guitar", "bass", "drums", "pro_drums", "vocals"]
        .into_iter()
        .map(|instrument| {
            let unrequested = request.condition == "model-free-vocals" && instrument != "vocals";
            json!({"instrument":instrument,"success":false,
                "status":if unrequested { "not_run" } else { "not_exported" },
                "error":if unrequested { "Instrument not requested by model-free-vocals condition" }
                    else { "Generation did not reach a validated export for this instrument" }})
        })
        .collect();
    let start = Instant::now();
    let result = (|| -> Result<PathBuf> {
        let analysis_manifest = if request.condition == "full-mix" {
            let separation = output.join("separation");
            stage(
                &engine,
                &output,
                "separation",
                vec![
                    "separate".into(),
                    "--audio".into(),
                    p(&source),
                    "--model-dir".into(),
                    p(&models),
                    "--output".into(),
                    p(&separation),
                ],
                &mut stages,
            )?;
            separation.join("stem-manifest.json")
        } else {
            manifest
        };
        if request.condition != "model-free-vocals" {
            stage(
                &engine,
                &output,
                "transcription",
                vec![
                    "transcribe".into(),
                    "--audio".into(),
                    p(&source),
                    "--stem-manifest".into(),
                    p(&analysis_manifest),
                    "--model-dir".into(),
                    p(&models),
                    "--instruments".into(),
                    "guitar,bass,drums".into(),
                    "--output".into(),
                    p(&output.join("events.json")),
                ],
                &mut stages,
            )?;
            stage(
                &engine,
                &output,
                "mapping",
                vec![
                    "build".into(),
                    "--events".into(),
                    p(&output.join("events.json")),
                    "--audio".into(),
                    p(&source),
                    "--bpm".into(),
                    "120".into(),
                    "--title".into(),
                    "Blind reference benchmark".into(),
                    "--artist".into(),
                    "YACE benchmark".into(),
                    "--output".into(),
                    p(&output.join("song")),
                ],
                &mut stages,
            )?;
        }
        let mut args = vec![
            "vocals".into(),
            "generate".into(),
            "--audio".into(),
            p(&source),
            "--stem-manifest".into(),
            p(&analysis_manifest),
            "--bpm".into(),
            "120".into(),
            "--lyric-source".into(),
            p(&oracle.join("lyric-source.json")),
            "--model-dir".into(),
            p(&models),
            "--output".into(),
            p(&output.join("vocals")),
        ];
        if request.condition != "model-free-vocals" {
            args.extend(["--recognize".into(), "--neural-pitch".into()]);
        }
        let vocal_stage = stage(&engine, &output, "vocals", args, &mut stages);
        let chart = if request.condition == "model-free-vocals" {
            None
        } else {
            Some(Chart::parse(fs::read_to_string(
                output.join("song/notes.chart"),
            )?)?)
        };
        let timing = TempoMap::constant(120.)?;
        let (band_midi, delay) = if let Some(chart) = &chart {
            game_export::song(chart, None)?
        } else {
            (
                chart_engine::midi::new_midi(&timing, &Default::default())?,
                0,
            )
        };
        ensure!(
            delay == 0,
            "unexpected generated offset; refuse silent timing change"
        );
        let vocal_path = output.join("vocals/vocals.json");
        let report_path = output.join("vocals/generation.json");
        let mut abstention = None;
        // A failed vocal stage can leave a partial JSON file. Never consume it
        // as a completed chart, and never discard the already mapped band.
        let vocal_result = vocal_stage.and_then(|()| -> Result<Option<Vec<u8>>> {
            let report = chart_benchmark::evaluate::validate_vocal_generation(
                &report_path,
                &analysis_manifest,
                &source_hash,
                mix["duration_seconds"]
                    .as_f64()
                    .context("Missing source sample clock")?,
                &timing,
            )?;
            if let Some(vocal) = &report.chart {
                let exported: chart_engine::vocals::VocalChart = serde_json::from_slice(
                    &fs::read(&vocal_path)
                        .context("Vocal generation did not export its reported chart")?,
                )?;
                ensure!(
                    &exported == vocal,
                    "Vocal export contradicts generation report"
                );
                Ok(Some(chart_engine::vocals::append_midi(&band_midi, vocal)?))
            } else {
                ensure!(
                    !vocal_path.exists(),
                    "Vocal abstention contradicts an exported chart"
                );
                abstention = Some(
                    json!({"report":report_path,"report_sha256":hash_file(&report_path)?,
                    "manifest_sha256":hash_file(&analysis_manifest)?,"status":report.status}),
                );
                Ok(None)
            }
        });
        let midi = vocal_result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .unwrap_or(&band_midi);
        // Keep the editable .chart song unambiguous. The comparator consumes
        // this separate combined MIDI explicitly; no duplicate game masters.
        let destination = output.join("generated.mid");
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?
            .write_all(midi)?;
        artifact = Some(destination.clone());
        for outcome in &mut instrument_outcomes {
            if outcome["instrument"] == "vocals" {
                outcome["success"] = json!(vocal_result.is_ok());
                outcome["abstained"] = json!(vocal_result.is_ok() && abstention.is_some());
                outcome["status"] = json!(if vocal_result.is_err() {
                    "failed"
                } else if abstention.is_some() {
                    "abstained"
                } else {
                    "exported"
                });
                outcome["abstention"] = json!(abstention);
                outcome["error"] = json!(vocal_result.as_ref().err().map(|e| format!("{e:#}")));
            } else if request.condition != "model-free-vocals" {
                outcome["success"] = json!(true);
                outcome["status"] = json!("exported");
                outcome["error"] = Value::Null;
            }
        }
        // Retain an overall failure for callers/batch receipts even though
        // the source-bound partial MIDI is independently usable.
        let _ = vocal_result?;
        Ok(destination)
    })();
    let vocal_status = fs::read(output.join("vocals/generation.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|v| {
            json!({
            "status":v["status"],"exported_chart":!v["chart"].is_null(),
            "pitch_candidates":v["pitch_candidates"].as_array().map(Vec::len),
            "review_issues":v["issues"].as_array().map(Vec::len)})
        });
    let receipt = json!({"version":1,"condition":request.condition,"engine":engine,"engine_sha256":engine_hash,
        "engine_unchanged_after_run":engine_hash==hash_file(&engine)?,
        "source_sha256":source_hash,"source_archive_sha256":mix["source_archive_sha256"],"models":model_hashes,
        "oracle_preparation_receipt":oracle.join("preparation.json"),"fixed_bpm":120,
        "targets_enter_generation":false,"lyrics_condition":"supplied reference plain text only; no authored timing, pitch or phrases",
        "cache_condition":"persistent model/separation caches may be warm; stage wall times are not cold-start or human editing-time measurements",
        "elapsed_seconds":start.elapsed().as_secs_f64(),"stages":stages,"vocal_outcome":vocal_status,
        "success":result.is_ok(),"artifact":artifact.as_ref().or(result.as_ref().ok()),
        "artifact_sha256":artifact.as_ref().or(result.as_ref().ok()).map(|p|hash_file(p)).transpose()?,
        "instrument_outcomes":instrument_outcomes,"error":result.as_ref().err().map(|e|format!("{e:#}")),
        "model_free_scope":if request.condition=="model-free-vocals" {Some("Oracle stem DSP only; missing word alignment must remain an abstention. This does not establish model-free full-band audio-to-chart capability.")}else{None}});
    fs::write(
        output.join("run.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    result.context("Generation failed; run.json and logs retained")?;
    Ok(())
}

/// All predeclared cases remain present, including failed preparation/inference.
/// This command deliberately does not compare targets, so baseline inference can
/// finish while a development-only candidate is being frozen.
pub fn batch(
    engine: &Path,
    batch: &Path,
    output: &Path,
    models: &Path,
    condition: &str,
) -> Result<()> {
    ensure!(
        matches!(condition, "oracle" | "full-mix" | "model-free-vocals"),
        "unknown condition"
    );
    ensure!(!output.exists(), "refusing existing batch result directory");
    let frozen: Value = serde_json::from_slice(&fs::read(batch.join("frozen-batch.json"))?)?;
    ensure!(
        frozen["songs"].as_array().is_some_and(|s| s.len() == 6),
        "six frozen cases required"
    );
    fs::create_dir(output)?;
    let count = if condition == "full-mix" { 2 } else { 6 };
    let mut outcomes = Vec::new();
    for index in 1..=count {
        let label = format!("heldout-{index:02}");
        let case = batch.join(&label);
        let preparation = output.join(format!("{label}-inputs"));
        let preparation_log = fs::File::create(output.join(format!("{label}-preparation.log")))?;
        let result = (|| -> Result<()> {
            let status = Command::new(std::env::current_exe()?)
                .arg("prepare")
                .arg(&case)
                .arg(&preparation)
                .stdout(Stdio::from(preparation_log.try_clone()?))
                .stderr(Stdio::from(preparation_log))
                .status()?;
            ensure!(status.success(), "input preparation failed");
            let prepared: Value =
                serde_json::from_slice(&fs::read(preparation.join("preparation.json"))?)?;
            chart_benchmark::validate_frozen_case(
                &frozen,
                index - 1,
                prepared["source_archive_sha256"]
                    .as_str()
                    .context("Preparation archive identity missing")?,
            )?;
            println!("{label}: {condition} inference starting");
            run(Request {
                engine,
                case: &case,
                oracle: &preparation,
                output: &output.join(&label),
                models,
                condition,
            })
        })();
        outcomes.push(json!({"case":label,"success":result.is_ok(),"error":result.as_ref().err().map(|e|format!("{e:#}"))}));
        fs::write(
            output.join("batch-run.json"),
            serde_json::to_vec_pretty(&json!({"version":1,
            "frozen_batch_sha256":hash_file(&batch.join("frozen-batch.json"))?,"condition":condition,
            "expected_cases":count,"outcomes":outcomes,"complete":index==count,"target_comparisons_performed":false}))?,
        )?;
        println!("{label}: success={}", result.is_ok());
    }
    ensure!(
        outcomes.iter().all(|o| o["success"] == true),
        "one or more cases failed; retained all outcomes"
    );
    Ok(())
}
