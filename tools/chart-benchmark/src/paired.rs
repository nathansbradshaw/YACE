//! Frozen old/new instrumental policies, each using its revised production output.
//! Internal recovery-off charts are deliberately never read or scored here.
use crate::{
    evaluate::metrics,
    recoveries::{aggregate, load, mapped_delta, save},
};
use anyhow::{Context, Result, ensure};
use chart_engine::{
    benchmark::{self, Instrument},
    chart::Chart,
    mapping::{Transcription, Voice},
    models::hash_file,
    stem_review::{StemId, StemManifest},
    timing::TempoMap,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

const INSTRUMENTS: [(&str, Instrument, &str); 4] = [
    ("guitar", Instrument::Guitar, "ExpertSingle"),
    ("bass", Instrument::Bass, "ExpertDoubleBass"),
    ("drums", Instrument::Drums, "ExpertDrums"),
    ("pro_drums", Instrument::ProDrums, "ExpertDrums"),
];
const SIDES: [&str; 2] = ["baseline", "candidate"];
const SCOPE: &str = "Paired historical/candidate revised policies; Expert Guitar/Bass/Drums and native Pro Drums; six once-prepared direct-stem cases; no vocals, keyboards, full-mix or lower-difficulty evaluation";
const OAF_FILENAME: &str = "oaf_drums.onnx";
const OAF_SHA256: &str = "23d4b05ce7e9715bdb4396823d68bc886e8a958fa9f6266c0f7132b78098b521";
const OAF_BYTES: u64 = 5_966_247;

// Preserve the historical two-model recovery contract. Only this paired
// experiment can additionally bind a present, exactly pinned OAF model.
fn paired_model_hashes(models: &Path) -> Result<Value> {
    let mut hashes = crate::recoveries::model_hashes(models)?;
    let path = models.join(OAF_FILENAME);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => {
            ensure!(path.is_file(), "Present OAF model is not a readable file");
            ensure!(
                fs::metadata(&path)?.len() == OAF_BYTES,
                "Present OAF model byte count mismatch"
            );
            let hash = hash_file(&path)?;
            ensure!(hash == OAF_SHA256, "Present OAF model checksum mismatch");
            hashes[OAF_FILENAME] = json!(hash);
        }
    }
    Ok(hashes)
}

struct Frozen {
    prepared: PathBuf,
    output: PathBuf,
    models: PathBuf,
    audits: [PathBuf; 2],
    audit_hashes: [String; 2],
    frozen_path: PathBuf,
    frozen_hash: String,
    model_hashes: Value,
    runner_hash: String,
    external_plan: Option<(PathBuf, String)>,
    plan_hash: String,
}
fn source_bundle(output: &Path) -> Result<Value> {
    let folder = output.join("runner-source");
    fs::create_dir(&folder)?;
    let mut hashes = json!({});
    for (name, bytes) in [
        ("paired.rs", include_bytes!("paired.rs").as_slice()),
        ("main.rs", include_bytes!("main.rs").as_slice()),
        ("lib.rs", include_bytes!("lib.rs").as_slice()),
        ("evaluate.rs", include_bytes!("evaluate.rs").as_slice()),
        ("recoveries.rs", include_bytes!("recoveries.rs").as_slice()),
        ("Cargo.toml", include_bytes!("../Cargo.toml").as_slice()),
        ("Cargo.lock", include_bytes!("../Cargo.lock").as_slice()),
        (
            "engine-Cargo.toml",
            include_bytes!("../../../Cargo.toml").as_slice(),
        ),
        (
            "engine-Cargo.lock",
            include_bytes!("../../../Cargo.lock").as_slice(),
        ),
        (
            "engine-midi.rs",
            include_bytes!("../../../src/midi.rs").as_slice(),
        ),
        (
            "engine-benchmark.rs",
            include_bytes!("../../../src/benchmark.rs").as_slice(),
        ),
        (
            "engine-chart.rs",
            include_bytes!("../../../src/chart.rs").as_slice(),
        ),
        (
            "engine-timing.rs",
            include_bytes!("../../../src/timing.rs").as_slice(),
        ),
        (
            "engine-mapping.rs",
            include_bytes!("../../../src/mapping.rs").as_slice(),
        ),
        (
            "engine-stem-review.rs",
            include_bytes!("../../../src/stem_review.rs").as_slice(),
        ),
    ] {
        let path = folder.join(name);
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        std::io::Write::write_all(&mut f, bytes)?;
        hashes[name] = json!(hash_file(&path)?);
    }
    Ok(hashes)
}
fn inputs(case: &Path, inputs: &Path, index: usize, declaration: &Value) -> Result<Value> {
    crate::prepare(case, inputs)?;
    let preparation = load(&inputs.join("preparation.json"))?;
    crate::validate_frozen_case(
        declaration,
        index,
        preparation["source_archive_sha256"]
            .as_str()
            .context("Missing prepared archive")?,
    )?;
    let source = case.join("source.wav");
    let source_hash = hash_file(&source)?;
    let manifest_path = inputs.join("stem-manifest.json");
    let manifest = StemManifest::read(&manifest_path)?;
    manifest.validate_binding(&source_hash, manifest.frames as f64 / 44100.)?;
    ensure!(
        [StemId::Bass, StemId::Guitar, StemId::Drums, StemId::Vocals]
            .iter()
            .all(|id| manifest.ids().contains(id)),
        "Required direct oracle role unavailable"
    );
    let mut stems = vec![];
    for asset in &manifest.stems {
        ensure!(
            hash_file(&asset.path)? == asset.sha256,
            "Prepared stem changed"
        );
        stems.push(json!({"id":asset.id,"sha256":asset.sha256}));
    }
    Ok(
        json!({"source_sha256":source_hash,"source_archive_sha256":preparation["source_archive_sha256"],"preparation_sha256":hash_file(&inputs.join("preparation.json"))?,"manifest_sha256":hash_file(&manifest_path)?,"reference_receipt_sha256":hash_file(&case.join("reference-source.json"))?,"mix_receipt_sha256":hash_file(&case.join("source.mix.json"))?,"reference_notes_sha256":preparation["reference_notes_sha256"],"stems":stems,"frames":manifest.frames,"sample_rate":manifest.sample_rate}),
    )
}
fn check(f: &Frozen, index: usize, witness: &Value) -> Result<()> {
    ensure!(
        hash_file(&f.frozen_path)? == f.frozen_hash
            && paired_model_hashes(&f.models)? == f.model_hashes,
        "Frozen selection/models changed"
    );
    ensure!(
        hash_file(&std::env::current_exe()?)? == f.runner_hash,
        "Benchmark executable changed"
    );
    ensure!(
        hash_file(&f.output.join("plan.json"))? == f.plan_hash,
        "Inference plan changed"
    );
    for (audit, expected) in f.audits.iter().zip(&f.audit_hashes) {
        ensure!(
            hash_file(audit)? == *expected,
            "Frozen audit executable changed"
        );
    }
    if let Some((path, expected)) = &f.external_plan {
        ensure!(
            hash_file(path)? == *expected,
            "External declaration plan changed"
        );
    }
    let case = f.prepared.join(format!("heldout-{:02}", index + 1));
    let inputs = f.output.join(format!("heldout-{:02}/inputs", index + 1));
    for (path, key) in [
        (case.join("source.wav"), "source_sha256"),
        (
            case.join("reference-source.json"),
            "reference_receipt_sha256",
        ),
        (case.join("source.mix.json"), "mix_receipt_sha256"),
        (inputs.join("preparation.json"), "preparation_sha256"),
        (inputs.join("stem-manifest.json"), "manifest_sha256"),
    ] {
        ensure!(
            witness[key] == hash_file(&path)?,
            "Prepared source/receipt changed: {key}"
        );
    }
    let manifest = StemManifest::read(&inputs.join("stem-manifest.json"))?;
    manifest.validate_binding(
        witness["source_sha256"].as_str().unwrap(),
        witness["frames"].as_u64().unwrap() as f64 / 44100.,
    )?;
    for asset in &manifest.stems {
        ensure!(
            hash_file(&asset.path)? == asset.sha256,
            "Prepared stem bytes changed"
        );
    }
    let reference = crate::load_case(&case)?;
    ensure!(
        hash_file(&reference.notes)? == witness["reference_notes_sha256"],
        "Reference identity changed"
    );
    crate::validate_frozen_case(
        &load(&f.frozen_path)?,
        index,
        &reference.mix.source_archive_sha256,
    )?;
    Ok(())
}

