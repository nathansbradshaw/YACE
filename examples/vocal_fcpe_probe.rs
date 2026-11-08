//! Experimental FCPE ONNX evaluation. No recognition or reference chart input.
//! Mel frontend translated from CNChTu/FCPE 6a149c1 (MIT, 2023 CN_ChiTu).
use anyhow::{Result, ensure};
use chart_engine::models;
use clap::Parser;
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
#[path = "support/onnx_proto.rs"]
mod proto;
const SHA: &str = "0467e0a627c4dd7e774c2d3a05fb5ca1830af7491451ca0158e62cb2621e0908";
#[derive(Parser)]
struct Args {
    input: PathBuf,
    model: PathBuf,
    output: PathBuf,
}
fn reflect(i: isize, n: usize) -> usize {
    let p = 2 * (n - 1);
    let x = i.rem_euclid(p as isize) as usize;
    if x < n { x } else { p - x }
}
fn frontend(pcm: &[f32]) -> Vec<f32> {
    let htom = |f: f64| {
        if f < 1000. {
            f / (200. / 3.)
        } else {
            15. + (f / 1000.).ln() / (6.4f64.ln() / 27.)
        }
    };
    let mtoh = |m: f64| {
        if m < 15. {
            m * (200. / 3.)
        } else {
            1000. * ((m - 15.) * (6.4f64.ln() / 27.)).exp()
        }
    };
    let points: Vec<_> = (0..130)
        .map(|i| mtoh(htom(8000.) * i as f64 / 129.))
        .collect();
    let filters: Vec<Vec<(usize, f32)>> = (0..128)
        .map(|m| {
            (0..=512)
                .filter_map(|b| {
                    let f = b as f64 * 16000. / 1024.;
                    let w = ((f - points[m]) / (points[m + 1] - points[m]))
                        .min((points[m + 2] - f) / (points[m + 2] - points[m + 1]))
                        .max(0.)
                        * 2.
                        / (points[m + 2] - points[m]);
                    (w > 0.).then_some((b, w as f32))
                })
                .collect()
        })
        .collect();
    let count = pcm.len() / 160;
    let mut mel = vec![0.; count * 128];
    let fft = FftPlanner::new().plan_fft_forward(1024);
    for i in 0..count {
        let mut x: Vec<_> = (0..1024)
            .map(|j| {
                Complex32::new(
                    pcm[reflect((i * 160 + j) as isize - 432, pcm.len())]
                        * (0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / 1024.).cos()),
                    0.,
                )
            })
            .collect();
        fft.process(&mut x);
        let mag: Vec<_> = x
            .iter()
            .take(513)
            .map(|v| (v.norm_sqr() + 1e-9).sqrt())
            .collect();
        for (m, row) in filters.iter().enumerate() {
            mel[i * 128 + m] = row
                .iter()
                .map(|&(b, w)| mag[b] * w)
                .sum::<f32>()
                .max(1e-5)
                .ln()
        }
    }
    mel
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new output file");
    ensure!(models::hash_file(&a.model)? == SHA, "Unpinned FCPE model");
    let source = models::hash_file(&a.input)?;
    let pcm = models::decode(&a.input, 16000, 1)?;
    ensure!(
        pcm.len() > 1024 && pcm.len() <= 3600 * 16000 && pcm.iter().all(|v| v.is_finite()),
        "Invalid PCM"
    );
    let mel = frontend(&pcm);
    let count = mel.len() / 128;
    let model_bytes = fs::read(&a.model)?;
    let mut tables = vec![];
    for graph in proto::fields(&model_bytes)?
        .into_iter()
        .filter(|f| f.id == 7)
    {
        for init in proto::fields(graph.data)?.into_iter().filter(|f| f.id == 5) {
            for raw in proto::fields(init.data)?
                .into_iter()
                .filter(|f| f.id == 9 && f.data.len() == 1440)
            {
                let v: Vec<_> = raw
                    .data
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                if v[0] > 1000. && v[359] < 10000. && v.windows(2).all(|w| w[0] < w[1]) {
                    tables.push(v)
                }
            }
        }
    }
    ensure!(
        tables.len() == 1,
        "Cannot identify the pinned FCPE cents table"
    );
    let cents = &tables[0];
    let patched = proto::output(&model_bytes, "/Sigmoid_output_0")?;
    let patched_sha = format!("{:x}", Sha256::digest(&patched));
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_memory(&patched)?;
    let mut frames = vec![];
    for start in (0..count).step_by(2000) {
        let end = (start + 2000).min(count);
        let left = start.saturating_sub(100);
        let right = (end + 100).min(count);
        let input = Tensor::from_array((
            [1, right - left, 128],
            mel[left * 128..right * 128].to_vec().into_boxed_slice(),
        ))?;
        let threshold =
            Tensor::from_array((Vec::<usize>::new(), vec![0.006f32].into_boxed_slice()))?;
        let out = session.run(ort::inputs![input, threshold])?;
        let (shape, salience) = out[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [1, (right - left) as i64, 360]
                && salience
                    .iter()
                    .all(|p| p.is_finite() && (-1e-6..=1. + 1e-6).contains(p)),
            "FCPE shape/numerics: {shape:?}; expected {}; invalid {:?}",
            (right - left),
            salience
                .iter()
                .enumerate()
                .find(|(_, p)| !p.is_finite() || !(0. ..=1.).contains(*p))
        );
        for i in start..end {
            let row = &salience[(i - left) * 360..(i - left + 1) * 360];
            let peak = (0..360).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
            let confidence = row[peak].clamp(0., 1.);
            let mut sum = 0.;
            let mut weighted = 0.;
            for j in 0..9 {
                let k = (peak as isize + j - 4).clamp(0, 359) as usize;
                sum += row[k].clamp(0., 1.) as f64;
                weighted += row[k].clamp(0., 1.) as f64 * cents[k] as f64
            }
            let hz =
                (confidence > 0.006 && sum > 0.).then(|| 10. * 2f64.powf(weighted / sum / 1200.));
            frames.push(json!({"time_seconds":i as f64*0.01+0.005,"frequency_hz":hz,"confidence":confidence}))
        }
        eprintln!("FCPE {end}/{count}")
    }
    ensure!(
        models::hash_file(&a.input)? == source && models::hash_file(&a.model)? == SHA,
        "Source/model changed"
    );
    let mut hash = Sha256::new();
    for p in &pcm {
        hash.update(p.to_le_bytes())
    }
    let result = json!({"status":"raw_research_only","model_id":"fcpe-deiteris-onnx","model_revision":"a6317b2f21619f35087d870b37c7a3eeb6bb3b4d","model_sha256":SHA,"raw_graph_sha256":patched_sha,"source_sha256":source,"pcm_sha256":format!("{:x}",hash.finalize()),"sample_rate":16000,"hop_samples":160,"duration_seconds":pcm.len()as f64/16000.,"frontend":"Slaney128-0to8000;Hann1024;hop160;reflect432;ln-magnitude;2000core100context;time-window-centre+5ms","decoder":"official local argmax9;edge indices clipped;zero-denominator gap;salience>0.006","confidence_meaning":"peak neural salience; not calibrated probability","frames":frames});
    fs::write(a.output, serde_json::to_vec(&result)?)?;
    Ok(())
}
