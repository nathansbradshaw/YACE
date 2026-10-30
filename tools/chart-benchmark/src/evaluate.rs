//! Independent evaluation after candidate policy freeze. No generation here.
use anyhow::{Context, Result, ensure};
use chart_engine::models::hash_file;
use chart_engine::{
    stem_review::{StemId, StemManifest},
    timing::TempoMap,
    vocal_generation::{Generation, Status},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

pub(crate) fn ratio(a: f64, b: f64) -> Option<f64> {
    (b > 0.).then_some(a / b)
}
pub(crate) fn metrics(reports: &[&Value]) -> Value {
    let mut counts = BTreeMap::<u64, (u64, u64, u64, Vec<f64>, Vec<f64>)>::new();
    for report in reports {
        if let Some(attacks) = report["attacks"].as_array() {
            for attack in attacks {
                let entry = counts
                    .entry(attack["tolerance_ms"].as_u64().unwrap())
                    .or_default();
                entry.0 += attack["matched"].as_u64().unwrap();
                entry.1 += attack["reference_attacks"].as_u64().unwrap();
                entry.2 += attack["generated_attacks"].as_u64().unwrap();
                if let Some(p) = attack["precision"].as_f64() {
                    entry.3.push(p);
                }
                if let Some(r) = attack["recall"].as_f64() {
                    entry.4.push(r);
                }
            }
        }
    }
    json!(counts.into_iter().map(|(t,(m,r,g,mp,mr))|json!({"tolerance_ms":t,"matched":m,
        "reference_attacks":r,"generated_attacks":g,"micro_precision":ratio(m as f64,g as f64),
        "micro_recall":ratio(m as f64,r as f64),"micro_f1":ratio(2.*m as f64,(r+g)as f64),
        "macro_precision":ratio(mp.iter().sum(),mp.len()as f64),"macro_precision_defined_cases":mp.len(),
        "macro_recall":ratio(mr.iter().sum(),mr.len()as f64),"macro_recall_defined_cases":mr.len()})).collect::<Vec<_>>())
}
pub(crate) fn interval_metrics(reports: &[&Value], field: &str, fields: &[&str]) -> Value {
    let mut totals = serde_json::Map::new();
    for name in fields {
        totals.insert(
            (*name).into(),
            json!(
                reports
                    .iter()
                    .filter_map(|r| r[field][*name].as_f64())
                    .sum::<f64>()
            ),
        );
    }
    json!(totals)
}

/// Shared by inference and evaluation: an explicit abstention is meaningful
/// only when its typed report belongs to the actual source, stem and clock.
pub fn validate_vocal_generation(
    report_path: &Path,
    manifest_path: &Path,
    source_hash: &str,
    duration: f64,
    timing: &TempoMap,
) -> Result<Generation> {
    let report: Generation = serde_json::from_slice(&fs::read(report_path)?)?;
    let manifest = StemManifest::read(manifest_path)?;
    manifest.validate_binding(source_hash, duration)?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Vocals)
        .context("Vocal stem is absent")?;
    ensure!(
        report.version == 1 && !report.algorithm.trim().is_empty(),
        "Invalid vocal report identity"
    );
    ensure!(
        duration.is_finite()
            && duration > 0.
            && report.duration_seconds.is_finite()
            && (report.duration_seconds - duration).abs() <= 1. / 44100.
            && (manifest.frames as f64 / 44100. - duration).abs() <= 1. / 44100.,
        "Vocal report sample clock changed"
    );
    ensure!(
        report.audio_sha256.eq_ignore_ascii_case(source_hash)
            && report.vocal_stem_sha256.eq_ignore_ascii_case(&stem.sha256)
            && hash_file(&stem.path)?.eq_ignore_ascii_case(&stem.sha256),
        "Vocal report source/stem changed"
    );
    ensure!(
        report.timing.as_ref() == Some(timing),
        "Vocal report tempo/offset changed"
    );
    report.policy.validate()?;
    for pitch in &report.pitch_candidates {
        ensure!(
            pitch.start_seconds.is_finite()
                && pitch.end_seconds.is_finite()
                && pitch.start_seconds >= 0.
                && pitch.end_seconds > pitch.start_seconds
                && pitch.end_seconds <= duration
                && pitch.midi <= 127
                && pitch.clarity.is_finite()
                && (0.0..=1.0).contains(&pitch.clarity),
            "Invalid vocal candidate span"
        );
    }
    match (&report.chart, report.status) {
        (Some(chart), Status::ReviewRequired) => chart.validate()?,
        (None, Status::NeedsLyrics) => ensure!(
            !report.pitch_candidates.is_empty(),
            "NeedsLyrics requires measured pitch candidates"
        ),
        (None, Status::NoVoicedPitch) => ensure!(
            report.pitch_candidates.is_empty(),
            "NoVoicedPitch contradicts voiced candidates"
        ),
        _ => anyhow::bail!("Vocal status contradicts chart presence"),
    }
    Ok(report)
}