fn raw_count(events: &Transcription, instrument: &str) -> usize {
    match instrument {
        "guitar" => events.guitar.len(),
        "bass" => events.bass.len(),
        _ => events.drums.len(),
    }
}
fn validate_part(
    events: &Transcription,
    chart: &Chart,
    name: &str,
    instrument: &str,
) -> Result<bool> {
    let exists = chart.sections.iter().any(|s| s.name == name);
    let raw = raw_count(events, instrument);
    let notes = chart.notes(name)?;
    ensure!(
        exists || raw == 0,
        "Missing requested section with nonempty raw events: {name}"
    );
    let played = notes
        .iter()
        .any(|n| n.lane <= 4 || (name != "ExpertDrums" && n.lane == 7));
    ensure!(
        !played || raw > 0,
        "Played chart part lacks corresponding raw events: {name}"
    );
    Ok(!played)
}
fn run_side(f: &Frozen, index: usize, side: usize, witness: &Value) -> Value {
    let start = Instant::now();
    let label = format!("heldout-{:02}", index + 1);
    let case = f.prepared.join(&label);
    let folder = f.output.join(&label).join(SIDES[side]);
    let mut stage = "frozen_input_validation";
    let result = (|| -> Result<Value> {
        fs::create_dir(&folder)?;
        check(f, index, witness)?;
        let destination = folder.join("audit");
        let source = case.join("source.wav");
        let manifest = f.output.join(&label).join("inputs/stem-manifest.json");
        let args = vec![
            "--audio".to_owned(),
            source.to_string_lossy().into_owned(),
            "--manifest".into(),
            manifest.to_string_lossy().into_owned(),
            "--model-dir".into(),
            f.models.to_string_lossy().into_owned(),
            "--bpm".into(),
            "120".into(),
            "--title".into(),
            "Blind paired instrumental study".into(),
            "--artist".into(),
            "YACE benchmark".into(),
            "--guitar-source".into(),
            "guitar".into(),
            "--output".into(),
            destination.to_string_lossy().into_owned(),
        ];
        stage = "native_audit";
        let log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join("audit.log"))?;
        let status = Command::new(&f.audits[side])
            .args(&args)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .status();
        save(
            &folder.join("invocation.json"),
            &json!({"side":SIDES[side],"arguments":args,"audit_binary_sha256":f.audit_hashes[side],"plan_sha256":f.plan_hash,"source_sha256":witness["source_sha256"],"manifest_sha256":witness["manifest_sha256"],"success":status.as_ref().is_ok_and(|s|s.success()),"exit_code":status.as_ref().ok().and_then(|s|s.code())}),
        )?;
        ensure!(status?.success(), "Native audit failed; log retained");
        stage = "artifact_validation";
        check(f, index, witness)?;
        let provenance = load(&destination.join("comparison.json"))?;
        ensure!(
            provenance["version"] == 1
                && provenance["audio_sha256"] == witness["source_sha256"]
                && provenance["stem_manifest_sha256"] == witness["manifest_sha256"],
            "Audit source/manifest provenance mismatch"
        );
        let event_path = destination.join("revised-events.json");
        let events_json = load(&event_path)?;
        ensure!(
            ["guitar", "bass", "drums"]
                .iter()
                .all(|k| events_json[*k].is_array()),
            "Audit omitted an explicit raw instrument array"
        );
        let events: Transcription = serde_json::from_value(events_json.clone())?;
        events.validate()?;
        ensure!(
            events.guitar_source == chart_engine::mapping::GuitarSource::Guitar,
            "Audit changed the declared Guitar audio role"
        );
        let chart_path = destination.join("revised/notes.chart");
        let chart = Chart::parse(fs::read_to_string(&chart_path)?)?;
        ensure!(
            chart.timing == TempoMap::constant(120.)?,
            "Audit output changed fixed tempo/offset"
        );
        let event_hash = hash_file(&event_path)?;
        let chart_hash = hash_file(&chart_path)?;
        let reference = crate::load_case(&case)?;
        stage = "comparison";
        let mut results = vec![];
        for (instrument, kind, track) in INSTRUMENTS {
            let measured = (|| -> Result<Value> {
                let abstained = validate_part(&events, &chart, track, instrument)?;
                let report = benchmark::compare(benchmark::Request {
                    reference_midi: &reference.notes,
                    generated_midi: &chart_path,
                    reference_audio: &source,
                    generated_audio: &source,
                    reference_delay_ms: reference.delay,
                    generated_delay_ms: 0.,
                    instrument: kind,
                })?;
                ensure!(
                    report.recording_sha256 == witness["source_sha256"]
                        && report.reference_notes_sha256 == witness["reference_notes_sha256"]
                        && report.generated_notes_sha256 == chart_hash,
                    "Comparison artifact/source binding changed"
                );
                Ok(
                    json!({"instrument":instrument,"success":true,"abstained":abstained,"report":report}),
                )
            })();
            results.push(match measured{Ok(v)=>v,Err(e)=>json!({"instrument":instrument,"success":false,"failure_code":"comparison_failed","error":format!("{e:#}")})});
        }
        check(f, index, witness)?;
        ensure!(
            hash_file(&event_path)? == event_hash && hash_file(&chart_path)? == chart_hash,
            "Audit artifacts changed during scoring"
        );
        Ok(
            json!({"success":results.iter().all(|r|r["success"]==true),"execution_success":true,"audit_binary_sha256":f.audit_hashes[side],"source_sha256":witness["source_sha256"],"manifest_sha256":witness["manifest_sha256"],"events_sha256":event_hash,"chart_sha256":chart_hash,"comparison_sha256":hash_file(&destination.join("comparison.json"))?,"raw":raw_summary(&events_json),"results":results}),
        )
    })();
    let mut value = match result {
        Ok(v) => v,
        Err(e) => {
            json!({"success":false,"execution_success":false,"failure_stage":stage,"error":format!("{e:#}")})
        }
    };
    value["elapsed_seconds"] = json!(start.elapsed().as_secs_f64());
    value
}

