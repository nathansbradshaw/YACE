//! Optional native LarsNet masks. The five weights remain CC-BY-NC4.0; the
//! independent Rust frontend/converter does not relicense them. No model is
//! bundled, downloaded, selected by default, or used to approve chart notes.
use anyhow::{Context, Result, ensure};
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    process::Command,
    time::Instant,
};

pub const ALGORITHM: &str = "larsnet-native-masks-v1";
pub const WEIGHT_LICENSE: &str =
    "CC-BY-NC-4.0; optional local LarsNet/LARS weights, not relicensed by the MIT Rust frontend";
pub const SAMPLE_RATE: u32 = 44100;
pub const RECEIPT_FILENAME: &str = "drum-parts-receipt.json";
pub const MODEL_BYTES: u64 = 39_341_756;
/// ONNX sigmoid SIMD roundoff only; never an instrument admission threshold.
pub const MASK_EPSILON: f32 = 1e-6;
pub const MODELS: [(&str, &str); 5] = [
    (
        "kick",
        "15f1dcd32d883a7c76827d374a56e960b87679fddfe9e35527811fedeb3c3351",
    ),
    (
        "snare",
        "16b8d4c7882473f3cccfdfad36583aa0cdbcb0c782a2db3f0ed4b7b870bf8555",
    ),
    (
        "toms",
        "f8f279925383d539d3fef7ab43ed024be2ca483ad0e5e855484b045e31ea0e00",
    ),
    (
        "hihat",
        "972d6bfe012436156f9fc66dc062bc774fb620fcad3f895c35f2acedd5499b32",
    ),
    (
        "cymbals",
        "69110dd7705a09935590ef9a22dfecd5e8d1af5656890b2d3dbd764015f9a727",
    ),
];
const FFT: usize = 4096;
const HOP: usize = 1024;
const BINS: usize = 2048;
const BLOCK: usize = 512;
const MAX_FRAMES: usize = 44100 * 600;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputReceipt {
    pub part: String,
    pub filename: String,
    pub frames: u64,
    pub pcm_sha256: String,
    pub wav_sha256: Option<String>,
    pub rms: f64,
    pub peak: f64,
    pub mask_numeric: MaskNumeric,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaskNumeric {
    pub raw_min: f32,
    pub raw_max: f32,
    pub clipped_values: u64,
}

fn sanitized_mask<'a>(mask: &'a [f32], numeric: &mut MaskNumeric) -> Result<Cow<'a, [f32]>> {
    ensure!(!mask.is_empty(), "Empty LarsNet mask");
    let mut clipped = 0;
    for (index, value) in mask.iter().copied().enumerate() {
        ensure!(
            value.is_finite() && (-MASK_EPSILON..=1. + MASK_EPSILON).contains(&value),
            "Invalid LarsNet mask index={index} value={value:?}; refusing unsafe audio"
        );
        numeric.raw_min = numeric.raw_min.min(value);
        numeric.raw_max = numeric.raw_max.max(value);
        clipped += u64::from(!(0. ..=1.).contains(&value));
    }
    numeric.clipped_values += clipped;
    Ok(if clipped == 0 {
        Cow::Borrowed(mask)
    } else {
        Cow::Owned(mask.iter().map(|v| v.clamp(0., 1.)).collect())
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeparationReceipt {
    pub version: u32,
    pub algorithm: String,
    pub implementation_sha256: String,
    pub source_audio_sha256: Option<String>,
    pub source_pcm_sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub models: BTreeMap<String, String>,
    pub weight_license: String,
    pub outputs: Vec<OutputReceipt>,
    pub elapsed_seconds: f64,
    pub warnings: Vec<String>,
}
#[derive(Debug)]
pub struct PartAudio {
    pub part: String,
    pub samples: Vec<f32>,
}
#[derive(Debug)]
pub struct Separation {
    pub parts: Vec<PartAudio>,
    pub receipt: SeparationReceipt,
}

fn pcm_hash(values: &[f32]) -> String {
    let mut hash = Sha256::new();
    for v in values {
        hash.update(v.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}
pub fn implementation_hash() -> String {
    format!("{:x}", Sha256::digest(include_bytes!("larsnet.rs")))
}
fn validate_input(input: &[f32]) -> Result<usize> {
    ensure!(
        !input.is_empty() && input.len().is_multiple_of(2),
        "LarsNet requires nonempty interleaved stereo44.1kHz PCM"
    );
    ensure!(
        input.len() / 2 <= MAX_FRAMES,
        "LarsNet in-memory route supports at most ten minutes; no source truncation"
    );
    ensure!(
        input.iter().all(|v| v.is_finite()),
        "Nonfinite LarsNet audio"
    );
    Ok(input.len() / 2)
}
fn verify_models(directory: &Path) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for (part, expected) in MODELS {
        let path = directory.join(format!("{part}.onnx"));
        let meta = fs::symlink_metadata(&path)
            .with_context(|| format!("Missing optional LarsNet model {}", path.display()))?;
        ensure!(
            meta.is_file() && meta.len() == MODEL_BYTES,
            "Invalid LarsNet model file/length: {part}"
        );
        ensure!(
            file_hash(&path)? == expected,
            "LarsNet model checksum differs: {part}"
        );
        hashes.insert(part.into(), expected.into());
    }
    Ok(hashes)
}
fn reflect(index: i64, length: usize) -> usize {
    if length == 1 {
        return 0;
    }
    let period = 2 * (length as i64 - 1);
    let index = index.rem_euclid(period);
    if index >= length as i64 {
        (period - index) as usize
    } else {
        index as usize
    }
}
fn window() -> Vec<f32> {
    (0..FFT)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT as f64).cos()) as f32)
        .collect()
}
fn spectrum_block(
    input: &[f32],
    start: usize,
    win: &[f32],
    fft: &dyn rustfft::Fft<f32>,
) -> (Vec<Complex32>, Vec<f32>, usize) {
    let frames = input.len() / 2;
    let padded = frames.div_ceil(HOP) * HOP;
    let count = (padded / HOP + 1 - start).min(BLOCK);
    let mut original = vec![Complex32::new(0., 0.); 2 * BLOCK * (FFT / 2 + 1)];
    let mut magnitude = vec![0.; 2 * BINS * BLOCK];
    let mut buffer = vec![Complex32::new(0., 0.); FFT];
    for channel in 0..2 {
        for time in 0..count {
            for (i, b) in buffer.iter_mut().enumerate() {
                let index = reflect(((start + time) * HOP + i) as i64 - (FFT / 2) as i64, padded);
                *b = Complex32::new(
                    if index < frames {
                        input[index * 2 + channel] * win[i]
                    } else {
                        0.
                    },
                    0.,
                );
            }
            fft.process(&mut buffer);
            for (freq, value) in buffer[..=FFT / 2].iter().enumerate() {
                original[(channel * BLOCK + time) * (FFT / 2 + 1) + freq] = *value;
                if freq < BINS {
                    magnitude[(channel * BINS + freq) * BLOCK + time] = value.norm();
                }
            }
        }
    }
    (original, magnitude, count)
}
fn synthesize(
    original: &[Complex32],
    mask: &[f32],
    start: usize,
    count: usize,
    win: &[f32],
    ifft: &dyn rustfft::Fft<f32>,
    output: &mut [f32],
) {
    let mut buffer = vec![Complex32::new(0., 0.); FFT];
    for channel in 0..2 {
        for time in 0..count {
            for freq in 0..BINS {
                let value = original[(channel * BLOCK + time) * (FFT / 2 + 1) + freq]
                    * mask[(channel * BINS + freq) * BLOCK + time];
                buffer[freq] = value;
                if freq > 0 {
                    buffer[FFT - freq] = value.conj();
                }
            }
            buffer[FFT / 2] = Complex32::new(0., 0.); // Official frontend pads the Nyquist mask with zero.
            ifft.process(&mut buffer);
            for (i, (value, weight)) in buffer.iter().zip(win).enumerate() {
                let index = ((start + time) * HOP + i) as i64 - (FFT / 2) as i64;
                if index >= 0 && (index as usize) < output.len() / 2 {
                    output[index as usize * 2 + channel] += value.re * weight / FFT as f32;
                }
            }
        }
    }
}
fn denominator(frames: usize, win: &[f32]) -> Vec<f32> {
    let mut result = vec![0.; frames];
    let padded = frames.div_ceil(HOP) * HOP;
    for time in 0..=padded / HOP {
        for (i, w) in win.iter().enumerate() {
            let at = (time * HOP + i) as i64 - (FFT / 2) as i64;
            if at >= 0 && (at as usize) < frames {
                result[at as usize] += w * w;
            }
        }
    }
    result
}

