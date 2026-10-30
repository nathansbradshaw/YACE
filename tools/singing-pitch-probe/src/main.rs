//! Native, source-bound raw CREPE research evidence. Never generates charts.
use anyhow::{Context, Result, ensure};
use clap::{Parser, ValueEnum};
use ort::{session::Session, value::Tensor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

const RATE: usize = 16000;
const HOP: usize = 160;
const WINDOW: usize = 1024;
const BATCH: usize = 32;
const MAX_SECONDS: usize = 3600;
const REVISION: &str = "946f80e7f61fb78c72a037beac46b8b2dc3892bc";
// Retain the upstream decimal verbatim, rather than changing its presentation
// during migration of the measured decoder.
#[allow(clippy::excessive_precision)]
const CREPE_CENTS_OFFSET: f64 = 1997.3794084376191;

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Model {
    Full,
    Tiny,
}
impl Model {
    fn id(self) -> &'static str {
        match self {
            Self::Full => "crepe-full-onnx-v1.1.0",
            Self::Tiny => "crepe-tiny-onnx-v1.1.0",
        }
    }
    fn sha(self) -> &'static str {
        match self {
            Self::Full => "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610",
            Self::Tiny => "91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3",
        }
    }
    fn url(self) -> &'static str {
        match self {
            Self::Full => {
                "https://github.com/yqzhishen/onnxcrepe/releases/download/v1.1.0/full.onnx"
            }
            Self::Tiny => {
                "https://github.com/yqzhishen/onnxcrepe/releases/download/v1.1.0/tiny.onnx"
            }
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    about = "Raw native singing-pitch research evidence; no chart generation or model downloads"
)]
struct Args {
    #[arg(long, value_enum)]
    model: Model,
    #[arg(long)]
    model_file: PathBuf,
    #[arg(
        long,
        required_unless_present = "controls",
        conflicts_with = "controls"
    )]
    input: Option<PathBuf>,
    #[arg(long)]
    controls: bool,
    /// Must not already exist. Evidence and full raw salience are retained.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct Frame {
    time_seconds: f64,
    frequency_hz: f64,
    // Exact widening preserves the model's float32 probability in JSON.
    confidence: f64,
    local_rms: f64,
}
#[derive(Debug, Serialize, Deserialize)]
struct Source {
    path: PathBuf,
    sha256: String,
    decoded_44100_samples: usize,
    decoded_44100_pcm_sha256: String,
    decoded_44100_duration_seconds: f64,
}
#[derive(Debug, Serialize, Deserialize)]
struct Evidence {
    version: u32,
    status: String,
    model_id: String,
    model_revision: String,
    model_sha256: String,
    model_url: String,
    model_bytes: u64,
    implementation_sha256: String,
    source: Option<Source>,
    pcm_sha256: String,
    sample_rate: usize,
    hop_samples: usize,
    window_samples: usize,
    duration_seconds: f64,
    decoder: String,
    padding: String,
    normalization: String,
    confidence_meaning: String,
    threads: usize,
    batch_frames: usize,
    observed_salience_range: [f64; 2],
    salience_file: String,
    salience_shape: [usize; 2],
    salience_sha256: String,
    inference_elapsed_seconds: f64,
    frames: Vec<Frame>,
}
fn implementation_hash() -> String {
    let mut digest = Sha256::new();
    digest.update(include_bytes!("main.rs"));
    digest.update(include_bytes!("../Cargo.toml"));
    digest.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", digest.finalize())
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("Read {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Original CREPE weighted decoder, without torchcrepe's optional dither or
/// the converter's erroneous second sigmoid. Raw graph probabilities survive
/// unchanged; only decoder roundoff within 1e-6 is clamped.
fn decode(salience: &[f32]) -> Result<(f64, f32)> {
    ensure!(
        salience.len() == 360
            && salience
                .iter()
                .all(|v| v.is_finite() && (-1e-6..=1.0 + 1e-6).contains(v)),
        "Invalid CREPE sigmoid probabilities"
    );
    let peak = (0..360)
        .max_by(|&a, &b| salience[a].total_cmp(&salience[b]))
        .unwrap();
    let (mut weighted, mut sum) = (0., 0.);
    for (index, p) in salience
        .iter()
        .enumerate()
        .take((peak + 5).min(360))
        .skip(peak.saturating_sub(4))
    {
        let p = p.clamp(0., 1.) as f64;
        weighted += (20. * index as f64 + CREPE_CENTS_OFFSET) * p;
        sum += p;
    }
    ensure!(sum > 0., "Empty CREPE decoder mass");
    Ok((
        10. * 2f64.powf(weighted / sum / 1200.),
        salience[peak].clamp(0., 1.),
    ))
}
fn extract_frame(pcm: &[f32], index: usize) -> Vec<f32> {
    (0..WINDOW)
        .map(|j| {
            let source = index as isize * HOP as isize + j as isize - WINDOW as isize / 2;
            if source < 0 {
                0.
            } else {
                pcm.get(source as usize).copied().unwrap_or(0.)
            }
        })
        .collect()
}

/// Stream and bound FFmpeg output, retaining canonical little-endian PCM
/// hashes. Source-clock probing discards samples rather than allocating an
/// additional full-length buffer. Oversized audio is rejected, never clipped.
fn decode_audio(path: &Path, rate: usize, retain: bool) -> Result<(Vec<f32>, usize, String)> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-f",
            "f32le",
            "-ar",
            &rate.to_string(),
            "-ac",
            "1",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("FFmpeg is required on PATH")?;
    let result = (|| -> Result<_> {
        let mut stdout = child.stdout.take().context("Decode pipe")?;
        let mut buffer = [0u8; 65536];
        let mut pending = Vec::new();
        let mut pcm = Vec::new();
        let mut size = 0usize;
        let mut digest = Sha256::new();
        loop {
            let count = stdout.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size.checked_add(count).context("Decode size overflow")?;
            ensure!(
                size <= rate * MAX_SECONDS * 4,
                "Audio exceeds one-hour decode budget"
            );
            digest.update(&buffer[..count]);
            if retain {
                pending.extend_from_slice(&buffer[..count]);
                let complete = pending.len() / 4 * 4;
                for bytes in pending[..complete].chunks_exact(4) {
                    let value = f32::from_le_bytes(bytes.try_into().unwrap());
                    ensure!(value.is_finite(), "Nonfinite source PCM");
                    pcm.push(value);
                }
                let tail = pending[complete..].to_vec();
                pending.clear();
                pending.extend_from_slice(&tail);
            }
        }
        ensure!(
            size > 0 && size.is_multiple_of(4),
            "Empty or partial float PCM"
        );
        ensure!(pending.is_empty(), "Partial float PCM tail");
        Ok((pcm, size / 4, format!("{:x}", digest.finalize())))
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let decoded = result?;
    ensure!(status.success(), "FFmpeg decode failed");
    Ok(decoded)
}

fn infer(pcm: &[f32], model: Model, model_path: &Path, stage: &Path) -> Result<Evidence> {
    ensure!(
        !pcm.is_empty() && pcm.len() <= RATE * MAX_SECONDS && pcm.iter().all(|v| v.is_finite()),
        "PCM must be finite, nonempty and at most one hour"
    );
    ensure!(
        hash_file(model_path)? == model.sha(),
        "Pinned model checksum mismatch"
    );
    let timer = Instant::now();
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model_path)?;
    let count = pcm.len() / HOP + 1;
    let mut frames = Vec::with_capacity(count);
    let mut salience_file = BufWriter::new(
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("salience.f32le"))?,
    );
    let mut salience_hash = Sha256::new();
    let mut observed = [f32::INFINITY, f32::NEG_INFINITY];
    for start in (0..count).step_by(BATCH) {
        let end = (start + BATCH).min(count);
        let values: Vec<_> = (start..end).flat_map(|i| extract_frame(pcm, i)).collect();
        let input = Tensor::from_array(([end - start, WINDOW], values.into_boxed_slice()))?;
        let output = session.run(ort::inputs!["frames" => input])?;
        let (shape, values) = output[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.len() == 2
                && shape[0] == (end - start) as i64
                && shape[1] == 360
                && values.len() == (end - start) * 360,
            "Unexpected CREPE tensor contract"
        );
        for (local, row) in values.chunks_exact(360).enumerate() {
            let (hz, confidence) = decode(row)?;
            let index = start + local;
            let a = (index * HOP).saturating_sub(HOP / 2).min(pcm.len());
            let b = (index * HOP + HOP / 2).min(pcm.len());
            let rms = if b > a {
                (pcm[a..b].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / (b - a) as f64).sqrt()
            } else {
                0.
            };
            frames.push(Frame {
                time_seconds: index as f64 * 0.010,
                frequency_hz: hz,
                confidence: confidence as f64,
                local_rms: rms,
            });
            for value in row {
                observed[0] = observed[0].min(*value);
                observed[1] = observed[1].max(*value);
                let bytes = value.to_le_bytes();
                salience_file.write_all(&bytes)?;
                salience_hash.update(bytes);
            }
        }
        if start == 0 || end == count || (start / BATCH).is_multiple_of(100) {
            eprintln!(
                "{} frames {end}/{count}; {:.2}s",
                model.id(),
                timer.elapsed().as_secs_f64()
            );
        }
    }
    salience_file.flush()?;
    ensure!(
        hash_file(model_path)? == model.sha(),
        "Model changed during inference"
    );
    let mut pcm_hash = Sha256::new();
    for value in pcm {
        pcm_hash.update(value.to_le_bytes());
    }
    Ok(Evidence {
        version: 1,
        status: "raw_research_evidence_not_chart_notes".into(),
        model_id: model.id().into(),
        model_revision: REVISION.into(),
        model_sha256: model.sha().into(),
        model_url: model.url().into(),
        model_bytes: fs::metadata(model_path)?.len(),
        implementation_sha256: implementation_hash(),
        source: None,
        pcm_sha256: format!("{:x}", pcm_hash.finalize()),
        sample_rate: RATE,
        hop_samples: HOP,
        window_samples: WINDOW,
        duration_seconds: pcm.len() as f64 / RATE as f64,
        decoder: "Original CREPE local salience-weighted mean +/-4 bins; no second sigmoid, dither, Viterbi or confidence calibration".into(),
        padding: "Centered 1024 samples, zero outside source, first frame at 0, exact 10ms hop, EOF frame retained when sample count is divisible by hop".into(),
        normalization: "Z-score embedded in pinned ONNX; raw amplitude input".into(),
        confidence_meaning: "Maximum model salience, not probability of correct lead singing; nonvoice periodic signals can receive high confidence".into(),
        threads: 2,
        batch_frames: BATCH,
        observed_salience_range: observed.map(f64::from),
        salience_file: "salience.f32le".into(),
        salience_shape: [count, 360],
        salience_sha256: format!("{:x}", salience_hash.finalize()),
        inference_elapsed_seconds: timer.elapsed().as_secs_f64(),
        frames,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "Output already exists");
    ensure!(
        hash_file(&args.model_file)? == args.model.sha(),
        "Pinned model checksum mismatch"
    );
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    if args.controls {
        run_controls(args.model, &args.model_file, stage.path())?;
    } else {
        let input = args.input.as_ref().context("Input is required")?;
        let source_sha = hash_file(input)?;
        let (_, samples, source_pcm_sha) = decode_audio(input, 44100, false)?;
        let (pcm, _, _) = decode_audio(input, RATE, true)?;
        let source_duration = samples as f64 / 44100.;
        ensure!(
            (pcm.len() as f64 / RATE as f64 - source_duration).abs()
                <= 1. / RATE as f64 + 1. / 44100.,
            "Resampling changed the original source clock"
        );
        let mut evidence = infer(&pcm, args.model, &args.model_file, stage.path())?;
        ensure!(
            hash_file(input)? == source_sha,
            "Input changed during inference"
        );
        evidence.source = Some(Source {
            path: fs::canonicalize(input)?,
            sha256: source_sha,
            decoded_44100_samples: samples,
            decoded_44100_pcm_sha256: source_pcm_sha,
            decoded_44100_duration_seconds: source_duration,
        });
        write_json(&stage.path().join("evidence.json"), &evidence)?;
    }
    ensure!(!args.output.exists(), "Output appeared during inference");
    fs::rename(stage.path(), &args.output)?;
    println!(
        "{}",
        serde_json::json!({"output":args.output,"status":"raw_research_evidence_only"})
    );
    Ok(())
}

fn run_controls(model: Model, model_path: &Path, stage: &Path) -> Result<()> {
    let mut summaries = Vec::new();
    for case in ["tone110", "tone220", "tone440", "silence", "noise"] {
        let folder = stage.join(case);
        fs::create_dir(&folder)?;
        let mut seed = 91u32;
        let pcm: Vec<_> = (0..RATE)
            .map(|index| {
                let t = index as f64 / RATE as f64;
                match case {
                    "silence" => 0.,
                    "noise" => {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (seed as f64 / u32::MAX as f64 * 0.3 - 0.15) as f32
                    }
                    _ => {
                        let hz = if case == "tone110" {
                            110.
                        } else if case == "tone220" {
                            220.
                        } else {
                            440.
                        };
                        (0.20 * (std::f64::consts::TAU * hz * t).sin()
                            + 0.06 * (2. * std::f64::consts::TAU * hz * t).sin())
                            as f32
                    }
                }
            })
            .collect();
        let evidence = infer(&pcm, model, model_path, &folder)?;
        let middle: Vec<_> = evidence
            .frames
            .iter()
            .filter(|f| (0.1..0.9).contains(&f.time_seconds))
            .collect();
        let mut confidences: Vec<_> = middle.iter().map(|f| f.confidence).collect();
        let mut hz: Vec<_> = middle.iter().map(|f| f.frequency_hz).collect();
        confidences.sort_by(f64::total_cmp);
        hz.sort_by(f64::total_cmp);
        let summary = serde_json::json!({"case":case,"median_hz":hz[hz.len()/2],"median_confidence":confidences[confidences.len()/2],"max_confidence":confidences.last(),"inference_elapsed_seconds":evidence.inference_elapsed_seconds,"meaning":"Synthetic pitch/abstention evidence, not a lead voice identification test"});
        write_json(&folder.join("evidence.json"), &evidence)?;
        eprintln!("{summary}");
        summaries.push(summary);
    }
    write_json(&stage.join("controls.json"), &summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_keeps_original_probability_and_local_weighted_cents() {
        let mut values = vec![0.; 360];
        values[140] = 0.9;
        values[141] = 0.3;
        values[145] = 0.1; // Outside the inclusive +/-4 neighborhood.
        let (hz, confidence) = decode(&values).unwrap();
        let cents =
            (140. * 20. + CREPE_CENTS_OFFSET) * 0.75 + (141. * 20. + CREPE_CENTS_OFFSET) * 0.25;
        assert!((hz - 10. * 2f64.powf(cents / 1200.)).abs() < 1e-6);
        assert_eq!(confidence, 0.9);
        values[0] = -5.9604645e-8;
        assert!(decode(&values).is_ok());
        values[0] = f32::NAN;
        assert!(decode(&values).is_err());
        values[0] = -1e-3;
        assert!(decode(&values).is_err());
        assert!(decode(&[0.; 360]).is_err());
        assert!(decode(&[0.; 359]).is_err());
    }
    #[test]
    fn centered_padding_preserves_origin_and_short_input() {
        let pcm: Vec<_> = (0..2000).map(|i| i as f32).collect();
        let first = extract_frame(&pcm, 0);
        assert!(first[..512].iter().all(|v| *v == 0.));
        assert_eq!(first[513], 1.);
        assert_eq!(extract_frame(&pcm, 4)[512], 640.);
        let single = extract_frame(&[0.2], 0);
        assert_eq!(single.iter().filter(|&&v| v != 0.).count(), 1);
        assert_eq!(single[512], 0.2);
        let eof = extract_frame(&pcm, 2000 / HOP + 1);
        assert!(eof[512..].iter().all(|v| *v == 0.));
    }
    #[test]
    fn explicit_model_and_single_input_condition_are_required() {
        assert!(
            Args::try_parse_from([
                "probe",
                "--model",
                "tiny",
                "--model-file",
                "tiny.onnx",
                "--input",
                "song.wav",
                "--output",
                "new"
            ])
            .is_ok()
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--model",
                "full",
                "--model-file",
                "full.onnx",
                "--controls",
                "--output",
                "new"
            ])
            .is_ok()
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--model-file",
                "x",
                "--controls",
                "--output",
                "new"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--model",
                "tiny",
                "--model-file",
                "x",
                "--controls",
                "--input",
                "song.wav",
                "--output",
                "new"
            ])
            .is_err()
        );
        assert_ne!(Model::Tiny.sha(), Model::Full.sha());
    }
    #[test]
    fn model_mismatch_and_nonfinite_pcm_fail_before_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("wrong.onnx");
        fs::write(&file, b"not a model").unwrap();
        assert!(infer(&[0.; 160], Model::Tiny, &file, dir.path()).is_err());
        assert!(infer(&[f32::NAN], Model::Tiny, &file, dir.path()).is_err());
        assert!(!dir.path().join("salience.f32le").exists());
    }

    /// Opt-in native control. Requires an explicit local pinned file; never
    /// downloads weights or contacts a provider. Raw controls use their own
    /// temporary output and do not enter production generation.
    #[test]
    #[ignore = "set YACE_CREPE_MODEL=full|tiny and YACE_CREPE_MODEL_FILE to a pinned local ONNX"]
    fn pinned_native_controls_and_salience_receipts() -> Result<()> {
        let choice = std::env::var("YACE_CREPE_MODEL").context("YACE_CREPE_MODEL is required")?;
        let model = match choice.as_str() {
            "full" => Model::Full,
            "tiny" => Model::Tiny,
            _ => anyhow::bail!("Expected full or tiny"),
        };
        let model_path = PathBuf::from(
            std::env::var("YACE_CREPE_MODEL_FILE").context("YACE_CREPE_MODEL_FILE is required")?,
        );
        let folder = tempfile::tempdir()?;
        run_controls(model, &model_path, folder.path())?;
        let summaries: Vec<serde_json::Value> =
            serde_json::from_reader(fs::File::open(folder.path().join("controls.json"))?)?;
        for (index, target) in [110f64, 220., 440.].into_iter().enumerate() {
            let measured = summaries[index]["median_hz"].as_f64().unwrap();
            ensure!(
                (12. * (measured / target).log2()).abs() < 0.2,
                "Known pitch control failed"
            );
            ensure!(
                summaries[index]["median_confidence"].as_f64().unwrap() >= 0.5,
                "Known pitch confidence control failed"
            );
        }
        for summary in &summaries[3..] {
            ensure!(
                summary["max_confidence"].as_f64().unwrap() < 0.5,
                "Silence/noise control failed"
            );
        }
        for summary in &summaries {
            let dir = folder.path().join(summary["case"].as_str().unwrap());
            let evidence: Evidence =
                serde_json::from_reader(fs::File::open(dir.join("evidence.json"))?)?;
            ensure!(
                evidence.frames.len() == 101
                    && evidence.frames[0].time_seconds == 0.
                    && evidence.frames[100].time_seconds == 1.,
                "Control clock changed"
            );
            ensure!(
                fs::metadata(dir.join("salience.f32le"))?.len() == 101 * 360 * 4,
                "Raw salience incomplete"
            );
            ensure!(
                hash_file(&dir.join("salience.f32le"))? == evidence.salience_sha256,
                "Raw salience receipt differs"
            );
        }
        Ok(())
    }
}
