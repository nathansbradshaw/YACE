//! Aggregate predeclared diagnostic counts; no detection, matching or tuning.
use anyhow::{Context, Result, ensure};
use chart_engine::models;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const TOLERANCES: [u32; 3] = [25, 50, 100];
const FAMILIES: [&str; 5] = ["kick", "snare", "hihat", "toms", "cymbals"];
#[derive(Parser)]
struct Args {
    #[arg(long)]
    summary: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    markdown: Option<PathBuf>,
    /// Optional private exposure-history update; all three options are required.
    #[arg(long, requires_all=["selection","exposure_output"])]
    history: Option<PathBuf>,
    #[arg(long, requires_all=["history","exposure_output"])]
    selection: Option<PathBuf>,
    #[arg(long, requires_all=["history","selection"])]
    exposure_output: Option<PathBuf>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Counts {
    reference_attacks: u64,
    detected_attacks: u64,
    matched: u64,
}
impl Counts {
    fn add(&mut self, other: &Self) -> Result<()> {
        self.reference_attacks = self
            .reference_attacks
            .checked_add(other.reference_attacks)
            .context("Count overflow")?;
        self.detected_attacks = self
            .detected_attacks
            .checked_add(other.detected_attacks)
            .context("Count overflow")?;
        self.matched = self
            .matched
            .checked_add(other.matched)
            .context("Count overflow")?;
        Ok(())
    }
    fn values(&self, tolerance: u32) -> Value {
        let denominator = self.reference_attacks as f64 + self.detected_attacks as f64;
        json!({"tolerance_ms":tolerance,"reference_attacks":self.reference_attacks,"detected_attacks":self.detected_attacks,"matched":self.matched,
            "micro_precision":(self.detected_attacks>0).then(||self.matched as f64/self.detected_attacks as f64),
            "micro_recall":(self.reference_attacks>0).then(||self.matched as f64/self.reference_attacks as f64),
            "micro_f1":(denominator>0.).then(||2.*self.matched as f64/denominator)})
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Complementary {
    overlap: u64,
    independent_only: u64,
    independent_only_reference_hits: u64,
    independent_only_extras: u64,
    newly_covered_reference_attacks: u64,
}
impl Complementary {
    fn add(&mut self, other: &Self) -> Result<()> {
        macro_rules! add {
            ($field:ident) => {
                self.$field = self
                    .$field
                    .checked_add(other.$field)
                    .context("Complementary count overflow")?;
            };
        }
        add!(overlap);
        add!(independent_only);
        add!(independent_only_reference_hits);
        add!(independent_only_extras);
        add!(newly_covered_reference_attacks);
        Ok(())
    }
    fn value(&self, tolerance: u32) -> Value {
        let mut value = serde_json::to_value(self).expect("integer fields serialize");
        value["tolerance_ms"] = json!(tolerance);
        value
    }
}
#[derive(Default)]
struct Pool {
    scored_cases: usize,
    retained: [Counts; 3],
    independent: [Counts; 3],
    complementary: [Complementary; 3],
}
struct Comparison {
    retained: [Counts; 3],
    independent: [Counts; 3],
    complementary: [Complementary; 3],
}
fn rows(value: &Value) -> Result<[Counts; 3]> {
    let array = value.as_array().context("Metric rows missing")?;
    ensure!(array.len() == 3, "Expected three tolerances");
    let mut result = std::array::from_fn(|_| Counts::default());
    for (index, tolerance) in TOLERANCES.iter().enumerate() {
        let matches: Vec<_> = array
            .iter()
            .filter(|v| v["tolerance_ms"] == *tolerance)
            .collect();
        ensure!(matches.len() == 1, "Tolerance missing or duplicated");
        result[index] = serde_json::from_value(matches[0].clone())?;
        ensure!(
            result[index].matched <= result[index].reference_attacks
                && result[index].matched <= result[index].detected_attacks,
            "Impossible match count"
        );
    }
    Ok(result)
}
fn comparison(value: &Value) -> Result<Comparison> {
    let retained = rows(&value["retained"])?;
    let independent = rows(&value["independent"])?;
    for index in 0..3 {
        ensure!(
            retained[index].reference_attacks == independent[index].reference_attacks
                && retained[index].reference_attacks == retained[0].reference_attacks
                && retained[index].detected_attacks == retained[0].detected_attacks
                && independent[index].detected_attacks == independent[0].detected_attacks,
            "Metric tolerances do not describe the same attack lists"
        );
    }
    let agreement = value["agreement"]
        .as_array()
        .context("Complementary rows missing")?;
    ensure!(
        agreement.len() == 3,
        "Expected three complementary tolerances"
    );
    let mut complementary = std::array::from_fn(|_| Complementary::default());
    for (index, tolerance) in TOLERANCES.iter().enumerate() {
        let matches: Vec<_> = agreement
            .iter()
            .filter(|v| v["tolerance_ms"] == *tolerance)
            .collect();
        ensure!(
            matches.len() == 1,
            "Complementary tolerance missing/duplicated"
        );
        let counts: Complementary = serde_json::from_value(matches[0].clone())?;
        ensure!(
            counts.overlap.checked_add(counts.independent_only)
                == Some(independent[index].detected_attacks)
                && counts
                    .independent_only_reference_hits
                    .checked_add(counts.independent_only_extras)
                    == Some(counts.independent_only)
                && counts.newly_covered_reference_attacks <= independent[index].matched
                && counts.newly_covered_reference_attacks
                    <= independent[index]
                        .reference_attacks
                        .saturating_sub(retained[index].matched)
                && counts.overlap <= retained[index].detected_attacks
                && counts.independent_only_reference_hits <= independent[index].matched,
            "Inconsistent complementary counts"
        );
        complementary[index] = counts;
    }
    Ok(Comparison {
        retained,
        independent,
        complementary,
    })
}
impl Pool {
    fn add(&mut self, comparison: &Comparison) -> Result<()> {
        for index in 0..3 {
            self.retained[index].add(&comparison.retained[index])?;
            self.independent[index].add(&comparison.independent[index])?;
            self.complementary[index].add(&comparison.complementary[index])?;
        }
        self.scored_cases += 1;
        Ok(())
    }
    fn value(&self) -> Value {
        json!({"scored_cases":self.scored_cases,"expected_cases":6,"unscored_cases":6usize.saturating_sub(self.scored_cases),
        "retained":TOLERANCES.iter().enumerate().map(|(i,t)|self.retained[i].values(*t)).collect::<Vec<_>>(),
        "independent":TOLERANCES.iter().enumerate().map(|(i,t)|self.independent[i].values(*t)).collect::<Vec<_>>(),
        "complementary":TOLERANCES.iter().enumerate().map(|(i,t)|self.complementary[i].value(*t)).collect::<Vec<_>>()})
    }
}
fn comparison_value(value: &Comparison) -> Value {
    json!({"retained":TOLERANCES.iter().enumerate().map(|(i,t)|value.retained[i].values(*t)).collect::<Vec<_>>(),
        "independent":TOLERANCES.iter().enumerate().map(|(i,t)|value.independent[i].values(*t)).collect::<Vec<_>>(),
        "complementary":TOLERANCES.iter().enumerate().map(|(i,t)|value.complementary[i].value(*t)).collect::<Vec<_>>()})
}
fn aggregate(input: &Value) -> Result<Value> {
    ensure!(
        input["version"] == 1
            && input["experiment"] == "independent-part-evidence-batch-v1"
            && input["expected_cases"] == 6,
        "Unexpected batch schema"
    );
    let cases = input["cases"].as_array().context("Batch cases missing")?;
    let mut guitar = Pool::default();
    let mut drums: BTreeMap<_, Pool> = FAMILIES.iter().map(|f| (*f, Pool::default())).collect();
    let mut output = vec![];
    for index in 0..cases.len().max(6) {
        let label = format!("heldout-{:02}", index + 1);
        let case = cases.get(index);
        let mut errors = Vec::<String>::new();
        let mut stages = BTreeMap::new();
        for stage in ["selection", "prepare", "generate", "split", "audit"] {
            let status = case
                .and_then(|c| c[stage]["status"].as_str())
                .filter(|s| ["success", "failed", "not_run"].contains(s))
                .unwrap_or("unknown");
            stages.insert(stage, status);
        }
        let mut row = json!({"label":label,"stages":stages,"guitar":null,"drums":FAMILIES.iter().map(|f|(*f,Value::Null)).collect::<BTreeMap<_,_>>()});
        if index >= 6 {
            errors.push("unexpected_case_outside_predeclared_six".into());
        } else if case.is_none() {
            errors.push("case_missing_from_summary".into());
        } else if case.is_some_and(|c| c["label"] != label) {
            errors.push("case_label_or_order_invalid".into());
        } else if ["selection", "prepare", "generate", "audit"]
            .iter()
            .any(|s| stages[s] != "success")
        {
            errors.push("required_stage_not_successful".into());
        } else {
            let stats = &case.context("Case missing")?["statistics"];
            if stats["version"] != 1 || stats["experiment"] != "independent-part-evidence-v1" {
                errors.push("statistics_missing_or_invalid".into());
            } else {
                match comparison(&stats["guitar"]["comparison"]) {
                    Ok(c) => {
                        guitar.add(&c)?;
                        row["guitar"] = comparison_value(&c);
                    }
                    Err(_) => errors.push("guitar_counts_invalid".into()),
                }
                if stats["drums"].is_null() {
                    errors.push("kit_comparison_unavailable_not_scored".into());
                } else if stages["split"] != "success" {
                    errors.push("kit_statistics_without_successful_split".into());
                } else {
                    for family in FAMILIES {
                        match comparison(&stats["drums"]["families"][family]) {
                            Ok(c) => {
                                drums.get_mut(family).context("Family missing")?.add(&c)?;
                                row["drums"][family] = comparison_value(&c);
                            }
                            Err(_) => errors.push(format!("{family}_counts_invalid")),
                        }
                    }
                }
            }
        }
        row["errors_or_unavailable"] = json!(errors);
        output.push(row);
    }
    Ok(
        json!({"version":1,"experiment":"independent-part-evidence-aggregate-v1","expected_cases":6,"actual_batch_rows":cases.len(),"cases":output,
        "guitar":guitar.value(),"drums":drums.into_iter().map(|(f,p)|(f,p.value())).collect::<BTreeMap<_,_>>(),
        "interpretation":"Diagnostic attack agreement only; not a generator chart improvement, calibrated confidence or musical acceptance.",
        "limits":["Micro metrics come from integer matched/reference/detected sums, never case precision averages.","Failed/unavailable cases remain explicit and unscored; successful empty outputs preserve reference denominators and zero recall.","Independent-only statistics use separate one-to-one assignments and are not a proposed union chart.","Five-family drum activity does not resolve ride/crash or high/mid/low toms. Guitar attacks do not establish pitch/fret/sustain quality or fun."]}),
    )
}
fn load(path: &Path) -> Result<Value> {
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 64 * 1024 * 1024, "Input exceeds budget");
    Ok(serde_json::from_slice(&bytes)?)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(())
}
fn exposure(history: &Value, selection: &Value) -> Result<Value> {
    let prior = history["excluded_artists"]
        .as_array()
        .context("Artist history missing")?;
    let mut names = BTreeMap::<String, String>::new();
    for name in prior {
        let name = name.as_str().context("Artist name is not text")?.trim();
        ensure!(
            !name.is_empty() && names.insert(name.to_lowercase(), name.into()).is_none(),
            "Invalid/duplicate prior artist"
        );
    }
    let prior_count = names.len();
    let songs = selection["songs"].as_array().context("Selection missing")?;
    ensure!(songs.len() == 6, "Expected six selected artists");
    let mut added = vec![];
    for song in songs {
        let artist = song["metadata"]["artist"]
            .as_str()
            .context("Selected artist missing")?
            .trim();
        ensure!(
            !artist.is_empty() && names.insert(artist.to_lowercase(), artist.into()).is_none(),
            "Selected artist duplicates an earlier exposure"
        );
        added.push(artist.to_owned());
    }
    Ok(
        json!({"version":1,"scope":"Future artist-separated validation","excluded_artists":names.into_values().collect::<Vec<_>>(),
        "exposure":{"previous_distinct_artists":prior_count,"new_distinct_artists":6,"total_distinct_artists":prior_count+6,"added_artists":added,"exclude_from_future_validation":true},
        "exclusions_basis":"All previously exposed artists plus every artist in this frozen six-case diagnostic batch, including failed or unavailable cases; originals preserved."}),
    )
}
fn markdown(report: &Value) -> String {
    let percent = |value: &Value| {
        value
            .as_f64()
            .map(|v| format!("{:.1}%", v * 100.))
            .unwrap_or_else(|| "—".into())
    };
    let mut text = format!(
        "# Independent part evidence\n\nDiagnostic only: no generated chart changed. Expected 6 cases; {} batch rows. Guitar scored {}/6.\n\n| Guitar tolerance | Retained P / R / F1 | Independent P / R / F1 | Newly covered attacks | Independent-only extras |\n|---|---|---|---|---|\n",
        report["actual_batch_rows"], report["guitar"]["scored_cases"]
    );
    for (i, t) in TOLERANCES.iter().enumerate() {
        let g = &report["guitar"];
        let r = &g["retained"][i];
        let d = &g["independent"][i];
        text.push_str(&format!(
            "| {t} ms | {} / {} / {} | {} / {} / {} | {} | {} |\n",
            percent(&r["micro_precision"]),
            percent(&r["micro_recall"]),
            percent(&r["micro_f1"]),
            percent(&d["micro_precision"]),
            percent(&d["micro_recall"]),
            percent(&d["micro_f1"]),
            g["complementary"][i]["newly_covered_reference_attacks"],
            g["complementary"][i]["independent_only_extras"]
        ));
    }
    text.push_str("\n| Drum family, 50 ms | Scored cases | ADTOF P / R / F1 | Independent P / R / F1 |\n|---|---|---|---|\n");
    for family in FAMILIES {
        let g = &report["drums"][family];
        let r = &g["retained"][1];
        let d = &g["independent"][1];
        text.push_str(&format!(
            "| {family} | {}/6 | {} / {} / {} | {} / {} / {} |\n",
            g["scored_cases"],
            percent(&r["micro_precision"]),
            percent(&r["micro_recall"]),
            percent(&r["micro_f1"]),
            percent(&d["micro_precision"]),
            percent(&d["micro_recall"]),
            percent(&d["micro_f1"])
        ));
    }
    text.push_str("\nCases with errors or unavailable evidence:\n");
    for case in report["cases"].as_array().into_iter().flatten() {
        if case["errors_or_unavailable"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
        {
            text.push_str(&format!(
                "- {}: {}\n",
                case["label"].as_str().unwrap_or("case"),
                case["errors_or_unavailable"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    text.push_str("\nMicro metrics use pooled counts. Failed cases remain unscored, not fabricated zeros. Full JSON retains all tolerances and per-case comparisons. Five families do not resolve Pro tom register or ride/crash. Extra attacks are listening prompts, not proven omitted notes.\n");
    text
}
fn main() -> Result<()> {
    let args = Args::parse();
    for path in [&args.output]
        .into_iter()
        .chain(args.markdown.as_ref())
        .chain(args.exposure_output.as_ref())
    {
        ensure!(!path.exists(), "Refusing existing report output");
    }
    let hash = models::hash_file(&args.summary)?;
    let mut report = aggregate(&load(&args.summary)?)?;
    report["batch_summary_sha256"] = json!(hash);
    let private_history = if let (Some(history), Some(selection)) = (&args.history, &args.selection)
    {
        let history_hash = models::hash_file(history)?;
        let selection_hash = models::hash_file(selection)?;
        let mut merged = exposure(&load(history)?, &load(selection)?)?;
        ensure!(
            models::hash_file(history)? == history_hash
                && models::hash_file(selection)? == selection_hash,
            "Exposure inputs changed"
        );
        merged["previous_history_sha256"] = json!(history_hash);
        merged["selection_sha256"] = json!(selection_hash);
        Some(merged)
    } else {
        None
    };
    ensure!(
        models::hash_file(&args.summary)? == hash,
        "Summary changed while aggregating"
    );
    write_new(&args.output, &serde_json::to_vec_pretty(&report)?)?;
    if let Some(path) = args.markdown {
        write_new(&path, markdown(&report).as_bytes())?;
    }
    if let (Some(path), Some(history)) = (args.exposure_output, private_history) {
        write_new(&path, &serde_json::to_vec_pretty(&history)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn group(reference: u64, retained: u64, rhit: u64, detected: u64, dhit: u64) -> Value {
        let rows = |count, hit| {
            TOLERANCES.iter().map(|t|json!({"tolerance_ms":t,"reference_attacks":reference,"detected_attacks":count,"matched":hit})).collect::<Vec<_>>()
        };
        json!({"retained":rows(retained,rhit),"independent":rows(detected,dhit),"agreement":TOLERANCES.iter().map(|t|json!({"tolerance_ms":t,"overlap":0,"independent_only":detected,"independent_only_reference_hits":dhit,"independent_only_extras":detected-dhit,"newly_covered_reference_attacks":dhit.saturating_sub(rhit)})).collect::<Vec<_>>()})
    }
    fn batch(groups: Vec<Value>) -> Value {
        json!({"version":1,"experiment":"independent-part-evidence-batch-v1","expected_cases":6,"cases":groups.into_iter().enumerate().map(|(i,g)|json!({"label":format!("heldout-{:02}",i+1),"selection":{"status":"success"},"prepare":{"status":"success"},"generate":{"status":"success"},"split":{"status":"failed"},"audit":{"status":"success"},"statistics":{"version":1,"experiment":"independent-part-evidence-v1","guitar":{"comparison":g},"drums":null}})).collect::<Vec<_>>()})
    }
    #[test]
    fn unequal_case_sizes_use_summed_denominators_and_keep_failures() {
        let input = batch(vec![group(100, 100, 50, 100, 90), group(1, 1, 1, 1, 0)]);
        let output = aggregate(&input).unwrap();
        let m = &output["guitar"]["independent"][1];
        assert_eq!(m["matched"], 90);
        assert_eq!(m["reference_attacks"], 101);
        assert_eq!(m["micro_recall"].as_f64().unwrap(), 90. / 101.);
        assert_eq!(output["guitar"]["scored_cases"], 2);
        assert_eq!(output["guitar"]["unscored_cases"], 4);
        assert_eq!(output["cases"].as_array().unwrap().len(), 6);
        assert_eq!(output["drums"]["kick"]["scored_cases"], 0);
        assert!(output["drums"]["kick"]["independent"][1]["micro_recall"].is_null());
        assert_eq!(
            output["guitar"]["complementary"][1]["independent_only_extras"],
            11
        );
    }
    #[test]
    fn successful_empty_outputs_keep_reference_recall_zero_and_bad_counts_do_not_score() {
        let mut input = batch(vec![group(5, 0, 0, 0, 0)]);
        let result = aggregate(&input).unwrap();
        let m = &result["guitar"]["independent"][1];
        assert_eq!(m["micro_recall"], 0.);
        assert_eq!(m["micro_f1"], 0.);
        assert!(m["micro_precision"].is_null());
        input["cases"][0]["statistics"]["guitar"]["comparison"]["independent"][0]["matched"] =
            json!(1);
        let result = aggregate(&input).unwrap();
        assert_eq!(result["guitar"]["scored_cases"], 0);
        assert_eq!(
            result["cases"][0]["errors_or_unavailable"][0],
            "guitar_counts_invalid"
        );
    }
    #[test]
    fn failed_audit_never_counts_saved_statistics_and_exposure_keeps_all_artists() {
        let mut input = batch(vec![group(10, 10, 10, 10, 10)]);
        input["cases"][0]["audit"]["status"] = json!("failed");
        assert_eq!(aggregate(&input).unwrap()["guitar"]["scored_cases"], 0);
        let selection = json!({"songs":(0..6).map(|i|json!({"metadata":{"artist":format!("New {i}")}})).collect::<Vec<_>>()});
        let history = json!({"excluded_artists":["Prior"]});
        let result = exposure(&history, &selection).unwrap();
        assert_eq!(result["exposure"]["total_distinct_artists"], 7);
        let mut duplicated = selection;
        duplicated["songs"][0]["metadata"]["artist"] = json!("PRIOR");
        assert!(exposure(&history, &duplicated).is_err());
    }
}
