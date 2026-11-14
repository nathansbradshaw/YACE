//! Local acoustic note/rest training. Labels never enter feature extraction or prediction.
mod segmentation;
use anyhow::{Context, Result, ensure};
use candle_core::{DType, Device, Tensor, Var};
use chart_engine::{
    midi, models, rmvpe_frontend,
    vocal_benchmark::{self as vb, Reference},
    vocal_generation::{Generation, NoteEvidence, PitchSupport, Status},
    vocal_rmvpe,
    vocals::{self, Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
const H: usize = 64;
const EPOCHS: usize = 64;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    artist: String,
    generation: PathBuf,
    stem: PathBuf,
    reference: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    recordings: Vec<Entry>,
}
#[derive(Clone, Serialize, Deserialize)]
struct FeatureMeta {
    entry: Entry,
    source_sha256: String,
    stem_sha256: String,
    generation_sha256: String,
    pcm_sha256: String,
    feature_sha256: String,
    rows: usize,
    cols: usize,
    duration: f64,
    frontend: String,
}
struct Data {
    meta: FeatureMeta,
    g: Generation,
    x: Vec<Vec<f32>>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Recipe {
    bias: f64,
    penalty: f64,
    locked: bool,
    /// Missing in existing frozen recipes: retain their original 0.16 reward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attack_reward: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    full_run: bool,
}
fn is_false(v: &bool) -> bool {
    !v
}
#[derive(Serialize, Deserialize)]
struct Model {
    version: u32,
    mean: Vec<f32>,
    sd: Vec<f32>,
    parameters: Vec<Vec<Vec<f32>>>,
    training: Vec<Value>,
    features: usize,
    hidden: usize,
    epochs: usize,
    helper_sha256: String,
}
fn read<T: serde::de::DeserializeOwned>(p: impl AsRef<Path>) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn save(p: impl AsRef<Path>, v: &impl Serialize) -> Result<()> {
    use std::io::Write;
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(p)?
        .write_all(&serde_json::to_vec(v)?)?;
    Ok(())
}
fn entries(p: &Path) -> Result<Vec<Entry>> {
    let m: Manifest = read(p)?;
    let (mut ids, mut artists) = (BTreeSet::new(), BTreeSet::new());
    for e in &m.recordings {
        ensure!(
            !e.id.is_empty()
                && e.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(e.id.clone())
                && !e.artist.trim().is_empty()
                && artists.insert(e.artist.trim().to_lowercase()),
            "Duplicate/invalid source or artist"
        );
    }
    ensure!(!m.recordings.is_empty(), "Empty manifest");
    Ok(m.recordings)
}
fn extract(manifest: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let mut receipt = vec![];
    for e in entries(manifest)? {
        let g: Generation = read(&e.generation)?;
        let raw = g
            .rmvpe_pitch
            .as_ref()
            .context("Missing original RMVPE; no substitute")?;
        vocal_rmvpe::validate(
            raw,
            &g.audio_sha256,
            &g.vocal_stem_sha256,
            g.duration_seconds,
        )?;
        ensure!(
            models::hash_file(&e.stem)? == g.vocal_stem_sha256,
            "Stem source differs"
        );
        let pcm = models::decode(&e.stem, 16000, 1)?;
        let mut hash = Sha256::new();
        for p in &pcm {
            hash.update(p.to_le_bytes());
        }
        let pcm_sha256 = format!("{:x}", hash.finalize());
        ensure!(
            pcm_sha256 == raw.pcm_sha256
                && (pcm.len() as f64 / 16000. - g.duration_seconds).abs() <= 1. / 16000.
                && raw.frames.len() == pcm.len() / 160 + 1,
            "Original PCM/clock differs"
        );
        let n = raw.frames.len();
        let mut mel = vec![vec![0f32; 128]; n];
        for start in (0..n).step_by(3000) {
            let m = rmvpe_frontend::input(&pcm, start, (start + 3000).min(n))?;
            for i in 0..m.count {
                for (b, value) in mel[start + i].iter_mut().enumerate() {
                    *value = m.values[b * m.frames + m.skip + i];
                }
            }
        }
        let mut x = vec![];
        for i in 0..n {
            let f = &raw.frames[i];
            let pitch = f
                .frequency_hz
                .filter(|_| f.confidence >= 0.03 && f.rms >= 0.0015)
                .map(|h| 69. + 12. * (h / 440.).log2());
            let mut row = mel[i].clone();
            for (next, prev) in mel[(i + 4).min(n - 1)]
                .iter()
                .zip(&mel[i.saturating_sub(4)])
            {
                row.push(next - prev);
            }
            for w in [8usize, 32] {
                for side in [false, true] {
                    let (lo, hi) = if side {
                        (i + 1, (i + w + 1).min(n))
                    } else {
                        (i.saturating_sub(w), i)
                    };
                    for band in 0..16 {
                        let mut sum = 0.;
                        for frame in &mel[lo..hi] {
                            sum += frame[band * 8..band * 8 + 8].iter().sum::<f32>() / 8.;
                        }
                        row.push(if hi > lo {
                            sum / (hi - lo) as f32
                        } else {
                            mel[i][band * 8..band * 8 + 8].iter().sum::<f32>() / 8.
                        });
                    }
                }
            }
            row.extend([
                f.confidence,
                pitch.is_some() as u8 as f32,
                (f.rms.max(1e-6).log10() / 6.) as f32,
                pitch.map_or(0., |p| (p - p.round()) as f32),
            ]);
            for w in [4usize, 16, 32] {
                let a = &raw.frames[i.saturating_sub(w)..(i + w + 1).min(n)];
                let mut p = a
                    .iter()
                    .filter(|f| f.confidence >= 0.03 && f.rms >= 0.0015)
                    .filter_map(|f| f.frequency_hz.map(|h| 69. + 12. * (h / 440.).log2()))
                    .collect::<Vec<_>>();
                p.sort_by(f64::total_cmp);
                let center = p.get(p.len() / 2).copied().unwrap_or(60.);
                let origin = pitch.unwrap_or(center);
                let sd = (p.iter().map(|v| (v - center).powi(2)).sum::<f64>()
                    / p.len().max(1) as f64)
                    .sqrt();
                row.extend([
                    ((center - origin) / 4.).clamp(-1., 1.) as f32,
                    (sd / 4.).min(1.) as f32,
                    p.len() as f32 / a.len() as f32,
                    a.iter().map(|f| f.confidence).sum::<f32>() / a.len() as f32,
                ]);
            }
            ensure!(
                row.iter().all(|v| v.is_finite()),
                "Nonfinite acoustic features"
            );
            x.push(row);
        }
        let blob = out.join(format!("{}.f32le", e.id));
        let mut bytes = Vec::with_capacity(n * x[0].len() * 4);
        for row in &x {
            for v in row {
                bytes.extend(v.to_le_bytes());
            }
        }
        fs::write(&blob, bytes)?;
        let meta = FeatureMeta {
            entry: e.clone(),
            source_sha256: g.audio_sha256.clone(),
            stem_sha256: g.vocal_stem_sha256.clone(),
            generation_sha256: models::hash_file(&e.generation)?,
            pcm_sha256,
            feature_sha256: models::hash_file(&blob)?,
            rows: n,
            cols: x[0].len(),
            duration: g.duration_seconds,
            frontend: "RUST_RMVPE_SHARED_LOGMEL128_DELTA80MS_BAND_CONTEXT80_320MS_PITCH_CONTEXT_V1"
                .into(),
        };
        save(out.join(format!("{}.json", e.id)), &meta)?;
        receipt.push(json!({"id":e.id,"feature_sha256":meta.feature_sha256,"generation_sha256":meta.generation_sha256,"raw_rmvpe_retained":true}));
        println!("Acoustic features {}: {}frames x{}", e.id, n, meta.cols);
    }
    save(
        out.join("receipt.json"),
        &json!({"reference_values_read":false,"manifest_sha256":models::hash_file(manifest)?,"rows":receipt}),
    )
}
fn load(e: &Entry, root: &Path) -> Result<Data> {
    let mut m: FeatureMeta = read(root.join(format!("{}.json", e.id)))?;
    ensure!(
        m.cols == 336
            && m.rows > 0
            && m.rows <= 400_000
            && m.entry.id == e.id
            && m.entry.artist == e.artist
            && models::hash_file(&e.generation)? == m.generation_sha256
            && models::hash_file(&e.stem)? == m.stem_sha256,
        "Feature/source identity changed"
    );
    let p = root.join(format!("{}.f32le", e.id));
    ensure!(
        models::hash_file(&p)? == m.feature_sha256,
        "Feature blob changed"
    );
    let bytes = fs::read(p)?;
    ensure!(bytes.len() == m.rows * m.cols * 4, "Feature shape differs");
    let flat = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    ensure!(
        flat.iter().all(|x| x.is_finite()),
        "Nonfinite saved features"
    );
    let x = flat.chunks_exact(m.cols).map(<[f32]>::to_vec).collect();
    // References belong to the scoring/training manifest, never the acoustic
    // cache. This permits independent annotators without re-extracting audio.
    m.entry.reference = e.reference.clone();
    Ok(Data {
        g: read(&e.generation)?,
        meta: m,
        x,
    })
}
fn forward(x: &Tensor, p: &[Var]) -> Result<Tensor> {
    Ok(x.matmul(p[0].as_tensor())?
        .broadcast_add(p[1].as_tensor())?
        .tanh()?
        .matmul(p[2].as_tensor())?
        .broadcast_add(p[3].as_tensor())?)
}
fn fit(data: &[Data], ids: &[usize], out: &Path) -> Result<Model> {
    let f = data[0].meta.cols;
    let mut rows = vec![];
    let mut ys = vec![];
    let mut weights = vec![];
    let mut training = vec![];
    for &s in ids {
        let d = &data[s];
        let rp = d
            .meta
            .entry
            .reference
            .as_ref()
            .context("Training reference missing")?;
        let r: Reference = read(rp)?;
        ensure!(
            r.reviewed
                && r.audio_sha256 == d.g.audio_sha256
                && (r.duration_seconds - d.g.duration_seconds).abs() <= 1. / 16000.,
            "Training labels have wrong source/clock"
        );
        for i in (0..d.x.len()).step_by(2) {
            let t = i as f64 * 0.01;
            let y = [
                r.notes
                    .iter()
                    .any(|n| n.start_seconds <= t && t < n.end_seconds),
                r.notes.iter().any(|n| (n.start_seconds - t).abs() <= 0.015),
                r.notes.iter().any(|n| (n.end_seconds - t).abs() <= 0.015),
            ];
            rows.push(d.x[i].clone());
            for (h, v) in y.into_iter().enumerate() {
                ys.push(v as u32);
                weights.push(
                    (if h == 0 {
                        if v { 1. } else { 2. }
                    } else if v {
                        5.
                    } else {
                        1.
                    }) / d.x.len() as f32,
                );
            }
        }
        training.push(json!({"id":d.meta.entry.id,"artist":d.meta.entry.artist,"audio_sha256":d.g.audio_sha256,"feature_sha256":d.meta.feature_sha256,"reference_sha256":models::hash_file(rp)?}));
    }
    let n = rows.len();
    let mut mean = vec![0.; f];
    let mut sd = vec![0.; f];
    for r in &rows {
        for k in 0..f {
            mean[k] += r[k] / n as f32;
        }
    }
    for r in &rows {
        for k in 0..f {
            sd[k] += (r[k] - mean[k]).powi(2) / n as f32;
        }
    }
    for v in &mut sd {
        *v = v.sqrt().max(0.01);
    }
    let values = rows
        .iter()
        .flat_map(|r| r.iter().enumerate().map(|(k, v)| (*v - mean[k]) / sd[k]))
        .collect::<Vec<_>>();
    drop(rows);
    let dev = Device::Cpu;
    let x = Tensor::from_vec(values, (n, f), &dev)?;
    let y = Tensor::from_vec(ys, (n, 3, 1), &dev)?;
    let sum = weights.iter().sum::<f32>() as f64;
    let weights = Tensor::from_vec(weights, (n, 3), &dev)?;
    let mut seed = 20261113u64;
    let mut random = |n, scale: f32| {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                ((seed >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.) as f32 * scale
            })
            .collect::<Vec<_>>()
    };
    let p = vec![
        Var::from_vec(random(f * H, (6. / (f + H) as f32).sqrt()), (f, H), &dev)?,
        Var::zeros(H, DType::F32, &dev)?,
        Var::from_vec(random(H * 6, (6. / (H + 6) as f32).sqrt()), (H, 6), &dev)?,
        Var::zeros(6, DType::F32, &dev)?,
    ];
    let mut m = p
        .iter()
        .map(|p| p.zeros_like())
        .collect::<candle_core::Result<Vec<_>>>()?;
    let mut v = m.clone();
    let mut losses = vec![];
    for epoch in 1..=EPOCHS {
        let z = forward(&x, &p)?.reshape((n, 3, 2))?;
        let log = z.broadcast_sub(&z.log_sum_exp(2)?.unsqueeze(2)?)?;
        let loss = log
            .gather(&y, 2)?
            .squeeze(2)?
            .mul(&weights)?
            .sum_all()?
            .affine(-1. / sum, 0.)?;
        let scalar = loss.to_scalar::<f32>()?;
        ensure!(scalar.is_finite(), "Nonfinite training loss");
        let grads = loss.backward()?;
        let rate =
            0.003 * (1. - 0.999f64.powi(epoch as i32)).sqrt() / (1. - 0.9f64.powi(epoch as i32));
        for j in 0..p.len() {
            let g = grads.get(p[j].as_tensor()).context("Missing gradient")?;
            m[j] = (m[j].affine(0.9, 0.)? + g.affine(0.1, 0.)?)?.detach();
            v[j] = (v[j].affine(0.999, 0.)? + g.sqr()?.affine(0.001, 0.)?)?.detach();
            p[j].set(
                &(p[j].affine(1. - 0.003 * 0.0001, 0.)?
                    - m[j]
                        .div(&v[j].sqrt()?.affine(1., 1e-8)?)?
                        .affine(rate, 0.)?)?,
            )?;
        }
        if epoch % 16 == 0 {
            losses.push(json!({"epoch":epoch,"loss":scalar}));
            println!(
                "Acoustic head {} epoch{epoch} loss{scalar:.5}",
                out.display()
            );
        }
    }
    let model = Model {
        version: 1,
        features: f,
        hidden: H,
        epochs: EPOCHS,
        mean,
        sd,
        parameters: vec![
            p[0].to_vec2()?,
            vec![p[1].to_vec1()?],
            p[2].to_vec2()?,
            vec![p[3].to_vec1()?],
        ],
        training,
        helper_sha256: models::hash_file(&std::env::current_exe()?)?,
    };
    save(out, &model)?;
    save(out.with_extension("loss.json"), &losses)?;
    Ok(model)
}
fn infer(d: &Data, m: &Model) -> Result<Vec<[f64; 3]>> {
    ensure!(
        m.version == 1
            && m.features == d.meta.cols
            && m.hidden == H
            && m.mean.len() == m.features
            && m.sd.len() == m.features
            && m.mean.iter().all(|x| x.is_finite())
            && m.parameters.len() == 4
            && m.parameters
                .iter()
                .flatten()
                .flatten()
                .all(|x| x.is_finite())
            && m.parameters[0].len() == m.features
            && m.parameters[0].iter().all(|r| r.len() == H)
            && m.parameters[1].len() == 1
            && m.parameters[1][0].len() == H
            && m.parameters[2].len() == H
            && m.parameters[2].iter().all(|r| r.len() == 6)
            && m.parameters[3].len() == 1
            && m.parameters[3][0].len() == 6
            && m.sd.iter().all(|s| s.is_finite() && *s >= 0.01),
        "Model schema differs"
    );
    let dev = Device::Cpu;
    let mut p = vec![];
    for i in 0..4 {
        let rows = &m.parameters[i];
        let shape = if i == 1 || i == 3 {
            vec![rows[0].len()]
        } else {
            vec![rows.len(), rows[0].len()]
        };
        p.push(Var::from_vec(
            rows.iter().flatten().copied().collect::<Vec<_>>(),
            shape.as_slice(),
            &dev,
        )?);
    }
    let x = Tensor::from_vec(
        d.x.iter()
            .flat_map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(k, v)| (*v - m.mean[k]) / m.sd[k])
            })
            .collect::<Vec<_>>(),
        (d.meta.rows, d.meta.cols),
        &dev,
    )?;
    let z = forward(&x, &p)?.reshape((d.meta.rows, 3, 2))?;
    let probs = z
        .broadcast_sub(&z.log_sum_exp(2)?.unsqueeze(2)?)?
        .exp()?
        .to_vec3::<f32>()?;
    Ok(probs
        .iter()
        .map(|r| [r[0][1] as f64, r[1][1] as f64, r[2][1] as f64])
        .collect())
}
fn decode(d: &Data, prob: &[[f64; 3]], r: Recipe) -> Result<Generation> {
    ensure!(
        r.attack_reward
            .is_none_or(|v| v.is_finite() && (0. ..=4.).contains(&v)),
        "Invalid attack reward"
    );
    ensure!(
        r.bias.is_finite() && r.penalty.is_finite() && r.penalty >= 0.,
        "Invalid decoding recipe"
    );
    ensure!(
        prob.iter()
            .flatten()
            .all(|p| p.is_finite() && (0. ..=1.).contains(p)),
        "Invalid head probabilities"
    );
    let raw = d.g.rmvpe_pitch.as_ref().context("Original RMVPE absent")?;
    ensure!(prob.len() == raw.frames.len(), "Prediction clock differs");
    ensure!(!prob.is_empty(), "Empty observations");
    vocal_rmvpe::validate(
        raw,
        &d.g.audio_sha256,
        &d.g.vocal_stem_sha256,
        d.g.duration_seconds,
    )?;
    let n = prob.len();
    let pitch = raw
        .frames
        .iter()
        .map(|f| {
            f.frequency_hz
                .filter(|_| f.confidence >= 0.03 && f.rms >= 0.0015)
                .map(|h| 69. + 12. * (h / 440.).log2())
                .filter(|p| (35.5..84.5).contains(p))
        })
        .collect::<Vec<_>>();
    let mut back = vec![[0; 2]; n];
    let mut cost = [0f64; 2];
    for i in 0..n {
        let p = prob[i][0].clamp(1e-6, 1. - 1e-6);
        let emissions = [
            -(1. - p).ln(),
            if pitch[i].is_some() {
                -p.ln() + r.bias
            } else {
                1e12
            },
        ];
        let mut next = [0.; 2];
        for k in 0..2 {
            let other = 1 - k;
            let transition = 1.5 * (1. - prob[i][if k == 1 { 1 } else { 2 }]);
            let previous = if i == 0 || cost[k] <= cost[other] + transition {
                k
            } else {
                other
            };
            back[i][k] = previous;
            next[k] = emissions[k]
                + if i == 0 {
                    0.
                } else {
                    cost[previous] + if previous == k { 0. } else { transition }
                };
        }
        cost = next;
    }
    let mut state = usize::from(cost[1] < cost[0]);
    let mut active = vec![false; n];
    for i in (0..n).rev() {
        active[i] = state == 1 && pitch[i].is_some();
        state = back[i][state];
    }
    let peaks = (0..n)
        .map(|i| {
            let p = prob[i][1];
            if p >= 0.1
                && (i.saturating_sub(1)..(i + 2).min(n))
                    .all(|j| j == i || prob[j][1] < p || (prob[j][1] == p && j > i))
            {
                p
            } else {
                0.
            }
        })
        .collect::<Vec<_>>();
    let clock = d.g.timing.as_ref().context("MIDI clock")?;
    let mut notes = vec![];
    let mut remaining_work = 30_000_000;
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
        let spans = if r.full_run {
            let values = pitch[a..b].iter().map(|p| p.unwrap()).collect::<Vec<_>>();
            let boundary = (0..len)
                .map(|i| r.penalty - r.attack_reward.unwrap_or(0.16) * peaks[a + i])
                .collect::<Vec<_>>();
            let fitted = segmentation::fit(&values, &boundary, remaining_work)?;
            ensure!(fitted.cost.is_finite(), "Nonfinite full-run segmentation");
            remaining_work -= fitted.evaluations;
            fitted
                .spans
                .into_iter()
                .map(|(lo, hi)| (a + lo, a + hi))
                .collect::<Vec<_>>()
        } else {
            let mut sx = vec![0.; len + 1];
            let mut sxx = sx.clone();
            for k in 0..len {
                let p = pitch[a + k].unwrap();
                sx[k + 1] = sx[k] + p;
                sxx[k + 1] = sxx[k] + p * p;
            }
            let mut cost = vec![f64::INFINITY; len + 1];
            let mut prev = vec![0; len + 1];
            cost[0] = 0.;
            for end in 1..=len {
                for start in end.saturating_sub(300)..end {
                    let count = end - start;
                    let sum = sx[end] - sx[start];
                    let sse = ((sxx[end] - sxx[start]) - sum * sum / count as f64).max(0.) * 0.01;
                    let c = cost[start]
                        + sse
                        + if start == 0 {
                            0.
                        } else {
                            r.penalty - r.attack_reward.unwrap_or(0.16) * peaks[a + start]
                        };
                    if c < cost[end] {
                        cost[end] = c;
                        prev[end] = start;
                    }
                }
            }
            let mut spans = vec![];
            let mut end = len;
            while end > 0 {
                let start = prev[end];
                spans.push((a + start, a + end));
                end = start;
            }
            spans.reverse();
            spans
        };
        for (start, end) in spans {
            let mut p = pitch[start..end]
                .iter()
                .map(|p| p.unwrap())
                .collect::<Vec<_>>();
            p.sort_by(f64::total_cmp);
            let center = p[p.len() / 2].round();
            ensure!(
                (36. ..=84.).contains(&center),
                "Pitch outside playable range; no folding/clamping"
            );
            let lo = (start as f64 * 0.01 - 0.005).max(0.);
            let hi = ((end - 1) as f64 * 0.01 + 0.005).min(d.g.duration_seconds);
            let from = clock.tick(lo).context("Onset")?;
            let to = clock.tick(hi).context("Offset")?;
            ensure!(from < to, "MIDI grid erased source evidence");
            notes.push(VocalNote {
                start: from,
                end: to,
                tone: VocalTone::Pitched { midi: center as u8 },
                lyric: Syllable::Missing,
                midi: None,
            });
        }
        a = b;
    }
    if r.locked {
        let old = d.g.chart.as_ref().context("Baseline chart")?;
        let mut additions = vec![];
        for note in &notes {
            let mut cursor = note.start;
            for old in &old.notes {
                if old.end <= cursor {
                    continue;
                }
                if old.start >= note.end {
                    break;
                }
                if cursor < old.start {
                    let mut p = note.clone();
                    p.start = cursor;
                    p.end = old.start.min(note.end);
                    additions.push(p);
                }
                cursor = cursor.max(old.end);
                if cursor >= note.end {
                    break;
                }
            }
            if cursor < note.end {
                let mut p = note.clone();
                p.start = cursor;
                additions.push(p);
            }
        }
        notes = old.notes.clone();
        notes.extend(additions);
        notes.sort_by_key(|n| (n.start, n.end));
    }
    let mut g = d.g.clone();
    g.chart = notes.first().zip(notes.last()).map(|(a, b)| VocalChart {
        version: 1,
        resolution: clock.resolution,
        source:
            "EXPERIMENTAL acoustic presence/attack/release; immutable original RMVPE; no lyrics"
                .into(),
        phrases: vec![VocalPhrase {
            midi: None,
            start: a.start,
            end: b.end,
            player: 1,
        }],
        notes: notes.clone(),
    });
    g.status = Status::ReviewRequired;
    g.algorithm = format!(
        "Experimental acoustic note/rest {}",
        serde_json::to_string(&r)?
    );
    g.note_evidence=notes.iter().filter_map(|n|if let VocalTone::Pitched{midi}=n.tone{Some(NoteEvidence{start:n.start,end:n.end,selected_midi:midi,alternatives:vec![],support:vec![PitchSupport::AcousticAlternative],uncertain:true,reasons:vec!["Acoustic note/rest research, not promoted; original source pitch measurements retained unchanged".into()],models:vec![]})}else{None}).collect();
    g.pitch_candidates.clear();
    ensure!(
        serde_json::to_value(&g.rmvpe_pitch)? == serde_json::to_value(&d.g.rmvpe_pitch)?,
        "Raw pitches changed"
    );
    Ok(g)
}
fn roundtrip(g: &Generation, p: &Path) -> Result<()> {
    if let Some(c) = &g.chart {
        c.validate()?;
        let mut map = g.timing.clone().unwrap();
        map.offset = 0.;
        let bytes = vocals::append_midi(&midi::new_midi(&map, &Default::default())?, c)?;
        let got = vocals::import_midi(&bytes)?;
        ensure!(
            c.notes.len() == got.notes.len()
                && c.notes
                    .iter()
                    .zip(&got.notes)
                    .all(|(a, b)| a.start == b.start && a.end == b.end && a.tone == b.tone),
            "MIDI roundtrip differs"
        );
        fs::write(p, bytes)?;
    }
    Ok(())
}
fn score(d: &Data, g: &Generation) -> Result<Value> {
    let rp = d
        .meta
        .entry
        .reference
        .as_ref()
        .context("Evaluation reference missing")?;
    let r: Reference = read(rp)?;
    let e = match vb::score_exported_notes(&r, g) {
        Ok(e) => e,
        Err(e) => return Ok(json!({"unavailable_reason":format!("{e:#}")})),
    };
    let p = e.note_matches as f64 / e.predicted_notes.max(1) as f64;
    let recall = e.note_matches as f64 / e.reference_notes.max(1) as f64;
    match vb::score_note_intervals(&r, g) {
        Ok(m) => Ok(
            json!({"rates":[m.pitch_accuracy,m.note_presence_recall,m.wrong_pitch_proportion,m.false_note_time_rate,if p+recall>0.{2.*p*recall/(p+recall)}else{0.}],"intervals":m,"events_and_legacy_f0":e}),
        ),
        Err(e2) => Ok(json!({"unavailable_reason":format!("{e2:#}"),"events_and_legacy_f0":e})),
    }
}
fn train(manifest: &Path, features: &Path, out: &Path) -> Result<()> {
    let entries = entries(manifest)?;
    ensure!(
        entries.len() == 4,
        "Four distinct training artists required"
    );
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let data = entries
        .iter()
        .map(|e| load(e, features))
        .collect::<Result<Vec<_>>>()?;
    let mut recipes = Vec::new();
    for locked in [false, true] {
        for bias in [0., 1., 2.] {
            for penalty in [0.08, 0.16] {
                recipes.push(Recipe {
                    bias,
                    penalty,
                    locked,
                    attack_reward: None,
                    full_run: false,
                });
                // A certain attack must be able to justify a repeated-pitch boundary.
                // Keep the legacy recipes too: stronger attacks remain experimental.
                if penalty == 0.16 {
                    recipes.push(Recipe {
                        bias,
                        penalty,
                        locked,
                        attack_reward: Some(2. * penalty),
                        full_run: false,
                    });
                }
            }
        }
    }
    let full_run = recipes
        .iter()
        .copied()
        .map(|r| Recipe {
            full_run: true,
            ..r
        })
        .collect::<Vec<_>>();
    recipes.extend(full_run);
    save(
        out.join("protocol.json"),
        &json!({"status":"TRAINING_AND_SOURCE_EXCLUDED_DEVELOPMENT_NOT_PROMOTION","reference_use":"Four trusted game-chart training sources only; each calibration source excluded from its fold; final four-source model frozen before fresh targets","manifest_sha256":models::hash_file(manifest)?,"epochs":EPOCHS,"hidden":H,"heads":["pitched-note-presence","attack","release"],"pitch_policy":"Immutable original RMVPE; classifier predicts no frequencies or note pitches; median only for MIDI after segmentation; absent F0 is a gap","recipes":recipes,"helper_sha256":models::hash_file(&std::env::current_exe()?)?}),
    )?;
    let mut predictions = vec![];
    for held in 0..4 {
        let ids = (0..4).filter(|i| *i != held).collect::<Vec<_>>();
        let m = fit(&data, &ids, &out.join(format!("fold{held}.model.json")))?;
        ensure!(
            m.training
                .iter()
                .all(|r| r["artist"] != data[held].meta.entry.artist
                    && r["audio_sha256"] != data[held].g.audio_sha256),
            "Training source leaked"
        );
        let p = infer(&data[held], &m)?;
        save(out.join(format!("fold{held}.probabilities.json")), &p)?;
        let mut row = vec![];
        for (j, r) in recipes.iter().enumerate() {
            let g = decode(&data[held], &p, *r)?;
            save(
                out.join(format!("{}-{j}.json", data[held].meta.entry.id)),
                &g,
            )?;
            roundtrip(
                &g,
                &out.join(format!("{}-{j}.mid", data[held].meta.entry.id)),
            )?;
            row.push(g);
        }
        predictions.push(row);
    }
    let mut means = vec![[0.; 5]; recipes.len()];
    let mut baseline = [0.; 5];
    let mut details = vec![];
    let mut available = 0;
    for i in 0..4 {
        let b = score(&data[i], &data[i].g)?;
        let Some(br) = b["rates"].as_array() else {
            details.push(json!({"id":data[i].meta.entry.id,"baseline":b}));
            continue;
        };
        available += 1;
        for k in 0..5 {
            baseline[k] += br[k].as_f64().unwrap();
        }
        let mut cs = vec![];
        for j in 0..recipes.len() {
            let s = score(&data[i], &predictions[i][j])?;
            for k in 0..5 {
                means[j][k] += s["rates"][k]
                    .as_f64()
                    .context("Candidate scoring unavailable")?;
            }
            cs.push(s);
        }
        details.push(json!({"id":data[i].meta.entry.id,"baseline":b,"candidates":cs}));
    }
    ensure!(available > 0, "No valid training calibration references");
    for k in 0..5 {
        baseline[k] /= available as f64;
        for m in &mut means {
            m[k] /= available as f64;
        }
    }
    let eligible = means
        .iter()
        .map(|m| {
            m[1] >= baseline[1] - 0.02
                && m[2] <= baseline[2]
                && m[3] <= baseline[3] + 0.02
                && m[4] >= baseline[4]
        })
        .collect::<Vec<_>>();
    let pool = (0..recipes.len())
        .filter(|i| eligible[*i])
        .collect::<Vec<_>>();
    let pool = if pool.is_empty() {
        (0..recipes.len()).collect()
    } else {
        pool
    };
    let chosen = *pool
        .iter()
        .min_by(|a, b| {
            ((1. - means[**a][0]) + means[**a][3])
                .total_cmp(&((1. - means[**b][0]) + means[**b][3]))
        })
        .unwrap();
    let model = fit(&data, &[0, 1, 2, 3], &out.join("model.json"))?;
    save(
        out.join("frozen.json"),
        &json!({"promotion_eligible":false,"model_sha256":models::hash_file(&out.join("model.json"))?,"recipe":recipes[chosen],"recipe_index":chosen,"development_constraints_passed":eligible[chosen],"calibration_valid_sources":available,"baseline":baseline,"means":means,"selection_objective":"Minimum macro missing-or-wrong note time plus false gap time among no-worse wrong-share/coverage/F1 profiles when available; otherwise same objective marked failed","training":model.training,"fresh_targets_read":false}),
    )?;
    save(out.join("development.json"), &details)?;
    println!(
        "Frozen game-chart acoustic model; selected recipe{chosen}, constraints{}",
        eligible[chosen]
    );
    Ok(())
}
fn predict(manifest: &Path, features: &Path, run: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    let frozen: Value = read(run.join("frozen.json"))?;
    ensure!(
        models::hash_file(&run.join("model.json"))? == frozen["model_sha256"].as_str().unwrap(),
        "Frozen model changed"
    );
    let model: Model = read(run.join("model.json"))?;
    let recipe: Recipe = serde_json::from_value(frozen["recipe"].clone())?;
    fs::create_dir(out)?;
    let mut rows = vec![];
    for e in entries(manifest)? {
        ensure!(
            model
                .training
                .iter()
                .all(|r| r["artist"].as_str().unwrap().trim().to_lowercase()
                    != e.artist.trim().to_lowercase()),
            "Fresh artist appeared in training"
        );
        let d = load(&e, features)?;
        ensure!(
            model
                .training
                .iter()
                .all(|r| r["audio_sha256"] != d.g.audio_sha256),
            "Fresh recording appeared in training"
        );
        let p = infer(&d, &model)?;
        let g = decode(&d, &p, recipe)?;
        save(out.join(format!("{}.json", e.id)), &g)?;
        save(out.join(format!("{}.probabilities.json", e.id)), &p)?;
        roundtrip(&g, &out.join(format!("{}.mid", e.id)))?;
        rows.push(json!({"id":e.id,"artist":e.artist,"source_sha256":d.g.audio_sha256,"original_rmvpe_retained":true,"prediction_sha256":models::hash_file(&out.join(format!("{}.json",e.id)))?}));
        println!("Fresh prediction {}saved without reference access", e.id);
    }
    save(
        out.join("receipt.json"),
        &json!({"model_sha256":frozen["model_sha256"],"recipe":recipe,"reference_values_read":false,"rows":rows}),
    )
}
fn evaluate(manifest: &Path, features: &Path, pred: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    let receipt: Value = read(pred.join("receipt.json"))?;
    let es = entries(manifest)?;
    ensure!(
        receipt["rows"]
            .as_array()
            .is_some_and(|r| r.len() == es.len()),
        "Incomplete prediction batch"
    );
    let mut rows = vec![];
    for e in es {
        let d = load(&e, features)?;
        let path = pred.join(format!("{}.json", e.id));
        let witness = receipt["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == e.id)
            .context("Missing prediction witness")?;
        ensure!(
            models::hash_file(&path)? == witness["prediction_sha256"].as_str().unwrap(),
            "Prediction changed before scoring"
        );
        let g: Generation = read(path)?;
        rows.push(json!({"id":e.id,"artist":e.artist,"baseline":score(&d,&d.g)?,"candidate":score(&d,&g)?}));
    }
    let paired = rows
        .iter()
        .filter(|r| r["baseline"]["rates"].is_array() && r["candidate"]["rates"].is_array())
        .collect::<Vec<_>>();
    let mut means = [[0.; 5]; 2];
    for r in &paired {
        for (i, key) in ["baseline", "candidate"].iter().enumerate() {
            for k in 0..5 {
                means[i][k] +=
                    r[key]["rates"][k].as_f64().context("Malformed score")? / paired.len() as f64;
            }
        }
    }
    let guards = !paired.is_empty()
        && means[1][1] >= means[0][1] - 0.02
        && means[1][2] <= means[0][2]
        && means[1][3] <= means[0][3] + 0.02
        && means[1][4] >= means[0][4];
    let error_reduction = if !paired.is_empty() && means[0][0] < 1. {
        Some((means[1][0] - means[0][0]) / (1. - means[0][0]))
    } else {
        None
    };
    save(
        out,
        &json!({"status":"FROZEN_FRESH_SONG_DIAGNOSTIC_NOT_AUTOMATIC_PROMOTION","promotion_eligible":false,"receipt_sha256":models::hash_file(&pred.join("receipt.json"))?,"paired_valid_recordings":paired.len(),"total_recordings":rows.len(),"rates_order":["correct_note_pitch_time","note_presence_recall","wrong_pitch_share","false_note_time_in_gaps","event_f1"],"macro_baseline":if paired.is_empty(){None}else{Some(means[0])},"macro_candidate":if paired.is_empty(){None}else{Some(means[1])},"relative_missing_or_wrong_reduction":error_reduction,"diagnostic_guards_passed":guards,"numerical_gate_passed":guards && paired.len()==rows.len() && rows.len()>=8 && error_reduction.is_some_and(|e|e>=0.2),"limitations":"Numerical summary alone never establishes independence, represented-language non-regression, mixture generalization, retention, listening or game acceptance. Invalid references remain unavailable; no zero score or target fitting.","rows":rows}),
    )
}
fn main() -> Result<()> {
    let a = std::env::args().collect::<Vec<_>>();
    match a.get(1).map(String::as_str) {
        Some("extract") if a.len() == 4 => extract(Path::new(&a[2]), Path::new(&a[3])),
        Some("train") if a.len() == 5 => {
            train(Path::new(&a[2]), Path::new(&a[3]), Path::new(&a[4]))
        }
        Some("predict") if a.len() == 6 => predict(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Path::new(&a[5]),
        ),
        Some("score") if a.len() == 6 => evaluate(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Path::new(&a[5]),
        ),
        _ => anyhow::bail!(
            "extract MANIFEST OUT | train MANIFEST FEATURES OUT | predict MANIFEST FEATURES FROZEN_RUN OUT | score MANIFEST FEATURES PREDICTIONS NEW_REPORT"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart_engine::{timing::TempoMap, vocal_generation as vg, vocal_pitch::Frame};
    fn data() -> Data {
        let mut raw = vocal_rmvpe::finish_pcm(
            &vec![0.; 3200],
            (0..=20)
                .map(|i| Frame {
                    time_seconds: i as f64 * 0.01,
                    frequency_hz: Some(440.),
                    confidence: 0.9,
                    rms: 0.1,
                })
                .collect(),
        )
        .unwrap();
        raw.audio_sha256 = "a".repeat(64);
        raw.vocal_stem_sha256 = "b".repeat(64);
        let g = Generation {
            version: 1,
            audio_sha256: raw.audio_sha256.clone(),
            vocal_stem_sha256: raw.vocal_stem_sha256.clone(),
            duration_seconds: 0.2,
            algorithm: "fixture".into(),
            timing: Some(TempoMap::constant(120.).unwrap()),
            policy: vg::Policy {
                analysis_mode: vg::AnalysisMode::HighAccuracy,
                ..Default::default()
            },
            status: Status::ReviewRequired,
            chart: None,
            pitch_candidates: vec![],
            pitch_frames: vec![],
            words: None,
            lyrics: None,
            issues: vec![],
            warnings: vec![],
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: Some(raw),
            note_evidence: vec![],
        };
        Data {
            g,
            x: vec![],
            meta: FeatureMeta {
                entry: Entry {
                    id: "test".into(),
                    artist: "singer".into(),
                    generation: PathBuf::new(),
                    stem: PathBuf::new(),
                    reference: None,
                },
                source_sha256: String::new(),
                stem_sha256: String::new(),
                generation_sha256: String::new(),
                pcm_sha256: String::new(),
                feature_sha256: String::new(),
                rows: 21,
                cols: 336,
                duration: 0.2,
                frontend: String::new(),
            },
        }
    }
    fn recipe(locked: bool) -> Recipe {
        Recipe {
            bias: 0.,
            penalty: 0.16,
            locked,
            attack_reward: Some(0.32),
            full_run: true,
        }
    }
    #[test]
    fn confident_attack_splits_repeated_pitch_without_silence() {
        let d = data();
        let mut p = vec![[0.999, 0., 0.]; 21];
        p[10][1] = 0.99;
        let g = decode(&d, &p, recipe(false)).unwrap();
        let c = g.chart.unwrap();
        assert_eq!(c.notes.len(), 2);
        assert_eq!(c.notes[0].end, c.notes[1].start);
        assert!(
            c.notes
                .iter()
                .all(|n| n.tone == VocalTone::Pitched { midi: 69 })
        );
        let plain = decode(&d, &vec![[0.999, 0., 0.]; 21], recipe(false)).unwrap();
        assert_eq!(plain.chart.unwrap().notes.len(), 1);
    }
    #[test]
    fn attacks_preserve_short_legato_semitone_changes() {
        let mut d = data();
        for f in &mut d.g.rmvpe_pitch.as_mut().unwrap().frames[8..10] {
            f.frequency_hz = Some(440. * 2f64.powf(1. / 12.));
        }
        let mut p = vec![[0.999, 0., 0.]; 21];
        p[8][1] = 0.99;
        p[10][1] = 0.99;
        let g = decode(&d, &p, recipe(false)).unwrap();
        let c = g.chart.unwrap();
        assert_eq!(c.notes.len(), 3);
        assert_eq!(c.notes[1].tone, VocalTone::Pitched { midi: 70 });
        assert_eq!(c.notes[0].end, c.notes[1].start);
        assert_eq!(c.notes[1].end, c.notes[2].start);
    }
    #[test]
    fn legacy_frozen_recipe_keeps_its_original_behavior_and_encoding() {
        let json = r#"{"bias":0.0,"penalty":0.16,"locked":false}"#;
        let legacy: Recipe = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&legacy).unwrap(), json);
        let mut p = vec![[0.999, 0., 0.]; 21];
        p[10][1] = 0.99;
        let g = decode(&data(), &p, legacy).unwrap();
        assert_eq!(g.chart.unwrap().notes.len(), 1);
        let mut bad = recipe(false);
        bad.attack_reward = Some(f64::NAN);
        assert!(decode(&data(), &p, bad).is_err());
    }
    #[test]
    fn full_run_note_exceeds_legacy_window_and_roundtrips_unchanged() {
        let mut d = data();
        let mut raw = vocal_rmvpe::finish_pcm(
            &vec![0.; 64_000],
            (0..=400)
                .map(|i| Frame {
                    time_seconds: i as f64 * 0.01,
                    frequency_hz: Some(440.),
                    confidence: 0.9,
                    rms: 0.1,
                })
                .collect(),
        )
        .unwrap();
        raw.audio_sha256 = d.g.audio_sha256.clone();
        raw.vocal_stem_sha256 = d.g.vocal_stem_sha256.clone();
        d.g.rmvpe_pitch = Some(raw);
        d.g.duration_seconds = 4.;
        let before = serde_json::to_value(&d.g.rmvpe_pitch).unwrap();
        let p = vec![[0.999, 0., 0.]; 401];
        let g = decode(&d, &p, recipe(false)).unwrap();
        let c = g.chart.as_ref().unwrap();
        assert_eq!(c.notes.len(), 1);
        assert_eq!(
            c.notes[0].end,
            d.g.timing.as_ref().unwrap().tick(4.).unwrap()
        );
        assert_eq!(serde_json::to_value(&g.rmvpe_pitch).unwrap(), before);
        let mut legacy = recipe(false);
        legacy.full_run = false;
        assert_eq!(
            decode(&d, &p, legacy).unwrap().chart.unwrap().notes.len(),
            2
        );
        let path = std::env::temp_dir().join(format!(
            "yace-acoustic-note-rest-{}-long.mid",
            std::process::id()
        ));
        assert!(!path.exists());
        roundtrip(&g, &path).unwrap();
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn missing_f0_stays_gap_even_with_certain_singing() {
        let mut d = data();
        d.g.rmvpe_pitch.as_mut().unwrap().frames[10].frequency_hz = None;
        let before = serde_json::to_value(&d.g.rmvpe_pitch).unwrap();
        let g = decode(&d, &vec![[0.999, 0., 0.]; 21], recipe(false)).unwrap();
        assert_eq!(before, serde_json::to_value(&g.rmvpe_pitch).unwrap());
        let c = g.chart.unwrap();
        assert_eq!(c.notes.len(), 2);
        let t = d.g.timing.as_ref().unwrap();
        let lo = t.tick(0.095).unwrap();
        let hi = t.tick(0.105).unwrap();
        assert!(c.notes.iter().all(|n| n.end <= lo || n.start >= hi));
        assert!(
            c.notes
                .iter()
                .all(|n| n.tone == VocalTone::Pitched { midi: 69 })
        );
    }
    #[test]
    fn rest_predictions_cannot_erase_locked_notes() {
        let mut d = data();
        let old = VocalNote {
            start: 20,
            end: 70,
            tone: VocalTone::Pitched { midi: 69 },
            lyric: Syllable::Missing,
            midi: None,
        };
        d.g.chart = Some(VocalChart {
            version: 1,
            resolution: 192,
            source: "reviewed baseline".into(),
            notes: vec![old.clone()],
            phrases: vec![],
        });
        let g = decode(&d, &vec![[0.001, 0., 0.]; 21], recipe(true)).unwrap();
        assert_eq!(
            serde_json::to_value(&g.chart.unwrap().notes).unwrap(),
            serde_json::to_value(vec![old]).unwrap()
        );
    }
    #[test]
    fn locked_additions_do_not_move_or_overlap_existing_notes() {
        let mut d = data();
        let t = d.g.timing.as_ref().unwrap();
        let old = VocalNote {
            start: t.tick(0.05).unwrap(),
            end: t.tick(0.15).unwrap(),
            tone: VocalTone::Pitched { midi: 68 },
            lyric: Syllable::Missing,
            midi: None,
        };
        d.g.chart = Some(VocalChart {
            version: 1,
            resolution: t.resolution,
            source: "baseline".into(),
            notes: vec![old.clone()],
            phrases: vec![],
        });
        let g = decode(&d, &vec![[0.999, 0., 0.]; 21], recipe(true)).unwrap();
        let c = g.chart.as_ref().unwrap();
        c.validate().unwrap();
        assert_eq!(c.notes.len(), 3);
        assert!(
            c.notes
                .iter()
                .any(|n| serde_json::to_value(n).unwrap() == serde_json::to_value(&old).unwrap())
        );
        assert!(c.notes.windows(2).all(|w| w[0].end <= w[1].start));
    }
    #[test]
    fn twenty_millisecond_note_survives_and_roundtrips() {
        let mut d = data();
        for (i, f) in
            d.g.rmvpe_pitch
                .as_mut()
                .unwrap()
                .frames
                .iter_mut()
                .enumerate()
        {
            if !(8..10).contains(&i) {
                f.frequency_hz = None;
            }
        }
        let g = decode(&d, &vec![[0.999, 0., 0.]; 21], recipe(false)).unwrap();
        assert_eq!(g.chart.as_ref().unwrap().notes.len(), 1);
        let p = std::env::temp_dir().join(format!(
            "yace-acoustic-note-rest-{}-test.mid",
            std::process::id()
        ));
        assert!(!p.exists());
        roundtrip(&g, &p).unwrap();
        fs::remove_file(p).unwrap();
    }
    #[test]
    fn malformed_probabilities_and_clock_are_rejected() {
        let d = data();
        assert!(decode(&d, &vec![[f64::NAN, 0., 0.]; 21], recipe(false)).is_err());
        assert!(decode(&d, &vec![[0.9, 0., 0.]; 20], recipe(false)).is_err());
        let mut d = data();
        d.g.rmvpe_pitch.as_mut().unwrap().audio_sha256 = "c".repeat(64);
        assert!(decode(&d, &vec![[0.9, 0., 0.]; 21], recipe(false)).is_err());
    }
    #[test]
    fn scoring_manifest_selects_annotation_without_reading_cached_reference() {
        let mut d = data();
        let root =
            std::env::temp_dir().join(format!("yace-acoustic-reference-{}", std::process::id()));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        d.meta.entry.generation = root.join("source.json");
        d.meta.entry.stem = root.join("stem.bin");
        d.meta.entry.reference = Some(root.join("unavailable-A1.json"));
        save(&d.meta.entry.generation, &d.g).unwrap();
        fs::write(&d.meta.entry.stem, b"fixture").unwrap();
        d.meta.generation_sha256 = models::hash_file(&d.meta.entry.generation).unwrap();
        d.meta.stem_sha256 = models::hash_file(&d.meta.entry.stem).unwrap();
        let blob = root.join("test.f32le");
        fs::write(&blob, vec![0u8; 21 * 336 * 4]).unwrap();
        d.meta.feature_sha256 = models::hash_file(&blob).unwrap();
        save(root.join("test.json"), &d.meta).unwrap();
        let mut entry = d.meta.entry.clone();
        entry.reference = Some(root.join("unavailable-A2.json"));
        let got = load(&entry, &root).unwrap();
        assert_eq!(got.meta.entry.reference, entry.reference);
        // Neither annotation exists: acoustic loading must not open either.
        entry.reference = None;
        assert!(load(&entry, &root).unwrap().meta.entry.reference.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
