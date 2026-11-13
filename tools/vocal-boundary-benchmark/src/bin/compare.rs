//! Paired musical-note retention audit; predictions must precede references.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::{self as vb, Metrics, Reference},
    vocal_generation::Generation,
    vocals::VocalTone,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};
use vocal_boundary_benchmark::{Note, canonical_note_reference, emit, rates, read, roundtrip};
fn load(root: &Path, id: &str, rid: usize, source_sha256: &str) -> Result<Vec<Note>> {
    let file = format!("{id}-{rid:03}.json");
    ensure!(
        fs::metadata(root.join(&file))?.len() <= 128 * 1024 * 1024,
        "Prediction exceeds budget"
    );
    let bytes = fs::read(root.join(&file))?;
    let receipt: Value = read(root.join("prediction-receipt.json"))?;
    let pins: Vec<_> = receipt["outputs"]
        .as_array()
        .context("Pins")?
        .iter()
        .filter(|p| p["file"] == file)
        .collect();
    ensure!(pins.len() == 1, "Missing or duplicate prediction pin");
    ensure!(
        pins[0]["sha256"] == format!("{:x}", Sha256::digest(&bytes)),
        "Prediction changed"
    );
    let v: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        v["id"] == id && v["source_sha256"] == source_sha256,
        "Prediction recording differs"
    );
    Ok(serde_json::from_value(v["notes"].clone())?)
}
fn exported(g: &Generation) -> Vec<Note> {
    let t = g.timing.as_ref().unwrap();
    g.chart
        .as_ref()
        .map(|c| {
            c.notes
                .iter()
                .filter_map(|n| match n.tone {
                    VocalTone::Pitched { midi } => Some(Note {
                        start: t.seconds(n.start),
                        end: t.seconds(n.end),
                        midi,
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}
fn matched(r: &Reference, n: &[Note]) -> BTreeSet<usize> {
    let mut used = BTreeSet::new();
    let mut hit = BTreeSet::new();
    for (i, t) in r.notes.iter().enumerate() {
        let best = n
            .iter()
            .enumerate()
            .filter(|(j, n)| {
                !used.contains(j)
                    && n.midi == t.midi
                    && (n.start - t.start_seconds).abs() <= 0.05
                    && (n.end - t.end_seconds).abs()
                        <= 0.05_f64.max(0.2 * (t.end_seconds - t.start_seconds))
            })
            .min_by(|a, b| {
                (a.1.start - t.start_seconds)
                    .abs()
                    .total_cmp(&(b.1.start - t.start_seconds).abs())
            });
        if let Some((j, _)) = best {
            used.insert(j);
            hit.insert(i);
        }
    }
    hit
}
fn lost_time(r: &Reference, old: &[Note], new: &[Note]) -> f64 {
    let mut lost = 0.;
    for n in old {
        for t in &r.notes {
            if n.midi != t.midi {
                continue;
            }
            let a = n.start.max(t.start_seconds);
            let b = n.end.min(t.end_seconds);
            if a < b {
                let recovered = new
                    .iter()
                    .filter(|n| n.midi == t.midi)
                    .map(|n| (b.min(n.end) - a.max(n.start)).max(0.))
                    .sum::<f64>();
                lost += (b - a - recovered).max(0.);
            }
        }
    }
    lost
}
fn event_recall(m: &Metrics) -> f64 {
    m.note_matches as f64 / m.reference_notes.max(1) as f64
}
fn check_inputs(
    receipt: &Value,
    e: &Value,
    source_bytes: &[u8],
    baseline_receipt_sha256: &str,
) -> Result<()> {
    if let Some(inputs) = receipt["inputs"].as_array() {
        let input: Vec<_> = inputs.iter().filter(|i| i["id"] == e["id"]).collect();
        ensure!(input.len() == 1, "Missing or duplicate source input pin");
        ensure!(
            input[0]["generation_sha256"] == format!("{:x}", Sha256::digest(source_bytes)),
            "Source report changed"
        );
        let refs = e["references"].as_array().context("References")?;
        let pins = input[0]["references"]
            .as_array()
            .context("Reference pins")?;
        ensure!(refs.len() == pins.len(), "Reference set changed");
        for (r, pin) in refs.iter().zip(pins) {
            ensure!(
                pin["sha256"]
                    == format!(
                        "{:x}",
                        Sha256::digest(fs::read(r.as_str().context("Reference path")?)?)
                    ),
                "Reference changed since predictions"
            );
        }
    } else {
        ensure!(
            receipt["parent_receipt_sha256"] == baseline_receipt_sha256
                || receipt["parent_sha256"] == baseline_receipt_sha256,
            "Candidate lacks input pins or a verified baseline parent"
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().collect();
    ensure!(
        a.len() == 7,
        "compare MANIFEST BASELINE_RUN BASELINE_RECIPE CANDIDATE_RUN CANDIDATE_RECIPE NEW_REPORT"
    );
    let manifest: Value = read(&a[1])?;
    let oldroot = Path::new(&a[2]);
    let oldrid: usize = a[3].parse()?;
    let newroot = Path::new(&a[4]);
    let rid: usize = a[5].parse()?;
    let oldreceipt: Value = read(oldroot.join("prediction-receipt.json"))?;
    let newreceipt: Value = read(newroot.join("prediction-receipt.json"))?;
    ensure!(
        oldreceipt["inputs"].is_array(),
        "Baseline must pin source reports and references"
    );
    let old_receipt_sha256 = format!(
        "{:x}",
        Sha256::digest(fs::read(oldroot.join("prediction-receipt.json"))?)
    );
    let entries = manifest["recordings"]
        .as_array()
        .context("Missing recordings")?;
    ensure!(
        entries.len() == 16,
        "Paired exposed study requires eight solo and eight mixed sources"
    );
    let mut ids = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    let mut prepared = vec![];
    // Verify and export every frozen prediction before opening reference labels.
    for (i, e) in entries.iter().enumerate() {
        let id = e["id"].as_str().context("Missing ID")?;
        ensure!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && id.starts_with(if i < 8 { "solo-" } else { "mixed-" })
                && ids.insert(id),
            "Invalid or duplicate study ID"
        );
        let generation_path = e["generation"].as_str().context("Missing generation")?;
        let g: Generation = read(generation_path)?;
        let source_bytes = fs::read(generation_path)?;
        check_inputs(&oldreceipt, e, &source_bytes, &old_receipt_sha256)?;
        check_inputs(&newreceipt, e, &source_bytes, &old_receipt_sha256)?;
        ensure!(hashes.insert(g.audio_sha256.clone()), "Duplicate source");
        let old = emit(&g, &load(oldroot, id, oldrid, &g.audio_sha256)?)?;
        let new = emit(&g, &load(newroot, id, rid, &g.audio_sha256)?)?;
        roundtrip(&old)?;
        roundtrip(&new)?;
        prepared.push((old, new));
    }
    let mut rows = vec![];
    let mut reference_pins = vec![];
    for (i, (e, (old, new))) in entries.iter().zip(&prepared).enumerate() {
        let id = e["id"].as_str().unwrap();
        let (oldnotes, newnotes) = (exported(old), exported(new));
        let refs = e["references"].as_array().context("Missing references")?;
        ensure!(
            refs.len() == if i < 8 { 2 } else { 1 },
            "Expected two solo annotations and one mixed reference"
        );
        for (annotator, rp) in refs.iter().enumerate() {
            let rp = rp.as_str().context("Invalid reference path")?;
            let r: Reference = read(rp)?;
            ensure!(
                r.reviewed
                    && r.audio_sha256 == old.audio_sha256
                    && (r.duration_seconds - old.duration_seconds).abs() <= 1. / 16000.,
                "Unreviewed or different reference source/clock"
            );
            reference_pins.push(json!({"id":id,"annotation":annotator+1,"reference_kind":r.reference_kind,"sha256":format!("{:x}",Sha256::digest(fs::read(rp)?))}));
            let (canon, normalized) = canonical_note_reference(&r)?;
            let om = vb::score_exported_notes(&r, old)?;
            let nm = vb::score_exported_notes(&r, new)?;
            let oi = vb::score_note_intervals(&canon, old)?;
            let ni = vb::score_note_intervals(&canon, new)?;
            let oldmatched = matched(&r, &oldnotes);
            let newmatched = matched(&r, &newnotes);
            ensure!(
                oldmatched.len() == om.note_matches && newmatched.len() == nm.note_matches,
                "Auxiliary event matching differs from owning scorer"
            );
            let lost: Vec<_> = oldmatched.difference(&newmatched).copied().collect();
            let gained: Vec<_> = newmatched.difference(&oldmatched).copied().collect();
            let lost_correct_seconds = lost_time(&canon, &oldnotes, &newnotes);
            let no_event_regression =
                rates(&nm)["f1"].as_f64().unwrap() + 1e-12 >= rates(&om)["f1"].as_f64().unwrap();
            rows.push(json!({"id":id,"annotation":annotator+1,"reference_kind":r.reference_kind,"baseline":{"events":rates(&om),"note_intervals":oi,"event_recall":event_recall(&om)},"candidate":{"events":rates(&nm),"note_intervals":ni,"event_recall":event_recall(&nm)},"lost_previously_matched_reference_notes":lost,"gained_reference_notes":gained,"lost_previously_correct_pitch_seconds":lost_correct_seconds,"reference_boundary_normalizations":normalized,"per_recording_no_regression":no_event_regression&&lost.is_empty()&&lost_correct_seconds<=1e-8&&ni.note_presence_recall+1e-12>=oi.note_presence_recall&&ni.pitch_accuracy+1e-12>=oi.pitch_accuracy&&ni.false_note_time_rate<=oi.false_note_time_rate+1e-12&&ni.wrong_pitch_proportion<=oi.wrong_pitch_proportion+1e-12}));
        }
    }
    let mut sums = serde_json::Map::new();
    for group in ["solo", "mixed"] {
        for annotation in 1..=if group == "solo" { 2 } else { 1 } {
            let rr: Vec<_> = rows
                .iter()
                .filter(|r| {
                    r["id"].as_str().unwrap().starts_with(group) && r["annotation"] == annotation
                })
                .collect();
            let means = |view: &str| {
                let mean = |field: &str| {
                    rr.iter()
                        .map(|r| r[view]["note_intervals"][field].as_f64().unwrap())
                        .sum::<f64>()
                        / rr.len() as f64
                };
                let f1 = rr
                    .iter()
                    .map(|r| r[view]["events"]["f1"].as_f64().unwrap())
                    .sum::<f64>()
                    / rr.len() as f64;
                json!({"pitch_accuracy":mean("pitch_accuracy"),"recall":mean("note_presence_recall"),"wrong":mean("wrong_pitch_proportion"),"false":mean("false_note_time_rate"),"event_F1":f1,"event_recall":rr.iter().map(|r|r[view]["event_recall"].as_f64().unwrap()).sum::<f64>()/rr.len()as f64})
            };
            let old = means("baseline");
            let new = means("candidate");
            let lost: usize = rr
                .iter()
                .map(|r| {
                    r["lost_previously_matched_reference_notes"]
                        .as_array()
                        .unwrap()
                        .len()
                })
                .sum();
            let lost_seconds = rr
                .iter()
                .map(|r| r["lost_previously_correct_pitch_seconds"].as_f64().unwrap())
                .sum::<f64>();
            let gates = old["false"].as_f64().unwrap() > 0.
                && new["false"].as_f64().unwrap() <= old["false"].as_f64().unwrap() * 0.8
                && ["pitch_accuracy", "recall", "event_F1", "event_recall"]
                    .iter()
                    .all(|f| new[f].as_f64().unwrap() + 1e-12 >= old[f].as_f64().unwrap())
                && new["wrong"].as_f64().unwrap() <= old["wrong"].as_f64().unwrap() + 1e-12
                && lost == 0
                && lost_seconds <= 1e-8
                && rr.iter().all(|r| r["per_recording_no_regression"] == true);
            sums.insert(format!("{group}-A{annotation}"),json!({"n":rr.len(),"baseline":old,"candidate":new,"lost_reference_notes":lost,"gained_reference_notes":rr.iter().map(|r|r["gained_reference_notes"].as_array().unwrap().len()).sum::<usize>(),"lost_previously_correct_pitch_seconds":lost_seconds,"passes_development_retention_guard":gates,"all_recordings_no_regression":rr.iter().all(|r|r["per_recording_no_regression"]==true)}));
        }
    }
    let result = json!({"status":"PAIRED_EXPOSED_DEVELOPMENT_AUDIT_NOT_PROMOTION","promotion_eligible":false,"baseline_recipe":oldrid,"candidate_recipe":rid,"baseline_receipt_sha256":format!("{:x}",Sha256::digest(fs::read(oldroot.join("prediction-receipt.json"))?)),"candidate_receipt_sha256":format!("{:x}",Sha256::digest(fs::read(newroot.join("prediction-receipt.json"))?)),"executable_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?)),"manifest_sha256":format!("{:x}",Sha256::digest(fs::read(&a[1])?)),"implementation_sha256":format!("{:x}",Sha256::digest(include_bytes!("compare.rs"))),"reference_pins":reference_pins,"summary":sums,"rows":rows});
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&a[6])?;
    serde_json::to_writer_pretty(output, &result)?;
    println!("{}", serde_json::to_string_pretty(&result["summary"])?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart_engine::vocal_benchmark::ReferenceNote;
    fn note(start: f64, end: f64, midi: u8) -> Note {
        Note { start, end, midi }
    }
    fn reference(notes: &[Note]) -> Reference {
        Reference {
            reference_kind: "authored_chart".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 10.,
            provenance: "Synthetic retention test".into(),
            reviewed: true,
            frames: vec![],
            words: vec![],
            notes: notes
                .iter()
                .map(|n| ReferenceNote {
                    start_seconds: n.start,
                    end_seconds: n.end,
                    midi: n.midi,
                })
                .collect(),
        }
    }
    #[test]
    fn net_match_gain_keeps_the_identity_of_a_lost_real_note() {
        let truth = vec![note(1., 1.5, 60), note(2., 2.5, 62), note(3., 3.5, 64)];
        let r = reference(&truth);
        let old = matched(&r, &truth[..1]);
        let new = matched(&r, &truth[1..]);
        assert_eq!(old.difference(&new).copied().collect::<Vec<_>>(), [0]);
        assert_eq!(new.difference(&old).copied().collect::<Vec<_>>(), [1, 2]);
        assert!(new.len() > old.len());
    }
    #[test]
    fn pitch_identity_and_short_note_timing_are_not_relaxed() {
        let r = reference(&[note(1., 1.1, 60)]);
        assert_eq!(matched(&r, &[note(1.049, 1.149, 60)]).len(), 1);
        assert!(matched(&r, &[note(1.051, 1.151, 60)]).is_empty());
        assert!(matched(&r, &[note(1., 1.1, 72)]).is_empty());
        let repeated = reference(&[note(1., 1.1, 60), note(1.01, 1.11, 60)]);
        assert_eq!(matched(&repeated, &[note(1., 1.1, 60)]).len(), 1);
    }
    #[test]
    fn wrong_pitch_does_not_replace_previously_correct_time() {
        let r = reference(&[note(0., 2., 60)]);
        let old = vec![note(0., 2., 60)];
        let new = vec![note(0., 1., 60), note(1., 2., 72)];
        assert!((lost_time(&r, &old, &new) - 1.).abs() < 1e-12);
        assert!(lost_time(&r, &old, &[note(0., 1., 60), note(1., 2., 60)]) < 1e-12);
    }
    #[test]
    fn equal_total_correct_time_does_not_hide_a_local_loss() {
        let r = reference(&[note(0., 2., 60)]);
        let old = vec![note(0., 1., 60)];
        let new = vec![note(1., 2., 60)];
        assert_eq!(lost_time(&r, &old, &new), 1.);
    }
    #[test]
    fn edited_predictions_wrong_sources_and_duplicate_pins_are_rejected() {
        let root = std::env::temp_dir().join(format!(
            "yace-retention-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let file = "mixed-1-002.json";
        let bytes = serde_json::to_vec(
            &json!({"id":"mixed-1","source_sha256":"a","notes":[note(1.,2.,60)]}),
        )
        .unwrap();
        fs::write(root.join(file), &bytes).unwrap();
        let pin = json!({"file":file,"sha256":format!("{:x}",Sha256::digest(&bytes))});
        fs::write(
            root.join("prediction-receipt.json"),
            serde_json::to_vec(&json!({"outputs":[pin]})).unwrap(),
        )
        .unwrap();
        assert_eq!(load(&root, "mixed-1", 2, "a").unwrap().len(), 1);
        assert!(load(&root, "mixed-1", 2, "b").is_err());
        fs::write(root.join(file), b"{}").unwrap();
        assert!(load(&root, "mixed-1", 2, "a").is_err());
        fs::write(root.join(file), &bytes).unwrap();
        fs::write(
            root.join("prediction-receipt.json"),
            serde_json::to_vec(&json!({"outputs":[pin,pin]})).unwrap(),
        )
        .unwrap();
        assert!(load(&root, "mixed-1", 2, "a").is_err());
        fs::remove_file(root.join(file)).unwrap();
        fs::remove_file(root.join("prediction-receipt.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
