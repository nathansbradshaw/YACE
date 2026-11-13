//! Source-excluded development study. Never a fresh validation or promotion.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::{self as vb, Reference},
    vocal_generation::Generation,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use vocal_boundary_benchmark::{Note, canonical_note_reference, emit, rates, read, roundtrip};
#[derive(Clone)]
struct Cell {
    t: f64,
    p: Option<f64>,
    c: f64,
    r: f64,
    j: f64,
}
#[derive(Clone)]
struct Data {
    id: String,
    hash: String,
    g: Generation,
    c: Vec<Cell>,
    x: Vec<Vec<f64>>,
    refs: Vec<String>,
}
fn nearest<T>(v: &[T], t: f64, time: impl Fn(&T) -> f64) -> usize {
    let i = v.partition_point(|f| time(f) < t);
    [i.saturating_sub(1), i.min(v.len() - 1)]
        .into_iter()
        .min_by(|&a, &b| (time(&v[a]) - t).abs().total_cmp(&(time(&v[b]) - t).abs()))
        .unwrap()
}
fn features(c: &[Cell]) -> Vec<Vec<f64>> {
    (0..c.len())
        .map(|i| {
            // A missing cell uses nearby evidence as its pitch origin. A fixed
            // MIDI placeholder would leak absolute register into differences.
            let center = c[i]
                .p
                .or_else(|| {
                    c[i.saturating_sub(32)..(i + 33).min(c.len())]
                        .iter()
                        .filter(|f| f.p.is_some())
                        .min_by(|a, b| (a.t - c[i].t).abs().total_cmp(&(b.t - c[i].t).abs()))
                        .and_then(|f| f.p)
                })
                .unwrap_or(60.);
            let mut x = vec![
                f64::from(c[i].p.is_some()),
                c[i].c,
                c[i].r,
                c[i].j,
                center - center.round(),
            ];
            for w in [2, 4, 8, 16, 32] {
                let stats = |lo: usize, hi: usize| {
                    let a = &c[lo..hi];
                    let n = a.len().max(1) as f64;
                    let mut p: Vec<_> = a.iter().filter_map(|v| v.p).collect();
                    p.sort_by(f64::total_cmp);
                    let m = p.get(p.len() / 2).copied().unwrap_or(center);
                    let sd = (p.iter().map(|p| (p - m).powi(2)).sum::<f64>()
                        / p.len().max(1) as f64)
                        .sqrt();
                    [
                        m - center,
                        sd,
                        p.len() as f64 / n,
                        a.iter().map(|v| v.c).sum::<f64>() / n,
                        a.iter().map(|v| v.r).sum::<f64>() / n,
                        a.iter().map(|v| v.j).sum::<f64>() / n,
                    ]
                };
                let left = stats(i.saturating_sub(w), i);
                let right = stats(i + 1, (i + w + 1).min(c.len()));
                x.extend(left);
                x.extend(right);
                x.extend([
                    right[0] - left[0],
                    (right[0] - left[0]).abs(),
                    right[4] - left[4],
                    c[i].r - left[4].min(right[4]),
                ]);
            }
            x
        })
        .collect()
}
#[derive(Clone, Serialize, Deserialize)]
enum Tree {
    Leaf(f64),
    Split {
        f: usize,
        t: f64,
        left: Box<Tree>,
        right: Box<Tree>,
    },
}
impl Tree {
    fn eval(&self, x: &[f64]) -> f64 {
        match self {
            Self::Leaf(v) => *v,
            Self::Split { f, t, left, right } => {
                if x[*f] <= *t {
                    left.eval(x)
                } else {
                    right.eval(x)
                }
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Model {
    bias: f64,
    trees: Vec<Tree>,
    features: usize,
    training_sources: Vec<String>,
}
fn sigmoid(v: f64) -> f64 {
    1. / (1. + (-v.clamp(-30., 30.)).exp())
}
impl Model {
    fn predict(&self, x: &[f64]) -> f64 {
        assert_eq!(self.features, x.len());
        sigmoid(self.bias + 0.1 * self.trees.iter().map(|t| t.eval(x)).sum::<f64>())
    }
}
struct Row {
    source: usize,
    at: usize,
    y: f64,
    w: f64,
}
fn build(
    rows: &[usize],
    bins: &[Vec<u8>],
    grad: &[f64],
    hess: &[f64],
    cuts: &[Vec<f64>],
    depth: usize,
) -> Tree {
    let g = rows.iter().map(|&i| grad[i]).sum::<f64>();
    let h = rows.iter().map(|&i| hess[i]).sum::<f64>();
    let leaf = Tree::Leaf((g / (h + 5.)).clamp(-3., 3.));
    if depth == 0 || rows.len() < 50 {
        return leaf;
    }
    let mut best = (0., 0usize, 0usize);
    for (f, ts) in cuts.iter().enumerate() {
        let mut gs = vec![0.; ts.len() + 1];
        let mut hs = gs.clone();
        let mut counts = vec![0usize; gs.len()];
        for &i in rows {
            let bin = bins[f][i] as usize;
            gs[bin] += grad[i];
            hs[bin] += hess[i];
            counts[bin] += 1;
        }
        let (mut gl, mut hl, mut n) = (0., 0., 0);
        for k in 0..ts.len() {
            gl += gs[k];
            hl += hs[k];
            n += counts[k];
            if n < 25 || rows.len() - n < 25 {
                continue;
            }
            let gain = gl * gl / (hl + 5.) + (g - gl).powi(2) / (h - hl + 5.) - g * g / (h + 5.);
            if gain > best.0 {
                best = (gain, f, k);
            }
        }
    }
    if best.0 < 0.05 {
        return leaf;
    }
    let f = best.1;
    let t = cuts[f][best.2];
    let (a, b): (Vec<_>, Vec<_>) = rows
        .iter()
        .copied()
        .partition(|&i| bins[f][i] as usize <= best.2);
    Tree::Split {
        f,
        t,
        left: Box::new(build(&a, bins, grad, hess, cuts, depth - 1)),
        right: Box::new(build(&b, bins, grad, hess, cuts, depth - 1)),
    }
}
fn train(data: &[Data], sources: &[usize], kind: usize) -> Result<Model> {
    let mut rows = vec![];
    for &s in sources {
        let r: Reference = read(&data[s].refs[0])?;
        ensure!(
            r.reviewed
                && r.audio_sha256 == data[s].hash
                && (r.duration_seconds - data[s].g.duration_seconds).abs() <= 1. / 16000.,
            "Training reference source/clock differs: {}",
            data[s].id
        );
        for (i, c) in data[s].c.iter().enumerate() {
            if i % 2 != 0 {
                continue;
            }
            let y = if kind == 0 {
                r.notes
                    .iter()
                    .any(|n| n.start_seconds <= c.t && c.t < n.end_seconds)
            } else {
                r.notes.iter().any(|n| {
                    ((if kind == 1 {
                        n.start_seconds
                    } else {
                        n.end_seconds
                    }) - c.t)
                        .abs()
                        <= 0.015
                })
            };
            rows.push(Row {
                source: s,
                at: i,
                y: f64::from(y),
                w: 1000. / data[s].c.len() as f64,
            });
        }
    }
    let nfeatures = data[0].x[0].len();
    let mut cuts = vec![];
    for f in 0..nfeatures {
        let mut v: Vec<_> = rows.iter().map(|r| data[r.source].x[r.at][f]).collect();
        v.sort_by(f64::total_cmp);
        let mut cut: Vec<_> = (1..16).map(|i| v[v.len() * i / 16]).collect();
        cut.dedup();
        cuts.push(cut);
    }
    let prior = (rows.iter().map(|r| r.y * r.w).sum::<f64>()
        / rows.iter().map(|r| r.w).sum::<f64>())
    .clamp(1e-5, 1. - 1e-5);
    let bias = (prior / (1. - prior)).ln();
    let mut pred = vec![bias; rows.len()];
    let mut trees = vec![];
    let ids: Vec<_> = (0..rows.len()).collect();
    let bins: Vec<Vec<u8>> = cuts
        .iter()
        .enumerate()
        .map(|(f, ts)| {
            rows.iter()
                .map(|r| ts.partition_point(|&t| t < data[r.source].x[r.at][f]) as u8)
                .collect()
        })
        .collect();
    for _ in 0..64 {
        let mut grad = vec![];
        let mut hess = vec![];
        for (i, r) in rows.iter().enumerate() {
            let p = sigmoid(pred[i]);
            grad.push((r.y - p) * r.w);
            hess.push((p * (1. - p)).max(1e-6) * r.w);
        }
        let tree = build(&ids, &bins, &grad, &hess, &cuts, 3);
        for (i, r) in rows.iter().enumerate() {
            pred[i] += 0.1 * tree.eval(&data[r.source].x[r.at]);
        }
        trees.push(tree);
    }
    Ok(Model {
        bias,
        trees,
        features: nfeatures,
        training_sources: sources.iter().map(|&s| data[s].hash.clone()).collect(),
    })
}
#[derive(Clone, Copy, Serialize)]
struct Recipe {
    penalty: f64,
    boundary_weight: f64,
    presence: f64,
    gap_seconds: f64,
}
fn decode(c: &[Cell], duration_seconds: f64, prob: &[[f64; 3]], r: Recipe) -> Vec<Note> {
    let n = c.len();
    let dt = 0.01;
    let mut p: Vec<_> = c.iter().map(|c| c.p).collect();
    // Only interpolate a bounded hole between agreeing source pitch observations.
    let mut i = 0;
    while i < n {
        if p[i].is_some() {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && p[i].is_none() {
            i += 1;
        }
        if a > 0
            && i < n
            && (i - a) as f64 * dt <= r.gap_seconds
            && ((p[a - 1].unwrap() - p[i].unwrap()).abs() < 0.75)
        {
            for j in a..i {
                p[j] = Some(
                    p[a - 1].unwrap()
                        + (p[i].unwrap() - p[a - 1].unwrap()) * (j - a + 1) as f64
                            / (i - a + 1) as f64,
                );
            }
        }
    }
    let mut active: Vec<_> = (0..n)
        .map(|i| p[i].is_some() && prob[i][0] >= r.presence)
        .collect();
    let mut i = 0;
    while i < n {
        if active[i] {
            i += 1;
            continue;
        }
        let a = i;
        while i < n && !active[i] {
            i += 1;
        }
        if a > 0
            && i < n
            && (i - a) as f64 * dt <= r.gap_seconds
            && p[a..i].iter().all(Option::is_some)
            && ((p[a - 1].unwrap() - p[i].unwrap()).abs() < 0.75)
        {
            active[a..i].fill(true);
        }
    }
    let peak: Vec<_> = (0..n)
        .map(|i| {
            let s = prob[i][1];
            if s >= 0.05
                && (i.saturating_sub(2)..(i + 3).min(n))
                    .all(|j| j == i || prob[j][1] < s || (prob[j][1] == s && j > i))
            {
                s
            } else {
                0.
            }
        })
        .collect();
    let mut notes: Vec<Note> = vec![];
    let mut a = 0;
    while a < n {
        if !active[a] {
            a += 1;
            continue;
        }
        let mut b = a + 1;
        while b < n && active[b] {
            b += 1;
        }
        let len = b - a;
        let mut sx = vec![0.; len + 1];
        let mut sxx = sx.clone();
        for k in 0..len {
            let v = p[a + k].unwrap();
            sx[k + 1] = sx[k] + v;
            sxx[k + 1] = sxx[k] + v * v;
        }
        let mut cost = vec![f64::INFINITY; len + 1];
        let mut back = vec![0; len + 1];
        cost[0] = 0.;
        for end in 1..=len {
            for start in end.saturating_sub(300)..end {
                let count = end - start;
                let sum = sx[end] - sx[start];
                let sse = ((sxx[end] - sxx[start]) - sum * sum / count as f64).max(0.) * dt;
                let boundary = if start == 0 {
                    0.
                } else {
                    r.penalty - r.boundary_weight * peak[a + start]
                };
                let v = cost[start] + sse + boundary;
                if v < cost[end] {
                    cost[end] = v;
                    back[end] = start;
                }
            }
        }
        let mut parts = vec![];
        let mut end = len;
        while end > 0 {
            let start = back[end];
            parts.push((a + start, a + end));
            end = start;
        }
        parts.reverse();
        for (start, end) in parts {
            let mut pitches: Vec<_> = p[start..end].iter().map(|p| p.unwrap()).collect();
            pitches.sort_by(f64::total_cmp);
            let midi = pitches[pitches.len() / 2].round().clamp(36., 84.) as u8;
            let note = Note {
                start: (c[start].t - dt / 2.).max(0.),
                end: (c[end - 1].t + dt / 2.).min(duration_seconds),
                midi,
            };
            if let Some(last) = notes.last_mut().filter(|l| {
                l.midi == midi && (l.end - note.start).abs() < 1e-6 && peak[start] < 0.1
            }) {
                last.end = note.end;
            } else {
                notes.push(note);
            }
        }
        a = b;
    }
    notes
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        (3..=5).contains(&args.len()),
        "vocal-boundary-benchmark MANIFEST NEW_OUTPUT [all-sources|solo-only] [REUSE_MODEL_RUN]"
    );
    ensure!(
        args.get(3)
            .is_none_or(|a| a == "all-sources" || a == "solo-only"),
        "Unknown training protocol"
    );
    let manifest: Value = read(&args[1])?;
    let out = Path::new(&args[2]);
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let mut data = vec![];
    let entries = manifest["recordings"]
        .as_array()
        .context("Missing recordings")?;
    ensure!(
        entries.len() == 16,
        "This exposed study requires 8 solo and 8 mixed recordings"
    );
    let mut input_pins = vec![];
    for entry in entries {
        let id = entry["id"].as_str().context("Missing ID")?;
        ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Invalid recording ID"
        );
        let refs = entry["references"]
            .as_array()
            .context("Missing references")?;
        ensure!(
            !refs.is_empty() && refs.len() <= 2 && refs.iter().all(|r| r.as_str().is_some()),
            "Invalid reference paths"
        );
        let generation_path = entry["generation"]
            .as_str()
            .context("Missing generation path")?;
        let jdc_path = entry["jdc"].as_str().context("Missing JDC path")?;
        input_pins.push(json!({"id":id,"generation_sha256":format!("{:x}",Sha256::digest(fs::read(generation_path)?)),"jdc_sha256":format!("{:x}",Sha256::digest(fs::read(jdc_path)?)),"references":refs.iter().map(|r|Ok(json!({"sha256":format!("{:x}",Sha256::digest(fs::read(r.as_str().unwrap())?))}))).collect::<Result<Vec<_>>>()?}));
        let g: Generation = read(entry["generation"].as_str().unwrap())?;
        let j: Value = read(entry["jdc"].as_str().unwrap())?;
        ensure!(
            j["source_sha256"].as_str() == Some(g.vocal_stem_sha256.as_str()),
            "JDC source differs"
        );
        let js = j["frames"].as_array().context("jdc")?;
        let raw = g.rmvpe_pitch.as_ref().context("raw")?;
        ensure!(
            (0.01..=360.).contains(&g.duration_seconds)
                && raw.frames.len() >= 2
                && raw.frames.len() <= 36_001
                && js.len() >= 2
                && js.len() <= 72_001,
            "Frame/duration budget exceeded"
        );
        ensure!(
            raw.frames
                .windows(2)
                .all(|w| (w[1].time_seconds - w[0].time_seconds - 0.01).abs() < 1e-8)
                && raw.frames.iter().all(|f| f.time_seconds.is_finite()
                    && f.time_seconds >= 0.
                    && f.rms.is_finite()
                    && f.rms >= 0.
                    && f.confidence.is_finite()
                    && (0.0..=1.).contains(&f.confidence)),
            "Invalid 100 Hz RMVPE observations"
        );
        ensure!(
            js.iter().all(|j| j["time_seconds"]
                .as_f64()
                .is_some_and(|t| t.is_finite() && t >= 0.)
                && (j["non_singing_probability"]
                    .as_f64()
                    .is_some_and(|p| (0.0..=1.).contains(&p))
                    || j["note_band_mass"].as_array().is_some_and(|a| a
                        .iter()
                        .all(|v| v.as_f64().is_some_and(|p| (0.0..=1.).contains(&p))))))
                && js
                    .windows(2)
                    .all(|w| w[0]["time_seconds"].as_f64() < w[1]["time_seconds"].as_f64()),
            "Invalid JDC observations"
        );
        ensure!(
            raw.audio_sha256 == g.audio_sha256 && raw.vocal_stem_sha256 == g.vocal_stem_sha256,
            "Raw source differs"
        );
        let c: Vec<_> = raw
            .frames
            .iter()
            .map(|f| {
                let at = nearest(js, f.time_seconds, |j| j["time_seconds"].as_f64().unwrap());
                let j = &js[at];
                let mass = j["non_singing_probability"]
                    .as_f64()
                    .map(|v| 1. - v)
                    .unwrap_or_else(|| {
                        j["note_band_mass"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .sum()
                    });
                Cell {
                    t: f.time_seconds,
                    p: f.frequency_hz
                        .filter(|_| f.confidence >= 0.03 && f.rms >= 0.0015)
                        .map(|h| 69. + 12. * (h / 440.).log2())
                        .filter(|p| (35.5..84.5).contains(p)),
                    c: f.confidence as f64,
                    r: 20. * f.rms.max(1e-8).log10(),
                    j: mass,
                }
            })
            .collect();
        let x = features(&c);
        data.push(Data {
            id: entry["id"].as_str().unwrap().into(),
            hash: g.audio_sha256.clone(),
            g,
            c,
            x,
            refs: entry["references"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().into())
                .collect(),
        });
    }
    ensure!(
        data.len() == 16 && data[..8].iter().all(|d| d.id.starts_with("solo-")),
        "Explicit exposed study only"
    );
    ensure!(
        data.iter()
            .map(|d| &d.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == 16
            && data
                .iter()
                .map(|d| &d.hash)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 16,
        "Duplicate source/ID would leak across folds"
    );
    let all_sources = args.get(3).is_some_and(|s| s == "all-sources");
    let reused: Option<Vec<Value>> = if let Some(parent) = args.get(4) {
        let root = Path::new(parent);
        let receipt: Value = read(root.join("prediction-receipt.json"))?;
        ensure!(
            receipt["feature_schema"] == "rmvpe_jdc_relative_context_v2_85",
            "Reused model feature schema differs; refit required"
        );
        let bytes = fs::read(root.join("models.json"))?;
        ensure!(
            receipt["models_sha256"] == format!("{:x}", Sha256::digest(&bytes)),
            "Reused model hash differs"
        );
        Some(serde_json::from_slice(&bytes)?)
    } else {
        None
    };
    let mut probabilities = vec![vec![]; 16];
    let mut model_records = vec![];
    for fold in 0..if all_sources { 4 } else { 5 } {
        let training: Vec<_> = (0..if all_sources { 16 } else { 8 })
            .filter(|s| fold == 4 || s % 4 != fold)
            .collect();
        let models: Vec<Model> = if let Some(saved) = &reused {
            serde_json::from_value(saved[fold]["models"].clone())?
        } else {
            let mut m = vec![];
            for kind in 0..3 {
                eprintln!("training fold {fold} target {kind}");
                m.push(train(&data, &training, kind)?);
            }
            m
        };
        let expected: Vec<_> = training.iter().map(|&s| data[s].hash.clone()).collect();
        ensure!(
            models.len() == 3
                && models
                    .iter()
                    .all(|m| m.training_sources == expected && m.features == data[0].x[0].len()),
            "Reused model training sources or feature schema differ"
        );
        for i in 0..16 {
            if (all_sources && i % 4 == fold)
                || (!all_sources && ((fold < 4 && i < 8 && i % 4 == fold) || (fold == 4 && i >= 8)))
            {
                ensure!(
                    models
                        .iter()
                        .all(|m| !m.training_sources.contains(&data[i].hash)),
                    "Source leakage"
                );
                probabilities[i] = data[i]
                    .x
                    .iter()
                    .map(|x| {
                        [
                            models[0].predict(x),
                            models[1].predict(x),
                            models[2].predict(x),
                        ]
                    })
                    .collect();
            }
        }
        model_records.push(json!({"fold":fold,"models":models}));
        fs::write(
            out.join(format!("fold-{fold}.json")),
            serde_json::to_vec(model_records.last().unwrap())?,
        )?;
    }
    fs::write(out.join("models.json"), serde_json::to_vec(&model_records)?)?;
    let mut recipes = vec![];
    for penalty in [0.04, 0.08, 0.16] {
        for boundary_weight in [0., 0.16, 0.32] {
            for presence in [0.5, 0.7, 0.8, 0.9, 0.95, 0.98] {
                for gap_seconds in [0., 0.04, 0.08] {
                    recipes.push(Recipe {
                        penalty,
                        boundary_weight,
                        presence,
                        gap_seconds,
                    });
                }
            }
        }
    }
    let mut predictions = vec![];
    let mut receipt = vec![];
    let mut probability_pins = vec![];
    for (i, d) in data.iter().enumerate() {
        let probability_bytes = serde_json::to_vec(&probabilities[i])?;
        probability_pins.push(json!({"file":format!("{}-probabilities.json",d.id),"sha256":format!("{:x}",Sha256::digest(&probability_bytes))}));
        fs::write(
            out.join(format!("{}-probabilities.json", d.id)),
            probability_bytes,
        )?;
        let mut output = vec![];
        for (rid, &r) in recipes.iter().enumerate() {
            let notes = decode(&d.c, d.g.duration_seconds, &probabilities[i], r);
            roundtrip(&emit(&d.g, &notes)?)?;
            let bytes = serde_json::to_vec(
                &json!({"id":d.id,"source_sha256":d.hash,"recipe":r,"notes":notes,"source_training_excluded":true}),
            )?;
            let name = format!("{}-{rid:03}.json", d.id);
            fs::write(out.join(&name), &bytes)?;
            receipt.push(json!({"file":name,"sha256":format!("{:x}",Sha256::digest(&bytes))}));
            output.push(notes);
        }
        predictions.push(output);
    }
    fs::write(
        out.join("prediction-receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"status":"predictions_complete_before_scoring_pass","implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("main.rs").as_slice(),include_bytes!("lib.rs").as_slice()].concat())),"executable_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?)),"models_sha256":format!("{:x}",Sha256::digest(fs::read(out.join("models.json"))?)),"training_protocol":if all_sources {"4fold source-excluded all16 exposed solo+mixed development, A1 note labels only"}else{"4fold source-excluded solo development, A1 note labels only; mixed train8solo without mixed refs"},"all_sources":all_sources,"feature_schema":"rmvpe_jdc_relative_context_v2_85","reused_models_from":args.get(4),"inputs":input_pins,"probability_outputs":probability_pins,"outputs":receipt}),
        )?,
    )?;
    let mut rows = vec![];
    for (i, d) in data.iter().enumerate() {
        for (a, rp) in d.refs.iter().enumerate() {
            let reference: Reference = read(rp)?;
            let (canonical, normalizations) = canonical_note_reference(&reference)?;
            for (rid, notes) in predictions[i].iter().enumerate() {
                let report = emit(&d.g, notes)?;
                ensure!(
                    report.chart.as_ref().map_or(0, |c| c.notes.len()) == notes.len(),
                    "Unrepresentable predicted notes"
                );
                let metrics = vb::score_exported_notes(&reference, &report)?;
                let intervals = vb::score_note_intervals(&canonical, &report)?;
                rows.push(json!({"id":d.id,"annotation":a+1,"recipe_index":rid,"events":rates(&metrics),"reference_frame_diagnostic":metrics,"reference_kind":reference.reference_kind,"note_intervals":intervals,"reference_boundary_normalizations":normalizations}));
            }
        }
    }
    fs::write(
        out.join("evaluation.json"),
        serde_json::to_vec(
            &json!({"status":"source_excluded_exposed_development_not_fresh_not_promotion","recipes":recipes,"rows":rows}),
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cells(p: &[Option<f64>]) -> Vec<Cell> {
        p.iter()
            .enumerate()
            .map(|(i, &p)| Cell {
                t: i as f64 * 0.01,
                p,
                c: 0.9,
                r: -20.,
                j: 0.99,
            })
            .collect()
    }
    fn recipe() -> Recipe {
        Recipe {
            penalty: 0.16,
            boundary_weight: 0.32,
            presence: 0.7,
            gap_seconds: 0.08,
        }
    }
    #[test]
    fn bounded_holes_need_agreeing_source_pitch() {
        let mut p = vec![Some(60.); 70];
        p[30..35].fill(None);
        let c = cells(&p);
        let prob = vec![[0.99, 0., 0.]; 70];
        let d = decode(&c, 0.7, &prob, recipe());
        assert_eq!(d.len(), 1);
        let mut r = recipe();
        r.gap_seconds = 0.04;
        assert_eq!(decode(&c, 0.7, &prob, r).len(), 2);
        p[35..].fill(Some(63.));
        assert_eq!(decode(&cells(&p), 0.7, &prob, recipe()).len(), 2);
        p[30..45].fill(None);
        assert_eq!(decode(&cells(&p), 0.7, &prob, recipe()).len(), 2);
    }
    #[test]
    fn independently_detected_attacks_keep_short_notes() {
        let p = [vec![Some(60.); 30], vec![Some(61.); 4], vec![Some(60.); 30]].concat();
        let c = cells(&p);
        let mut prob = vec![[0.99, 0., 0.]; 64];
        prob[30][1] = 0.95;
        prob[34][1] = 0.95;
        let d = decode(&c, 0.64, &prob, recipe());
        assert_eq!(d.iter().map(|n| n.midi).collect::<Vec<_>>(), [60, 61, 60]);
        assert!((d[1].end - d[1].start - 0.04).abs() < 1e-9);
    }
    #[test]
    fn smooth_vibrato_does_not_create_note_attacks() {
        let p: Vec<_> = (0..200)
            .map(|i| Some(60. + 0.8 * (i as f64 * 0.01 * 6. * std::f64::consts::TAU).sin()))
            .collect();
        let d = decode(&cells(&p), 2., &vec![[0.99, 0., 0.]; 200], recipe());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].midi, 60);
    }
    #[test]
    fn repeated_pitch_attack_and_empty_input() {
        let c = cells(&vec![Some(60.); 100]);
        let mut prob = vec![[0.99, 0., 0.]; 100];
        prob[50][1] = 0.95;
        assert_eq!(decode(&c, 1., &prob, recipe()).len(), 2);
        assert!(decode(&[], 0., &[], recipe()).is_empty());
    }
    #[test]
    fn features_use_relative_pitch() {
        let mut a = cells(
            &(0..100)
                .map(|i| Some(60. + (i as f64 / 12.).sin()))
                .collect::<Vec<_>>(),
        );
        for i in (0..5).chain(35..42).chain(90..100) {
            a[i].p = None;
        }
        let mut b = a.clone();
        for c in &mut b {
            c.p = c.p.map(|p| p + 12.);
        }
        for (a, b) in features(&a).iter().zip(features(&b)) {
            assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-10));
        }
    }
    #[test]
    fn histogram_tree_matches_brute_one_feature_split() {
        let n = 100;
        let ids: Vec<_> = (0..n).collect();
        let cuts = vec![vec![24., 49., 74.]];
        let bins = vec![
            (0..n)
                .map(|i| cuts[0].partition_point(|&t| t < i as f64) as u8)
                .collect(),
        ];
        let g: Vec<_> = (0..n).map(|i| if i < 50 { -1. } else { 1. }).collect();
        let h = vec![0.25; n];
        let t = build(&ids, &bins, &g, &h, &cuts, 1);
        assert!((t.eval(&[49.]) + 50. / 17.5).abs() < 1e-12);
        assert!((t.eval(&[50.]) - 50. / 17.5).abs() < 1e-12);
        let bytes = serde_json::to_vec(&t).unwrap();
        let copy: Tree = serde_json::from_slice(&bytes).unwrap();
        for x in [24., 49., 49.1, 50., 74.] {
            assert!((t.eval(&[x]) - copy.eval(&[x])).abs() < 1e-12);
        }
    }
}
