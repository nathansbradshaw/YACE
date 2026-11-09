//! Explicit local JDC research. Never downloads models or edits projects.
mod convert;
mod onnx;
use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_runtime, vocal_singing as jdc};
use clap::{Parser, Subcommand};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Convert the original pinned HDF5 checkpoint without executing Keras.
    Convert {
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Analyze an isolated vocal stem on its original clock. No lyric input.
    Infer {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        runtime: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        /// Optional full 722-column research posterior; existing files refused.
        #[arg(long)]
        posterior: Option<PathBuf>,
    },
    /// Build a lyric-free experimental draft from a source-bound acoustic grid.
    Analyze {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        rmvpe_model: PathBuf,
        #[arg(long)]
        runtime: Option<PathBuf>,
        /// New directory, published only after both models finish.
        #[arg(long)]
        output: PathBuf,
    },
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}
fn infer(
    input: &Path,
    root: &Path,
    explicit_runtime: Option<PathBuf>,
    output: &Path,
    raw: Option<&Path>,
) -> Result<()> {
    ensure!(
        !output.exists() && raw.is_none_or(|p| !p.exists()),
        "Use new JDC output files"
    );
    let graph = root.join("original-rust.onnx");
    ensure!(
        models::hash_file(&graph)? == jdc::MODEL_SHA256,
        "JDC graph checksum differs; another model was not substituted"
    );
    let norm = jdc::Normalization::from_npy(
        &fs::read(root.join("mean.npy"))?,
        &fs::read(root.join("std.npy"))?,
    )?;
    let exe = std::env::current_exe()?;
    let runtime = vocal_runtime::resolve(
        exe.parent().context("Probe directory")?,
        explicit_runtime
            .map(Into::into)
            .or_else(|| std::env::var_os("ORT_DYLIB_PATH")),
    )?
    .context("Set --runtime for standalone JDC research")?;
    ensure!(
        runtime.source_sha256 == vocal_runtime::SOURCE_SHA256,
        "JDC research requires the pinned ONNX Runtime 1.24.4 macOS arm64 runtime"
    );
    let meta = chart_engine::pipeline::metadata(input)?;
    ensure!(
        meta.duration.is_finite() && (0.129..=3600.).contains(&meta.duration),
        "JDC audio duration is unsupported"
    );
    let source = models::hash_file(input)?;
    let pcm = models::decode(input, jdc::RATE as u32, 1)?;
    let f = jdc::features(&pcm, &norm, &|| false)?;
    ort::init_from(runtime.path.to_str().context("Runtime path")?).commit()?;
    let mut session = ort::session::Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_file(&graph)?;
    let mut frames = Vec::with_capacity(f.frame_count);
    let mut posterior = raw
        .map(|p| {
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(p)
                .map(std::io::BufWriter::new)
        })
        .transpose()?;
    for batch in f.values.chunks(16 * jdc::WINDOW_FRAMES * jdc::BINS) {
        let b = batch.len() / (jdc::WINDOW_FRAMES * jdc::BINS);
        let out = session.run(ort::inputs!["features"=>ort::value::Tensor::from_array(([b,1,31,513],batch.to_vec()))?])?;
        let (shape, pitch) = out[0].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [b as i64, 31, 722],
            "JDC pitch graph shape differs"
        );
        let (shape, activity) = out[1].try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [b as i64, 31, 2]
                && activity
                    .iter()
                    .all(|p| p.is_finite() && (0. ..=1.).contains(p)),
            "JDC activity graph shape or values differ"
        );
        for (i, row) in pitch.chunks_exact(jdc::CLASSES).enumerate() {
            if frames.len() == f.frame_count {
                break;
            }
            frames.push(jdc::frame(frames.len(), row, activity[i * 2 + 1])?);
            if let Some(writer) = &mut posterior {
                for p in row {
                    writer.write_all(&p.to_le_bytes())?;
                }
            }
        }
    }
    if let Some(writer) = &mut posterior {
        writer.flush()?;
    }
    ensure!(
        models::hash_file(input)? == source && models::hash_file(&graph)? == jdc::MODEL_SHA256,
        "JDC source or graph changed during inference"
    );
    let evidence = jdc::Evidence {
        version: 1,
        source_sha256: source,
        pcm_sha256: f.pcm_sha256,
        checkpoint_sha256: jdc::CHECKPOINT_SHA256.into(),
        model_sha256: jdc::MODEL_SHA256.into(),
        implementation_sha256: jdc::implementation_hash(),
        runtime_sha256: runtime.sha256,
        runtime_publisher_sha256: runtime.source_sha256,
        sample_count: f.sample_count,
        sample_rate: jdc::RATE,
        hop_samples: jdc::HOP,
        duration_seconds: f.sample_count as f64 / jdc::RATE as f64,
        frames,
    };
    jdc::validate(
        &evidence,
        &evidence.source_sha256,
        evidence.duration_seconds,
    )?;
    write_new(output, &serde_json::to_vec(&evidence)?)
}
fn main() -> Result<()> {
    match Args::parse().command {
        Action::Convert { checkpoint, output } => convert::convert(&checkpoint, &output),
        Action::Infer {
            input,
            model_dir,
            runtime,
            output,
            posterior,
        } => infer(&input, &model_dir, runtime, &output, posterior.as_deref()),
        Action::Analyze {
            baseline,
            audio,
            input,
            model_dir,
            rmvpe_model,
            runtime,
            output,
        } => analyze(
            &baseline,
            &audio,
            &input,
            &model_dir,
            &rmvpe_model,
            runtime,
            &output,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn analyze(
    baseline: &Path,
    audio: &Path,
    input: &Path,
    model_dir: &Path,
    rmvpe_model: &Path,
    runtime: Option<PathBuf>,
    output: &Path,
) -> Result<()> {
    use chart_engine::{vocal_generation::Generation, vocal_rmvpe, vocal_singing_tracking};
    ensure!(!output.exists(), "Use a new singing draft output directory");
    let mut base: Generation = serde_json::from_slice(&fs::read(baseline)?)?;
    ensure!(
        models::hash_file(audio)? == base.audio_sha256
            && models::hash_file(input)? == base.vocal_stem_sha256
            && models::hash_file(rmvpe_model)? == vocal_rmvpe::SHA256,
        "Singing draft source or RMVPE model differs"
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".singing-draft-")
        .tempdir_in(parent)?;
    infer(
        input,
        model_dir,
        runtime,
        &stage.path().join("jdc.json"),
        None,
    )?;
    let evidence: jdc::Evidence =
        serde_json::from_slice(&fs::read(stage.path().join("jdc.json"))?)?;
    base.rmvpe_pitch=Some(vocal_rmvpe::analyze(audio,input,&chart_engine::clean::StemAlignment {audio_sha256:base.audio_sha256.clone(),stem_sha256:base.vocal_stem_sha256.clone(),evidence:"Explicit source-bound acoustic baseline; isolated vocal stem retained on the original clock".into()},rmvpe_model,&||false)?);
    let result = vocal_singing_tracking::decode(&base, &evidence, &|| false)?;
    write_new(
        &stage.path().join("generation.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    if let Some(chart) = &result.chart {
        write_new(
            &stage.path().join("vocals.json"),
            &serde_json::to_vec_pretty(chart)?,
        )?;
        let timing = result.timing.as_ref().context("Source tempo map")?;
        if timing.offset == 0. {
            let midi = chart_engine::vocals::append_midi(
                &chart_engine::midi::new_midi(timing, &Default::default())?,
                chart,
            )?;
            let back = chart_engine::vocals::import_midi(&midi)?;
            ensure!(
                back.notes == chart.notes && back.phrases == chart.phrases,
                "Singing draft MIDI round-trip differs"
            );
            write_new(&stage.path().join("vocals.mid"), &midi)?;
        }
    }
    ensure!(
        models::hash_file(audio)? == base.audio_sha256
            && models::hash_file(input)? == base.vocal_stem_sha256,
        "Singing draft inputs changed"
    );
    ensure!(!output.exists(), "Singing output appeared during analysis");
    chart_engine::pipeline::publish(stage.path(), output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_wrong_models_before_loading_a_runtime_or_creating_outputs() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("original-rust.onnx"), b"wrong").unwrap();
        let out = root.path().join("evidence.json");
        assert!(
            infer(
                &root.path().join("voice.wav"),
                root.path(),
                None,
                &out,
                None
            )
            .unwrap_err()
            .to_string()
            .contains("checksum differs")
        );
        assert!(!out.exists());
        assert!(convert::convert(&root.path().join("original-rust.onnx"), &out).is_err());
        assert!(!out.exists());
    }
    #[test]
    fn existing_results_survive() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("evidence.json");
        fs::write(&out, b"saved").unwrap();
        assert!(infer(Path::new("absent"), root.path(), None, &out, None).is_err());
        assert_eq!(fs::read(out).unwrap(), b"saved");
    }
}
