//! Offline note-agreement calibration. Never enabled in the application.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models, vocal_benchmark::Reference, vocal_generation::Generation, vocals::VocalTone,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};
use vocal_boundary_benchmark::{
    Note, read,
    rejection::{self, Example, Rejector},
};
fn save(p: impl AsRef<Path>, v: &impl serde::Serialize) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)?
        .write_all(&serde_json::to_vec(v)?)?;
    Ok(())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn consensus_keeps(region: &Value, x: &[f64]) -> bool {
    let (Some(a), Some(b), Some(ca), Some(cb)) = (
        region["a"].as_u64(),
        region["b"].as_u64(),
        region["cut_a"].as_f64(),
        region["cut_b"].as_f64(),
    ) else {
        return true;
    };
    let (Some(a), Some(b)) = (x.get(a as usize), x.get(b as usize)) else {
        return true;
    };
    !a.is_finite() || !b.is_finite() || !ca.is_finite() || !cb.is_finite() || *a > ca || *b > cb
}
fn notes(g: &Generation) -> Vec<Note> {
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
fn features(g: &Generation, p: &[[f64; 4]], n: &Note) -> Result<Vec<f64>> {
    let raw = g.rmvpe_pitch.as_ref().context("Raw")?;
    ensure!(p.len() == raw.frames.len() && !p.is_empty(), "Clock");
    let lo = ((n.start * 100.).ceil() as usize).min(p.len() - 1);
    let hi = ((n.end * 100.).ceil() as usize).max(lo + 1).min(p.len());
    let mut x = vec![];
    for head in 0..4 {
        let values = p[lo..hi].iter().map(|p| p[head]).collect::<Vec<_>>();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        x.extend([
            mean,
            values.iter().copied().fold(1f64, f64::min),
            values.iter().copied().fold(0f64, f64::max),
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt(),
        ]);
    }
    for side in [false, true] {
        let (a, b) = if side {
            (hi.min(p.len() - 1), (hi + 32).min(p.len()))
        } else {
            (lo.saturating_sub(32), lo.max(1))
        };
        for head in [0, 3] {
            x.push(p[a..b].iter().map(|p| p[head]).sum::<f64>() / (b - a) as f64);
        }
    }
    let f = &raw.frames[lo..hi];
    let rms = (f.iter().map(|f| f.rms.powi(2)).sum::<f64>() / f.len() as f64).sqrt();
    let neighbor = &raw.frames[lo.saturating_sub(32)..(hi + 32).min(p.len())];
    let background =
        (neighbor.iter().map(|f| f.rms.powi(2)).sum::<f64>() / neighbor.len() as f64).sqrt();
    let pitch = f
        .iter()
        .filter_map(|f| f.frequency_hz.map(|h| 69. + 12. * (h / 440.).log2()))
        .collect::<Vec<_>>();
    x.extend([
        (n.end - n.start).ln(),
        (n.midi as f64 - 60.) / 24.,
        f.iter().map(|f| f.confidence as f64).sum::<f64>() / f.len() as f64,
        f.iter().map(|f| f.confidence as f64).fold(0f64, f64::max),
        rms.max(1e-6).ln(),
        (rms.max(1e-6) / background.max(1e-6)).ln(),
        pitch.iter().filter(|p| p.round() == n.midi as f64).count() as f64 / f.len() as f64,
        pitch.iter().map(|p| (p - n.midi as f64).abs()).sum::<f64>() / pitch.len().max(1) as f64,
        pitch.len() as f64 / f.len() as f64,
    ]);
    ensure!(x.iter().all(|v| v.is_finite()), "Features");
    Ok(x)
}
fn fit_agreement(points: &[Example]) -> Result<Vec<Value>> {
    ensure!(
        !points.is_empty()
            && points.len() <= 50000
            && points.iter().all(|p| p.features.len() >= 15
                && p.features.iter().all(|v| v.is_finite())
                && p.recording < 16)
            && points.iter().any(|p| p.correct),
        "Invalid agreement examples"
    );
    let mut regions = vec![];
    for (a, b) in [(0usize, 12usize), (2, 14)] {
        let quantiles = |k: usize| {
            let mut v = points.iter().map(|p| p.features[k]).collect::<Vec<_>>();
            v.sort_by(f64::total_cmp);
            (1..32).map(|i| v[v.len() * i / 32]).collect::<Vec<_>>()
        };
        let mut best = json!({"a":a,"b":b,"cut_a":-1.,"cut_b":-1.,"removed":0});
        let mut best_count = 0;
        for ca in quantiles(a) {
            for cb in quantiles(b) {
                let matches = |p: &&Example| p.features[a] <= ca && p.features[b] <= cb;
                if points.iter().filter(|p| p.correct).any(|p| matches(&p)) {
                    continue;
                }
                let mut sources = [0usize; 16];
                let mut count = 0;
                for p in points.iter().filter(matches) {
                    sources[p.recording] += 1;
                    count += 1;
                }
                if count >= 20
                    && sources.iter().filter(|&&n| n >= 5).count() >= 3
                    && count > best_count
                {
                    best = json!({"a":a,"b":b,"cut_a":ca,"cut_b":cb,"removed":count});
                    best_count = count;
                }
            }
        }
        regions.push(best);
    }

    Ok(regions)
}
fn fit(
    manifest: &Path,
    frames: &Path,
    out: &Path,
    proposals: Option<&Path>,
    consensus: bool,
) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let manifest: Value = read(manifest)?;
    let frame_model: Value = read(frames.join("model.json"))?;
    let mut points = vec![];
    let mut pins = vec![];
    let mut artists = std::collections::BTreeMap::new();
    for e in manifest["recordings"].as_array().context("rows")? {
        let id = e["id"].as_str().context("id")?;
        ensure!(valid_id(id), "Invalid calibration ID");
        let artist = e["artist"].as_str().context("artist")?;
        ensure!(
            frame_model["training"]
                .as_array()
                .context("Training pins")?
                .iter()
                .all(|r| r["artist"].as_str().unwrap().trim().to_lowercase()
                    != artist.trim().to_lowercase()),
            "Calibration artist appeared in frame training"
        );
        let next = artists.len();
        let recording = *artists.entry(artist.to_owned()).or_insert(next);
        let g: Generation = read(e["generation"].as_str().context("generation")?)?;
        ensure!(
            frame_model["training"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["audio_sha256"] != g.audio_sha256),
            "Calibration source appeared in frame training"
        );
        let rp = e["reference"].as_str().context("reference")?;
        let r: Reference = read(rp)?;
        ensure!(
            r.reviewed
                && r.audio_sha256 == g.audio_sha256
                && (r.duration_seconds - g.duration_seconds).abs() < 1. / 16000.,
            "Calibration source"
        );
        let pp = frames.join(format!("{id}.probabilities.json"));
        let p: Vec<[f64; 4]> = read(&pp)?;
        let candidates = if let Some(root) = proposals {
            let path = root.join(format!("{id}.json"));
            let receipt: Value = read(root.join("receipt.json"))?;
            ensure!(
                receipt["reference_values_read"] == false
                    && receipt["rows"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["id"] == id
                            && r["proposal_sha256"] == models::hash_file(&path).unwrap()),
                "Candidate receipt"
            );
            let v: Value = read(path)?;
            ensure!(v["source_sha256"] == g.audio_sha256, "Candidate source");
            let mut notes = vec![];
            for p in v["proposals"].as_array().context("proposals")? {
                if p["penalty"] != 0.16 {
                    continue;
                }
                notes.extend(serde_json::from_value::<Vec<Note>>(p["notes"].clone())?);
            }
            notes
        } else if let Some(path) = e["baseline_generation"].as_str() {
            let candidate: Generation = read(path)?;
            ensure!(
                candidate.audio_sha256 == g.audio_sha256
                    && candidate.vocal_stem_sha256 == g.vocal_stem_sha256
                    && candidate.timing == g.timing
                    && candidate.duration_seconds == g.duration_seconds
                    && serde_json::to_value(&candidate.rmvpe_pitch)?
                        == serde_json::to_value(&g.rmvpe_pitch)?,
                "Calibration candidate source or raw pitch differs"
            );
            notes(&candidate)
        } else {
            notes(&g)
        };
        for n in candidates {
            points.push(Example {
                features: features(&g, &p, &n)?,
                correct: rejection::has_reference_support(&r, &n),
                recording,
            });
        }
        pins.push(json!({"id":id,"artist":artist,"generation_sha256":models::hash_file(Path::new(e["generation"].as_str().unwrap()))?,"candidate_generation_sha256":e["baseline_generation"].as_str().map(|p|models::hash_file(Path::new(p))).transpose()?,"reference_sha256":models::hash_file(Path::new(rp))?,"probabilities_sha256":models::hash_file(&pp)?}));
    }
    ensure!(artists.len() == 8, "Eight calibration artists");
    save(
        out.join("protocol.json"),
        &json!({"status":"FROZEN_24_ARTIST_FRAME_MODEL_PLUS_SEPARATE_8_ARTIST_CONDITIONAL_CALIBRATION","frame_model_sha256":models::hash_file(&frames.join("model.json"))?,"calibration":pins,"candidate_receipt_sha256":proposals.map(|p|models::hash_file(&p.join("receipt.json"))).transpose()?,"reference_values_read":true,"fresh_targets_read":false,"primary":"one rejection region, preexisting High same-pitch overlap>=0.5 protected; do not move notes or replace pitch","guarded_variants":[1,3,8],"unguarded_control":1,"executable_sha256":models::hash_file(&std::env::current_exe()?)?}),
    )?;
    if consensus {
        let regions = fit_agreement(&points)?;
        save(
            out.join("rejector.json"),
            &json!({"kind":"dual_head_consensus_v1","regions":regions}),
        )?;
        save(
            out.join("frozen.json"),
            &json!({"consensus":true,"rejector_sha256":models::hash_file(&out.join("rejector.json"))?,"calibration_regions":regions,"protected_removed":0,"calibration_examples":points.len(),"protected":points.iter().filter(|p|p.correct).count(),"promotion_eligible":false,"fresh_targets_read":false}),
        )?;
        return Ok(());
    }
    let rejector = rejection::fit(&points)?;
    save(out.join("rejector.json"), &rejector)?;
    let counts=[1,3,8].into_iter().map(|limit|{let model=rejector.limited(limit);json!({"limit":limit,"removed":points.iter().filter(|p|!model.keeps(&p.features)).count(),"protected_removed":points.iter().filter(|p|p.correct&&!model.keeps(&p.features)).count()})}).collect::<Vec<_>>();
    save(
        out.join("frozen.json"),
        &json!({"rejector_sha256":models::hash_file(&out.join("rejector.json"))?,"calibration_counts":counts,"calibration_examples":points.len(),"protected":points.iter().filter(|p|p.correct).count(),"promotion_eligible":false,"fresh_targets_read":false}),
    )?;
    Ok(())
}
fn predict(
    manifest: &Path,
    frames: &Path,
    frame_run: &Path,
    run: &Path,
    baseline: &Path,
    base_recipe: usize,
    out: &Path,
) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    let m: Value = read(manifest)?;
    let frozen: Value = read(run.join("frozen.json"))?;
    ensure!(
        models::hash_file(&run.join("rejector.json"))? == frozen["rejector_sha256"],
        "Rules changed"
    );
    let consensus = frozen["consensus"] == true;
    let rules: Option<Rejector> = if consensus {
        None
    } else {
        Some(read(run.join("rejector.json"))?)
    };
    let agreement: Value = if consensus {
        read(run.join("rejector.json"))?
    } else {
        Value::Null
    };
    let model: Value = read(frame_run.join("model.json"))?;
    let frame_receipt: Value = read(frames.join("receipt.json"))?;
    let protocol: Value = read(run.join("protocol.json"))?;
    let model_sha256 = models::hash_file(&frame_run.join("model.json"))?;
    ensure!(
        frame_receipt["reference_values_read"] == false
            && frame_receipt["model_sha256"] == model_sha256
            && protocol["frame_model_sha256"] == model_sha256,
        "Frozen frame model lineage differs"
    );
    let old_receipt: Value = read(baseline.join("prediction-receipt.json"))?;
    fs::create_dir(out)?;
    let mut outputs = vec![];
    let mut cuts = vec![];
    for e in m["recordings"].as_array().context("rows")? {
        let id = e["id"].as_str().context("id")?;
        ensure!(valid_id(id), "Invalid prediction ID");
        let g: Generation = read(e["generation"].as_str().context("generation")?)?;
        let path = baseline.join(format!("{id}-{base_recipe:03}.json"));
        let old: Value = read(&path)?;
        ensure!(
            old_receipt["outputs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["file"] == path.file_name().unwrap().to_str().unwrap()
                    && p["sha256"] == models::hash_file(&path).unwrap()),
            "Baseline pin"
        );
        ensure!(old["source_sha256"] == g.audio_sha256, "Source");
        let notes: Vec<Note> = serde_json::from_value(old["notes"].clone())?;
        let mixed = e["group"].as_str().unwrap().contains("mixed");
        let p: Vec<[f64; 4]> = if mixed {
            let path = frames.join(format!("{id}.probabilities.json"));
            let pin = frame_receipt["frame_observations"]
                .as_array()
                .context("Frame pins")?
                .iter()
                .find(|p| p["id"] == id)
                .context("Frame source absent")?;
            ensure!(
                pin["source_sha256"] == g.audio_sha256
                    && pin["probabilities_sha256"] == models::hash_file(&path)?,
                "Frame observations changed"
            );
            read(path)?
        } else {
            vec![]
        };
        if mixed {
            ensure!(
                model["training"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r["audio_sha256"] != g.audio_sha256),
                "Training source leaked"
            );
        }
        let high = self::notes(&g);
        for (variant, (limit, guard)) in [(1, true), (3, true), (8, true), (1, false)]
            .into_iter()
            .enumerate()
        {
            let r = rules.as_ref().map(|r| r.limited(limit));
            let keep = notes
                .iter()
                .filter(|n| {
                    !mixed || {
                        let overlap = high
                            .iter()
                            .filter(|h| h.midi == n.midi)
                            .map(|h| (n.end.min(h.end) - n.start.max(h.start)).max(0.))
                            .sum::<f64>()
                            / (n.end - n.start);
                        (guard
                            && if consensus && variant == 2 {
                                overlap > 0.
                            } else {
                                overlap >= 0.5
                            })
                            || {
                                let x = features(&g, &p, n).unwrap();
                                if consensus {
                                    let region = &agreement["regions"][usize::from(variant == 1)];
                                    consensus_keeps(region, &x)
                                } else {
                                    r.as_ref().unwrap().keeps(&x)
                                }
                            }
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            let file = format!("{id}-{variant:03}.json");
            let path = out.join(&file);
            save(
                &path,
                &json!({"id":id,"source_sha256":g.audio_sha256,"notes":keep}),
            )?;
            vocal_boundary_benchmark::roundtrip(&vocal_boundary_benchmark::emit(&g, &keep)?)?;
            outputs.push(json!({"file":file,"sha256":models::hash_file(&path)?}));
            cuts.push(json!({"id":id,"variant":variant,"removed":notes.len()-keep.len()}));
        }
    }
    save(
        out.join("prediction-receipt.json"),
        &json!({"parent_receipt_sha256":models::hash_file(&baseline.join("prediction-receipt.json"))?,"frame_receipt_sha256":models::hash_file(&frames.join("receipt.json"))?,"frame_model_sha256":model_sha256,"rule_sha256":frozen["rejector_sha256"],"outputs":outputs,"cuts":cuts,"reference_values_read":false}),
    )
}
fn main() -> Result<()> {
    let a = std::env::args().collect::<Vec<_>>();
    match a.get(1).map(String::as_str) {
        Some("fit") if a.len() == 5 => fit(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            None,
            false,
        ),
        Some("fit-proposals") if a.len() == 6 => fit(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Some(Path::new(&a[5])),
            false,
        ),
        Some("fit-consensus") if a.len() == 6 => fit(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Some(Path::new(&a[5])),
            true,
        ),
        Some("fit-consensus") if a.len() == 5 => fit(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            None,
            true,
        ),
        Some("predict") if a.len() == 9 => predict(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Path::new(&a[5]),
            Path::new(&a[6]),
            a[7].parse()?,
            Path::new(&a[8]),
        ),
        _ => anyhow::bail!(
            "fit CAL_MANIFEST FRAME_RUN OUT | fit-consensus CAL_MANIFEST FRAME_RUN OUT PROPOSALS | predict STUDY_MANIFEST FRAME_PREDICTIONS FRAME_RUN RULE_RUN BASELINE BASELINE_RECIPE OUT"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_heads_must_reject_and_protected_examples_survive() {
        let point = |a, b, correct, recording| {
            let mut features = vec![0.; 29];
            features[0] = a;
            features[12] = b;
            features[2] = a;
            features[14] = b;
            Example {
                features,
                correct,
                recording,
            }
        };
        let mut points = vec![point(0.01, 0.8, true, 0), point(0.8, 0.01, true, 1)];
        for recording in 0..3 {
            for _ in 0..8 {
                points.push(point(0.03, 0.03, false, recording));
            }
        }
        let regions = fit_agreement(&points).unwrap();
        for r in &regions {
            assert!(
                points
                    .iter()
                    .filter(|p| p.correct)
                    .all(|p| consensus_keeps(r, &p.features))
            );
            assert_eq!(
                points
                    .iter()
                    .filter(|p| !consensus_keeps(r, &p.features))
                    .count(),
                24
            );
        }
        assert!(consensus_keeps(&json!({"a":1000}), &[0.]));
    }
}
