//! Six frozen cases; diagnostic evidence only, never generator policy.
use anyhow::{Context, Result, ensure};
use chart_engine::{larsnet, models};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    prepared: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    preparer: PathBuf,
    #[arg(long)]
    generator: PathBuf,
    #[arg(long)]
    splitter: PathBuf,
    #[arg(long)]
    audit: PathBuf,
    #[arg(long)]
    models: PathBuf,
    #[arg(long)]
    kit_models: PathBuf,
}
struct Witnesses(Vec<(PathBuf, String)>, Vec<PathBuf>);
impl Witnesses {
    fn verify(&self) -> Result<()> {
        ensure!(
            self.1.iter().all(|p| !p.exists()),
            "Previously absent optional model appeared"
        );
        for (path, expected) in &self.0 {
            ensure!(
                models::hash_file(path)? == *expected,
                "Frozen executable/model/selection changed"
            );
        }
        Ok(())
    }
    fn add(&mut self, path: PathBuf) -> Result<String> {
        let hash = models::hash_file(&path)?;
        self.0.push((path, hash.clone()));
        Ok(hash)
    }
}
fn load(path: &Path) -> Result<Value> {
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 64 * 1024 * 1024, "Receipt exceeds budget");
    Ok(serde_json::from_slice(&bytes)?)
}
fn save(path: &Path, value: &Value) -> Result<()> {
    let mut stage =
        tempfile::NamedTempFile::new_in(path.parent().context("Output parent missing")?)?;
    serde_json::to_writer_pretty(&mut stage, value)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    stage.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn selected(declaration: &Value, index: usize, reference: &Value) -> Result<()> {
    let expected = declaration["songs"][index]["archive_sha256"]
        .as_str()
        .context("Frozen archive missing")?;
    ensure!(
        expected.len() == 64
            && expected.bytes().all(|b| b.is_ascii_hexdigit())
            && reference["source_archive_sha256"]
                .as_str()
                .is_some_and(|h| h.eq_ignore_ascii_case(expected)),
        "Case differs from frozen archive order"
    );
    Ok(())
}
fn command(binary: &Path, args: impl IntoIterator<Item = OsString>) -> Command {
    let mut cmd = Command::new(binary);
    cmd.args(args);
    cmd
}
fn arg(value: impl AsRef<std::ffi::OsStr>) -> OsString {
    value.as_ref().to_owned()
}
fn skipped(reason: &str) -> Value {
    json!({"status":"not_run","reason":reason})
}
fn invoke(mut command: Command, label: &str, folder: &Path, witnesses: &Witnesses) -> Value {
    let started = Instant::now();
    if witnesses.verify().is_err() {
        return json!({"status":"failed","reason":"frozen_input_changed_before_call","seconds":started.elapsed().as_secs_f64()});
    }
    let result = (|| -> Result<_> {
        let log = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(folder.join(format!("{label}.log")))?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        Ok(command.status()?)
    })();
    let integrity = witnesses.verify().is_ok();
    json!({"status":if result.as_ref().is_ok_and(|s|s.success()) && integrity {"success"}else{"failed"},
        "exit_code":result.as_ref().ok().and_then(|s|s.code()),"frozen_inputs_unchanged":integrity,
        "reason":if !integrity {Some("frozen_input_changed_after_call")}else if result.is_err(){Some("child_spawn_or_log_failure")}else if result.as_ref().is_ok_and(|s|!s.success()){Some("child_exit_failure")}else{None},
        "seconds":started.elapsed().as_secs_f64()})
}
fn succeeded(value: &Value) -> bool {
    value["status"] == "success"
}
fn require_artifact(stage: &mut Value, path: &Path) {
    if succeeded(stage) && !path.is_file() {
        stage["status"] = json!("failed");
        stage["reason"] = json!("successful_exit_missing_artifact");
    }
}
fn sanitized_evidence(path: &Path) -> Result<Value> {
    let value = load(path)?;
    ensure!(
        value["version"] == 1
            && value["experiment"] == "independent-part-evidence-v1"
            && value["guitar"].is_object(),
        "Unexpected audit report schema"
    );
    // This is a frozen, hash-bound trusted audit, but still reject accidental
    // identity/path fields rather than copying arbitrary future payloads.
    fn safe(value: &Value) -> bool {
        match value {
            Value::Object(fields) => fields.iter().all(|(key, v)| {
                ![
                    "path",
                    "paths",
                    "name",
                    "title",
                    "artist",
                    "lyrics",
                    "notes",
                    "targets",
                    "case",
                    "directory",
                    "filename",
                ]
                .contains(&key.as_str())
                    && safe(v)
            }),
            Value::Array(values) => values.iter().all(safe),
            _ => true,
        }
    }
    ensure!(
        safe(&value),
        "Audit output contains private identity or target fields"
    );
    Ok(value)
}
fn one_case(
    args: &Args,
    declaration: &Value,
    index: usize,
    folder: &Path,
    witnesses: &Witnesses,
) -> Value {
    let label = format!("heldout-{:02}", index + 1);
    let case = args.prepared.join(&label);
    let mut result = json!({"label":label,"selection":{"status":"not_run"},"prepare":skipped("selection_not_verified"),"generate":skipped("preparation_unavailable"),"split":skipped("preparation_unavailable"),"audit":skipped("generation_unavailable"),"statistics":null});
    let binding = (|| -> Result<()> {
        witnesses.verify()?;
        selected(
            declaration,
            index,
            &load(&case.join("reference-source.json"))?,
        )
    })();
    if binding.is_err() {
        result["selection"] = json!({"status":"failed","reason":"case_frozen_binding_invalid"});
        return result;
    }
    result["selection"] = json!({"status":"success"});
    let inputs = folder.join("inputs");
    let generation = folder.join("generation");
    let kit = folder.join("kit");
    let evidence = folder.join("evidence.json");
    result["prepare"] = invoke(
        command(&args.preparer, [arg("prepare"), arg(&case), arg(&inputs)]),
        "prepare",
        folder,
        witnesses,
    );
    require_artifact(&mut result["prepare"], &inputs.join("preparation.json"));
    if succeeded(&result["prepare"])
        && (|| -> Result<()> {
            selected(declaration, index, &load(&inputs.join("preparation.json"))?)?;
            ensure!(
                inputs.join("stem-manifest.json").is_file()
                    && inputs.join("guitar.wav").is_file()
                    && inputs.join("drums.wav").is_file(),
                "Required direct roles missing"
            );
            Ok(())
        })()
        .is_err()
    {
        result["prepare"]["status"] = json!("failed");
        result["prepare"]["reason"] = json!("prepared_roles_or_archive_binding_invalid");
    }
    if !succeeded(&result["prepare"]) {
        return result;
    }
    result["generate"] = invoke(
        command(
            &args.generator,
            [
                arg("--audio"),
                arg(case.join("source.wav")),
                arg("--manifest"),
                arg(inputs.join("stem-manifest.json")),
                arg("--model-dir"),
                arg(&args.models),
                arg("--bpm"),
                arg("120"),
                arg("--title"),
                arg("Blind evidence study"),
                arg("--artist"),
                arg("YACE benchmark"),
                arg("--guitar-source"),
                arg("guitar"),
                arg("--output"),
                arg(&generation),
            ],
        ),
        "generate",
        folder,
        witnesses,
    );
    require_artifact(
        &mut result["generate"],
        &generation.join("revised-events.json"),
    );
    // Separation is an independent diagnostic even if transcription failed.
    result["split"] = invoke(
        command(
            &args.splitter,
            [
                arg("--audio"),
                arg(inputs.join("drums.wav")),
                arg("--models"),
                arg(&args.kit_models),
                arg("--output"),
                arg(&kit),
            ],
        ),
        "split",
        folder,
        witnesses,
    );
    require_artifact(&mut result["split"], &kit.join(larsnet::RECEIPT_FILENAME));
    if succeeded(&result["generate"]) {
        let mut audit_args = vec![
            arg("--prepared-case"),
            arg(&case),
            arg("--events"),
            arg(generation.join("revised-events.json")),
            arg("--manifest"),
            arg(inputs.join("stem-manifest.json")),
            arg("--output"),
            arg(&evidence),
        ];
        if succeeded(&result["split"]) {
            audit_args.extend([arg("--kit"), arg(&kit)]);
        }
        result["audit"] = invoke(command(&args.audit, audit_args), "audit", folder, witnesses);
        require_artifact(&mut result["audit"], &evidence);
        if succeeded(&result["audit"]) {
            match sanitized_evidence(&evidence) {
                Ok(stats) => {
                    result["statistics"] = stats;
                    result["evidence_sha256"] = json!(models::hash_file(&evidence).ok());
                }
                Err(_) => {
                    result["audit"]["status"] = json!("failed");
                    result["audit"]["reason"] = json!("invalid_or_private_audit_report");
                }
            }
        }
    }
    result
}
fn run(mut args: Args) -> Result<()> {
    ensure!(!args.output.exists(), "Refusing an existing batch output");
    args.prepared = args.prepared.canonicalize()?;
    args.models = args.models.canonicalize()?;
    args.kit_models = args.kit_models.canonicalize()?;
    let mut witnesses = Witnesses(vec![], vec![]);
    let mut executables = BTreeMap::new();
    for (role, path) in [
        ("preparer", &mut args.preparer),
        ("generator", &mut args.generator),
        ("splitter", &mut args.splitter),
        ("audit", &mut args.audit),
    ] {
        *path = path.canonicalize()?;
        executables.insert(role, witnesses.add(path.clone())?);
    }
    let frozen = args.prepared.join("frozen-batch.json");
    let frozen_hash = witnesses.add(frozen.clone())?;
    let declaration = load(&frozen)?;
    ensure!(
        declaration["songs"]
            .as_array()
            .is_some_and(|s| s.len() == 6),
        "Exactly six predeclared cases required"
    );
    let mut model_hashes = BTreeMap::new();
    for id in ["pitch", "drums", "drum-articulation"] {
        let asset = models::assets()
            .into_iter()
            .find(|m| m.id == id)
            .context("Unknown registry model")?;
        let path = args.models.join(asset.filename);
        if id == "drum-articulation" && !path.exists() {
            witnesses.1.push(path);
            model_hashes.insert(id.to_owned(), json!(null));
            continue;
        }
        let hash = witnesses.add(path)?;
        ensure!(hash == asset.sha256, "Generator model hash differs");
        model_hashes.insert(id.to_owned(), json!(hash));
    }
    for (name, expected) in larsnet::MODELS {
        let hash = witnesses.add(args.kit_models.join(format!("{name}.onnx")))?;
        ensure!(hash == expected, "Kit model hash differs");
        model_hashes.insert(format!("kit_{name}"), json!(hash));
    }
    fs::create_dir(&args.output)?;
    args.output = args.output.canonicalize()?;
    let mut summary = json!({"version":1,"experiment":"independent-part-evidence-batch-v1","expected_cases":6,"frozen_batch_sha256":frozen_hash,"executables":executables,
        "runner_sha256":models::hash_file(&std::env::current_exe()?)?,"runner_source_sha256":format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(include_bytes!("part-evidence-batch.rs"))),"models":model_hashes,
        "targets_enter_inference":false,"scope":"Diagnostic only; Expert Guitar grouped attacks and optional five merged Expert Pro drum families; no generation changes",
        "cases":(1..=6).map(|i|json!({"label":format!("heldout-{i:02}"),"status":"not_run","statistics":null})).collect::<Vec<_>>()});
    save(&args.output.join("freeze.json"), &summary)?;
    save(&args.output.join("summary.json"), &summary)?;
    for index in 0..6 {
        let folder = args.output.join(format!("heldout-{:02}", index + 1));
        let outcome = match fs::create_dir(&folder) {
            Ok(()) => one_case(&args, &declaration, index, &folder, &witnesses),
            Err(_) => {
                json!({"label":format!("heldout-{:02}",index+1),"status":"failed","reason":"case_output_creation_failed","statistics":null})
            }
        };
        summary["cases"][index] = outcome;
        save(&args.output.join("summary.json"), &summary)?;
    }
    summary["frozen_inputs_unchanged"] = json!(witnesses.verify().is_ok());
    save(&args.output.join("summary.json"), &summary)?;
    Ok(())
}
fn main() -> Result<()> {
    run(Args::parse())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swapped_selection_and_private_or_missing_reports_fail() {
        let declaration =
            json!({"songs":[{"archive_sha256":"a".repeat(64)},{"archive_sha256":"b".repeat(64)}]});
        let reference = json!({"source_archive_sha256":"a".repeat(64)});
        selected(&declaration, 0, &reference).unwrap();
        assert!(selected(&declaration, 1, &reference).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evidence.json");
        assert!(sanitized_evidence(&path).is_err());
        fs::write(&path,serde_json::to_vec(&json!({"version":1,"experiment":"independent-part-evidence-v1","guitar":{"path":"private"}})).unwrap()).unwrap();
        assert!(sanitized_evidence(&path).is_err());
        let mut stage = json!({"status":"success"});
        require_artifact(&mut stage, &dir.path().join("absent"));
        assert!(!succeeded(&stage));
    }
    #[test]
    fn rust_child_failure_fixture() {
        if std::env::var_os("YACE_PART_EVIDENCE_CHILD_FAILURE").is_some() {
            eprintln!("private retained fixture log");
            std::process::exit(7);
        }
    }
    #[test]
    fn failed_rust_child_is_retained_and_changed_executable_blocks_launch() {
        let directory = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut witnesses = Witnesses(vec![], vec![]);
        witnesses.add(exe.clone()).unwrap();
        let mut child = Command::new(exe);
        child
            .args([
                "--exact",
                "tests::rust_child_failure_fixture",
                "--nocapture",
            ])
            .env("YACE_PART_EVIDENCE_CHILD_FAILURE", "1");
        let result = invoke(child, "failed", directory.path(), &witnesses);
        assert_eq!(result["status"], "failed");
        assert_eq!(result["exit_code"], 7);
        assert!(
            fs::read_to_string(directory.path().join("failed.log"))
                .unwrap()
                .contains("private retained fixture log")
        );
        witnesses.0[0].1 = "0".repeat(64);
        let result = invoke(
            Command::new("must-not-launch"),
            "blocked",
            directory.path(),
            &witnesses,
        );
        assert_eq!(result["reason"], "frozen_input_changed_before_call");
        assert!(!directory.path().join("blocked.log").exists());
    }
}
