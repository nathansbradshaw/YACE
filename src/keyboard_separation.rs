//! Optional BS-RoFormer SW six-stem inference. This independent Rust frontend
//! uses published MIT MSS_ONNX_TensorRT tensor/math contracts, not GPL code.
//! Original weight terms are unresolved; see licenses/BS-RoFormer-SW-NOTICE.md.
use crate::{
    models,
    stem_review::{SeparationIdentity, StemId},
};
use anyhow::{Context, Result, ensure};
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    tensor::TensorElementType,
    value::{Tensor, ValueType},
};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const MODEL_ID: &str = "keyboard-separation-bs-roformer-sw";
pub const MODEL_FILENAME: &str = "bs_roformer_sw.onnx";
pub const MODEL_REVISION: &str = "440487b8300dcd61453cc52ec244a38150b03456";
pub const MODEL_URL: &str = "https://huggingface.co/scragnog/HOT-Step-CPP-SuperSep/resolve/440487b8300dcd61453cc52ec244a38150b03456/bs_roformer_sw.onnx";
pub const MODEL_SHA256: &str = "d2a13ad34eb26a8aa3135314b2cf1efb41e70289ffb9d03124a4201736a56df3";
pub const MODEL_BYTES: u64 = 704_536_949;
pub const WEIGHT_LICENSE_NOTICE: &str = "Original BS-RoFormer SW weight license unresolved (checkpoint mirror declares unknown). MIT covers conversion/packaging only and does not relicense weights. Not included with source/default model set; optional local model under its own unresolved terms. See BS-RoFormer-SW-NOTICE.md.";
pub const SAMPLE_RATE: u32 = 44_100;
pub const FFT_SIZE: usize = 2048;
pub const FFT_HOP: usize = 512;
pub const FREQUENCIES: usize = FFT_SIZE / 2 + 1;
pub const CHUNK_SAMPLES: usize = 588800;
pub const STEP_SAMPLES: usize = CHUNK_SAMPLES / 2;
pub const TIME_FRAMES: usize = CHUNK_SAMPLES / FFT_HOP + 1;
pub const RECEIPT_FILENAME: &str = "keyboard-separation-receipt.json";
pub const STEM_ORDER: [StemId; 6] = [
    StemId::Bass,
    StemId::Drums,
    StemId::Other,
    StemId::Vocals,
    StemId::Guitar,
    StemId::Piano,
];
const MASK_ELEMENTS: usize = 6 * FREQUENCIES * 2 * TIME_FRAMES * 2;