fn validate_instrument_outcome(
    run: &Value,
    result: &Value,
    generated: &Path,
    mix: &Value,
) -> Result<()> {
    let instrument = result["instrument"]
        .as_str()
        .context("Missing instrument identity")?;
    let Some(outcomes) = run.get("instrument_outcomes") else {
        // Version-one all-success receipts predate per-instrument outcomes.
        ensure!(run["success"] == true, "Generation failed");
        return Ok(());
    };
    let outcomes = outcomes.as_array().context("Invalid instrument outcomes")?;
    let matching: Vec<_> = outcomes
        .iter()
        .filter(|o| o["instrument"] == instrument)
        .collect();
    ensure!(
        matching.len() == 1,
        "Missing or duplicate instrument outcome"
    );
    let outcome = matching[0];
    ensure!(
        outcome["success"] == true,
        "{}",
        outcome["error"]
            .as_str()
            .unwrap_or("Instrument generation failed")
    );
    let stages = run["stages"].as_array().context("Missing stage outcomes")?;
    let required: &[&str] = if instrument == "vocals" {
        &["vocals"]
    } else {
        &["transcription", "mapping"]
    };
    for name in required {
        let matching: Vec<_> = stages.iter().filter(|s| s["name"] == *name).collect();
        ensure!(
            matching.len() == 1 && matching[0]["success"] == true,
            "Required {name} stage did not succeed"
        );
    }
    if outcome["abstained"] == true {
        ensure!(
            instrument == "vocals",
            "Unsupported instrument abstention declaration"
        );
        let evidence = &outcome["abstention"];
        let report = generated
            .parent()
            .context("Missing artifact parent")?
            .join("vocals/generation.json");
        let reported = evidence["report"]
            .as_str()
            .context("Missing abstention report identity")?;
        ensure!(
            fs::canonicalize(reported)? == fs::canonicalize(&report)?
                && evidence["report_sha256"] == hash_file(&report)?,
            "Abstention report changed"
        );
        let stage = stages.iter().find(|s| s["name"] == "vocals").unwrap();
        let args = stage["arguments"]
            .as_array()
            .context("Missing vocal stage arguments")?;
        let manifest_arg = args
            .windows(2)
            .find(|a| a[0] == "--stem-manifest")
            .and_then(|a| a[1].as_str())
            .context("Missing vocal input manifest")?;
        let manifest = Path::new(manifest_arg);
        ensure!(
            evidence["manifest_sha256"] == hash_file(manifest)?,
            "Abstention stem manifest changed"
        );
        ensure!(run["fixed_bpm"] == 120, "Unknown abstention tempo");
        let typed = validate_vocal_generation(
            &report,
            manifest,
            run["source_sha256"]
                .as_str()
                .context("Missing source hash")?,
            mix["duration_seconds"]
                .as_f64()
                .context("Missing source duration")?,
            &TempoMap::constant(120.)?,
        )?;
        ensure!(
            typed.chart.is_none() && evidence["status"] == serde_json::to_value(typed.status)?,
            "Abstention report contains a chart or changed status"
        );
        ensure!(
            !generated
                .parent()
                .unwrap()
                .join("vocals/vocals.json")
                .exists(),
            "Abstention contradicts a vocal export"
        );
        let attacks = result["report"]["attacks"]
            .as_array()
            .context("Abstention MIDI did not parse")?;
        ensure!(
            !attacks.is_empty() && attacks.iter().all(|a| a["generated_attacks"] == 0),
            "Abstention contradicts generated vocal notes"
        );
    }
    Ok(())
}
pub fn evaluate(batch: &Path, runs: &Path, output: &Path) -> Result<()> {
    ensure!(!output.exists(), "Refusing existing evaluation directory");
    let receipt: Value = serde_json::from_slice(&fs::read(runs.join("batch-run.json"))?)?;
    ensure!(receipt["complete"] == true, "Inference batch is incomplete");
    ensure!(
        receipt["frozen_batch_sha256"] == hash_file(&batch.join("frozen-batch.json"))?,
        "Frozen batch changed"
    );
    let count = receipt["expected_cases"]
        .as_u64()
        .context("missing case count")?;
    ensure!((1..=6).contains(&count), "Invalid case count");
    let frozen: Value = serde_json::from_slice(&fs::read(batch.join("frozen-batch.json"))?)?;
    fs::create_dir(output)?;
    let mut cases = Vec::new();
    for i in 1..=count {
        let label = format!("heldout-{i:02}");
        let case = batch.join(&label);
        let generated = runs.join(&label).join("generated.mid");
        let path = output.join(format!("{label}.json"));
        crate::compare(&case, &generated, &path)?;
        let mut report: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let run: Result<Value> = fs::read(runs.join(&label).join("run.json"))
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?));
        let provenance = (|| -> Result<()> {
            let run = run.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
            let mix: Value = serde_json::from_slice(&fs::read(case.join("source.mix.json"))?)?;
            crate::validate_frozen_case(
                &frozen,
                (i - 1) as usize,
                mix["source_archive_sha256"]
                    .as_str()
                    .context("Reference archive identity missing")?,
            )?;
            ensure!(
                (run["success"] == true || run["instrument_outcomes"].is_array())
                    && run["engine_unchanged_after_run"] == true,
                "Generation failed or executable changed"
            );
            ensure!(
                run["source_sha256"] == mix["output_sha256"]
                    && run["source_archive_sha256"] == mix["source_archive_sha256"],
                "Generation/reference provenance mismatch"
            );
            let artifact = run["artifact"]
                .as_str()
                .context("Missing generated artifact identity")?;
            ensure!(
                fs::canonicalize(artifact)? == fs::canonicalize(&generated)?,
                "Wrong generated artifact"
            );
            ensure!(
                run["artifact_sha256"] == hash_file(&generated)?,
                "Generated artifact changed after inference"
            );
            Ok(())
        })();
        if let Err(error) = provenance {
            for result in report["results"].as_array_mut().unwrap() {
                *result = json!({"instrument":result["instrument"],"success":false,"error":format!("Generation provenance invalid: {error:#}")});
            }
        } else {
            let run = run.as_ref().unwrap();
            let mix: Value = serde_json::from_slice(&fs::read(case.join("source.mix.json"))?)?;
            for result in report["results"].as_array_mut().unwrap() {
                if let Err(error) = validate_instrument_outcome(run, result, &generated, &mix) {
                    *result = json!({"instrument":result["instrument"],"success":false,
                        "error":format!("Instrument generation failed: {error:#}")});
                }
            }
        }
        report["case"] = json!(label);
        fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
        cases.push(report);
    }
    let mut instruments = Vec::new();
    for instrument in ["guitar", "bass", "drums", "pro_drums", "vocals"] {
        let mut reports = Vec::new();
        let mut failures = Vec::new();
        for case in &cases {
            let result = case["results"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["instrument"] == instrument)
                .context("Missing instrument outcome")?;
            if result["success"] == true {
                reports.push(&result["report"]);
            } else {
                failures.push(json!({"case":case["case"],"error":result["error"]}));
            }
        }
        let pro: Vec<_> = reports
            .iter()
            .map(|r| &r["pro_drum_hits"])
            .filter(|r| !r.is_null())
            .collect();
        let mut entry = json!({"instrument":instrument,"expected_cases":count,"scored_cases":reports.len(),"failures":failures,
            "attacks":metrics(&reports),"pro_lane_articulation_hits":metrics(&pro)});
        if instrument == "vocals" {
            let fields = [
                "reference_pitched_seconds",
                "generated_pitched_seconds",
                "overlapping_pitched_seconds",
                "same_pitch_seconds",
                "same_pitch_class_seconds",
                "unambiguous_compared_seconds",
            ];
            let mut pitch = interval_metrics(&reports, "vocal_pitch", &fields);
            let r = pitch["reference_pitched_seconds"].as_f64().unwrap();
            let g = pitch["generated_pitched_seconds"].as_f64().unwrap();
            let exact = pitch["same_pitch_seconds"].as_f64().unwrap();
            let folded = pitch["same_pitch_class_seconds"].as_f64().unwrap();
            pitch["exact_precision"] = json!(ratio(exact, g));
            pitch["exact_recall"] = json!(ratio(exact, r));
            pitch["exact_f1"] = json!(ratio(2. * exact, g + r));
            pitch["octave_folded_precision"] = json!(ratio(folded, g));
            pitch["octave_folded_recall"] = json!(ratio(folded, r));
            entry["vocal_pitch"] = pitch;
        }
        if instrument == "guitar" {
            let mut sustain = interval_metrics(
                &reports,
                "guitar_sustain",
                &[
                    "reference_sustained_seconds",
                    "generated_sustained_seconds",
                    "overlapping_seconds",
                ],
            );
            let o = sustain["overlapping_seconds"].as_f64().unwrap();
            sustain["precision"] = json!(ratio(
                o,
                sustain["generated_sustained_seconds"].as_f64().unwrap()
            ));
            sustain["recall"] = json!(ratio(
                o,
                sustain["reference_sustained_seconds"].as_f64().unwrap()
            ));
            entry["guitar_sustain"] = sustain;
        }
        instruments.push(entry);
    }
    let summary = json!({"version":1,"condition":receipt["condition"],"frozen_batch_sha256":receipt["frozen_batch_sha256"],
        "batch_run_receipt_sha256":hash_file(&runs.join("batch-run.json"))?,"scope":"Expert and Pro only",
        "instruments":instruments,"limitations":["Successful-case micro metrics pool counts or seconds; macro metrics average defined per-case ratios. Explicit failures are never silently replaced or treated as scored successes.","No attack or fret-color score establishes acoustic pitch truth, enjoyable play, or editing-time savings. Vocal exact recall and precision use all reference/generated pitched time, not overlap alone.","This aggregate contains no recordings, lyrics, reference chart notes, song metadata or absolute paths. Private per-case diagnostics remain beside it."]});
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pooled_counts_do_not_average_ratios_or_turn_abstention_into_precision() {
        let a = json!({"attacks":[{"tolerance_ms":50,"matched":1,"reference_attacks":2,"generated_attacks":1,"precision":1.,"recall":0.5}]});
        let b = json!({"attacks":[{"tolerance_ms":50,"matched":0,"reference_attacks":8,"generated_attacks":0,"precision":null,"recall":0.}]});
        let result = metrics(&[&a, &b]);
        assert_eq!(result[0]["micro_precision"], 1.);
        assert_eq!(result[0]["micro_recall"], 0.1);
        assert_eq!(result[0]["macro_recall"], 0.25);
        assert_eq!(result[0]["macro_precision_defined_cases"], 1);
        let abstained = metrics(&[&b]);
        assert_eq!(abstained[0]["micro_recall"], 0.);
        assert_eq!(abstained[0]["micro_f1"], 0.);
        assert!(abstained[0]["micro_precision"].is_null());
    }
}