/// Process local 44.1 kHz stereo without changing gain or timing.
/// The independent masks may not sum back to the original mixture.
pub fn separate_stereo(input: &[f32], model_dir: &Path) -> Result<Separation> {
    separate_stereo_with_progress(input, model_dir, |_, _| Ok(()))
}
/// Call progress before and after each 512-frame ONNX block.
/// Returning Err cancels between blocks and leaves source files untouched.
/// A block already running must finish first.
pub fn separate_stereo_with_progress(
    input: &[f32],
    model_dir: &Path,
    mut progress: impl FnMut(usize, usize) -> Result<()>,
) -> Result<Separation> {
    let start_time = Instant::now();
    let frames = validate_input(input)?;
    progress(0, 1)?;
    let models = verify_models(model_dir)?;
    let mut sessions = Vec::new();
    for (part, _) in MODELS {
        progress(0, 1)?;
        sessions.push(
            Session::builder()?
                .with_intra_threads(2)?
                .commit_from_file(model_dir.join(format!("{part}.onnx")))?,
        );
    }
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT);
    let ifft = planner.plan_fft_inverse(FFT);
    let win = window();
    let mut parts: Vec<_> = MODELS
        .iter()
        .map(|(part, _)| PartAudio {
            part: (*part).into(),
            samples: vec![0.; input.len()],
        })
        .collect();
    let blocks = (frames.div_ceil(HOP) + 1).div_ceil(BLOCK);
    let total = blocks * MODELS.len();
    let mut done = 0;
    let mut mask_numeric = vec![
        MaskNumeric {
            raw_min: f32::INFINITY,
            raw_max: f32::NEG_INFINITY,
            clipped_values: 0
        };
        MODELS.len()
    ];
    for block in 0..blocks {
        let (first, magnitude, count) = spectrum_block(input, block * BLOCK, &win, fft.as_ref());
        for (part_index, (session, part)) in sessions.iter_mut().zip(&mut parts).enumerate() {
            progress(done, total)?;
            let tensor =
                Tensor::from_array(([1, 2, BINS, BLOCK], magnitude.clone().into_boxed_slice()))?;
            let prediction = session.run(ort::inputs!["magnitude"=>tensor])?;
            let (shape, mask) = prediction["mask"].try_extract_tensor::<f32>()?;
            ensure!(
                shape.as_ref() == [1, 2, BINS as i64, BLOCK as i64],
                "Unexpected LarsNet output shape"
            );
            let mask = sanitized_mask(mask, &mut mask_numeric[part_index])
                .with_context(|| format!("LarsNet {} block {block}", part.part))?;
            synthesize(
                &first,
                &mask,
                block * BLOCK,
                count,
                &win,
                ifft.as_ref(),
                &mut part.samples,
            );
            done += 1;
            progress(done, total)?;
        }
    }
    let denominator = denominator(frames, &win);
    let mut outputs = Vec::new();
    for (part, numeric) in parts.iter_mut().zip(mask_numeric) {
        for (i, frame) in part.samples.chunks_exact_mut(2).enumerate() {
            ensure!(denominator[i] > 0., "Uncovered synthesis frame");
            for sample in frame {
                *sample /= denominator[i];
            }
        }
        ensure!(
            part.samples.iter().all(|v| v.is_finite()),
            "Nonfinite separated audio"
        );
        outputs.push(OutputReceipt {
            part: part.part.clone(),
            filename: format!("{}.wav", part.part),
            frames: frames as u64,
            pcm_sha256: pcm_hash(&part.samples),
            wav_sha256: None,
            rms: (part
                .samples
                .iter()
                .map(|v| (*v as f64).powi(2))
                .sum::<f64>()
                / part.samples.len() as f64)
                .sqrt(),
            peak: part
                .samples
                .iter()
                .map(|v| (*v as f64).abs())
                .fold(0., f64::max),
            mask_numeric: numeric,
        });
    }
    ensure!(
        verify_models(model_dir)? == models,
        "LarsNet models changed during inference"
    );
    Ok(Separation{parts,receipt:SeparationReceipt{version:1,algorithm:ALGORITHM.into(),implementation_sha256:implementation_hash(),source_audio_sha256:None,source_pcm_sha256:pcm_hash(input),sample_rate:SAMPLE_RATE,channels:2,frames:frames as u64,models,weight_license:WEIGHT_LICENSE.into(),outputs,elapsed_seconds:start_time.elapsed().as_secs_f64(),warnings:vec!["Estimated drum components can contain bleed, omissions and artifacts; not approved physical-instrument truth.".into(),"Independent native graph/STFT implementation; upstream LibTorch numerical parity and professional-chart quality improvement have not been established.".into(),"Five non-normalized masks; component sum need not reconstruct the original drum mixture.".into()]}})
}
fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let bytes = u32::try_from(samples.len() * 4)?;
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&3u16.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&SAMPLE_RATE.to_le_bytes())?;
    out.write_all(&(SAMPLE_RATE * 8).to_le_bytes())?;
    out.write_all(&8u16.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        out.write_all(&sample.to_le_bytes())?;
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(())
}
pub fn separate_files(audio: &Path, model_dir: &Path, output: &Path) -> Result<SeparationReceipt> {
    separate_files_with_progress(audio, model_dir, output, |_, _| Ok(()))
}
pub fn separate_files_with_progress(
    audio: &Path,
    model_dir: &Path,
    output: &Path,
    mut progress: impl FnMut(usize, usize) -> Result<()>,
) -> Result<SeparationReceipt> {
    crate::pipeline::destination(output, None)?;
    progress(0, 1)?;
    verify_models(model_dir)?;
    let source = crate::models::hash_file(audio)?;
    // Decode up to 601 seconds to detect inputs exceeding the 600-second limit.
    // Reject those inputs instead of returning truncated stems.
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(audio)
        .args([
            "-map", "0:a:0", "-t", "601", "-f", "f32le", "-ar", "44100", "-ac", "2", "-",
        ])
        .output()
        .context("ffmpeg is required for optional LarsNet audio decoding")?;
    ensure!(
        decoded.status.success(),
        "LarsNet input audio decode failed"
    );
    ensure!(
        decoded.stdout.len().is_multiple_of(4),
        "Invalid decoded PCM length"
    );
    let samples: Vec<_> = decoded
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    drop(decoded);
    let mut result = separate_stereo_with_progress(&samples, model_dir, &mut progress)?;
    result.receipt.source_audio_sha256 = Some(source.clone());
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".drum-parts-")
        .tempdir_in(parent)?;
    for (part, receipt) in result.parts.iter().zip(&mut result.receipt.outputs) {
        progress(1, 1)?;
        let path = stage.path().join(&receipt.filename);
        write_wav(&path, &part.samples)?;
        receipt.wav_sha256 = Some(crate::models::hash_file(&path)?);
    }
    ensure!(
        crate::models::hash_file(audio)? == source,
        "LarsNet input audio changed during separation"
    );
    ensure!(
        verify_models(model_dir)? == result.receipt.models,
        "LarsNet model changed before publication"
    );
    let mut file = File::create(stage.path().join(RECEIPT_FILENAME))?;
    file.write_all(&serde_json::to_vec_pretty(&result.receipt)?)?;
    file.sync_all()?;
    progress(1, 1)?;
    crate::pipeline::publish(stage.path(), output)?;
    Ok(result.receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unity_stft_clock_and_level_survive_edges_and_block_boundary() {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT);
        let ifft = planner.plan_fft_inverse(FFT);
        let win = window();
        for frames in [1, 17, 4095, 4096, BLOCK * HOP + 89] {
            let input: Vec<_> = (0..frames)
                .flat_map(|i| {
                    let value =
                        (std::f32::consts::TAU * 440. * i as f32 / SAMPLE_RATE as f32).sin() * 0.3;
                    [value, value * 0.25]
                })
                .collect();
            let mut output = vec![0.; input.len()];
            let mask = vec![1.; 2 * BINS * BLOCK];
            for block in 0..(frames.div_ceil(HOP) + 1).div_ceil(BLOCK) {
                let (spectrum, _, count) =
                    spectrum_block(&input, block * BLOCK, &win, fft.as_ref());
                synthesize(
                    &spectrum,
                    &mask,
                    block * BLOCK,
                    count,
                    &win,
                    ifft.as_ref(),
                    &mut output,
                );
            }
            let denom = denominator(frames, &win);
            for (i, s) in output.iter_mut().enumerate() {
                *s /= denom[i / 2];
            }
            // The original frontend explicitly zeroes Nyquist; a nonperiodic
            // edge has tiny Nyquist energy. No delay or block seam is allowed.
            assert_eq!(output.len(), input.len());
            let rms = (input
                .iter()
                .zip(&output)
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>()
                / input.len() as f64)
                .sqrt();
            assert!(rms < 0.0001, "{frames} {rms}");
            if frames > BLOCK * HOP {
                for i in BLOCK * HOP - 4..BLOCK * HOP + 4 {
                    assert!((input[i * 2] - output[i * 2]).abs() < 0.0001);
                }
            }
        }
    }
    #[test]
    fn invalid_input_and_cancellation_abstain_before_model_load() {
        for input in [vec![], vec![0.], vec![f32::NAN, 0.]] {
            assert!(separate_stereo(&input, Path::new("missing")).is_err());
        }
        let result = separate_stereo_with_progress(&[0., 0.], Path::new("missing"), |_, _| {
            anyhow::bail!("cancelled")
        });
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    }

    #[test]
    fn sigmoid_roundoff_is_explicit_bounded_and_larger_errors_reject() {
        let mut stats = MaskNumeric {
            raw_min: f32::INFINITY,
            raw_max: f32::NEG_INFINITY,
            clipped_values: 0,
        };
        let raw = [-5.9604645e-8, 0., 0.5, 1., 1.0000001];
        let clean = sanitized_mask(&raw, &mut stats).unwrap();
        assert_eq!(clean.as_ref(), &[0., 0., 0.5, 1., 1.]);
        assert_eq!(stats.clipped_values, 2);
        assert_eq!(stats.raw_min, raw[0]);
        assert_eq!(stats.raw_max, raw[4]);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e-4, 1.0001] {
            assert!(sanitized_mask(&[bad], &mut stats).is_err());
        }
    }
}