pub fn asset() -> models::ModelAsset {
    models::ModelAsset {
        id: MODEL_ID.into(),
        filename: MODEL_FILENAME.into(),
        url: MODEL_URL.into(),
        sha256: MODEL_SHA256.into(),
        license: WEIGHT_LICENSE_NOTICE.into(),
    }
}
fn verify_path(path: &Path) -> Result<()> {
    ensure!(
        fs::metadata(path)
            .with_context(|| format!("Optional HQ model missing: {}", path.display()))?
            .len()
            == MODEL_BYTES,
        "HQ model byte length mismatch"
    );
    ensure!(
        models::hash_file(path)? == MODEL_SHA256,
        "HQ model checksum mismatch"
    );
    Ok(())
}
fn model_path_with_cache(
    model_dir: &Path,
    requested_present: bool,
    user_cache: Option<&Path>,
) -> Result<PathBuf> {
    if requested_present || !models::bundle_model_directory(model_dir) {
        return Ok(model_dir.join(MODEL_FILENAME));
    }
    Ok(user_cache
        .context(
            "Cannot locate a writable per-user HQ model directory; supply a custom model directory",
        )?
        .join(MODEL_FILENAME))
}
/// Use a bundled model if present; download a missing one to the user's cache.
/// This keeps signed apps unchanged. Custom model directories stay as supplied.
/// Call `verified` before inference; this function only chooses the path.
pub fn resolved_model_path(model_dir: &Path) -> Result<PathBuf> {
    let requested = model_dir.join(MODEL_FILENAME);
    let present = requested.try_exists()?;
    let canonical = model_dir.canonicalize().ok();
    let protected = models::bundle_model_directory(model_dir)
        || canonical
            .as_deref()
            .is_some_and(models::bundle_model_directory);
    if present || !protected {
        return Ok(requested);
    }
    let protected_dir = if models::bundle_model_directory(model_dir) {
        model_dir
    } else {
        canonical.as_deref().context("HQ bundle path unavailable")?
    };
    model_path_with_cache(protected_dir, false, models::user_model_cache().as_deref())
}
pub fn verified(model_dir: &Path) -> Result<PathBuf> {
    let path = resolved_model_path(model_dir)?;
    verify_path(&path)?;
    Ok(path)
}
/// Explicit optional fetch only. This is deliberately absent from global assets.
pub fn fetch(model_dir: &Path) -> Result<()> {
    let target = resolved_model_path(model_dir)?;
    if target.exists() {
        verify_path(&target)?;
        return Ok(());
    }
    let target_dir = target.parent().context("HQ model directory missing")?;
    fs::create_dir_all(target_dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(target_dir)?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(900))
        .user_agent("YACE-HQ-keyboard/0.1")
        .build()?
        .get(MODEL_URL)
        .send()?
        .error_for_status()?;
    std::io::copy(&mut response.take(MODEL_BYTES + 1), &mut temporary)?;
    temporary.flush()?;
    verify_path(temporary.path())?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(target).map_err(|e| e.error)?;
    Ok(())
}
fn implementation_hash() -> String {
    let mut h = Sha256::new();
    for b in [
        include_bytes!("keyboard_separation.rs").as_slice(),
        include_bytes!("../Cargo.lock").as_slice(),
    ] {
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    }
    format!("{:x}", h.finalize())
}
pub fn identity(model: &Path) -> Result<SeparationIdentity> {
    verify_path(model)?;
    let decoder = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .context("FFmpeg is required for HQ separation")?;
    ensure!(decoder.status.success(), "Cannot identify FFmpeg");
    let decoder = String::from_utf8_lossy(&decoder.stdout)
        .lines()
        .next()
        .context("Missing FFmpeg version")?
        .to_owned();
    Ok(SeparationIdentity{model_id:MODEL_ID.into(),model_sha256:MODEL_SHA256.into(),backend:"bs-roformer-sw-ort-cpu-fp32-stream-v1".into(),parameters:BTreeMap::from([
 ("model_revision".into(),MODEL_REVISION.into()),("native_code_sha256".into(),implementation_hash()),("runtime".into(),ort::info().into()),("ort_crate".into(),"2.0.0-rc.10".into()),("threads".into(),"4".into()),("optimization".into(),"ORT Level3".into()),("decoder".into(),decoder),
 ("source_clock".into(),"44100Hz stereo; entire source; no trim, shift, normalization or limiter".into()),("stft".into(),"2048FFT/512hop periodic Hann; centered reflect; unnormalized; frequency-major stereo real/imag".into()),("contract".into(),"input:f32[1,1151,4100];complex_masks:f32[1,6,2050,1151,2]".into()),("reconstruction".into(),"complex multiplication; inverse FFT/2048; Hann-square overlap normalization; original frame count".into()),("chunking".into(),"588800 samples; 294400 hop; 10% linear fades; reflect border when source>588800; skip only chunks disjoint from source crop".into()),("stem_order".into(),"bass,drums,other,vocals,guitar,piano".into()),("storage".into(),"disk-backed decoded PCM; bounded chunk/overlap RAM; streamed RF64 f32le stereo".into()),("weights_license".into(),WEIGHT_LICENSE_NOTICE.into())]),cache_key:String::new()})
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub source_audio_sha256: String,
    pub source_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub generation: SeparationIdentity,
    pub stem_sha256: BTreeMap<String, String>,
    pub chunk_starts: Vec<u64>,
    pub chunk_seconds: Vec<f64>,
    pub skipped_padding_chunks: u64,
}

