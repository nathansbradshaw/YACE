//! Fixed-scope native ablation. No target notes or authored tempo enter inference.
use crate::evaluate::{interval_metrics, metrics, ratio};
use anyhow::{Context, Result, ensure};
use chart_engine::{
    benchmark::{self, Instrument},
    chart::Chart,
    mapping::Transcription,
    models::hash_file,
    stem_review::{StemId, StemManifest},
    timing::TempoMap,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::{Command, Stdio},
    time::Instant,
};

const INSTRUMENTS: [(&str, Instrument); 4] = [
    ("guitar", Instrument::Guitar),
    ("bass", Instrument::Bass),
    ("drums", Instrument::Drums),
    ("pro_drums", Instrument::ProDrums),
];
const SCOPE: &str = "Expert Guitar/Bass/Drums and native Pro Drums; six direct-stem oracle cases; no vocals, keyboards, lower difficulties or full-mix inference";
pub(crate) fn save(path: &Path, value: &Value) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}
pub(crate) fn load(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub(crate) fn model_hashes(models: &Path) -> Result<Value> {
    Ok(
        json!({"basic_pitch.onnx":hash_file(&models.join("basic_pitch.onnx"))?,
        "adtof_frame_rnn.onnx":hash_file(&models.join("adtof_frame_rnn.onnx"))?}),
    )
}
pub(crate) fn aggregate(reports: &[&Value]) -> Value {
    let pro: Vec<_> = reports
        .iter()
        .map(|r| &r["pro_drum_hits"])
        .filter(|r| !r.is_null())
        .collect();
    let mut classes: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for report in &pro {
        if let Some(entries) = report["lane_articulation_classes"].as_array() {
            for entry in entries {
                if let Some(name) = entry["class"].as_str() {
                    classes.entry(name).or_default().push(entry);
                }
            }
        }
    }
    let classes: Vec<_> = classes
        .into_iter()
        .map(|(name, reports)| json!({"class":name,"attacks":metrics(&reports)}))
        .collect();
    let mut sustain = interval_metrics(
        reports,
        "guitar_sustain",
        &[
            "reference_sustained_seconds",
            "generated_sustained_seconds",
            "overlapping_seconds",
        ],
    );
    let overlap = sustain["overlapping_seconds"].as_f64().unwrap();
    sustain["precision"] = json!(ratio(
        overlap,
        sustain["generated_sustained_seconds"].as_f64().unwrap()
    ));
    sustain["recall"] = json!(ratio(
        overlap,
        sustain["reference_sustained_seconds"].as_f64().unwrap()
    ));
    json!({"attacks":metrics(reports),"pro_lane_articulation_hits":metrics(&pro),
        "pro_lane_articulation_classes":classes,"guitar_sustained_occupancy":sustain})
}
pub(crate) fn mapped_delta(before: &Chart, after: &Chart, name: &str) -> Result<Value> {
    let playable = |note: &chart_engine::chart::Note| {
        note.lane <= 4 || (name != "ExpertDrums" && note.lane == 7)
    };
    let before: Vec<_> = before.notes(name)?.into_iter().filter(playable).collect();
    let after: Vec<_> = after.notes(name)?.into_iter().filter(playable).collect();
    let heads = |notes: &[chart_engine::chart::Note]| {
        notes
            .iter()
            .map(|n| (n.tick, n.lane))
            .collect::<BTreeSet<_>>()
    };
    let ticks =
        |notes: &[chart_engine::chart::Note]| notes.iter().map(|n| n.tick).collect::<BTreeSet<_>>();
    Ok(
        json!({"baseline_gems":before.len(),"revised_gems":after.len(),
        "baseline_attacks":ticks(&before).len(),"revised_attacks":ticks(&after).len(),
        "added_heads":heads(&after).difference(&heads(&before)).count(),
        "removed_heads":heads(&before).difference(&heads(&after)).count(),
        "added_attack_ticks":ticks(&after).difference(&ticks(&before)).count(),
        "removed_attack_ticks":ticks(&before).difference(&ticks(&after)).count()}),
    )
}
pub(crate) fn bass_unchanged(
    before: &Transcription,
    after: &Transcription,
    a: &Chart,
    b: &Chart,
) -> Result<bool> {
    let bass_fields = |events: &Transcription| -> Result<Value> {
        let value = serde_json::to_value(events)?;
        Ok(Value::Object(
            value
                .as_object()
                .unwrap()
                .iter()
                .filter(|(key, _)| key.as_str() == "bass" || key.starts_with("bass_"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ))
    };
    Ok(bass_fields(before)? == bass_fields(after)?
        && a.timing == b.timing
        && a.notes("ExpertDoubleBass")? == b.notes("ExpertDoubleBass")?)
}

/// Preserves every predeclared case and failure. Existing outputs are refused.
pub fn batch(audit: &Path, prepared: &Path, output: &Path, models: &Path) -> Result<()> {
    ensure!(!output.exists(), "Refusing existing recovery-batch output");
    let audit = fs::canonicalize(audit)?;
    let prepared = fs::canonicalize(prepared)?;
    let models = fs::canonicalize(models)?;
    let frozen = prepared.join("frozen-batch.json");
    let declaration = load(&frozen)?;
    ensure!(
        declaration["songs"]
            .as_array()
            .is_some_and(|s| s.len() == 6),
        "Exactly six frozen cases required"
    );
    let frozen_hash = hash_file(&frozen)?;
    let audit_hash = hash_file(&audit)?;
    let models_hash = model_hashes(&models)?;
    let plan = prepared
        .parent()
        .unwrap()
        .join("recovery-followup-plan.json");
    let plan_hash = if plan.exists() {
        let plan_value = load(&plan)?;
        ensure!(
            plan_value["selection_sha256"] == frozen_hash,
            "Recovery plan selects another batch"
        );
        Some(hash_file(&plan)?)
    } else {
        None
    };
    fs::create_dir(output)?;
    let output = fs::canonicalize(output)?;
    let runner_hash = hash_file(&std::env::current_exe()?)?;
    save(
        &output.join("freeze.json"),
        &json!({"version":1,"scope":SCOPE,"condition":"oracle same-inference recovery ablation",
        "frozen_batch_sha256":frozen_hash,"plan_sha256":plan_hash,"audit_binary_sha256":audit_hash,
        "benchmark_binary_sha256":runner_hash,"models":models_hash,"fixed_bpm":120,"guitar_source":"guitar",
        "expected_cases":6,"targets_enter_inference":false,"executable_models_and_selection_recorded_before_inference":true}),
    )?;
    let mut cases = Vec::new();
    let total_start = Instant::now();
    for index in 1..=6 {
        let label = format!("heldout-{index:02}");
        let case_path = prepared.join(&label);
        let case_output = output.join(&label);
        fs::create_dir(&case_output)?;
        let start = Instant::now();
        let mut stage = "preparation";
        let mut timings = json!({});
        let result = (|| -> Result<Value> {
            let inputs = case_output.join("inputs");
            let prep_start = Instant::now();
            let prepared_result = crate::prepare(&case_path, &inputs);
            timings["preparation_seconds"] = json!(prep_start.elapsed().as_secs_f64());
            prepared_result?;
            let preparation = load(&inputs.join("preparation.json"))?;
            let expected_archive = declaration["songs"][index - 1]["archive_sha256"]
                .as_str()
                .context("Frozen archive identity missing")?;
            ensure!(
                preparation["source_archive_sha256"] == expected_archive,
                "Prepared case differs from frozen archive selection"
            );
            let manifest_path = inputs.join("stem-manifest.json");
            let manifest = StemManifest::read(&manifest_path)?;
            ensure!(
                [StemId::Bass, StemId::Guitar, StemId::Drums, StemId::Vocals]
                    .iter()
                    .all(|id| manifest.ids().contains(id)),
                "Required direct oracle role unavailable"
            );
            let source = case_path.join("source.wav");
            let source_hash = hash_file(&source)?;
            let manifest_hash = hash_file(&manifest_path)?;
            stage = "native_audit";
            ensure!(
                hash_file(&audit)? == audit_hash && model_hashes(&models)? == models_hash,
                "Audit/model identity changed before inference"
            );
            let destination = case_output.join("audit");
            let args = vec![
                "--audio".into(),
                source.to_string_lossy().into_owned(),
                "--manifest".into(),
                manifest_path.to_string_lossy().into_owned(),
                "--model-dir".into(),
                models.to_string_lossy().into_owned(),
                "--bpm".into(),
                "120".into(),
                "--title".into(),
                "Blind recovery ablation".into(),
                "--artist".into(),
                "YACE benchmark".into(),
                "--guitar-source".into(),
                "guitar".into(),
                "--output".into(),
                destination.to_string_lossy().into_owned(),
            ];
            let log = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(case_output.join("audit.log"))?;
            let inference_start = Instant::now();
            let status = Command::new(&audit)
                .args(&args)
                .stdout(Stdio::from(log.try_clone()?))
                .stderr(Stdio::from(log))
                .status();
            timings["inference_seconds"] = json!(inference_start.elapsed().as_secs_f64());
            save(
                &case_output.join("invocation.json"),
                &json!({"arguments":args,"success":status.as_ref().is_ok_and(|s|s.success()),
                "exit_code":status.as_ref().ok().and_then(|s|s.code()),"audit_binary_sha256":audit_hash,"models":models_hash,
                "source_sha256":source_hash,"manifest_sha256":manifest_hash}),
            )?;
            ensure!(status?.success(), "Native audit failed; retained log");
            stage = "artifact_validation";
            ensure!(
                hash_file(&audit)? == audit_hash
                    && model_hashes(&models)? == models_hash
                    && hash_file(&source)? == source_hash
                    && hash_file(&manifest_path)? == manifest_hash
                    && hash_file(&frozen)? == frozen_hash,
                "Frozen inference inputs changed"
            );
            let comparison = load(&destination.join("comparison.json"))?;
            ensure!(
                comparison["version"] == 1
                    && comparison["audio_sha256"] == source_hash
                    && comparison["stem_manifest_sha256"] == manifest_hash,
                "Audit source provenance mismatch"
            );
            let before: Transcription =
                serde_json::from_slice(&fs::read(destination.join("baseline-events.json"))?)?;
            let after: Transcription =
                serde_json::from_slice(&fs::read(destination.join("revised-events.json"))?)?;
            before.validate()?;
            after.validate()?;
            let a_path = destination.join("baseline/notes.chart");
            let b_path = destination.join("revised/notes.chart");
            let a = Chart::parse(fs::read_to_string(&a_path)?)?;
            let b = Chart::parse(fs::read_to_string(&b_path)?)?;
            ensure!(
                a.timing == TempoMap::constant(120.)? && b.timing == a.timing,
                "Unexpected generated tempo or offset"
            );
            ensure!(
                bass_unchanged(&before, &after, &a, &b)?,
                "Bass changed during recovery ablation"
            );
            let recovery = json!({"raw_guitar_short_hypotheses":after.guitar_short_candidates.len(),
                "retained_raw_guitar_short_candidates":after.guitar_short_candidates.iter().filter(|r|r.retained).count(),
                "raw_recovered_drum_hits":after.drum_recovered_hits.len(),"guitar":mapped_delta(&a,&b,"ExpertSingle")?,
                "drums":mapped_delta(&a,&b,"ExpertDrums")?,"bass_unchanged":true});
            stage = "comparison";
            let compare_start = Instant::now();
            let reference = crate::load_case(&case_path)?;
            let reference_hash = hash_file(&reference.notes)?;
            let mut results = Vec::new();
            for (instrument, kind) in INSTRUMENTS {
                let mut entry = json!({"instrument":instrument});
                for (variant, path) in [("baseline", &a_path), ("revised", &b_path)] {
                    let measured = benchmark::compare(benchmark::Request {
                        reference_midi: &reference.notes,
                        generated_midi: path,
                        reference_audio: &source,
                        generated_audio: &source,
                        reference_delay_ms: reference.delay,
                        generated_delay_ms: 0.,
                        instrument: kind,
                    });
                    let measured = measured.and_then(|report| {
                        ensure!(
                            report.recording_sha256 == source_hash
                                && report.reference_notes_sha256 == reference_hash,
                            "Comparison source/reference identity changed"
                        );
                        Ok(report)
                    });
                    entry[variant] = match measured {
                        Ok(report) => json!({"success":true,"report":report}),
                        Err(error) => {
                            json!({"success":false,"error":format!("{error:#}"),"failure_code":"comparison_failed"})
                        }
                    };
                }
                results.push(entry);
            }
            ensure!(
                hash_file(&source)? == source_hash
                    && hash_file(&reference.notes)? == reference_hash,
                "Source or reference changed during comparison"
            );
            timings["comparison_seconds"] = json!(compare_start.elapsed().as_secs_f64());
            let complete = results
                .iter()
                .all(|r| r["baseline"]["success"] == true && r["revised"]["success"] == true);
            Ok(
                json!({"case":label,"success":complete,"execution_success":true,
                "failure_stage":if complete {None}else{Some("comparison")},
                "source_sha256":source_hash,"source_archive_sha256":reference.mix.source_archive_sha256,
                "baseline_events_sha256":hash_file(&destination.join("baseline-events.json"))?,"revised_events_sha256":hash_file(&destination.join("revised-events.json"))?,
                "baseline_chart_sha256":hash_file(&a_path)?,"revised_chart_sha256":hash_file(&b_path)?,"recovery":recovery,"results":results}),
            )
        })();
        let mut private = match result {
            Ok(value) => value,
            Err(error) => {
                json!({"case":label,"success":false,"failure_stage":stage,"error":format!("{error:#}")})
            }
        };
        private["timings"] = timings;
        private["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
        save(&case_output.join("case.json"), &private)?;
        cases.push(private);
    }
    let mut summary = summarize(
        &cases,
        &frozen_hash,
        &audit_hash,
        &models_hash,
        total_start.elapsed().as_secs_f64(),
    );
    summary["freeze_receipt_sha256"] = json!(hash_file(&output.join("freeze.json"))?);
    save(&output.join("summary.json"), &summary)?;
    ensure!(
        summary["all_cases_successful"] == true,
        "Recovery batch retained one or more failures; see private receipts"
    );
    Ok(())
}

fn summarize(
    cases: &[Value],
    frozen_hash: &str,
    audit_hash: &str,
    models: &Value,
    elapsed: f64,
) -> Value {
    let mut instruments = Vec::new();
    let mut public_cases = Vec::new();
    let mut all_success = true;
    for case in cases {
        let mut entry = json!({"case":case["case"],"success":case["success"],"timings":case["timings"],
            "elapsed_seconds":case["elapsed_seconds"],"source_sha256":case["source_sha256"],
            "source_archive_sha256":case["source_archive_sha256"],"recovery":case["recovery"]});
        let mut outcomes = Vec::new();
        for (instrument, _) in INSTRUMENTS {
            let result = case["results"]
                .as_array()
                .and_then(|r| r.iter().find(|r| r["instrument"] == instrument));
            let mut row = json!({"instrument":instrument});
            for variant in ["baseline", "revised"] {
                let outcome = result.map(|r| &r[variant]);
                row[variant] = if let Some(value) = outcome.filter(|r| r["success"] == true) {
                    json!({"success":true,"metrics":aggregate(&[&value["report"]])})
                } else {
                    all_success = false;
                    json!({"success":false,"failure_code":if case["success"]==false{case["failure_stage"].as_str().unwrap_or("case_failed")}else{"comparison_failed"}})
                };
            }
            outcomes.push(row);
        }
        entry["results"] = json!(outcomes);
        public_cases.push(entry);
    }
    for (instrument, _) in INSTRUMENTS {
        let mut entry = json!({"instrument":instrument,"expected_cases":6});
        for variant in ["baseline", "revised"] {
            let reports: Vec<_> = cases
                .iter()
                .filter_map(|c| c["results"].as_array())
                .flatten()
                .filter(|r| r["instrument"] == instrument && r[variant]["success"] == true)
                .map(|r| &r[variant]["report"])
                .collect();
            entry[variant] = json!({"scored_cases":reports.len(),"failed_cases":6-reports.len(),"metrics":aggregate(&reports)});
        }
        let paired: Vec<_> = cases
            .iter()
            .filter_map(|c| c["results"].as_array())
            .flatten()
            .filter(|r| {
                r["instrument"] == instrument
                    && r["baseline"]["success"] == true
                    && r["revised"]["success"] == true
            })
            .collect();
        entry["paired_cases"] = json!(paired.len());
        for variant in ["baseline", "revised"] {
            let reports: Vec<_> = paired.iter().map(|r| &r[variant]["report"]).collect();
            entry["paired_metrics"][variant] = aggregate(&reports);
        }
        instruments.push(entry);
    }
    json!({"version":1,"scope":SCOPE,"condition":"oracle same-inference recovery ablation","expected_cases":6,"complete":cases.len()==6,
        "all_cases_successful":all_success,"frozen_batch_sha256":frozen_hash,"audit_binary_sha256":audit_hash,"models":models,
        "elapsed_seconds":elapsed,"cases":public_cases,"instruments":instruments,
        "limitations":["The baseline disables two recovery passes on identical native inference outputs; it is not the historical binary.",
        "Raw recovered candidates are not necessarily new played heads or attacks; overlap and mapping are reported separately.",
        "All predefined cases and tolerances remain present. Failures are not silently scored as successes or substituted.",
        "Oracle stem and chart representation agreement is not acoustic truth, listening/controller acceptance, isolation quality, enjoyment or editing-time savings."]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_pool_excludes_one_sided_failures_but_retains_their_outcomes() {
        let report = |matched: u64, reference: u64, generated: u64| {
            json!({"attacks":[{"tolerance_ms":50,
            "matched":matched,"reference_attacks":reference,"generated_attacks":generated,
            "precision":ratio(matched as f64,generated as f64),"recall":ratio(matched as f64,reference as f64)}]})
        };
        let success = |value: Value| json!({"success":true,"report":value});
        let cases = vec![
            json!({"case":"heldout-01","success":true,"results":[{"instrument":"guitar",
                "baseline":success(report(1,2,1)),"revised":success(report(2,2,2))}]}),
            json!({"case":"heldout-02","success":true,"results":[{"instrument":"guitar",
                "baseline":{"success":false,"error":"PRIVATE /reference/path"},"revised":success(report(8,8,8))}]}),
        ];
        let value = summarize(&cases, "frozen", "audit", &json!({}), 0.);
        let guitar = &value["instruments"][0];
        assert_eq!(guitar["baseline"]["scored_cases"], 1);
        assert_eq!(guitar["revised"]["scored_cases"], 2);
        assert_eq!(guitar["revised"]["metrics"]["attacks"][0]["matched"], 10);
        assert_eq!(guitar["paired_cases"], 1);
        assert_eq!(
            guitar["paired_metrics"]["revised"]["attacks"][0]["matched"],
            2
        );
        assert_eq!(value["all_cases_successful"], false);
        assert!(!serde_json::to_string(&value).unwrap().contains("PRIVATE"));
    }
}
