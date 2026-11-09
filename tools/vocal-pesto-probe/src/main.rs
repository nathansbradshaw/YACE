//! Pinned PESTO context research. No targets, lyrics, or chart edits enter inference.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use ort::{session::Session, value::Tensor};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};
const SHA: &str = "8048fd691f9018d86977bc3cb745e02f265d800df55513b921c64e464dee43e5";
const RATE: usize = 44100;
const HOP: usize = 512;
const CACHE: usize = 3840;
#[derive(Parser)]
struct Args {
    #[arg(long)]
    model: PathBuf,
    #[arg(
        long,
        required_unless_present = "controls",
        conflicts_with = "controls"
    )]
    input: Option<PathBuf>,
    #[arg(long)]
    controls: bool,
    /// Reproduce the published one-hop streaming wrapper (no future audio).
    #[arg(long)]
    streaming: bool,
    /// Offline core length; the same original-clock context is used per frame.
    #[arg(long, default_value_t = 128, conflicts_with = "streaming")]
    core_frames: usize,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Clone, Serialize)]
struct Frame {
    time_seconds: f64,
    midi: f32,
    confidence: f32,
    volume: f32,
    rms: f64,
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn decode(path: &Path) -> Result<Vec<f32>> {
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-f", "f32le", "-ar", "44100", "-ac", "1", "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| -> Result<_> {
        let mut bytes = vec![];
        child
            .stdout
            .take()
            .context("Decode pipe")?
            .take((RATE * 3600 * 4 + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= RATE * 3600 * 4 && bytes.len().is_multiple_of(4),
            "Finite audio of at most one hour required"
        );
        let pcm: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(pcm.iter().all(|v| v.is_finite()), "Nonfinite PCM");
        Ok(pcm)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let pcm = result?;
    ensure!(status.success(), "FFmpeg decode failed");
    Ok(pcm)
}
fn infer(
    pcm: &[f32],
    model: &Path,
    directory: &Path,
    core_frames: usize,
    offline: bool,
) -> Result<Vec<Frame>> {
    ensure!(
        !pcm.is_empty() && pcm.len() <= RATE * 3600 && pcm.iter().all(|v| v.is_finite()),
        "Invalid PCM"
    );
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(model)?;
    ensure!(
        session.inputs.len() == 2
            && session.inputs[0].name == "audio"
            && session.inputs[1].name == "cache"
            && session.inputs[1]
                .input_type
                .tensor_shape()
                .is_some_and(|s| s.as_ref() == [1, CACHE as i64]),
        "Unexpected pinned PESTO input contract"
    );
    ensure!(
        session.outputs.iter().map(|o| o.name.as_str()).eq([
            "prediction",
            "confidence",
            "volume",
            "activations",
            "cache_out"
        ]),
        "Unexpected PESTO outputs"
    );
    ensure!((1..=256).contains(&core_frames), "Invalid core budget");
    let mut frames = vec![];
    let mut activation_bins = None;
    let mut raw = BufWriter::new(
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("activations.f32le"))?,
    );
    let total = pcm.len().div_ceil(HOP);
    for first in (0..total).step_by(core_frames) {
        let count = core_frames.min(total - first);
        let evaluated = count + if offline { CACHE.div_ceil(HOP) } else { 0 };
        let (cache, chunk) = context(pcm, first, evaluated);
        let audio = Tensor::from_array(([1, chunk.len()], chunk.into_boxed_slice()))?;
        let state = Tensor::from_array(([1, CACHE], cache.into_boxed_slice()))?;
        let output = session.run(ort::inputs!["audio"=>audio,"cache"=>state])?;
        let (ps, p) = output["prediction"].try_extract_tensor::<f32>()?;
        let (cs, c) = output["confidence"].try_extract_tensor::<f32>()?;
        let (vs, v) = output["volume"].try_extract_tensor::<f32>()?;
        let (shape, a) = output["activations"].try_extract_tensor::<f32>()?;
        let (ss, s) = output["cache_out"].try_extract_tensor::<f32>()?;
        ensure!(
            ps.as_ref() == [1, evaluated as i64]
                && cs.as_ref() == [1, evaluated as i64]
                && vs.as_ref() == [1, evaluated as i64]
                && ss.as_ref() == [1, CACHE as i64]
                && p.len() == evaluated
                && c.len() == evaluated
                && v.len() == evaluated
                && s.len() == CACHE,
            "PESTO output clock/cache contract differs"
        );
        ensure!(
            shape.len() == 3
                && shape[0] == 1
                && shape[1] == evaluated as i64
                && shape[2] == 384
                && a.len() == evaluated * 384,
            "Activation grid differs"
        );
        if let Some(bins) = activation_bins {
            ensure!(bins == 384, "Activation bins changed");
        } else {
            activation_bins = Some(384);
        }
        ensure!(
            p.iter().all(|x| x.is_finite() && (0. ..=128.).contains(x))
                && c.iter().all(|x| x.is_finite() && (0. ..=1.).contains(x))
                && v.iter().all(|x| x.is_finite() && *x >= 0.)
                && s.iter().all(|x| x.is_finite())
                && a.iter().all(|x| x.is_finite() && *x >= 0. && *x <= 1.00001),
            "Invalid PESTO values"
        );
        for x in &a[..count * 384] {
            raw.write_all(&x.to_le_bytes())?;
        }
        for local in 0..count {
            let i = first + local;
            let source = &pcm[i * HOP..((i + 1) * HOP).min(pcm.len())];
            frames.push(Frame {
                time_seconds: (i * HOP + HOP / 2) as f64 / RATE as f64,
                midi: p[local],
                confidence: c[local],
                volume: v[local],
                rms: (source.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
                    / source.len() as f64)
                    .sqrt(),
            });
        }
    }
    raw.flush()?;
    fs::write(
        directory.join("activation-grid.json"),
        serde_json::to_vec(
            &serde_json::json!({"frames":frames.len(),"bins":activation_bins,"dtype":"f32le","sha256":hash_file(&directory.join("activations.f32le"))?}),
        )?,
    )?;
    Ok(frames)
}
/// Use only original-clock samples. A previous output cache would contain the
/// lookahead as well and would duplicate future audio at the next seam.
fn context(pcm: &[f32], first: usize, evaluated: usize) -> (Vec<f32>, Vec<f32>) {
    let start = first * HOP;
    let mut cache = vec![0.; CACHE];
    for (j, value) in cache.iter_mut().enumerate() {
        if let Some(at) = (start + j).checked_sub(CACHE) {
            *value = pcm.get(at).copied().unwrap_or(0.);
        }
    }
    let mut chunk = vec![0.; evaluated * HOP];
    let available = chunk.len().min(pcm.len().saturating_sub(start));
    chunk[..available].copy_from_slice(&pcm[start..start + available]);
    (cache, chunk)
}
fn tone(hz: f64, harmonics: bool) -> Vec<f32> {
    (0..RATE * 2)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            if !(0.25..1.75).contains(&t) {
                return 0.;
            }
            let phase = std::f64::consts::TAU * hz * t;
            let x = if harmonics {
                0.12 * phase.sin() + 0.22 * (2. * phase).sin() + 0.08 * (3. * phase).sin()
            } else {
                0.25 * phase.sin()
            };
            x as f32
        })
        .collect()
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        (1..=256).contains(&args.core_frames),
        "Choose 1–256 core frames"
    );
    let core_frames = if args.streaming { 1 } else { args.core_frames };
    ensure!(!args.output.exists(), "Output exists");
    ensure!(
        fs::metadata(&args.model)?.len() == 17019271 && hash_file(&args.model)? == SHA,
        "Pinned model differs"
    );
    let runtime = PathBuf::from(
        std::env::var_os("ORT_DYLIB_PATH").context("Explicit ORT_DYLIB_PATH is required")?,
    );
    let runtime_hash = hash_file(&runtime).context("Read explicit native ONNX Runtime")?;
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    let destination = args.output;
    let output = stage.path();
    let timer = Instant::now();
    let profile = serde_json::json!({
        "id":"pesto-mir1k-g7-44100-hop512-context-research-v1","model_sha256":SHA,
        "onnx_publisher_revision":"9fc9108a3578cfb3e842a44a35b8aecccc4b8d0a",
        "upstream_source_reviewed":"62bc0c9702558f19af4593752947fb9db1eadac9",
        "sample_rate":RATE,"hop_samples":HOP,"cache_floats":CACHE,"core_frames":core_frames,
        "mode":if args.streaming {"streaming-zero-future"}else{"offline-original-future"},
        "lookahead_samples":if args.streaming {0}else{CACHE.div_ceil(HOP)*HOP},
        "clock":"CQT window centre = source chunk start + 256 samples; final zero-padded frame retained explicitly",
        "output_units":"fractional MIDI semitones, not Hz",
        "implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("main.rs").as_slice(),include_bytes!("../Cargo.toml").as_slice(),include_bytes!("../Cargo.lock").as_slice()].concat())),
        "model_url":"https://media.githubusercontent.com/media/QosmoInc/pesto_tilde/9fc9108a3578cfb3e842a44a35b8aecccc4b8d0a/misc/mir-1k_g7_44100_512.onnx",
        "confidence_meaning":"model periodicity salience, not calibrated probability of correct lead singing",
        "threads":2,"model_bytes":17019271,
        "native_runtime_sha256":runtime_hash,
        "audio_decode":"FFmpeg first audio stream, mono 44100 Hz f32le; decoded PCM checksum retained",
        "target_values_read":false,"status":"experimental; not integrated or quality-approved"
    });
    fs::write(
        output.join("profile.json"),
        serde_json::to_vec_pretty(&profile)?,
    )?;
    if args.controls {
        let mut results = vec![];
        for (name, hz, harmonics) in [
            ("110_sine", 110., false),
            ("220_sine", 220., false),
            ("440_sine", 440., false),
            ("110_harmonics", 110., true),
            ("silence", 0., false),
        ] {
            let dir = output.join(name);
            fs::create_dir(&dir)?;
            let pcm = if hz == 0. {
                vec![0.; RATE * 2]
            } else {
                tone(hz, harmonics)
            };
            let frames = infer(&pcm, &args.model, &dir, core_frames, !args.streaming)?;
            let core: Vec<_> = frames
                .iter()
                .filter(|f| (0.5..1.5).contains(&f.time_seconds))
                .collect();
            let expected = (hz > 0.).then(|| 69. + 12. * (hz / 440.).log2());
            let mut pitches: Vec<_> = core
                .iter()
                .filter(|f| f.confidence >= 0.5)
                .map(|f| f.midi as f64)
                .collect();
            pitches.sort_by(f64::total_cmp);
            let median = pitches.get(pitches.len() / 2).copied();
            results.push(serde_json::json!({"name":name,"expected_midi":expected,"median_voiced_midi":median,"voiced_frames":pitches.len(),"frames":core.len(),"correct_pitch_frames":core.iter().filter(|f|expected.is_some_and(|p|(p-f.midi as f64).abs()<=0.5)&&f.confidence>=0.5).count(),"max_confidence":core.iter().map(|f|f.confidence).fold(0f32,f32::max)}));
            fs::write(dir.join("frames.json"), serde_json::to_vec(&frames)?)?;
        }
        fs::write(
            output.join("controls.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"profile":profile,"runtime_seconds":timer.elapsed().as_secs_f64(),"results":results}),
            )?,
        )?;
    } else {
        let source = args.input.context("Input required")?;
        let source_hash = hash_file(&source)?;
        let pcm = decode(&source)?;
        let mut pcm_hash = Sha256::new();
        for p in &pcm {
            pcm_hash.update(p.to_le_bytes());
        }
        let frames = infer(&pcm, &args.model, output, core_frames, !args.streaming)?;
        ensure!(
            hash_file(&source)? == source_hash,
            "Source changed during analysis"
        );
        fs::write(
            output.join("evidence.json"),
            serde_json::to_vec(
                &serde_json::json!({"profile":profile,"source_sha256":source_hash,"pcm_sha256":format!("{:x}",pcm_hash.finalize()),"duration_seconds":pcm.len() as f64/RATE as f64,"runtime_seconds":timer.elapsed().as_secs_f64(),"frames":frames}),
            )?,
        )?;
    }
    ensure!(
        hash_file(&args.model)? == SHA,
        "Model changed during inference"
    );
    ensure!(
        hash_file(&runtime)? == runtime_hash,
        "Native runtime changed during inference"
    );
    ensure!(!destination.exists(), "Output appeared during inference");
    fs::rename(output, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_preserves_impulses_at_seams_and_partial_tail() {
        let mut pcm = vec![0.; HOP * 130 + 7];
        pcm[0] = 0.1;
        pcm[128 * HOP - 1] = 0.2;
        pcm[128 * HOP] = 0.3;
        *pcm.last_mut().unwrap() = 0.4;
        let (past, first) = context(&pcm, 0, 136);
        assert!(past.iter().all(|&v| v == 0.));
        assert_eq!(first[0], 0.1);
        let (past, next) = context(&pcm, 128, 11);
        assert_eq!(past[CACHE - 1], 0.2);
        assert_eq!(next[0], 0.3);
        assert_eq!(next[2 * HOP + 6], 0.4);
        assert!(next[2 * HOP + 7..].iter().all(|&v| v == 0.));
        // The next core and the previous lookahead are the very same samples.
        assert_eq!(&first[128 * HOP..130 * HOP + 7], &next[..2 * HOP + 7]);
    }

    #[test]
    fn invalid_audio_is_rejected_before_loading_a_runtime_or_creating_evidence() {
        let directory = tempfile::tempdir().unwrap();
        for pcm in [vec![], vec![f32::NAN], vec![f32::INFINITY]] {
            assert!(infer(&pcm, Path::new("missing.onnx"), directory.path(), 128, true).is_err());
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn explicit_modes_cannot_silently_substitute() {
        for flags in [
            vec!["--controls"],
            vec!["--input", "song.wav"],
            vec!["--controls", "--streaming"],
        ] {
            let mut args = vec!["probe", "--model", "model.onnx", "--output", "new"];
            args.extend(flags);
            assert!(Args::try_parse_from(args).is_ok());
        }
        assert!(Args::try_parse_from(["probe", "--model", "x", "--output", "new"]).is_err());
        assert!(
            Args::try_parse_from([
                "probe",
                "--model",
                "x",
                "--output",
                "new",
                "--controls",
                "--input",
                "x"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "probe",
                "--model",
                "x",
                "--output",
                "new",
                "--controls",
                "--streaming",
                "--core-frames",
                "8"
            ])
            .is_err()
        );
    }

    #[test]
    #[ignore = "requires ORT_DYLIB_PATH and YACE_PESTO_MODEL_FILE; no downloads"]
    fn pinned_model_offline_pitch_is_invariant_to_core_length() -> Result<()> {
        let model = PathBuf::from(std::env::var("YACE_PESTO_MODEL_FILE")?);
        ensure!(hash_file(&model)? == SHA, "Model checksum differs");
        // Unaligned EOF, a short note near a core seam, abrupt pitch changes,
        // harmonic-rich singing-like signal and silence exercise actual CQT context.
        let mut pcm = tone(220., true);
        for (i, x) in pcm.iter_mut().enumerate() {
            let t = i as f64 / RATE as f64;
            if (1.42..1.49).contains(&t) {
                *x = (0.2 * (std::f64::consts::TAU * 330. * t).sin()) as f32;
            }
        }
        pcm.truncate(pcm.len() - 13);
        let one = tempfile::tempdir()?;
        let many = tempfile::tempdir()?;
        let a = infer(&pcm, &model, one.path(), 1, true)?;
        let b = infer(&pcm, &model, many.path(), 128, true)?;
        ensure!(
            a.len() == pcm.len().div_ceil(HOP) && a.len() == b.len(),
            "Clock differs"
        );
        let mut max_cents = 0f32;
        for (a, b) in a.iter().zip(&b) {
            ensure!(
                a.time_seconds == b.time_seconds && a.rms == b.rms,
                "Source clock changed"
            );
            max_cents = max_cents.max((a.midi - b.midi).abs() * 100.);
            ensure!(
                (a.midi - b.midi).abs() < 0.001 && (a.confidence - b.confidence).abs() < 0.0001,
                "Core-size-dependent pitch or confidence"
            );
        }
        eprintln!("Maximum core-size pitch difference: {max_cents} cents");
        Ok(())
    }
}