fn raw_summary(value: &Value) -> Value {
    fn symbolic(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 80
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }
    fn reasons(rows: Option<&Vec<Value>>, field: &str) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for row in rows.into_iter().flatten() {
            let reason = row[field]
                .as_str()
                .filter(|s| symbolic(s))
                .unwrap_or("structured_or_unrecognized_reason");
            *counts.entry(reason.into()).or_default() += 1;
        }
        counts
    }
    fn counters(value: &Value) -> BTreeMap<String, u64> {
        value
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, v)| symbolic(key) && v.as_u64().is_some())
            .map(|(key, v)| (key.clone(), v.as_u64().unwrap()))
            .collect()
    }
    fn arrays(value: &Value, path: &str, depth: usize, out: &mut BTreeMap<String, Value>) {
        if depth > 4 {
            return;
        }
        match value {
            Value::Array(rows) => {
                out.insert(path.into(),json!({"count":rows.len(),"retained":rows.iter().filter(|v|v["retained"]==true).count(),"accepted":rows.iter().filter(|v|v["accepted"]==true||v["status"]=="accepted").count(),"rejected":rows.iter().filter(|v|v["accepted"]==false||v["status"]=="rejected").count()}));
            }
            Value::Object(fields) => {
                for (key, v) in fields {
                    arrays(v, &format!("{path}.{key}"), depth + 1, out);
                }
            }
            _ => {}
        }
    }
    let mut evidence = BTreeMap::new();
    if let Some(fields) = value.as_object() {
        for (key, v) in fields {
            if (key.contains("guitar") || key.contains("drum") || key.contains("articulation"))
                && !["guitar", "drums"].contains(&key.as_str())
            {
                arrays(v, key, 0, &mut evidence);
            }
        }
    }
    let rows = value["drum_articulations"].as_array();
    let mut voices: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows
        .into_iter()
        .flatten()
        .filter(|r| !r["proposed"].is_null())
    {
        let voice = row["proposed"]
            .as_str()
            .filter(|v| symbolic(v))
            .unwrap_or("structured_or_unrecognized_voice");
        *voices.entry(voice.into()).or_default() += 1;
    }
    json!({"guitar_events":value["guitar"].as_array().map_or(0,Vec::len),"bass_events":value["bass"].as_array().map_or(0,Vec::len),"drum_events":value["drums"].as_array().map_or(0,Vec::len),"drum_articulation":{"total_proposals":rows.map_or(0,Vec::len),"proposed_some":rows.map_or(0,|r|r.iter().filter(|v|!v["proposed"].is_null()).count()),"map_eligible":rows.map_or(0,|r|r.iter().filter(|v|v["map_proposal"]==true).count()),"no_proposal":rows.map_or(0,|r|r.iter().filter(|v|v["proposed"].is_null()).count()),"proposed_voices":voices,"reasons":reasons(rows,"reason")},"guitar_reattack_diagnostics":counters(&value["guitar_reattack_diagnostics"]),"guitar_polyphonic_diagnostics":counters(&value["guitar_polyphonic_diagnostics"]),"guitar_polyphonic_notes":value["guitar_polyphonic_notes"].as_array().map_or(0,Vec::len),"guitar_polyphonic_original_rejections":reasons(value["guitar_polyphonic_notes"].as_array(),"original_rejection"),"guitar_short_candidate_first_reasons":reasons(value["guitar_short_candidates"].as_array(),"reason"),"evidence_arrays":evidence})
}
fn voice_code(voice: Voice) -> u8 {
    match voice {
        Voice::Kick => 0,
        Voice::Snare => 1,
        Voice::HiHat => 2,
        Voice::HighTom => 3,
        Voice::MidTom => 4,
        Voice::LowTom => 5,
        Voice::Crash => 6,
        Voice::Ride => 7,
        Voice::Tom => 8,
        Voice::Cymbal => 9,
    }
}
fn compatible(old: u8, new: u8) -> bool {
    old == new || (old == 8 && [3, 4, 5].contains(&new)) || (old == 9 && [6, 7].contains(&new))
}
fn raw_drum_invariants(a: &Transcription, b: &Transcription) -> Value {
    let groups = |e: &Transcription| {
        let mut v: BTreeMap<bool, Vec<u64>> = BTreeMap::new();
        for h in &e.drums {
            v.entry(h.voice == Voice::Kick)
                .or_default()
                .push(h.time.to_bits());
        }
        for times in v.values_mut() {
            times.sort_unstable();
        }
        v
    };
    let grouped = |e: &Transcription| {
        let mut v: BTreeMap<(u64, u32), Vec<u8>> = BTreeMap::new();
        for h in &e.drums {
            v.entry((h.time.to_bits(), h.confidence.to_bits()))
                .or_default()
                .push(voice_code(h.voice));
        }
        for voices in v.values_mut() {
            voices.sort_unstable();
        }
        v
    };
    let (left, right) = (grouped(a), grouped(b));
    let mut valid = left.len() == right.len();
    let mut changed = 0;
    for (key, old) in &left {
        let Some(new) = right.get(key) else {
            valid = false;
            continue;
        };
        let mut available = new.clone();
        let mut generic = vec![];
        for &voice in old {
            if let Some(at) = available.iter().position(|v| *v == voice) {
                available.remove(at);
            } else {
                generic.push(voice);
            }
        }
        for voice in generic {
            if let Some(at) = available.iter().position(|&v| compatible(voice, v)) {
                available.remove(at);
                changed += 1;
            } else {
                valid = false;
            }
        }
        valid &= available.is_empty();
    }
    let x = groups(a);
    let y = groups(b);
    let unique = |v: &BTreeMap<bool, Vec<u64>>| {
        v.iter()
            .map(|(k, v)| (*k, v.iter().copied().collect::<BTreeSet<_>>()))
            .collect::<BTreeMap<_, _>>()
    };
    json!({"kick_hand_time_multisets_unchanged":x==y,"kick_hand_time_sets_unchanged":unique(&x)==unique(&y),"confidence_and_voice_rules_preserved":valid,"allowed_voice_refinements":changed,"baseline_hits":a.drums.len(),"candidate_hits":b.drums.len()})
}
fn drum_ticks(chart: &Chart) -> Result<BTreeMap<bool, BTreeSet<u64>>> {
    let mut groups: BTreeMap<bool, BTreeSet<u64>> = BTreeMap::new();
    for n in chart.notes("ExpertDrums")? {
        if n.lane <= 4 {
            groups.entry(n.lane == 0).or_default().insert(n.tick);
        }
    }
    Ok(groups)
}
fn delta(a: &Chart, b: &Chart, track: &str) -> Result<Value> {
    let mut value = mapped_delta(a, b, track)?;
    let notes = |c: &Chart| -> Result<BTreeSet<(u64, u8, u64)>> {
        Ok(c.notes(track)?
            .into_iter()
            .filter(|n| n.lane <= 4 || (track != "ExpertDrums" && n.lane == 7))
            .map(|n| (n.tick, n.lane, n.sustain))
            .collect())
    };
    let before = notes(a)?;
    let after = notes(b)?;
    value["added_note_triples"] = json!(after.difference(&before).count());
    value["removed_note_triples"] = json!(before.difference(&after).count());
    value["changed_sustains_on_existing_heads"] = json!(
        before
            .iter()
            .filter(|(t, l, s)| after
                .iter()
                .any(|(tt, ll, ss)| t == tt && l == ll && s != ss))
            .count()
    );
    let controls = |c: &Chart| -> Result<BTreeSet<(u64, u8, u64)>> {
        Ok(c.notes(track)?
            .into_iter()
            .filter(|n| n.lane > 4 && (track == "ExpertDrums" || n.lane != 7))
            .map(|n| (n.tick, n.lane, n.sustain))
            .collect())
    };
    let old = controls(a)?;
    let new = controls(b)?;
    value["added_nonplayed_marker_triples"] = json!(new.difference(&old).count());
    value["removed_nonplayed_marker_triples"] = json!(old.difference(&new).count());
    Ok(value)
}
fn pair(f: &Frozen, index: usize, a: &Value, b: &Value) -> Result<Value> {
    ensure!(
        a["execution_success"] == true && b["execution_success"] == true,
        "Both validated audit executions are required for invariants"
    );
    let folder = f.output.join(format!("heldout-{:02}", index + 1));
    let path = |side: &str, name: &str| folder.join(side).join("audit").join(name);
    for (side, receipt) in [("baseline", a), ("candidate", b)] {
        ensure!(
            hash_file(&path(side, "revised-events.json"))? == receipt["events_sha256"]
                && hash_file(&path(side, "revised/notes.chart"))? == receipt["chart_sha256"]
                && hash_file(&path(side, "comparison.json"))? == receipt["comparison_sha256"],
            "Previously validated audit artifacts changed before pairing"
        );
    }
    let before: Transcription =
        serde_json::from_slice(&fs::read(path("baseline", "revised-events.json"))?)?;
    let after: Transcription =
        serde_json::from_slice(&fs::read(path("candidate", "revised-events.json"))?)?;
    let x = Chart::parse(fs::read_to_string(path("baseline", "revised/notes.chart"))?)?;
    let y = Chart::parse(fs::read_to_string(path(
        "candidate",
        "revised/notes.chart",
    ))?)?;
    let bass_provenance = paired_bass_invariants(&before, &after, &x, &y)?;
    let bass = bass_provenance["semantic_fields_and_map_unchanged"] == true;
    let drums = raw_drum_invariants(&before, &after);
    let mapped = drum_ticks(&x)? == drum_ticks(&y)?;
    let selected = |events: &Transcription, timing: &TempoMap| {
        chart_engine::mapping::played_drum_articulations(events, timing)
            .into_iter()
            .map(|(index, tick, _)| (index, tick))
            .collect::<BTreeSet<_>>()
    };
    let selected_before = selected(&before, &x.timing);
    let selected_after = selected(&after, &y.timing);
    // Equal counts/tick sets alone can conceal a previously discarded cymbal
    // displacing a selected tom. Voice relocation may not change source owners.
    let selected_unchanged = selected_before == selected_after;
    let valid = bass
        && drums["kick_hand_time_multisets_unchanged"] == true
        && drums["confidence_and_voice_rules_preserved"] == true
        && mapped
        && selected_unchanged;
    let guitar = |events: &Transcription| {
        events
            .guitar
            .iter()
            .map(|n| {
                (
                    n.start.to_bits(),
                    n.end.to_bits(),
                    n.pitch,
                    n.confidence.to_bits(),
                )
            })
            .collect::<BTreeSet<_>>()
    };
    let (old_guitar, new_guitar) = (guitar(&before), guitar(&after));
    let raw_guitar = json!({"baseline_events":before.guitar.len(),"candidate_events":after.guitar.len(),"added_event_tuples":new_guitar.difference(&old_guitar).count(),"removed_event_tuples":old_guitar.difference(&new_guitar).count(),"changed_releases_on_existing_attacks":old_guitar.iter().filter(|(t,e,p,c)|new_guitar.iter().any(|(tt,ee,pp,cc)|t==tt&&p==pp&&c==cc&&e!=ee)).count()});
    Ok(
        json!({"success":valid,"bass_raw_and_mapped_unchanged":bass,"bass_comparison":bass_provenance,"raw_drums":drums,"raw_guitar_delta":raw_guitar,"mapped_kick_hand_attack_tick_sets_unchanged":mapped,"selected_raw_drum_sources_unchanged":selected_unchanged,"baseline_selected_hand_sources":selected_before.len(),"candidate_selected_hand_sources":selected_after.len(),"guitar_mapping":delta(&x,&y,"ExpertSingle")?,"drum_mapping":delta(&x,&y,"ExpertDrums")?}),
    )
}