/// Exact graph contract checked before any PCM/output allocation.
pub fn validate_contract(input: &ValueType, output: &ValueType) -> Result<()> {
    ensure!(
        input.tensor_type() == Some(TensorElementType::Float32)
            && input.tensor_shape().map(|s| &s[..])
                == Some(&[1, TIME_FRAMES as i64, (FREQUENCIES * 4) as i64][..]),
        "Unexpected BS-RoFormer input tensor contract"
    );
    ensure!(
        output.tensor_type() == Some(TensorElementType::Float32)
            && output.tensor_shape().map(|s| &s[..])
                == Some(&[1, 6, (FREQUENCIES * 2) as i64, TIME_FRAMES as i64, 2][..]),
        "Unexpected BS-RoFormer output tensor contract"
    );
    Ok(())
}
fn reflect(mut i: i128, len: usize) -> usize {
    debug_assert!(len > 1);
    let len = len as i128;
    while i < 0 || i >= len {
        if i < 0 {
            i = -i;
        }
        if i >= len {
            i = 2 * len - 2 - i;
        }
    }
    i as usize
}
#[derive(Clone, Debug)]
pub struct ChunkPlan {
    pub source_frames: usize,
    pub border: usize,
    pub padded: usize,
    pub starts: Vec<usize>,
    pub skipped_padding_chunks: usize,
}
impl ChunkPlan {
    pub fn new(source_frames: usize) -> Result<Self> {
        ensure!(
            source_frames > 1,
            "HQ separation requires at least two source frames"
        );
        let border = if source_frames > CHUNK_SAMPLES {
            STEP_SAMPLES
        } else {
            0
        };
        let padded = source_frames
            .checked_add(border.checked_mul(2).context("PCM border overflow")?)
            .context("PCM frame count overflow")?;
        let end = source_frames
            .checked_add(border)
            .context("PCM crop overflow")?;
        let starts: Vec<_> = (0..padded)
            .step_by(STEP_SAMPLES)
            .filter(|start| *start < end && start.saturating_add(CHUNK_SAMPLES) > border)
            .collect();
        let skipped_padding_chunks = padded.div_ceil(STEP_SAMPLES) - starts.len();
        Ok(Self {
            source_frames,
            border,
            padded,
            starts,
            skipped_padding_chunks,
        })
    }
    pub fn weight(&self, start: usize, i: usize) -> f64 {
        let fade = CHUNK_SAMPLES / 10;
        if start == 0 && i < fade
            || start.saturating_add(STEP_SAMPLES) >= self.padded && i >= CHUNK_SAMPLES - fade
        {
            1.
        } else if i < fade {
            i as f64 / (fade - 1) as f64
        } else if i >= CHUNK_SAMPLES - fade {
            (CHUNK_SAMPLES - 1 - i) as f64 / (fade - 1) as f64
        } else {
            1.
        }
    }
    fn source_index(&self, start: usize, i: usize) -> Option<usize> {
        let available = (self.padded - start).min(CHUNK_SAMPLES);
        let j = if i < available {
            i
        } else if available > STEP_SAMPLES {
            reflect(i as i128, available)
        } else {
            return None;
        };
        Some(reflect(
            (start + j) as i128 - self.border as i128,
            self.source_frames,
        ))
    }
}
struct SourcePcm {
    temporary: tempfile::NamedTempFile,
    frames: usize,
}
impl SourcePcm {
    fn decode(audio: &Path, dir: &Path) -> Result<Self> {
        let stderr = tempfile::NamedTempFile::new_in(dir)?;
        let mut child = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(audio)
            .args([
                "-map", "0:a:0", "-f", "f32le", "-ar", "44100", "-ac", "2", "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(stderr.reopen()?)
            .spawn()
            .context("FFmpeg is required for HQ separation")?;
        let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
        let decoded = (|| -> Result<usize> {
            let mut input = BufReader::new(child.stdout.take().context("Missing decoder output")?);
            let mut bytes = [0u8; 65536];
            let mut total = 0usize;
            loop {
                let mut count = 0;
                while count < bytes.len() {
                    let n = input.read(&mut bytes[count..])?;
                    if n == 0 {
                        break;
                    }
                    count += n;
                }
                if count == 0 {
                    break;
                }
                ensure!(count % 8 == 0, "Decoded stereo frame is incomplete");
                ensure!(
                    bytes[..count]
                        .chunks_exact(4)
                        .all(|b| f32::from_le_bytes(b.try_into().unwrap()).is_finite()),
                    "Nonfinite decoded HQ source"
                );
                temporary.write_all(&bytes[..count])?;
                total = total
                    .checked_add(count)
                    .context("Decoded PCM byte count overflow")?;
            }
            ensure!(total >= 16, "Empty/one-frame HQ source");
            temporary.flush()?;
            Ok(total / 8)
        })();
        if decoded.is_err() {
            let _ = child.kill();
        }
        let status = child.wait()?;
        let frames = decoded?;
        ensure!(
            status.success(),
            "HQ source decode failed: {}",
            fs::read_to_string(stderr.path()).unwrap_or_default()
        );
        Ok(Self { temporary, frames })
    }
    fn chunk(&mut self, plan: &ChunkPlan, start: usize) -> Result<[Vec<f32>; 2]> {
        let mut min = usize::MAX;
        let mut max = 0;
        for i in 0..CHUNK_SAMPLES {
            if let Some(j) = plan.source_index(start, i) {
                min = min.min(j);
                max = max.max(j);
            }
        }
        ensure!(
            min <= max && max - min < CHUNK_SAMPLES,
            "Unexpected decoded PCM chunk span"
        );
        let count = (max - min + 1)
            .checked_mul(8)
            .context("PCM byte range overflow")?;
        let mut bytes = vec![0; count];
        self.temporary.as_file_mut().seek(SeekFrom::Start(
            (min as u64)
                .checked_mul(8)
                .context("PCM byte offset overflow")?,
        ))?;
        self.temporary.as_file_mut().read_exact(&mut bytes)?;
        let mut audio = std::array::from_fn(|_| vec![0.; CHUNK_SAMPLES]);
        let [left, right] = &mut audio;
        for (i, (left_sample, right_sample)) in left.iter_mut().zip(right).enumerate() {
            if let Some(j) = plan.source_index(start, i) {
                let offset = (j - min) * 8;
                for (ch, sample) in [left_sample, right_sample].into_iter().enumerate() {
                    *sample = f32::from_le_bytes(
                        bytes[offset + ch * 4..offset + ch * 4 + 4]
                            .try_into()
                            .unwrap(),
                    );
                }
            }
        }
        Ok(audio)
    }
}
fn hann() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT_SIZE as f64).cos()) as f32)
        .collect()
}
/// Model feature layout: [time,frequency,stereo,real/imag].
pub fn stft(audio: &[Vec<f32>; 2]) -> Result<(Vec<f32>, Vec<Complex32>)> {
    ensure!(
        audio
            .iter()
            .all(|c| c.len() == CHUNK_SAMPLES && c.iter().all(|x| x.is_finite())),
        "Invalid HQ STFT PCM"
    );
    let window = hann();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let mut original = vec![Complex32::new(0., 0.); 2 * FREQUENCIES * TIME_FRAMES];
    let mut input = vec![0.; TIME_FRAMES * FREQUENCIES * 4];
    let mut frame = vec![Complex32::new(0., 0.); FFT_SIZE];
    for ch in 0..2 {
        for t in 0..TIME_FRAMES {
            for j in 0..FFT_SIZE {
                frame[j] = Complex32::new(
                    audio[ch][reflect(
                        (t * FFT_HOP + j) as i128 - (FFT_SIZE / 2) as i128,
                        CHUNK_SAMPLES,
                    )] * window[j],
                    0.,
                );
            }
            fft.process(&mut frame);
            for f in 0..FREQUENCIES {
                let z = frame[f];
                original[(ch * FREQUENCIES + f) * TIME_FRAMES + t] = z;
                let index = t * FREQUENCIES * 4 + (f * 2 + ch) * 2;
                input[index] = z.re;
                input[index + 1] = z.im;
            }
        }
    }
    ensure!(input.iter().all(|x| x.is_finite()), "HQ STFT overflow");
    Ok((input, original))
}
/// A single source/channel plane; passing None reconstructs the original.
pub fn reconstruct(
    original: &[Complex32],
    mask: Option<(&[f32], usize)>,
    ch: usize,
) -> Result<Vec<f32>> {
    ensure!(
        ch < 2
            && original.len() == 2 * FREQUENCIES * TIME_FRAMES
            && original
                .iter()
                .all(|z| z.re.is_finite() && z.im.is_finite()),
        "Invalid original HQ spectrum"
    );
    if let Some((m, s)) = mask {
        ensure!(s < 6 && m.len() == MASK_ELEMENTS, "Invalid HQ mask plane");
    }
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_inverse(FFT_SIZE);
    let window = hann();
    let len = (TIME_FRAMES - 1) * FFT_HOP + FFT_SIZE;
    let mut out = vec![0f64; len];
    let mut norm = vec![0f64; len];
    let mut frame = vec![Complex32::new(0., 0.); FFT_SIZE];
    for t in 0..TIME_FRAMES {
        for f in 0..FREQUENCIES {
            let mut z = original[(ch * FREQUENCIES + f) * TIME_FRAMES + t];
            if let Some((m, s)) = mask {
                let i = ((s * FREQUENCIES * 2 + f * 2 + ch) * TIME_FRAMES + t) * 2;
                ensure!(
                    m[i].is_finite() && m[i + 1].is_finite(),
                    "Nonfinite HQ mask"
                );
                z *= Complex32::new(m[i], m[i + 1]);
            }
            frame[f] = z;
        }
        for f in 1..FFT_SIZE / 2 {
            frame[FFT_SIZE - f] = frame[f].conj();
        }
        fft.process(&mut frame);
        for j in 0..FFT_SIZE {
            let w = window[j] as f64;
            out[t * FFT_HOP + j] += frame[j].re as f64 / FFT_SIZE as f64 * w;
            norm[t * FFT_HOP + j] += w * w;
        }
    }
    (FFT_SIZE / 2..FFT_SIZE / 2 + CHUNK_SAMPLES)
        .map(|i| {
            ensure!(norm[i] > 1e-12, "Uncovered inverse-STFT frame");
            let value = (out[i] / norm[i]) as f32;
            ensure!(value.is_finite(), "Nonfinite HQ reconstruction");
            Ok(value)
        })
        .collect()
}
/// RF64 avoids a four-gigabyte RIFF limit without retaining song PCM in RAM.
struct WaveStream {
    temporary: tempfile::NamedTempFile,
    output: BufWriter<File>,
    frames: u64,
    written: u64,
}
impl WaveStream {
    fn new(dir: &Path, frames: u64) -> Result<Self> {
        let temporary = tempfile::NamedTempFile::new_in(dir)?;
        let mut output = BufWriter::with_capacity(262144, temporary.reopen()?);
        let bytes = frames.checked_mul(8).context("WAV size overflow")?;
        output.write_all(b"RF64")?;
        output.write_all(&u32::MAX.to_le_bytes())?;
        output.write_all(b"WAVEds64")?;
        output.write_all(&28u32.to_le_bytes())?;
        output.write_all(
            &bytes
                .checked_add(72)
                .context("WAV size overflow")?
                .to_le_bytes(),
        )?;
        output.write_all(&bytes.to_le_bytes())?;
        output.write_all(&frames.to_le_bytes())?;
        output.write_all(&0u32.to_le_bytes())?;
        output.write_all(b"fmt ")?;
        output.write_all(&16u32.to_le_bytes())?;
        output.write_all(&3u16.to_le_bytes())?;
        output.write_all(&2u16.to_le_bytes())?;
        output.write_all(&SAMPLE_RATE.to_le_bytes())?;
        output.write_all(&(SAMPLE_RATE * 8).to_le_bytes())?;
        output.write_all(&8u16.to_le_bytes())?;
        output.write_all(&32u16.to_le_bytes())?;
        output.write_all(b"data")?;
        output.write_all(&u32::MAX.to_le_bytes())?;
        Ok(Self {
            temporary,
            output,
            frames,
            written: 0,
        })
    }
    fn frame(&mut self, l: f32, r: f32) -> Result<()> {
        ensure!(
            l.is_finite() && r.is_finite() && self.written < self.frames,
            "Invalid/extra HQ output sample"
        );
        self.output.write_all(&l.to_le_bytes())?;
        self.output.write_all(&r.to_le_bytes())?;
        self.written += 1;
        Ok(())
    }
    fn finish(mut self, path: &Path) -> Result<()> {
        ensure!(self.written == self.frames, "Incomplete HQ output clock");
        self.output.flush()?;
        self.output.get_ref().sync_all()?;
        ensure!(
            self.temporary.as_file().metadata()?.len() == 80 + self.frames * 8,
            "HQ WAV byte count mismatch"
        );
        drop(self.output);
        self.temporary
            .persist_noclobber(path)
            .map_err(|e| e.error)?;
        Ok(())
    }
}
struct Overlap {
    planes: Vec<Vec<f32>>,
    counter: Vec<f64>,
}
impl Overlap {
    fn new() -> Self {
        Self {
            planes: (0..12).map(|_| vec![0.; CHUNK_SAMPLES]).collect(),
            counter: vec![0.; CHUNK_SAMPLES],
        }
    }
    fn add(&mut self, plane: usize, data: &[f32], weights: &[f64], available: usize) -> Result<()> {
        for i in 0..available {
            self.planes[plane][i] += (data[i] as f64 * weights[i]) as f32;
            ensure!(self.planes[plane][i].is_finite(), "HQ overlap overflow");
        }
        Ok(())
    }
    fn flush(
        &mut self,
        plan: &ChunkPlan,
        start: usize,
        until: usize,
        mut write: impl FnMut(usize, f32, f32) -> Result<()>,
    ) -> Result<()> {
        let count = until - start;
        ensure!(count <= CHUNK_SAMPLES, "Invalid HQ overlap flush");
        for i in 0..count {
            let position = start + i;
            if position >= plan.border && position < plan.border + plan.source_frames {
                ensure!(self.counter[i] > 0., "Uncovered HQ source sample");
                for s in 0..6 {
                    write(
                        s,
                        (self.planes[s * 2][i] as f64 / self.counter[i]) as f32,
                        (self.planes[s * 2 + 1][i] as f64 / self.counter[i]) as f32,
                    )?;
                }
            }
        }
        for plane in &mut self.planes {
            plane.copy_within(count.., 0);
            plane[CHUNK_SAMPLES - count..].fill(0.);
        }
        self.counter.copy_within(count.., 0);
        self.counter[CHUNK_SAMPLES - count..].fill(0.);
        Ok(())
    }
}
/// Write all six full-source stems into an existing empty transaction directory.
/// RAM is bounded by fixed model/chunk/overlap buffers; decoded source is spooled
/// to temporary disk and completed output frames stream to RF64 writers.
pub fn separate(audio: &Path, model: &Path, dir: &Path) -> Result<()> {
    ensure!(
        dir.is_dir() && fs::read_dir(dir)?.next().is_none(),
        "HQ separation output must be an existing empty directory"
    );
    let source_hash = models::hash_file(audio)?;
    let mut generation = identity(model)?;
    generation.cache_key = generation.calculated_cache_key(&source_hash)?;
    let mut session = Session::builder()?
        .with_intra_threads(4)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(model)?;
    ensure!(
        session.inputs.len() == 1
            && session.inputs[0].name == "input"
            && session.outputs.len() == 1
            && session.outputs[0].name == "output",
        "Unexpected HQ ONNX names/counts"
    );
    validate_contract(
        &session.inputs[0].input_type,
        &session.outputs[0].output_type,
    )?;
    let mut pcm = SourcePcm::decode(audio, dir)?;
    let plan = ChunkPlan::new(pcm.frames)?;
    ensure!(
        models::hash_file(audio)? == source_hash,
        "HQ source changed during decode"
    );
    let mut writers = (0..6)
        .map(|_| WaveStream::new(dir, pcm.frames as u64))
        .collect::<Result<Vec<_>>>()?;
    let mut overlap = Overlap::new();
    let mut seconds = Vec::with_capacity(plan.starts.len());
    for (number, &start) in plan.starts.iter().enumerate() {
        let begin = Instant::now();
        let audio = pcm.chunk(&plan, start)?;
        let (input, original) = stft(&audio)?;
        eprintln!(
            "HQ keyboard separation: chunk {}/{}",
            number + 1,
            plan.starts.len()
        );
        let result = session.run(
            ort::inputs!["input"=>Tensor::from_array(([1,TIME_FRAMES,FREQUENCIES*4],input))?],
        )?;
        let (shape, mask) = result["output"].try_extract_tensor::<f32>()?;
        ensure!(
            shape[..] == [1, 6, (FREQUENCIES * 2) as i64, TIME_FRAMES as i64, 2]
                && mask.len() == MASK_ELEMENTS,
            "HQ output shape changed"
        );
        ensure!(
            mask.iter().all(|x| x.is_finite()),
            "Nonfinite HQ model output"
        );
        let available = (plan.padded - start).min(CHUNK_SAMPLES);
        let weights: Vec<_> = (0..available).map(|i| plan.weight(start, i)).collect();
        for s in 0..6 {
            for ch in 0..2 {
                let data = reconstruct(&original, Some((mask, s)), ch)?;
                overlap.add(s * 2 + ch, &data, &weights, available)?;
            }
        }
        for (i, &weight) in weights.iter().enumerate() {
            overlap.counter[i] += weight;
        }
        let until = plan
            .starts
            .get(number + 1)
            .copied()
            .unwrap_or(plan.border + plan.source_frames);
        overlap.flush(&plan, start, until, |s, l, r| writers[s].frame(l, r))?;
        seconds.push(begin.elapsed().as_secs_f64());
    }
    ensure!(
        models::hash_file(audio)? == source_hash,
        "HQ source changed during inference"
    );
    verify_path(model)?;
    drop(session);
    drop(pcm);
    let mut hashes = BTreeMap::new();
    for (id, writer) in STEM_ORDER.into_iter().zip(writers) {
        let path = dir.join(format!("{}.wav", id.as_str()));
        writer.finish(&path)?;
        hashes.insert(id.as_str().into(), models::hash_file(&path)?);
    }
    ensure!(
        models::hash_file(audio)? == source_hash,
        "HQ source changed during export"
    );
    let receipt = Receipt {
        version: 1,
        source_audio_sha256: source_hash,
        source_frames: plan.source_frames as u64,
        sample_rate: SAMPLE_RATE,
        channels: 2,
        generation,
        stem_sha256: hashes,
        chunk_starts: plan.starts.iter().map(|s| *s as u64).collect(),
        chunk_seconds: seconds,
        skipped_padding_chunks: plan.skipped_padding_chunks as u64,
    };
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dir.join(RECEIPT_FILENAME))?;
    serde_json::to_writer_pretty(&mut output, &receipt)?;
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    #[test]
    fn optional_model_paths_preserve_bundles_and_explicit_directories() {
        let bundle = Path::new("/Applications/YACE.app/Contents/Resources/models");
        let cache = Path::new("/Users/test/Library/Application Support/YACE/models");
        assert_eq!(
            model_path_with_cache(bundle, false, Some(cache)).unwrap(),
            cache.join(MODEL_FILENAME)
        );
        assert_eq!(
            model_path_with_cache(bundle, true, Some(cache)).unwrap(),
            bundle.join(MODEL_FILENAME)
        );
        assert!(model_path_with_cache(bundle, false, None).is_err());
        for custom in [
            Path::new("models"),
            Path::new("/work/custom-models"),
            Path::new("/work/YACE.app-assets/models"),
            Path::new("/Applications/YACE.app/Contents/Resources/models/../../../../custom"),
        ] {
            assert_eq!(
                model_path_with_cache(custom, false, None).unwrap(),
                custom.join(MODEL_FILENAME)
            );
        }
        assert!(models::bundle_model_directory(&bundle.join("nested")));
        assert!(models::bundle_model_directory(Path::new(
            "/Applications/YACE.APP/Contents/Resources/models"
        )));
    }
    #[test]
    fn streamed_overlap_is_bit_exact_with_original_full_buffer_and_padding_chunks() {
        // A deterministic, chunk-dependent fake forward pass exercises every
        // source sample, overlap rounding, source boundary and all twelve planes.
        // No neural model, chart, or recording is involved in this parity proof.
        for frames in [2, STEP_SAMPLES + 1, CHUNK_SAMPLES + 1, 1_323_000] {
            let plan = ChunkPlan::new(frames).unwrap();
            let mut reference = vec![vec![0f32; plan.padded]; 12];
            let mut norm = vec![0f64; plan.padded];
            let sample = |start: usize, i: usize, plane: usize| {
                ((start % 1103) as f32 * 0.00001 + (i % 97) as f32 * 0.00003 + plane as f32 * 0.017)
                    - 0.1
            };
            for start in (0..plan.padded).step_by(STEP_SAMPLES) {
                let available = (plan.padded - start).min(CHUNK_SAMPLES);
                for i in 0..available {
                    let weight = plan.weight(start, i);
                    norm[start + i] += weight;
                    for (plane, samples) in reference.iter_mut().enumerate() {
                        samples[start + i] += (sample(start, i, plane) as f64 * weight) as f32;
                    }
                }
            }
            let mut overlap = Overlap::new();
            let mut observed = [0usize; 6];
            for (n, &start) in plan.starts.iter().enumerate() {
                let available = (plan.padded - start).min(CHUNK_SAMPLES);
                let weights: Vec<_> = (0..available).map(|i| plan.weight(start, i)).collect();
                for plane in 0..12 {
                    let data: Vec<_> = (0..CHUNK_SAMPLES)
                        .map(|i| sample(start, i, plane))
                        .collect();
                    overlap.add(plane, &data, &weights, available).unwrap();
                }
                for (i, &weight) in weights.iter().enumerate() {
                    overlap.counter[i] += weight;
                }
                let until = plan
                    .starts
                    .get(n + 1)
                    .copied()
                    .unwrap_or(plan.border + frames);
                overlap
                    .flush(&plan, start, until, |stem, l, r| {
                        let point = plan.border + observed[stem];
                        assert_eq!(
                            l.to_bits(),
                            ((reference[stem * 2][point] as f64 / norm[point]) as f32).to_bits()
                        );
                        assert_eq!(
                            r.to_bits(),
                            ((reference[stem * 2 + 1][point] as f64 / norm[point]) as f32)
                                .to_bits()
                        );
                        observed[stem] += 1;
                        Ok(())
                    })
                    .unwrap();
            }
            assert_eq!(observed, [frames; 6]);
        }
    }
    #[test]
    fn rf64_preserves_float_levels_clock_and_spooled_reflection() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("source.wav");
        let mut writer = WaveStream::new(dir.path(), 4).unwrap();
        for (l, r) in [(2., -1.5), (0.25, 0.5), (-0.75, 0.), (0.1, -0.1)] {
            writer.frame(l, r).unwrap();
        }
        writer.finish(&output).unwrap();
        let decoded = models::decode(&output, SAMPLE_RATE, 2).unwrap();
        assert_eq!(decoded, vec![2., -1.5, 0.25, 0.5, -0.75, 0., 0.1, -0.1]);
        let mut source = SourcePcm::decode(&output, dir.path()).unwrap();
        assert_eq!(source.frames, 4);
        let plan = ChunkPlan::new(4).unwrap();
        let chunk = source.chunk(&plan, 0).unwrap();
        assert_eq!(&chunk[0][..5], &[2., 0.25, -0.75, 0.1, 0.]);
        assert!(chunk.iter().all(|c| c[4..].iter().all(|s| *s == 0.)));
        // The long-source reflected border and partially reflected tail must
        // also agree with the original prototype's explicit padded buffer.
        for frames in [CHUNK_SAMPLES + 1, 1_323_000] {
            let plan = ChunkPlan::new(frames).unwrap();
            for &start in &plan.starts {
                let available = (plan.padded - start).min(CHUNK_SAMPLES);
                for i in [0, 1, available - 1, CHUNK_SAMPLES - 1] {
                    let expected = if i < available {
                        Some(reflect((start + i) as i128 - plan.border as i128, frames))
                    } else if available > STEP_SAMPLES {
                        Some(reflect(
                            (start + reflect(i as i128, available)) as i128 - plan.border as i128,
                            frames,
                        ))
                    } else {
                        None
                    };
                    assert_eq!(plan.source_index(start, i), expected);
                }
            }
        }
    }
    #[test]
    fn writer_rejects_nonfinite_and_incomplete_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = WaveStream::new(dir.path(), 2).unwrap();
        assert!(writer.frame(f32::NAN, 0.).is_err());
        writer.frame(0., 0.).unwrap();
        assert!(writer.finish(&dir.path().join("invalid.wav")).is_err());
        assert!(!dir.path().join("invalid.wav").exists());
        assert!(WaveStream::new(dir.path(), u64::MAX).is_err());
    }
}
