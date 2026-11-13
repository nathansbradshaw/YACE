//! Offline lead-pitch support research; never enabled in the application.
use anyhow::{Context, Result, ensure};
use candle_core::{DType, Device, Tensor, Var};
use chart_engine::{
    midi, models, rmvpe_frontend,
    vocal_benchmark::{self as vb, Reference},
    vocal_generation::{Generation, Status},
    vocal_rmvpe,
    vocals::{self, VocalTone},
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
const EPOCHS: usize = 48;
const HEADS: usize = 4;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    artist: String,
    generation: PathBuf,
    stem: PathBuf,
    reference: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    baseline_generation: Option<PathBuf>,
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
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Veto {
    head: usize,
    mean: f64,
    peak: f64,
}
fn compressed(p: &Path, bytes: &[u8]) -> Result<()> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(p)?;
    let mut child = Command::new("gzip")
        .args(["-n", "-1", "-c"])
        .stdin(Stdio::piped())
        .stdout(output)
        .spawn()?;
    child.stdin.take().unwrap().write_all(bytes)?;
    ensure!(child.wait()?.success(), "Compression failed");
    Ok(())
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
    let mut ids = BTreeSet::new();
    for e in &m.recordings {
        ensure!(
            !e.id.is_empty()
                && e.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(e.id.clone())
                && !e.artist.trim().is_empty(),
            "Duplicate/invalid source or artist"
        );
    }
    ensure!(!m.recordings.is_empty(), "Empty manifest");
    Ok(m.recordings)
}
fn acoustic(e: &Entry) -> Result<Data> {
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
            let sd = (p.iter().map(|v| (v - center).powi(2)).sum::<f64>() / p.len().max(1) as f64)
                .sqrt();
            row.extend([
                ((center - origin) / 4.).clamp(-1., 1.) as f32,
                (sd / 4.).min(1.) as f32,
                p.len() as f32 / a.len() as f32,
                a.iter().map(|f| f.confidence).sum::<f32>() / a.len() as f32,
            ]);
        }
        let hz_mel = |h: f64| 2595. * (1. + h / 700.).log10();
        let mel_at = |h: f64| {
            let b = (129. * (hz_mel(h) - hz_mel(30.)) / (hz_mel(8000.) - hz_mel(30.)) - 1.)
                .clamp(0., 127.);
            let k = b.floor() as usize;
            mel[i][k] as f64 * (1. - b.fract()) + mel[i][(k + 1).min(127)] as f64 * b.fract()
        };
        row.push(pitch.map_or(0., |p| ((p - 60.) / 24.) as f32));
        let level = mel[i].iter().sum::<f32>() / 128.;
        row.push(level);
        for harmonic in 1..=8 {
            let h = f.frequency_hz.unwrap_or(0.) * harmonic as f64;
            let (center, side) = if (30. ..=7200.).contains(&h) {
                (
                    mel_at(h),
                    (mel_at(h * 2f64.powf(-1. / 12.)) + mel_at(h * 2f64.powf(1. / 12.))) / 2.,
                )
            } else {
                (level as f64, level as f64)
            };
            row.extend([(center - level as f64) as f32, (center - side) as f32]);
        }
        ensure!(
            row.iter().all(|v| v.is_finite()),
            "Nonfinite acoustic features"
        );
        x.push(row);
    }

    let n = x.len();
    let mut feature_hash = Sha256::new();
    for row in &x {
        for v in row {
            feature_hash.update(v.to_le_bytes());
        }
    }
    let meta = FeatureMeta {
        entry: e.clone(),
        source_sha256: g.audio_sha256.clone(),
        stem_sha256: g.vocal_stem_sha256.clone(),
        generation_sha256: models::hash_file(&e.generation)?,
        pcm_sha256,
        feature_sha256: format!("{:x}", feature_hash.finalize()),
        rows: n,
        cols: x[0].len(),
        duration: g.duration_seconds,
        frontend: "RUST_RMVPE_SHARED_LOGMEL128_DELTA80MS_BAND_CONTEXT80_320MS_PITCH_CONTEXT_V1_PLUS_LEAD_HARMONICS18_V1"
            .into(),
    };
    Ok(Data { meta, g, x })
}
fn extract(manifest: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let mut receipt = vec![];
    for e in entries(manifest)? {
        let Data { g: _, x, mut meta } = acoustic(&e)?;
        let n = x.len();
        let blob = out.join(format!("{}.f32le.gz", e.id));
        let mut bytes = Vec::with_capacity(n * x[0].len() * 4);
        for row in &x {
            for v in row {
                bytes.extend(v.to_le_bytes());
            }
        }
        compressed(&blob, &bytes)?;

        meta.feature_sha256 = models::hash_file(&blob)?;

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
        m.cols == 354
            && m.rows > 0
            && m.rows <= 400_000
            && m.entry.id == e.id
            && m.entry.artist == e.artist
            && models::hash_file(&e.generation)? == m.generation_sha256
            && models::hash_file(&e.stem)? == m.stem_sha256,
        "Feature/source identity changed"
    );
    let p = root.join(format!("{}.f32le.gz", e.id));
    ensure!(
        models::hash_file(&p)? == m.feature_sha256,
        "Feature blob changed"
    );
    let output = std::process::Command::new("gzip")
        .args(["-dc"])
        .arg(p)
        .output()?;
    ensure!(output.status.success(), "Feature decompression failed");
    let bytes = output.stdout;
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
    m.entry.baseline_generation = e.baseline_generation.clone();
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
fn targets(r: &Reference, t: f64, frequency_hz: Option<f64>) -> [bool; HEADS] {
    [
        r.notes
            .iter()
            .any(|n| n.start_seconds <= t && t < n.end_seconds),
        r.notes.iter().any(|n| (n.start_seconds - t).abs() <= 0.015),
        r.notes.iter().any(|n| (n.end_seconds - t).abs() <= 0.015),
        frequency_hz.is_some_and(|h| {
            let pitch = (69. + 12. * (h / 440.).log2()).round();
            r.notes
                .iter()
                .any(|n| n.start_seconds <= t && t < n.end_seconds && n.midi as f64 == pitch)
        }),
    ]
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
        for i in (0..d.x.len()).step_by(4) {
            let t = i as f64 * 0.01;
            let y = targets(
                &r,
                t,
                d.g.rmvpe_pitch.as_ref().unwrap().frames[i].frequency_hz,
            );
            rows.push(d.x[i].clone());
            for (h, v) in y.into_iter().enumerate() {
                ys.push(v as u32);
                weights.push(
                    (if h == 0 || h == 3 {
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
    let y = Tensor::from_vec(ys, (n, HEADS, 1), &dev)?;
    let sum = weights.iter().sum::<f32>() as f64;
    let weights = Tensor::from_vec(weights, (n, HEADS), &dev)?;
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
        Var::from_vec(
            random(H * HEADS * 2, (6. / (H + HEADS * 2) as f32).sqrt()),
            (H, HEADS * 2),
            &dev,
        )?,
        Var::zeros(HEADS * 2, DType::F32, &dev)?,
    ];
    let mut m = p
        .iter()
        .map(|p| p.zeros_like())
        .collect::<candle_core::Result<Vec<_>>>()?;
    let mut v = m.clone();
    let mut losses = vec![];
    for epoch in 1..=EPOCHS {
        let z = forward(&x, &p)?.reshape((n, HEADS, 2))?;
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
fn infer(d: &Data, m: &Model) -> Result<Vec<[f64; HEADS]>> {
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
            && m.parameters[2].iter().all(|r| r.len() == HEADS * 2)
            && m.parameters[3].len() == 1
            && m.parameters[3][0].len() == HEADS * 2
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
    let z = forward(&x, &p)?.reshape((d.meta.rows, HEADS, 2))?;
    let probs = z
        .broadcast_sub(&z.log_sum_exp(2)?.unsqueeze(2)?)?
        .exp()?
        .to_vec3::<f32>()?;
    Ok(probs
        .iter()
        .map(|r| std::array::from_fn(|i| r[i][1] as f64))
        .collect())
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
fn note_list(g: &Generation) -> Vec<vocal_boundary_benchmark::Note> {
    let t = g.timing.as_ref().unwrap();
    g.chart
        .as_ref()
        .map(|c| {
            c.notes
                .iter()
                .filter_map(|n| {
                    if let VocalTone::Pitched { midi } = n.tone {
                        Some(vocal_boundary_benchmark::Note {
                            start: t.seconds(n.start),
                            end: t.seconds(n.end),
                            midi,
                        })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
fn veto(
    g: &Generation,
    prob: &[[f64; HEADS]],
    r: Veto,
) -> Result<(Generation, Vec<vocal_boundary_benchmark::Note>)> {
    if let Some(c) = &g.chart {
        c.validate()?;
    }
    let (_, removed) = filter_notes(g, &note_list(g), prob, r)?;
    let rejected = removed
        .iter()
        .map(|n| (n.start.to_bits(), n.end.to_bits(), n.midi))
        .collect::<BTreeSet<_>>();
    let mut out = g.clone();
    if let Some(c) = &mut out.chart {
        let clock = g.timing.as_ref().context("Original clock")?;
        c.notes.retain(|n| match n.tone {
            VocalTone::Pitched { midi } => !rejected.contains(&(
                clock.seconds(n.start).to_bits(),
                clock.seconds(n.end).to_bits(),
                midi,
            )),
            _ => true,
        });
        if c.notes.is_empty() {
            out.chart = None;
        } else if c.validate().is_err() {
            return Ok((g.clone(), vec![]));
        }
    }
    if let Some(c) = &out.chart {
        out.note_evidence.retain(|e| {
            c.notes.iter().any(|n| {
                e.start == n.start
                    && e.end == n.end
                    && matches!(n.tone,VocalTone::Pitched{midi}if midi==e.selected_midi)
            })
        });
    } else {
        out.note_evidence.clear();
    }
    out.algorithm = "EXPERIMENTAL_BROAD_LEAD_SUPPORT_EXACT_NOTE_REJECTION_V1".into();
    out.status = Status::ReviewRequired;
    Ok((out, removed))
}
fn filter_notes(
    g: &Generation,
    notes: &[vocal_boundary_benchmark::Note],
    prob: &[[f64; HEADS]],
    r: Veto,
) -> Result<(
    Vec<vocal_boundary_benchmark::Note>,
    Vec<vocal_boundary_benchmark::Note>,
)> {
    let raw = g.rmvpe_pitch.as_ref().context("Raw source")?;
    ensure!(
        prob.len() == raw.frames.len() && r.head < HEADS,
        "Veto clock"
    );
    let mut kept = vec![];
    let mut removed = vec![];
    for n in notes.iter().cloned() {
        let lo = (n.start * 100.).ceil() as usize;
        let hi = ((n.end * 100.).ceil() as usize).min(prob.len());
        let mut p = vec![];
        for (i, posterior) in prob.iter().enumerate().take(hi).skip(lo) {
            let f = &raw.frames[i];
            if f.confidence >= 0.03
                && f.rms >= 0.0015
                && f.frequency_hz
                    .is_some_and(|h| (69. + 12. * (h / 440.).log2()).round() == n.midi as f64)
            {
                p.push(posterior[r.head]);
            }
        }
        let reject = !p.is_empty()
            && p.len() as f64 >= (hi.saturating_sub(lo)) as f64 * 0.8
            && p.iter().sum::<f64>() / (p.len() as f64) < r.mean
            && p.iter().copied().fold(0f64, f64::max) < r.peak;
        if reject {
            removed.push(n)
        } else {
            kept.push(n)
        }
    }
    Ok((kept, removed))
}
fn broad_train(
    manifest: &Path,
    features: &Path,
    calibration: &Path,
    calfeatures: &Path,
    out: &Path,
) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    let train_entries = entries(manifest)?;
    let cal_entries = entries(calibration)?;
    let artists = train_entries
        .iter()
        .map(|e| e.artist.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    let cal_artists = cal_entries
        .iter()
        .map(|e| e.artist.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    ensure!(
        artists.len() == 24 && cal_artists.len() == 8 && artists.is_disjoint(&cal_artists),
        "Require frozen24+8 disjoint artists"
    );
    fs::create_dir(out)?;
    save(
        out.join("protocol.json"),
        &json!({"status":"BROAD_ARTIST_DISJOINT_CALIBRATION_NO_FRESH_TARGETS","training_manifest_sha256":models::hash_file(manifest)?,"calibration_manifest_sha256":models::hash_file(calibration)?,"training_artists":artists,"calibration_artists":cal_artists,"heads":["authored_lead_presence","attack","release","raw_candidate_pitch_matches_authored_lead"],"sampling_ms":40,"features":354,"epochs":EPOCHS,"geometry_policy":"exact baseline note deletion only; original raw F0 retained","selection":"most removed calibration notes with zero same-pitch interval overlap or strict-event-tolerance support; independently selected singing-only and pitch-aware heads","fresh_targets_read":false,"executable_sha256":models::hash_file(&std::env::current_exe()?)?}),
    )?;
    let data = train_entries
        .iter()
        .map(|e| {
            if features == Path::new("@live") {
                acoustic(e)
            } else {
                load(e, features)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let ids = (0..data.len()).collect::<Vec<_>>();
    let model = fit(&data, &ids, &out.join("model.json"))?;
    drop(data);
    let mut cal = vec![];
    let mut calibration_pins = vec![];
    for e in cal_entries {
        let d = load(&e, calfeatures)?;
        ensure!(
            model
                .training
                .iter()
                .all(|t| t["audio_sha256"] != d.g.audio_sha256),
            "Calibration source leak"
        );
        let p = infer(&d, &model)?;
        save(out.join(format!("{}.probabilities.json", e.id)), &p)?;
        let r: Reference = read(e.reference.as_ref().context("Calibration reference")?)?;
        ensure!(
            r.reviewed
                && r.audio_sha256 == d.g.audio_sha256
                && (r.duration_seconds - d.g.duration_seconds).abs() <= 1. / 16000.,
            "Calibration label source differs"
        );
        vb::score_note_intervals(&r, &d.g)
            .context("Invalid calibration reference or source chart")?;
        calibration_pins.push(json!({"id":e.id,"source_sha256":d.g.audio_sha256,"feature_sha256":d.meta.feature_sha256,"reference_sha256":models::hash_file(e.reference.as_ref().unwrap())?}));
        cal.push((d, p, r));
    }
    let mut all = vec![];
    let mut chosen = vec![];
    for head in [0usize, 3] {
        let mut best = Veto {
            head,
            mean: 0.,
            peak: 0.,
        };
        let mut best_removed = 0;
        for mean in [0., 0.01, 0.03, 0.06, 0.1, 0.15, 0.2, 0.3, 0.4] {
            for peak in [0.03, 0.06, 0.1, 0.2, 0.3, 0.4, 0.5, 0.65, 0.8] {
                if peak < mean {
                    continue;
                }
                let recipe = Veto { head, mean, peak };
                let (mut removed, mut protected) = (0usize, 0usize);
                for (d, p, r) in &cal {
                    let (_, cut) = veto(&d.g, p, recipe)?;
                    removed += cut.len();
                    protected += cut
                        .iter()
                        .filter(|n| {
                            vocal_boundary_benchmark::rejection::has_reference_support(r, n)
                        })
                        .count();
                }
                all.push(json!({"recipe":recipe,"removed":removed,"protected_removed":protected}));
                if protected == 0 && removed > best_removed {
                    best = recipe;
                    best_removed = removed;
                }
            }
        }
        chosen.push(json!({"recipe":best,"removed":best_removed}));
    }
    save(out.join("calibration.json"), &all)?;
    save(
        out.join("frozen.json"),
        &json!({"model_sha256":models::hash_file(&out.join("model.json"))?,"selected":chosen,"calibration_pins":calibration_pins,"promotion_eligible":false,"fresh_targets_read":false}),
    )?;
    Ok(())
}
fn broad_predict(manifest: &Path, features: &Path, run: &Path, out: &Path) -> Result<()> {
    ensure!(!out.exists(), "Output exists");
    let frozen: Value = read(run.join("frozen.json"))?;
    ensure!(
        models::hash_file(&run.join("model.json"))? == frozen["model_sha256"],
        "Model changed"
    );
    let model: Model = read(run.join("model.json"))?;
    fs::create_dir(out)?;
    let mut receipt = vec![];
    let mut frame_observations = vec![];
    for e in entries(manifest)? {
        ensure!(
            model
                .training
                .iter()
                .all(|r| r["artist"].as_str().unwrap().trim().to_lowercase()
                    != e.artist.trim().to_lowercase()),
            "Artist leak"
        );
        let d = load(&e, features)?;
        ensure!(
            model
                .training
                .iter()
                .all(|r| r["audio_sha256"] != d.g.audio_sha256),
            "Source leak"
        );
        let p = infer(&d, &model)?;
        save(out.join(format!("{}.probabilities.json", e.id)), &p)?;
        frame_observations.push(json!({"id":e.id,"artist":e.artist,"source_sha256":d.g.audio_sha256,"feature_sha256":d.meta.feature_sha256,"probabilities_sha256":models::hash_file(&out.join(format!("{}.probabilities.json",e.id)))?}));
        let baseline_sha256 = e
            .baseline_generation
            .as_ref()
            .map(|p| models::hash_file(p))
            .transpose()?;
        for c in frozen["selected"].as_array().unwrap() {
            let recipe: Veto = serde_json::from_value(c["recipe"].clone())?;
            let baseline = if let Some(path) = &e.baseline_generation {
                let g: Generation = read(path)?;
                ensure!(
                    g.audio_sha256 == d.g.audio_sha256
                        && g.vocal_stem_sha256 == d.g.vocal_stem_sha256
                        && g.timing == d.g.timing
                        && g.duration_seconds == d.g.duration_seconds
                        && serde_json::to_value(&g.rmvpe_pitch)?
                            == serde_json::to_value(&d.g.rmvpe_pitch)?,
                    "Baseline generation source, clock or raw pitch differs"
                );
                g
            } else {
                d.g.clone()
            };
            let (g, cut) = veto(&baseline, &p, recipe)?;
            let path = out.join(format!("{}-head{}.json", e.id, recipe.head));
            save(&path, &g)?;
            roundtrip(&g, &path.with_extension("mid"))?;
            receipt.push(json!({"id":e.id,"head":recipe.head,"path":path,"prediction_sha256":models::hash_file(&path)?,"removed":cut.len(),"source_sha256":g.audio_sha256,"baseline_generation_sha256":baseline_sha256}));
        }
        ensure!(
            e.baseline_generation
                .as_ref()
                .map(|p| models::hash_file(p))
                .transpose()?
                == baseline_sha256,
            "Baseline changed during prediction"
        );
        println!("Frozen broad prediction {}", e.id);
    }
    save(
        out.join("receipt.json"),
        &json!({"reference_values_read":false,"model_sha256":frozen["model_sha256"],"frame_observations":frame_observations,"rows":receipt}),
    )
}
fn main() -> Result<()> {
    let a = std::env::args().collect::<Vec<_>>();
    match a.get(1).map(String::as_str) {
        Some("extract") if a.len() == 4 => extract(Path::new(&a[2]), Path::new(&a[3])),
        Some("train") if a.len() == 7 => broad_train(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Path::new(&a[5]),
            Path::new(&a[6]),
        ),
        Some("predict") if a.len() == 6 => broad_predict(
            Path::new(&a[2]),
            Path::new(&a[3]),
            Path::new(&a[4]),
            Path::new(&a[5]),
        ),
        _ => anyhow::bail!(
            "extract MANIFEST OUT | train TRAIN_MANIFEST TRAIN_FEATURES CALIBRATION_MANIFEST CALIBRATION_FEATURES OUT | predict MANIFEST FEATURES FROZEN_RUN OUT"
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chart_engine::{timing::TempoMap, vocal_generation as vg, vocal_pitch::Frame};
    #[test]
    fn singing_does_not_label_an_octave_error_as_correct_pitch() {
        let r = Reference {
            reference_kind: "authored_chart".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 1.,
            provenance: "fixture".into(),
            reviewed: true,
            frames: vec![],
            notes: vec![chart_engine::vocal_benchmark::ReferenceNote {
                start_seconds: 0.,
                end_seconds: 1.,
                midi: 69,
            }],
            words: vec![],
        };
        assert_eq!(targets(&r, 0.5, Some(440.)), [true, false, false, true]);
        assert_eq!(targets(&r, 0.5, Some(880.)), [true, false, false, false]);
        assert_eq!(targets(&r, 0.5, None), [true, false, false, false]);
    }
    #[test]
    fn rejection_preserves_retained_lyrics_and_scoring_phrases() {
        let d = data();
        let mut g = vocal_boundary_benchmark::emit(
            &d.g,
            &[
                vocal_boundary_benchmark::Note {
                    start: 0.02,
                    end: 0.08,
                    midi: 69,
                },
                vocal_boundary_benchmark::Note {
                    start: 0.10,
                    end: 0.18,
                    midi: 70,
                },
            ],
        )
        .unwrap();
        let c = g.chart.as_mut().unwrap();
        c.notes[1].lyric = vocals::Syllable::Text {
            text: "keep".into(),
            join_next: false,
        };
        c.phrases[0].player = 2;
        let expected = c.notes[1].clone();
        let phrases = c.phrases.clone();
        let (out, cut) = veto(
            &g,
            &vec![[0.9, 0., 0., 0.001]; 21],
            Veto {
                head: 3,
                mean: 0.2,
                peak: 0.4,
            },
        )
        .unwrap();
        assert_eq!(cut.len(), 1);
        let c = out.chart.unwrap();
        assert_eq!(c.notes, vec![expected]);
        assert_eq!(c.phrases, phrases);
    }
    #[test]
    fn rejection_cannot_orphan_known_word_continuations() {
        let d = data();
        let mut g = vocal_boundary_benchmark::emit(
            &d.g,
            &[
                vocal_boundary_benchmark::Note {
                    start: 0.02,
                    end: 0.08,
                    midi: 69,
                },
                vocal_boundary_benchmark::Note {
                    start: 0.10,
                    end: 0.18,
                    midi: 70,
                },
            ],
        )
        .unwrap();
        let c = g.chart.as_mut().unwrap();
        c.notes[0].lyric = vocals::Syllable::Text {
            text: "stay".into(),
            join_next: false,
        };
        c.notes[1].lyric = vocals::Syllable::ContinueWord;
        c.validate().unwrap();
        let (out, cut) = veto(
            &g,
            &vec![[0.9, 0., 0., 0.001]; 21],
            Veto {
                head: 3,
                mean: 0.2,
                peak: 0.4,
            },
        )
        .unwrap();
        assert!(cut.is_empty());
        assert_eq!(out.chart, g.chart);
    }
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
                    baseline_generation: None,
                },
                source_sha256: String::new(),
                stem_sha256: String::new(),
                generation_sha256: String::new(),
                pcm_sha256: String::new(),
                feature_sha256: String::new(),
                rows: 21,
                cols: 354,
                duration: 0.2,
                frontend: String::new(),
            },
        }
    }
    #[test]
    fn rejection_retains_supported_or_unknown_pitch_notes_exactly() {
        let d = data();
        let ns = vec![
            vocal_boundary_benchmark::Note {
                start: 0.02,
                end: 0.08,
                midi: 69,
            },
            vocal_boundary_benchmark::Note {
                start: 0.10,
                end: 0.18,
                midi: 70,
            },
        ];
        let g = vocal_boundary_benchmark::emit(&d.g, &ns).unwrap();
        let mut probabilities = vec![[0.9, 0., 0., 0.001]; 21];
        probabilities[4][3] = 0.9;
        let (retained, cut) = veto(
            &g,
            &probabilities,
            Veto {
                head: 3,
                mean: 0.2,
                peak: 0.4,
            },
        )
        .unwrap();
        assert!(cut.is_empty());
        assert_eq!(
            serde_json::to_value(&retained.chart).unwrap(),
            serde_json::to_value(&g.chart).unwrap()
        );
        probabilities[4][3] = 0.001;
        let (retained, cut) = veto(
            &g,
            &probabilities,
            Veto {
                head: 3,
                mean: 0.2,
                peak: 0.4,
            },
        )
        .unwrap();
        assert_eq!(cut.len(), 1);
        assert_eq!(note_list(&retained)[0].midi, 70);
        assert_eq!(
            serde_json::to_value(retained.rmvpe_pitch).unwrap(),
            serde_json::to_value(g.rmvpe_pitch).unwrap()
        );
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
        let blob = root.join("test.f32le.gz");
        compressed(&blob, &vec![0u8; 21 * 354 * 4]).unwrap();
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