/// Historical and candidate executables necessarily have different broad
/// implementation hashes. Exempt exactly those two code IDs from the paired
/// musical invariant, retain them verbatim, and compare every other Bass field.
/// The single-executable recovery ablation deliberately retains its old contract.
fn paired_bass_invariants(
    before: &Transcription,
    after: &Transcription,
    a: &Chart,
    b: &Chart,
) -> Result<Value> {
    before.validate()?;
    after.validate()?;
    let fields = |events: &Transcription| -> Result<(Value, Value)> {
        let value = serde_json::to_value(events)?;
        let mut bass = Value::Object(
            value
                .as_object()
                .context("Transcription is not an object")?
                .iter()
                .filter(|(key, _)| key.as_str() == "bass" || key.starts_with("bass_"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        let mut ids = json!({});
        for name in ["bass_decoder", "bass_analysis"] {
            if let Some(receipt) = bass.get_mut(name) {
                let object = receipt.as_object_mut().context("Invalid Bass receipt")?;
                let id = object
                    .remove("implementation_sha256")
                    .context("Missing Bass implementation ID")?;
                ensure!(
                    id.as_str()
                        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
                    "Invalid Bass implementation ID"
                );
                ids[name] = id;
            } else {
                ids[name] = Value::Null;
            }
        }
        Ok((bass, ids))
    };
    let (left, left_ids) = fields(before)?;
    let (right, right_ids) = fields(after)?;
    let unchanged = left == right
        && a.timing == b.timing
        && a.notes("ExpertDoubleBass")? == b.notes("ExpertDoubleBass")?;
    Ok(
        json!({"semantic_fields_and_map_unchanged":unchanged,"implementation_ids_unchanged":left_ids==right_ids,"baseline_implementation_ids":left_ids,"candidate_implementation_ids":right_ids,"comparison_exemptions":["/bass_decoder/implementation_sha256","/bass_analysis/implementation_sha256"]}),
    )
}
fn aggregate_with_roles(reports: &[&Value]) -> Value {
    let mut result = aggregate(reports);
    let mut groups: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for report in reports {
        if let Some(roles) = report["drum_roles"].as_array() {
            for role in roles {
                if let Some(name) = role["class"].as_str() {
                    groups.entry(name).or_default().push(role);
                }
            }
        }
    }
    result["drum_kick_hand_attacks"] = json!(
        groups
            .into_iter()
            .map(|(name, rows)| json!({"class":name,"attacks":metrics(&rows)}))
            .collect::<Vec<_>>()
    );
    result
}

fn summarize(cases: &[Value], freeze: &Value, plan_sha: &str, elapsed: f64) -> Value {
    let mut public = vec![];
    let mut instruments = vec![];
    let mut all = true;
    for case in cases {
        let mut invariants = case["invariants"].clone();
        if let Some(object) = invariants.as_object_mut() {
            object.remove("error");
        }
        let mut row = json!({"case":case["case"],"success":case["success"],"preparation_success":case["preparation_success"],"preparation_seconds":case["preparation_seconds"],"invariants":invariants,"source_sha256":case["inputs"]["source_sha256"],"source_archive_sha256":case["inputs"]["source_archive_sha256"]});
        let mut outcomes = vec![];
        for (instrument, _, _) in INSTRUMENTS {
            let mut item = json!({"instrument":instrument});
            for side in SIDES {
                let outcome = case[side]["results"]
                    .as_array()
                    .and_then(|r| r.iter().find(|v| v["instrument"] == instrument));
                item[side] = if let Some(v) = outcome.filter(|v| v["success"] == true) {
                    json!({"success":true,"abstained":v["abstained"],"metrics":aggregate_with_roles(&[&v["report"]])})
                } else {
                    all = false;
                    json!({"success":false,"failure_code":case[side]["failure_stage"].as_str().unwrap_or("comparison_or_preparation_failed")})
                };
            }
            outcomes.push(item);
        }
        row["results"] = json!(outcomes);
        for side in SIDES {
            row[side] = json!({"success":case[side]["success"],"execution_success":case[side]["execution_success"],"elapsed_seconds":case[side]["elapsed_seconds"],"events_sha256":case[side]["events_sha256"],"chart_sha256":case[side]["chart_sha256"],"raw":case[side]["raw"]});
        }
        all &= case["success"] == true;
        public.push(row);
    }
    for (instrument, _, _) in INSTRUMENTS {
        let mut item = json!({"instrument":instrument,"expected_cases":6});
        for side in SIDES {
            let reports: Vec<_> = cases
                .iter()
                .filter_map(|c| c[side]["results"].as_array())
                .flatten()
                .filter(|v| v["instrument"] == instrument && v["success"] == true)
                .map(|v| &v["report"])
                .collect();
            item[side] = json!({"scored_cases":reports.len(),"failed_cases":6-reports.len(),"metrics":aggregate_with_roles(&reports)});
        }
        let paired: Vec<_> = cases
            .iter()
            .filter(|c| {
                c["invariants"]["success"] == true
                    && SIDES.iter().all(|s| {
                        c[*s]["results"].as_array().is_some_and(|r| {
                            r.iter()
                                .any(|v| v["instrument"] == instrument && v["success"] == true)
                        })
                    })
            })
            .collect();
        item["paired_cases"] = json!(paired.len());
        for side in SIDES {
            let reports: Vec<_> = paired
                .iter()
                .filter_map(|c| c[side]["results"].as_array())
                .flatten()
                .filter(|v| v["instrument"] == instrument)
                .map(|v| &v["report"])
                .collect();
            item["paired_metrics"][side] = aggregate_with_roles(&reports);
        }
        instruments.push(item);
    }
    json!({"version":1,"scope":SCOPE,"condition":"paired different executables, revised-policy output only","expected_cases":6,"complete":cases.len()==6,"all_cases_successful":all,"plan_sha256":plan_sha,"frozen_batch_sha256":freeze["frozen_batch_sha256"],"external_declaration_plan_sha256":freeze["external_declaration_plan_sha256"],"baseline_audit_sha256":freeze["baseline_audit_sha256"],"candidate_audit_sha256":freeze["candidate_audit_sha256"],"benchmark_binary_sha256":freeze["benchmark_binary_sha256"],"models":freeze["models"],"elapsed_seconds":elapsed,"cases":public,"instruments":instruments,"limitations":["Both scored outputs are each executable's revised production policy; internal recovery-off baseline charts are ignored.","Paired aggregates require both instrument scores and passing invariant checks; one-sided successes, invariant-failed measurements and all failures remain visible.","Invariant failures are explicit failures even when comparisons can be measured; no failed case is replaced.","Expert/Pro authored-arrangement agreement is not physical instrument truth, fun, listening/controller acceptance or editing-time savings."]})
}

/// Prepare every frozen case once, publish the complete plan, then invoke each
/// independently pinned policy. All failures survive in private and public receipts.
pub fn batch(
    baseline: &Path,
    candidate: &Path,
    prepared: &Path,
    output: &Path,
    models: &Path,
) -> Result<()> {
    ensure!(!output.exists(), "Refusing existing paired batch output");
    let audits = [fs::canonicalize(baseline)?, fs::canonicalize(candidate)?];
    let audit_hashes = [hash_file(&audits[0])?, hash_file(&audits[1])?];
    let prepared = fs::canonicalize(prepared)?;
    let models = fs::canonicalize(models)?;
    let frozen_path = prepared.join("frozen-batch.json");
    let declaration = load(&frozen_path)?;
    ensure!(
        declaration["songs"]
            .as_array()
            .is_some_and(|v| v.len() == 6),
        "Exactly six frozen cases required"
    );
    let frozen_hash = hash_file(&frozen_path)?;
    let model_hashes = paired_model_hashes(&models)?;
    let plans: Vec<_> = [
        "paired-instrument-followup-plan.json",
        "recovery-followup-plan.json",
    ]
    .into_iter()
    .map(|name| prepared.parent().unwrap().join(name))
    .filter(|p| p.exists())
    .collect();
    ensure!(plans.len() <= 1, "Ambiguous adjacent declaration plans");
    let external_plan = plans
        .first()
        .map(|path| -> Result<_> {
            let declared = load(path)?;
            ensure!(
                declared["selection_sha256"] == frozen_hash,
                "Declaration plan selects a different batch"
            );
            for (key, actual) in [
                ("baseline_audit_sha256", &audit_hashes[0]),
                ("candidate_audit_sha256", &audit_hashes[1]),
            ] {
                if let Some(expected) = declared.get(key) {
                    ensure!(
                        expected.as_str() == Some(actual.as_str()),
                        "Declared {key} differs from provided executable"
                    );
                }
            }
            Ok((path.clone(), hash_file(path)?))
        })
        .transpose()?;
    fs::create_dir(output)?;
    let output = fs::canonicalize(output)?;
    let runner_hash = hash_file(&std::env::current_exe()?)?;
    let sources = source_bundle(&output)?;
    let start = Instant::now();
    let mut cases = vec![];
    for index in 0..6 {
        let label = format!("heldout-{:02}", index + 1);
        let folder = output.join(&label);
        fs::create_dir(&folder)?;
        let t = Instant::now();
        let prepared_result = inputs(
            &prepared.join(&label),
            &folder.join("inputs"),
            index,
            &declaration,
        );
        let mut row = json!({"case":label,"preparation_seconds":t.elapsed().as_secs_f64()});
        match prepared_result {
            Ok(v) => {
                row["preparation_success"] = json!(true);
                row["inputs"] = v;
            }
            Err(e) => {
                row["preparation_success"] = json!(false);
                row["error"] = json!(format!("{e:#}"));
                row["failure_stage"] = json!("preparation");
            }
        }
        cases.push(row);
    }
    let freeze = json!({"version":1,"scope":SCOPE,"frozen_batch_sha256":frozen_hash,"external_declaration_plan_sha256":external_plan.as_ref().map(|(_,h)|h),"baseline_audit_sha256":audit_hashes[0],"candidate_audit_sha256":audit_hashes[1],"benchmark_binary_sha256":runner_hash,"models":model_hashes,"runner_source_files":sources,"fixed_bpm":120,"guitar_source":"guitar","expected_cases":6,"target_notes_or_lyrics_enter_inference":false,"scored_artifacts":"revised-events.json and revised/notes.chart only","all_case_preparation_finished_before_inference":true});
    let planned:Vec<_>=cases.iter().enumerate().map(|(i,c)|json!({"index":i,"case":c["case"],"expected_archive_sha256":declaration["songs"][i]["archive_sha256"],"preparation_success":c["preparation_success"],"inputs":c["inputs"],"failure_stage":c["failure_stage"]})).collect();
    save(
        &output.join("plan.json"),
        &json!({"freeze":freeze,"cases":planned}),
    )?;
    let plan_hash = hash_file(&output.join("plan.json"))?;
    let mut receipt = freeze.clone();
    receipt["plan_sha256"] = json!(plan_hash);
    save(&output.join("freeze.json"), &receipt)?;
    let f = Frozen {
        prepared,
        output,
        models,
        audits,
        audit_hashes,
        frozen_path,
        frozen_hash,
        model_hashes,
        runner_hash,
        external_plan,
        plan_hash,
    };
    for (index, c) in cases.iter_mut().enumerate() {
        if c["preparation_success"] == true {
            for (side, name) in SIDES.into_iter().enumerate() {
                c[name] = run_side(&f, index, side, &c["inputs"]);
            }
            c["invariants"] = match pair(&f, index, &c["baseline"], &c["candidate"]) {
                Ok(v) => v,
                Err(e) => {
                    json!({"success":false,"failure_code":"pair_unavailable","error":format!("{e:#}")})
                }
            };
        } else {
            for name in SIDES {
                c[name] = json!({"success":false,"execution_success":false,"failure_stage":"preparation"});
            }
            c["invariants"] = json!({"success":false,"failure_code":"preparation_failed"});
        }
        c["success"] = json!(
            c["baseline"]["success"] == true
                && c["candidate"]["success"] == true
                && c["invariants"]["success"] == true
        );
        save(
            &f.output
                .join(c["case"].as_str().unwrap())
                .join("execution-case.json"),
            c,
        )?;
    }
    // Later subprocesses have filesystem access too. Recheck all identities at
    // the batch boundary rather than trusting an earlier immediate comparison.
    for (index, c) in cases.iter_mut().enumerate() {
        if c["preparation_success"] == true {
            c["invariants"] = match check(&f, index, &c["inputs"])
                .and_then(|_| pair(&f, index, &c["baseline"], &c["candidate"]))
            {
                Ok(v) => v,
                Err(e) => {
                    json!({"success":false,"failure_code":"final_pair_validation_failed","error":format!("{e:#}")})
                }
            };
            c["success"] = json!(
                c["baseline"]["success"] == true
                    && c["candidate"]["success"] == true
                    && c["invariants"]["success"] == true
            );
        }
        save(
            &f.output.join(c["case"].as_str().unwrap()).join("case.json"),
            c,
        )?;
    }
    let mut summary = summarize(&cases, &freeze, &f.plan_hash, start.elapsed().as_secs_f64());
    summary["freeze_receipt_sha256"] = json!(hash_file(&f.output.join("freeze.json"))?);
    save(&f.output.join("summary.json"), &summary)?;
    ensure!(
        summary["all_cases_successful"] == true,
        "Paired batch retained one or more failures; see receipts"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart_engine::mapping::DrumHit;
    #[test]
    fn paired_bass_exempts_only_two_code_ids_and_retains_all_musical_witnesses() {
        use chart_engine::{
            bass_analysis::{Analysis, Decoder},
            mapping::{PitchCandidateEvidence, PitchCandidateSource, PitchNote},
        };
        fn digest(value: &impl serde::Serialize) -> String {
            let mut file = tempfile::NamedTempFile::new().unwrap();
            std::io::Write::write_all(&mut file, &serde_json::to_vec(value).unwrap()).unwrap();
            hash_file(file.path()).unwrap()
        }
        let receipt = |pitch| {
            let note = PitchNote {
                start: 0.5,
                end: 1.,
                pitch,
                confidence: 0.9,
            };
            let mut events = Transcription {
                bass: vec![note.clone()],
                bass_candidates: vec![PitchCandidateEvidence {
                    candidate: note,
                    source: PitchCandidateSource::NetworkOnset,
                    onset_score: 0.9,
                    inferred_onset_score: 0.9,
                }],
                ..Default::default()
            };
            let raw = digest(&events.bass_candidates);
            events.bass_decoder = Some(Decoder {
                version: 1,
                implementation_sha256: "a".repeat(64),
                model_sha256: "b".repeat(64),
                bass_audio_sha256: "c".repeat(64),
                candidates_sha256: raw.clone(),
                short_candidates_sha256: digest(&events.bass_short_candidates),
            });
            events.bass_analysis = Some(Analysis {
                version: 1,
                implementation_sha256: "d".repeat(64),
                bass_audio_sha256: "c".repeat(64),
                raw_candidates_sha256: raw,
                refined_notes_sha256: digest(&events.bass),
                original_notes_sha256: digest(&events.bass),
                refinement_evidence_sha256: digest(&(
                    &events.bass_corrections,
                    &events.bass_timing_corrections,
                    &events.bass_shadow_support,
                    &events.bass_attack_reviews,
                    &events.bass_harmonic_conflicts,
                    &events.bass_short_candidates,
                )),
                original_notes: events.bass.clone(),
                warnings: vec!["retained warning".into()],
            });
            events.validate().unwrap();
            events
        };
        let before = receipt(40);
        let mut after = before.clone();
        after.bass_decoder.as_mut().unwrap().implementation_sha256 = "e".repeat(64);
        after.bass_analysis.as_mut().unwrap().implementation_sha256 = "f".repeat(64);
        let mut tracks = chart_engine::chart::Tracks::new();
        tracks.insert(
            "ExpertDoubleBass".into(),
            vec![chart_engine::chart::Note {
                tick: 192,
                lane: 0,
                sustain: 0,
            }],
        );
        let chart = Chart::parse(chart_engine::chart::new_chart(
            "Synthetic Bass",
            "Fixture",
            &TempoMap::constant(120.).unwrap(),
            &tracks,
        ))
        .unwrap();
        let result = paired_bass_invariants(&before, &after, &chart, &chart).unwrap();
        assert_eq!(result["semantic_fields_and_map_unchanged"], true);
        assert_eq!(result["implementation_ids_unchanged"], false);
        assert_eq!(
            result["baseline_implementation_ids"]["bass_decoder"],
            "a".repeat(64)
        );
        assert_eq!(
            result["candidate_implementation_ids"]["bass_analysis"],
            "f".repeat(64)
        );
        assert_eq!(
            result["comparison_exemptions"],
            json!([
                "/bass_decoder/implementation_sha256",
                "/bass_analysis/implementation_sha256"
            ])
        );
        assert!(!crate::recoveries::bass_unchanged(&before, &after, &chart, &chart).unwrap());
        for mode in 0..8 {
            let mut changed = after.clone();
            match mode {
                0 => changed = receipt(41),
                1 => changed.bass_decoder.as_mut().unwrap().model_sha256 = "f".repeat(64),
                2 => changed.bass_decoder.as_mut().unwrap().candidates_sha256 = "f".repeat(64),
                3 => changed.bass_analysis.as_mut().unwrap().refined_notes_sha256 = "f".repeat(64),
                4 => changed
                    .bass_analysis
                    .as_mut()
                    .unwrap()
                    .warnings
                    .push("changed warning".into()),
                5 => changed.bass_analysis.as_mut().unwrap().original_notes[0].pitch = 41,
                6 => {
                    changed.bass_decoder.as_mut().unwrap().implementation_sha256 = "invalid".into()
                }
                _ => changed.bass_analysis.as_mut().unwrap().bass_audio_sha256 = "b".repeat(64),
            }
            assert!(
                match paired_bass_invariants(&before, &changed, &chart, &chart) {
                    Err(_) => true,
                    Ok(v) => v["semantic_fields_and_map_unchanged"] == false,
                },
                "mode {mode}"
            );
        }
        let mut missing = serde_json::to_value(&after).unwrap();
        missing["bass_decoder"]
            .as_object_mut()
            .unwrap()
            .remove("implementation_sha256");
        assert!(serde_json::from_value::<Transcription>(missing).is_err());
    }
    #[test]
    fn paired_optional_oaf_preserves_legacy_models_and_rejects_present_unpinned_files() {
        let asset = chart_engine::models::assets()
            .into_iter()
            .find(|a| a.id == "drum-articulation")
            .unwrap();
        assert_eq!(asset.filename, OAF_FILENAME);
        assert_eq!(asset.sha256, OAF_SHA256);
        assert_eq!(
            asset.url,
            "https://huggingface.co/thebluescreen/oaf-drums-onnx/resolve/771fd5529062b9f6f0b58f01cc4b9819db046f4a/oaf_drums.onnx"
        );
        let dir = tempfile::tempdir().unwrap();
        for name in ["basic_pitch.onnx", "adtof_frame_rnn.onnx"] {
            fs::write(dir.path().join(name), b"identity fixture only").unwrap();
        }
        let legacy = crate::recoveries::model_hashes(dir.path()).unwrap();
        assert_eq!(paired_model_hashes(dir.path()).unwrap(), legacy);
        let optional = dir.path().join(OAF_FILENAME);
        fs::write(&optional, b"wrong byte count").unwrap();
        assert!(
            paired_model_hashes(dir.path())
                .unwrap_err()
                .to_string()
                .contains("byte count")
        );
        fs::OpenOptions::new()
            .write(true)
            .open(&optional)
            .unwrap()
            .set_len(OAF_BYTES)
            .unwrap();
        assert!(
            paired_model_hashes(dir.path())
                .unwrap_err()
                .to_string()
                .contains("checksum")
        );
        assert_eq!(crate::recoveries::model_hashes(dir.path()).unwrap(), legacy);
        fs::remove_file(&optional).unwrap();
        fs::create_dir(&optional).unwrap();
        assert!(paired_model_hashes(dir.path()).is_err());
        fs::remove_dir(&optional).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("absent-target"), &optional).unwrap();
            assert!(paired_model_hashes(dir.path()).is_err());
            fs::remove_file(optional).unwrap();
        }
    }
    #[test]
    fn articulation_summary_preserves_counts_without_exposing_reason_prose() {
        let summary = raw_summary(&json!({"drum_articulations":[
            {"proposed":"HighTom","reason":"resonance_supported","map_proposal":true},
            {"proposed":null,"reason":"low_confidence"},
            {"proposed":"Crash","reason":"PRIVATE /local/song title"}
        ],"guitar_reattack_diagnostics":{"retained":3,"PRIVATE /song":1,"fraction":0.2,"negative":-1,"prose":"PRIVATE"},"guitar_polyphonic_diagnostics":{"candidate_count":5,"PRIVATE /song":1,"fraction":0.2,"negative":-1,"prose":"PRIVATE"},"guitar_polyphonic_notes":[{"original_rejection":"overlapping_lower_harmonic"},{"original_rejection":"PRIVATE /song"}],"guitar_short_candidates":[{"reason":"source_supported"},{"reason":"PRIVATE /song"}]}));
        assert_eq!(summary["drum_articulation"]["total_proposals"], 3);
        assert_eq!(summary["drum_articulation"]["proposed_some"], 2);
        assert_eq!(summary["drum_articulation"]["map_eligible"], 1);
        assert_eq!(
            summary["drum_articulation"]["proposed_voices"],
            json!({"HighTom":1,"Crash":1})
        );
        assert_eq!(summary["drum_articulation"]["no_proposal"], 1);
        assert_eq!(summary["drum_articulation"]["reasons"]["low_confidence"], 1);
        assert_eq!(
            summary["drum_articulation"]["reasons"]["structured_or_unrecognized_reason"],
            1
        );
        assert!(!summary.to_string().contains("PRIVATE"));
        assert_eq!(
            summary["guitar_reattack_diagnostics"],
            json!({"retained":3})
        );
        assert_eq!(
            summary["guitar_polyphonic_diagnostics"],
            json!({"candidate_count":5})
        );
        assert_eq!(summary["guitar_polyphonic_notes"], 2);
        assert_eq!(
            summary["guitar_polyphonic_original_rejections"],
            json!({"overlapping_lower_harmonic":1,"structured_or_unrecognized_reason":1})
        );
        assert_eq!(
            summary["guitar_short_candidate_first_reasons"]["source_supported"],
            1
        );
    }
    #[test]
    fn generic_voice_refinement_preserves_exact_times_confidence_and_multiplicity() {
        let hit = |voice| DrumHit {
            time: 1.,
            confidence: 0.9,
            voice,
        };
        let a = Transcription {
            drums: vec![hit(Voice::Tom), hit(Voice::HighTom), hit(Voice::Cymbal)],
            ..Default::default()
        };
        let b = Transcription {
            drums: vec![hit(Voice::LowTom), hit(Voice::HighTom), hit(Voice::Ride)],
            ..Default::default()
        };
        let r = raw_drum_invariants(&a, &b);
        assert_eq!(r["confidence_and_voice_rules_preserved"], true);
        assert_eq!(r["kick_hand_time_multisets_unchanged"], true);
        for mode in 0..4 {
            let mut bad = b.clone();
            match mode {
                0 => {
                    bad.drums.pop();
                }
                1 => bad.drums[0].voice = Voice::Snare,
                2 => bad.drums[0].confidence = 0.8,
                _ => bad.drums[0].time += 0.0001,
            };
            let v = raw_drum_invariants(&a, &bad);
            assert!(
                v["confidence_and_voice_rules_preserved"] == false
                    || v["kick_hand_time_multisets_unchanged"] == false
            );
        }
    }
    #[test]
    fn standard_drum_timing_ignores_only_lanes_and_control_markers() {
        let make = |lane| {
            let mut t = chart_engine::chart::Tracks::new();
            t.insert(
                "ExpertDrums".into(),
                vec![
                    chart_engine::chart::Note {
                        tick: 192,
                        lane,
                        sustain: 0,
                    },
                    chart_engine::chart::Note {
                        tick: 192,
                        lane: 66,
                        sustain: 0,
                    },
                ],
            );
            Chart::parse(chart_engine::chart::new_chart(
                "x",
                "y",
                &TempoMap::constant(120.).unwrap(),
                &t,
            ))
            .unwrap()
        };
        assert_eq!(drum_ticks(&make(2)).unwrap(), drum_ticks(&make(3)).unwrap());
        assert_ne!(drum_ticks(&make(0)).unwrap(), drum_ticks(&make(3)).unwrap());
    }
}
