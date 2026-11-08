//! Inspect independent contour support without treating a generated chart as truth.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_generation::Generation,
    vocals::{self, VocalTone},
};
use clap::Parser;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};
#[derive(Parser)]
struct Args {
    report: PathBuf,
    midi: PathBuf,
    #[arg(long)]
    corroborating: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Deserialize)]
struct Contour {
    source: Source,
    pcm_sha256: String,
    model_sha256: String,
    implementation_sha256: String,
    duration_seconds: f64,
    sample_rate: usize,
    hop_samples: usize,
    frames: Vec<Frame>,
}
#[derive(Deserialize)]
struct Source {
    sha256: String,
}
fn probe_implementation() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("../tools/singing-pitch-probe/src/main.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/src/rmvpe.rs"));
    h.update(include_bytes!("../src/rmvpe_frontend.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.toml"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.lock"));
    format!("{:x}", h.finalize())
}
#[derive(Deserialize)]
struct Frame {
    time_seconds: f64,
    frequency_hz: Option<f64>,
    confidence: f64,
}
fn nearest(time: f64, frames: &[Frame]) -> Option<&Frame> {
    let at = frames.partition_point(|f| f.time_seconds < time);
    [at.saturating_sub(1), at.min(frames.len().saturating_sub(1))]
        .into_iter()
        .filter_map(|i| frames.get(i))
        .min_by(|a, b| {
            (a.time_seconds - time)
                .abs()
                .total_cmp(&(b.time_seconds - time).abs())
        })
        .filter(|f| (f.time_seconds - time).abs() <= 0.02)
}
fn main() -> Result<()> {
    let args = Args::parse();
    let report: Generation = serde_json::from_slice(&fs::read(args.report)?)?;
    let corroborating: Option<Contour> = args
        .corroborating
        .map(|p| -> Result<_> { Ok(serde_json::from_slice(&fs::read(p)?)?) })
        .transpose()?;
    let model = report
        .rmvpe_pitch
        .as_ref()
        .context("Report has no independent contour")?;
    chart_engine::vocal_rmvpe::validate(
        model,
        &report.audio_sha256,
        &report.vocal_stem_sha256,
        report.duration_seconds,
    )?;
    if let Some(c) = &corroborating {
        ensure!(
            c.source.sha256 == report.vocal_stem_sha256
                && c.pcm_sha256 == model.pcm_sha256
                && (c.duration_seconds - model.duration_seconds).abs() <= 1. / 16000.
                && c.implementation_sha256 == probe_implementation()
                && matches!(
                    c.model_sha256.as_str(),
                    "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610"
                        | "91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3"
                )
                && c.sample_rate == 16000
                && c.hop_samples == 160
                && c.frames.len() == model.frames.len()
                && c.frames
                    .iter()
                    .enumerate()
                    .all(|(i, f)| (f.time_seconds - i as f64 * 0.01).abs() < 1e-9
                        && f.confidence.is_finite()
                        && (0. ..=1.).contains(&f.confidence)
                        && f.frequency_hz.is_none_or(|h| h.is_finite() && h >= 0.)),
            "Corroborating source, PCM, model, implementation or frame grid differs"
        )
    }
    let timing = report.timing.as_ref().context("Missing timing")?;
    let original = vocals::import_midi(&fs::read(args.midi)?)?;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut seconds = BTreeMap::<usize, BTreeMap<String, usize>>::new();
    let mut add = |time: f64, name: &str| {
        *counts.entry(name.into()).or_default() += 1;
        *seconds
            .entry(time as usize)
            .or_default()
            .entry(name.into())
            .or_default() += 1;
    };
    let note_at = |chart: &vocals::VocalChart, time| {
        chart
            .notes
            .iter()
            .find(|n| timing.seconds(n.start) <= time && time < timing.seconds(n.end))
            .and_then(|n| {
                if let VocalTone::Pitched { midi } = n.tone {
                    Some(midi as f64)
                } else {
                    None
                }
            })
    };
    let model_frames: Vec<_> = model
        .frames
        .iter()
        .map(|f| Frame {
            time_seconds: f.time_seconds,
            frequency_hz: f.frequency_hz,
            confidence: f.confidence as f64,
        })
        .collect();
    for f in &report.pitch_frames {
        let Some(raw) = nearest(f.time_seconds, &model_frames).filter(|r| r.confidence >= 0.1)
        else {
            continue;
        };
        let Some(p) = raw
            .frequency_hz
            .filter(|hz| *hz > 0.)
            .map(|hz| 69. + 12. * (hz / 440.).log2())
            .filter(|p| (35.5..84.5).contains(p))
        else {
            continue;
        };
        add(f.time_seconds, "model_supported");
        let agrees = |v: Option<f64>| v.is_some_and(|v| (v - p).abs() <= 0.75);
        let other = corroborating
            .as_ref()
            .and_then(|c| nearest(f.time_seconds, &c.frames))
            .filter(|r| r.confidence >= 0.5)
            .and_then(|r| r.frequency_hz)
            .filter(|hz| *hz > 0.)
            .map(|hz| 69. + 12. * (hz / 440.).log2());
        let corroborated = agrees(other);
        if corroborated {
            add(f.time_seconds, "two_models_agree");
        }
        if f.midi.is_none() && f.local_energy_active == Some(true) {
            add(f.time_seconds, "dropout_with_local_energy");
            if agrees(f.yin_midi) {
                add(f.time_seconds, "dropout_yin_agrees");
            }
            if agrees(f.mcleod_midi) {
                add(f.time_seconds, "dropout_mcleod_agrees");
            }
            if corroborated {
                add(f.time_seconds, "dropout_two_models_agree");
            }
            if corroborated && (agrees(f.yin_midi) || agrees(f.mcleod_midi)) {
                add(f.time_seconds, "dropout_two_models_and_detector");
            }
        }
        for (name, chosen) in [
            ("original", note_at(&original, f.time_seconds)),
            (
                "high",
                report
                    .chart
                    .as_ref()
                    .and_then(|c| note_at(c, f.time_seconds)),
            ),
        ] {
            match chosen {
                None => add(f.time_seconds, &format!("{name}_missing")),
                Some(chosen) => {
                    if (chosen - p).abs() <= 0.5 {
                        add(f.time_seconds, &format!("{name}_agrees"));
                    } else {
                        add(f.time_seconds, &format!("{name}_disagrees"));
                        if ((chosen - p).abs() - 12.).abs() <= 0.5 {
                            add(f.time_seconds, &format!("{name}_octave_disagreement"));
                        }
                    }
                }
            }
            if corroborated {
                match chosen {
                    None => add(f.time_seconds, &format!("{name}_missing_corroborated")),
                    Some(chosen) if (chosen - p).abs() <= 0.5 => {
                        add(f.time_seconds, &format!("{name}_agrees_corroborated"))
                    }
                    Some(_) => add(f.time_seconds, &format!("{name}_disagrees_corroborated")),
                }
            }
        }
    }
    let output = json!({"warning":"Model/detector agreement is diagnostic support, not measured accuracy or lead-singer identity","counts":counts,"per_second":seconds});
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(args.output)?
        .write_all(&serde_json::to_vec_pretty(&output)?)?;
    println!("{}", serde_json::to_string_pretty(&output["counts"])?);
    Ok(())
}
