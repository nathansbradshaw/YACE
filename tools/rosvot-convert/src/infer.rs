//! Bounded Rust inference experiment, independent of lyric text and reference charts.
use anyhow::{Context, Result, ensure};
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command, time::Instant};
const RATE: usize = 24000;
const HOP: usize = 128;
const FFT: usize = 512;
fn reflect(i: isize, n: usize) -> usize {
    let p = 2 * (n - 1);
    let x = i.rem_euclid(p as isize) as usize;
    if x < n { x } else { p - x }
}
fn mel(pcm: &[f32]) -> Vec<f32> {
    let hzmel = |f: f64| {
        if f < 1000. {
            f / (200. / 3.)
        } else {
            15. + (f / 1000.).ln() / (6.4f64.ln() / 27.)
        }
    };
    let melhz = |m: f64| {
        if m < 15. {
            m * (200. / 3.)
        } else {
            1000. * ((m - 15.) * (6.4f64.ln() / 27.)).exp()
        }
    };
    let lo = hzmel(30.);
    let hi = hzmel(12000.);
    let points: Vec<_> = (0..82)
        .map(|i| melhz(lo + (hi - lo) * i as f64 / 81.))
        .collect();
    let filters: Vec<Vec<(usize, f32)>> = (0..40)
        .map(|m| {
            (0..=256)
                .filter_map(|b| {
                    let hz = b as f64 * RATE as f64 / FFT as f64;
                    let w = ((hz - points[m]) / (points[m + 1] - points[m]))
                        .min((points[m + 2] - hz) / (points[m + 2] - points[m + 1]))
                        .max(0.)
                        * 2.
                        / (points[m + 2] - points[m]);
                    (w > 0.).then_some((b, w as f32))
                })
                .collect()
        })
        .collect();
    let frames = pcm.len().div_ceil(HOP);
    let mut out = vec![0.; frames * 40];
    let fft = FftPlanner::new().plan_fft_forward(FFT);
    for i in 0..frames {
        let mut x: Vec<_> = (0..FFT)
            .map(|j| {
                let a = pcm[reflect((i * HOP + j) as isize - 192, pcm.len())].clamp(-1., 1.);
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / FFT as f32).cos();
                Complex32::new(a * w, 0.)
            })
            .collect();
        fft.process(&mut x);
        let mag: Vec<_> = x
            .iter()
            .take(257)
            .map(|c| (c.norm_sqr() + 1e-9).sqrt())
            .collect();
        for (m, row) in filters.iter().enumerate() {
            out[i * 40 + m] = row
                .iter()
                .map(|&(b, w)| w * mag[b])
                .sum::<f32>()
                .max(1e-5)
                .log10()
        }
    }
    out
}
fn session(p: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(p)?)
}
fn regulate(logits: &[f32], count: usize, threshold: f32, reference: Option<&[f32]>) -> Vec<f32> {
    let mut bd = vec![0.; logits.len()];
    let mut start = None;
    let mut last = None;
    for (i, &v) in logits.iter().enumerate() {
        if 1. / (1. + (-v).exp()) > threshold {
            start.get_or_insert(i);
        } else if let Some(a) = start.take() {
            let mut idx = (a..i)
                .max_by(|&a, &b| logits[a].total_cmp(&logits[b]))
                .unwrap();
            if let Some(l) = last
                && l > 0
                && idx - l < 17
            {
                idx = ((idx + l) as f64 / 2.).round_ties_even() as usize;
                bd[l] = 0.
            }
            bd[idx] = 1.;
            last = Some(idx)
        }
    }
    if let Some(r) = reference {
        for (j, &v) in r.iter().enumerate() {
            if v == 0. {
                continue;
            }
            let a = j.saturating_sub(8);
            let b = (j + 8).min(bd.len());
            let n = bd[a..b].iter().filter(|&&v| v > 0.).count();
            if n == 1 && bd[j] == 0. {
                bd[a..b].copy_from_slice(&r[a..b])
            } else if n > 1 {
                for k in 1..=8 {
                    let a = j.saturating_sub(k);
                    let b = (j + k).min(bd.len() - 1);
                    if bd[a] > 0. && r[a] == 0. {
                        bd[a] = 0.;
                        break;
                    }
                    if bd[b] > 0. && r[b] == 0. {
                        bd[b] = 0.;
                        break;
                    }
                }
            }
            bd[j] = 1.;
        }
    }
    bd[0] = 0.;
    bd[count.saturating_sub(1)..].fill(0.);
    bd
}
fn hash(p: &Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(p)?)))
}
pub fn run(
    stem: &Path,
    report: &Path,
    model: &Path,
    word_model: &Path,
    output: &Path,
    seconds: Option<f64>,
) -> Result<()> {
    ensure!(!output.exists(), "Use a new output file");
    let timer = Instant::now();
    let report: Value = serde_json::from_slice(&fs::read(report)?)?;
    ensure!(
        report["vocal_stem_sha256"].as_str() == Some(hash(stem)?.as_str()),
        "Stem differs from pitch report"
    );
    let evidence = &report["rmvpe_pitch"];
    ensure!(
        evidence["vocal_stem_sha256"] == report["vocal_stem_sha256"]
            && evidence["model_sha256"]
                == "5370e71ac80af8b4b7c793d27efd51fd8bf962de3a7ede0766dac0befa3660fd",
        "Pitch-conditioning source/model differs"
    );
    for (path, sha) in [
        (
            model.join("encoder.onnx"),
            "09e88a7cd4497d130e87e1abca59974bfaf80d705f89c88ae8ba71d256635f6f",
        ),
        (
            model.join("decoder.onnx"),
            "6b3b678a34a78ab8c418160f43b30c427cca415595003451472ac0403259e911",
        ),
        (
            word_model.join("encoder.onnx"),
            "20aa5fc99e405655800531f72a0cc5ca789dbb82e462d6eae4bbc4d74cde7962",
        ),
    ] {
        ensure!(
            hash(&path)? == sha,
            "Unpinned note model {}",
            path.display()
        )
    }
    let frames = evidence["frames"].as_array().context("RMVPE frames")?;
    ensure!(!frames.is_empty(), "Empty RMVPE contour");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-i"])
        .arg(stem)
        .args(["-ac", "1", "-ar", "24000", "-f", "f32le", "pipe:1"]);
    let decoded = cmd.output()?;
    ensure!(decoded.status.success(), "Audio decode failed");
    ensure!(decoded.stdout.len() % 4 == 0, "PCM bytes");
    let mut pcm: Vec<_> = decoded
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    ensure!(
        pcm.len() > 512 && pcm.len() <= 3600 * RATE && pcm.iter().all(|x| x.is_finite()),
        "Invalid audio"
    );
    if let Some(s) = seconds {
        ensure!(s.is_finite() && s > 0., "Invalid excerpt");
        pcm.truncate((s * RATE as f64) as usize)
    }
    let mel = mel(&pcm);
    let count = mel.len() / 40;
    let mut encoder = session(&model.join("encoder.onnx"))?;
    let mut decoder = session(&model.join("decoder.onnx"))?;
    let mut rwbd = session(&word_model.join("encoder.onnx"))?;
    let mut notes = vec![];
    let mut diagnostics = vec![];
    // 16 s core + 2 s context; chunk behaviour is experimental, not upstream equivalence.
    let core = 3000;
    let context = 375;
    for start in (0..count).step_by(core) {
        let end = (start + core).min(count);
        let left = start.saturating_sub(context);
        let right = (end + context).min(count);
        let n = right - left;
        let padded = n.div_ceil(16) * 16;
        let mut m = vec![0.; padded * 40];
        m[..n * 40].copy_from_slice(&mel[left * 40..right * 40]);
        let mut pitch = vec![0.; padded];
        let mut uv = vec![0f32; padded];
        for j in 0..n {
            let t = (left + j) as f64 * HOP as f64 / RATE as f64;
            let at = ((t / 0.01).round() as usize).min(frames.len() - 1);
            let row = &frames[at];
            let hz = row["frequency_hz"].as_f64().unwrap_or(0.);
            let conf = row["confidence"].as_f64().unwrap_or(0.);
            if hz > 0. && conf > 0.03 {
                let f = hz.clamp(50., 900.);
                let h = 1127. * (1. + f / 700.).ln();
                let lo = 1127. * (1. + 50f64 / 700.).ln();
                let hi = 1127. * (1. + 900f64 / 700.).ln();
                pitch[j] = ((h - lo) * 254. / (hi - lo) + 1.)
                    .clamp(1., 255.)
                    .round_ties_even() as f32
            } else {
                pitch[j] = 1.;
                uv[j] = 1.
            }
        }
        let mt = Tensor::from_array(([1, padded, 40], m.clone().into_boxed_slice()))?;
        let pt = Tensor::from_array(([1, padded], pitch.clone().into_boxed_slice()))?;
        let ut = Tensor::from_array(([1, padded], uv.clone().into_boxed_slice()))?;
        let wr = rwbd.run(ort::inputs![mt, pt, ut])?;
        let (_, wb) = wr[0].try_extract_tensor::<f32>()?;
        ensure!(
            wb.len() == padded && wb.iter().all(|v| v.is_finite()),
            "Word boundary output"
        );
        let word = regulate(wb, n, 0.5, None);
        drop(wr);
        let mt = Tensor::from_array(([1, padded, 40], m.into_boxed_slice()))?;
        let pt = Tensor::from_array(([1, padded], pitch.into_boxed_slice()))?;
        let ut = Tensor::from_array(([1, padded], uv.into_boxed_slice()))?;
        let wt = Tensor::from_array(([1, padded], word.clone().into_boxed_slice()))?;
        let enc = encoder.run(ort::inputs![mt, pt, ut, wt])?;
        let (_, bd) = enc[0].try_extract_tensor::<f32>()?;
        let (_, feat) = enc[1].try_extract_tensor::<f32>()?;
        let (_, attn) = enc[2].try_extract_tensor::<f32>()?;
        ensure!(
            bd.len() == padded
                && feat.len() == padded * 256
                && attn.len() == padded * 4
                && feat.iter().chain(bd).chain(attn).all(|v| v.is_finite()),
            "Encoder shape/numeric contract"
        );
        let boundary = regulate(bd, n, 0.85, Some(&word));
        let boundaries: Vec<_> = std::iter::once(0)
            .chain(
                boundary
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &v)| (v > 0.).then_some(i)),
            )
            .chain(std::iter::once(n))
            .collect();
        let nn = boundaries.len() - 1;
        let mut aggregate = vec![0.; nn * 256];
        for (ni, pair) in boundaries.windows(2).enumerate() {
            let mut den = 1e-5;
            for j in pair[0]..pair[1] {
                let a = attn[j * 4..j * 4 + 4]
                    .iter()
                    .map(|v| 1. / (1. + (-v).exp()))
                    .sum::<f32>()
                    / 4.;
                den += a;
                for c in 0..256 {
                    aggregate[ni * 256 + c] += a * feat[j * 256 + c]
                }
            }
            for c in 0..256 {
                aggregate[ni * 256 + c] /= den
            }
        }
        drop(enc);
        let t = Tensor::from_array(([1, nn, 256], aggregate.into_boxed_slice()))?;
        let dec = decoder.run(ort::inputs![t])?;
        let (_, logits) = dec[0].try_extract_tensor::<f32>()?;
        ensure!(
            logits.len() == nn * 89 && logits.iter().all(|v| v.is_finite()),
            "Decoder output"
        );
        for (i, pair) in boundaries.windows(2).enumerate() {
            let row = &logits[i * 89..(i + 1) * 89];
            let p = (0..89).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
            let a = left + pair[0];
            let b = left + pair[1];
            let mid = (a + b) / 2;
            if mid >= start && mid < end {
                let alternatives: Vec<_> = row.iter().enumerate().map(|(m, &v)| (m, v)).collect();
                if (30..=85).contains(&p) {
                    notes.push(json!({"start_seconds":a as f64*128./24000.,"end_seconds":b as f64*128./24000.,"midi":p,"logits":alternatives}))
                }
            }
        }
        diagnostics.push(json!({"start_frame":start,"end_frame":end,"context_left":left,"context_right":right,"note_segments":nn,"elapsed_seconds":timer.elapsed().as_secs_f64()}));
        eprintln!(
            "ROSVOT {end}/{count}; {} notes; {:.2}s",
            notes.len(),
            timer.elapsed().as_secs_f64()
        );
    }
    let result = json!({"status":"experimental_not_quality_approved","implementation_sha256":crate::recipe::implementation_hash(),"algorithm":"rosvot-rust-onnx-v1;40-of-80-slaney;24k;hop128;rwbd0.5;notebd0.85;3000-core375-context","checkpoint_revision":"3c8332bf43adae35f6e4d64971862f2f6139b310","stem_sha256":hash(stem)?,"encoder_sha256":hash(&model.join("encoder.onnx"))?,"decoder_sha256":hash(&model.join("decoder.onnx"))?,"rwbd_sha256":hash(&word_model.join("encoder.onnx"))?,"duration_seconds":pcm.len()as f64/24000.,"notes":notes,"chunks":diagnostics});
    fs::write(output, serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}
