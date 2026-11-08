//! Research vocal activity tagging, not pitch, lyric or singer identification.
//! PANNs CNN6 translated from official d2f4b8c (MIT, Qiuqiang Kong).
mod checkpoint;
#[path = "../../rosvot-convert/src/onnx.rs"]
#[allow(dead_code)]
mod onnx;
use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Inspect {
        model: PathBuf,
    },
    Export {
        model: PathBuf,
        output: PathBuf,
    },
    Infer {
        input: PathBuf,
        checkpoint: PathBuf,
        model: PathBuf,
        output: PathBuf,
    },
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::Infer {
            input,
            checkpoint,
            model,
            output,
        } => infer(input, checkpoint, model, output)?,
        Command::Inspect { model } => {
            let ck = checkpoint::Checkpoint::read(&model, Some("model"))?;
            for (key, info) in &ck.tensors {
                println!("{key} {:?} {:?}", info.layout.dims(), info.dtype);
            }
        }
        Command::Export { model, output } => {
            ensure!(!output.exists(), "Output exists");
            let ck = checkpoint::Checkpoint::read(&model, None)?;
            let mut g = onnx::Graph::new(&ck);
            g.inputs.push(onnx::info("mel", &["B", "1", "T", "64"]));
            let mut x = g.tr("mel", &[0, 3, 2, 1]);
            x = bn(&mut g, &x, "bn0")?;
            x = g.tr(&x, &[0, 3, 2, 1]);
            for i in 1..=4 {
                let w = g.weight(&format!("conv_block{i}.conv1.weight"))?;
                x = g.node(
                    "Conv",
                    &[&x, &w],
                    vec![
                        onnx::Attr::Is("kernel_shape", &[5, 5]),
                        onnx::Attr::Is("pads", &[2, 2, 2, 2]),
                    ],
                );
                x = bn(&mut g, &x, &format!("conv_block{i}.bn1"))?;
                x = g.un("Relu", &x);
                x = g.node(
                    "AveragePool",
                    &[&x],
                    vec![
                        onnx::Attr::Is("kernel_shape", &[2, 2]),
                        onnx::Attr::Is("strides", &[2, 2]),
                    ],
                );
            }
            x = g.node(
                "ReduceMean",
                &[&x],
                vec![onnx::Attr::Is("axes", &[3]), onnx::Attr::I("keepdims", 0)],
            );
            let max = g.node(
                "ReduceMax",
                &[&x],
                vec![onnx::Attr::Is("axes", &[2]), onnx::Attr::I("keepdims", 0)],
            );
            let mean = g.node(
                "ReduceMean",
                &[&x],
                vec![onnx::Attr::Is("axes", &[2]), onnx::Attr::I("keepdims", 0)],
            );
            x = g.bin("Add", &max, &mean);
            x = g.linear(&x, "fc1", true)?;
            x = g.un("Relu", &x);
            x = g.linear(&x, "fc_audioset", true)?;
            x = g.un("Sigmoid", &x);
            g.outputs.push(onnx::info(&x, &["B", "527"]));
            fs::write(output, g.finish())?;
        }
    }
    Ok(())
}
fn hash(p: &std::path::Path) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs::read(p)?)))
}
fn reflect(i: isize, n: usize) -> usize {
    let p = 2 * (n - 1);
    let x = i.rem_euclid(p as isize) as usize;
    if x < n { x } else { p - x }
}
fn infer(input: PathBuf, checkpoint: PathBuf, model: PathBuf, output: PathBuf) -> Result<()> {
    ensure!(!output.exists(), "Output exists");
    ensure!(
        hash(&model)? == "a972a3f101b919e70f117135d8a968a785435d8f9748515f207a4ac3153e0718",
        "Unpinned ONNX conversion"
    );
    let ck = checkpoint::Checkpoint::read(&checkpoint, Some("model"))?;
    let filter = ck.get("logmel_extractor.melW")?;
    ensure!(filter.dims == [513, 64], "Pinned mel shape differs");
    let weights: Vec<_> = filter
        .raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    // Same explicit FFmpeg f32le decode used by YACE's native audio loader;
    // avoid linking unrelated app/model runtimes into this research probe.
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&input)
        .args([
            "-map", "0:a:0", "-f", "f32le", "-ar", "32000", "-ac", "1", "pipe:1",
        ])
        .output()?;
    ensure!(
        decoded.status.success() && decoded.stdout.len() % 4 == 0,
        "Audio decode failed"
    );
    let pcm: Vec<_> = decoded
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    ensure!(
        (1025..=3600 * 32000).contains(&pcm.len()) && pcm.iter().all(|x| x.is_finite()),
        "Invalid PCM/budget"
    );
    let mut ph = Sha256::new();
    for x in &pcm {
        ph.update(x.to_le_bytes());
    }
    let count = pcm.len() / 320 + 1;
    let fft = FftPlanner::new().plan_fft_forward(1024);
    let mut mel = vec![0.; count * 64];
    for i in 0..count {
        let mut x: Vec<_> = (0..1024)
            .map(|j| {
                Complex32::new(
                    pcm[reflect((i * 320 + j) as isize - 512, pcm.len())]
                        * (0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / 1024.).cos()),
                    0.,
                )
            })
            .collect();
        fft.process(&mut x);
        for m in 0..64 {
            let power = (0..513)
                .map(|b| x[b].norm_sqr() * weights[b * 64 + m])
                .sum::<f32>();
            mel[i * 64 + m] = 10. * power.max(1e-10).log10();
        }
    }
    let mut session = ort::session::Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_file(&model)?;
    let start = std::time::Instant::now();
    let windows: Vec<_> = (0..count).step_by(50).collect();
    let mut rows = vec![];
    for centres in windows.chunks(8) {
        let mut batch = vec![0.; centres.len() * 201 * 64];
        for (b, &centre) in centres.iter().enumerate() {
            for t in 0..201 {
                let at = reflect(centre as isize + t as isize - 100, count);
                batch[(b * 201 + t) * 64..(b * 201 + t + 1) * 64]
                    .copy_from_slice(&mel[at * 64..(at + 1) * 64]);
            }
        }
        let out=session.run(ort::inputs!["mel"=>ort::value::Tensor::from_array(([centres.len(),1,201,64],batch.into_boxed_slice()))?])?;
        let (shape, data) = out[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [centres.len() as i64, 527]
                && data.iter().all(|x| x.is_finite() && (0. ..=1.).contains(x)),
            "Invalid class output"
        );
        for (b, &centre) in centres.iter().enumerate() {
            let p = &data[b * 527..(b + 1) * 527];
            let singing = (27..=37).map(|i| p[i]).fold(0., f32::max);
            let voice = singing.max(p[0]).max(p[1]).max(p[2]).max(p[3]);
            rows.push(json!({"time_seconds":centre as f64*0.01,"singing":singing,"voice":voice,"scores":p}));
        }
    }
    fs::write(
        output,
        serde_json::to_vec(
            &json!({"version":1,"experimental":true,"source_sha256":hash(&input)?,"model_sha256":hash(&model)?,"checkpoint_sha256":ck.sha256,"pcm_sha256":format!("{:x}",ph.finalize()),"implementation_sha256":format!("{:x}",Sha256::digest(include_bytes!("main.rs"))),"sample_rate":32000,"duration_seconds":pcm.len() as f64/32000.,"recipe":"PANNs-CNN6;official-embedded-melW;32kFFT1024hop320-Hann-centred-reflect;power-dB-floor1e-10;201frame-global-context-reflect;step50frames;singing=max27to37;voice=maxsinging-or0to3;not-lead-identity-or-correctness-probability","inference_seconds":start.elapsed().as_secs_f64(),"frames":rows}),
        )?,
    )?;
    Ok(())
}
fn bn(g: &mut onnx::Graph<'_>, x: &str, p: &str) -> Result<String> {
    let w = g.weight(&format!("{p}.weight"))?;
    let b = g.weight(&format!("{p}.bias"))?;
    let m = g.weight(&format!("{p}.running_mean"))?;
    let v = g.weight(&format!("{p}.running_var"))?;
    Ok(g.node(
        "BatchNormalization",
        &[x, &w, &b, &m, &v],
        vec![onnx::Attr::F("epsilon", 1e-5)],
    ))
}
