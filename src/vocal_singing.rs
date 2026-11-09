//! Pinned JDC singing/pitch research frontend shared by native and browser.
//! Singing scores and model agreement are evidence, never correctness promises.
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const REVISION: &str = "86e6d2b225c441cf5641ca74c342348646ff1bdc";
pub const CHECKPOINT_SHA256: &str =
    "72c9c038a093031cfbfd2f61bb878e3351720e4230ab2a35596df1e5dd969005";
pub const MODEL_SHA256: &str = "7f6449c25f3c45e94eea6ce1403e6d40f6bfa9deac41621c5a13a97fe8757122";
pub const MEAN_SHA256: &str = "820df6eaa4da22c496e2299409d934eaebd77b89331d4618be2291abb6d7cf95";
pub const STD_SHA256: &str = "cb8015fa1119f9b8a2a2018357c71bbb5674ff940cb356fcea23bc606fd26d83";
pub const RUNTIME_PUBLISHER_SHA256: &str =
    "f958c8c8f50a787f64cd13aa5f07710b97075d9869c6983dc0001d2452100eaa";
pub const RATE: usize = 8000;
pub const HOP: usize = 80;
pub const WINDOW_FRAMES: usize = 31;
pub const BINS: usize = 513;
pub const CLASSES: usize = 722;
pub const BANDS: usize = 49;
const WINDOW: usize = WINDOW_FRAMES * BINS;

pub fn implementation_hash() -> String {
    format!("{:x}", Sha256::digest(include_bytes!("vocal_singing.rs")))
}

/// Position-specific publisher statistics. Read as f64 before normalization.
pub struct Normalization {
    mean: Vec<f64>,
    std: Vec<f64>,
}
impl Normalization {
    pub fn from_npy(mean: &[u8], std: &[u8]) -> Result<Self> {
        fn values(bytes: &[u8], sha: &str) -> Result<Vec<f64>> {
            ensure!(
                format!("{:x}", Sha256::digest(bytes)) == sha,
                "JDC normalization checksum differs"
            );
            ensure!(
                bytes.len() >= 10 && bytes.starts_with(b"\x93NUMPY\x01\x00"),
                "JDC requires NPY v1 normalization"
            );
            let n = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
            ensure!(10 + n <= bytes.len(), "Invalid normalization header bounds");
            let h = std::str::from_utf8(&bytes[10..10 + n])?;
            ensure!(
                h.contains("'<f8'")
                    && h.contains("'fortran_order': False")
                    && h.contains("(31, 513)")
                    && bytes.len() == 10 + n + WINDOW * 8,
                "Invalid JDC normalization dimensions"
            );
            let v: Vec<_> = bytes[10 + n..]
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
                .collect();
            ensure!(
                v.iter().all(|v| v.is_finite()),
                "Nonfinite JDC normalization"
            );
            Ok(v)
        }
        let mean = values(mean, MEAN_SHA256)?;
        let std = values(std, STD_SHA256)?;
        ensure!(
            std.iter().all(|s| *s >= 0.),
            "Negative JDC standard deviation"
        );
        Ok(Self { mean, std })
    }
}

pub struct Features {
    pub values: Vec<f32>,
    pub frame_count: usize,
    pub sample_count: usize,
    pub pcm_sha256: String,
}
fn reflect(i: isize, n: usize) -> usize {
    let p = 2 * (n - 1);
    let r = i.rem_euclid(p as isize) as usize;
    if r < n { r } else { p - r }
}

/// Requires source-bound mono 8 kHz PCM. The caller owns decoding/resampling.
/// The floor is applied to magnitude dB, exactly as in the frozen experiment.
pub fn features(
    pcm: &[f32],
    norm: &Normalization,
    cancelled: &dyn Fn() -> bool,
) -> Result<Features> {
    ensure!(
        pcm.len() > 1024 && pcm.len() <= RATE * 3600 && pcm.iter().all(|v| v.is_finite()),
        "JDC requires finite 8 kHz PCM between 129ms and one hour"
    );
    ensure!(!cancelled(), "Singing pitch analysis cancelled");
    let mut hash = Sha256::new();
    for p in pcm {
        hash.update(p.to_le_bytes());
    }
    let count = pcm.len() / HOP + 1;
    let fft = FftPlanner::new().plan_fft_forward(1024);
    let mut mag = Vec::with_capacity(count * BINS);
    let mut peak = 1e-10f32;
    for i in 0..count {
        ensure!(!cancelled(), "Singing pitch analysis cancelled");
        let mut x = (0..1024)
            .map(|j| {
                Complex32::new(
                    pcm[reflect((i * HOP + j) as isize - 512, pcm.len())]
                        * (0.5 - 0.5 * (std::f32::consts::TAU * j as f32 / 1024.).cos()),
                    0.,
                )
            })
            .collect::<Vec<_>>();
        fft.process(&mut x);
        for z in &x[..BINS] {
            let a = z.norm();
            peak = peak.max(a);
            mag.push(a);
        }
    }
    let reference_db = 10. * peak.log10();
    for m in &mut mag {
        *m = (10. * m.max(1e-10).log10() - reference_db).max(-80.);
    }
    let mut values = vec![0.; count.div_ceil(WINDOW_FRAMES) * WINDOW];
    for (i, x) in values.iter_mut().enumerate() {
        let p = i % WINDOW;
        *x = ((mag.get(i).copied().unwrap_or(0.) as f64 - norm.mean[p]) / (norm.std[p] + 0.0001))
            as f32;
    }
    ensure!(
        values.iter().all(|v| v.is_finite()) && !cancelled(),
        "Invalid or cancelled JDC frontend"
    );
    Ok(Features {
        values,
        frame_count: count,
        sample_count: pcm.len(),
        pcm_sha256: format!("{:x}", hash.finalize()),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub time_seconds: f64,
    pub midi: Option<f64>,
    pub detection_singing_probability: f32,
    /// f64 sums of the original f32 probabilities, in MIDI 36..84 order.
    pub note_band_mass: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub version: u32,
    pub source_sha256: String,
    pub pcm_sha256: String,
    pub checkpoint_sha256: String,
    pub model_sha256: String,
    pub implementation_sha256: String,
    pub runtime_sha256: String,
    pub runtime_publisher_sha256: String,
    pub sample_count: usize,
    pub sample_rate: usize,
    pub hop_samples: usize,
    pub duration_seconds: f64,
    pub frames: Vec<Frame>,
}
pub fn validate(e: &Evidence, source: &str, duration: f64) -> Result<()> {
    let sha = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        e.version == 1
            && sha(source)
            && e.source_sha256 == source
            && sha(&e.pcm_sha256)
            && sha(&e.runtime_sha256)
            && e.runtime_publisher_sha256 == RUNTIME_PUBLISHER_SHA256
            && e.checkpoint_sha256 == CHECKPOINT_SHA256
            && e.model_sha256 == MODEL_SHA256
            && e.implementation_sha256 == implementation_hash(),
        "JDC source, model, implementation or runtime identity differs"
    );
    ensure!(
        e.sample_count > 1024
            && e.sample_count <= RATE * 3600
            && e.sample_rate == RATE
            && e.hop_samples == HOP
            && duration.is_finite()
            && e.duration_seconds.is_finite()
            && (e.duration_seconds - e.sample_count as f64 / RATE as f64).abs() < 1e-9
            && (e.duration_seconds - duration).abs() <= 1. / RATE as f64
            && e.frames.len() == e.sample_count / HOP + 1,
        "JDC recording clock differs"
    );
    ensure!(
        e.frames
            .iter()
            .enumerate()
            .all(|(i, f)| f.time_seconds.is_finite()
                && (f.time_seconds - i as f64 * 0.01).abs() < 1e-9
                && f.midi
                    .is_none_or(|p| p.is_finite() && (38. ..=83.).contains(&p))
                && f.detection_singing_probability.is_finite()
                && (0. ..=1.).contains(&f.detection_singing_probability)
                && f.note_band_mass.len() == BANDS
                && f.note_band_mass
                    .iter()
                    .all(|p| p.is_finite() && (0. ..=1.0001).contains(p))
                && f.note_band_mass.iter().sum::<f64>() <= 1.0001),
        "Invalid JDC frame evidence"
    );
    Ok(())
}
pub fn frame(index: usize, posterior: &[f32], detection: f32) -> Result<Frame> {
    ensure!(
        posterior.len() == CLASSES
            && posterior
                .iter()
                .all(|p| p.is_finite() && (0. ..=1.).contains(p))
            && detection.is_finite()
            && (0. ..=1.).contains(&detection)
            && (posterior.iter().sum::<f32>() - 1.).abs() < 1e-4,
        "Invalid JDC graph probabilities"
    );
    let winner = (0..CLASSES)
        .max_by(|a, b| posterior[*a].total_cmp(&posterior[*b]))
        .unwrap();
    let mut bands = vec![0.; BANDS];
    for (j, &p) in posterior.iter().enumerate().skip(1) {
        let pitch = (38. + (j - 1) as f64 / 16.).round() as usize;
        bands[pitch - 36] += p as f64;
    }
    Ok(Frame {
        time_seconds: index as f64 * 0.01,
        midi: if winner == 0 {
            None
        } else {
            Some(38. + (winner - 1) as f64 / 16.)
        },
        detection_singing_probability: detection,
        note_band_mass: bands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semitone_bands_preserve_probability_and_fine_argmax() {
        let mut p = vec![0.; CLASSES];
        p[0] = 0.3;
        p[1 + 22 * 16] = 0.4;
        p[1 + 22 * 16 + 1] = 0.2;
        p[1 + 23 * 16] = 0.1;
        let f = frame(31, &p, 0.9).unwrap();
        assert_eq!(f.time_seconds, 0.31);
        assert_eq!(f.midi, Some(60.));
        assert_eq!(f.note_band_mass[24], 0.4f32 as f64 + 0.2f32 as f64);
        assert_eq!(f.note_band_mass[25], 0.1f32 as f64);
        p[0] = f32::NAN;
        assert!(frame(0, &p, 0.9).is_err());
    }
    #[test]
    fn padding_precedes_position_specific_normalization_and_clock_is_not_padded() {
        let norm = Normalization {
            mean: vec![1.; WINDOW],
            std: vec![2.; WINDOW],
        };
        let pcm = vec![0.; 1100];
        let f = features(&pcm, &norm, &|| false).unwrap();
        assert_eq!(f.frame_count, 14);
        assert_eq!(f.sample_count, 1100);
        assert_eq!(f.values.len(), WINDOW);
        assert_eq!(f.values[14 * BINS], (-1f64 / 2.0001) as f32);
        assert!(f.values.iter().all(|v| v.is_finite()));
        assert!(features(&pcm, &norm, &|| true).is_err());
        assert!(features(&[f32::NAN; 1100], &norm, &|| false).is_err());
        assert!(Normalization::from_npy(b"bad", b"bad").is_err());
    }
    #[test]
    fn changed_source_runtime_and_padded_clock_are_rejected() {
        let mut p = vec![0.; CLASSES];
        p[0] = 1.;
        let frames = (0..14).map(|i| frame(i, &p, 0.).unwrap()).collect();
        let mut e = Evidence {
            version: 1,
            source_sha256: "a".repeat(64),
            pcm_sha256: "b".repeat(64),
            checkpoint_sha256: CHECKPOINT_SHA256.into(),
            model_sha256: MODEL_SHA256.into(),
            implementation_sha256: implementation_hash(),
            runtime_sha256: "c".repeat(64),
            runtime_publisher_sha256: RUNTIME_PUBLISHER_SHA256.into(),
            sample_count: 1100,
            sample_rate: RATE,
            hop_samples: HOP,
            duration_seconds: 1100. / 8000.,
            frames,
        };
        assert!(validate(&e, &"a".repeat(64), e.duration_seconds).is_ok());
        assert!(validate(&e, &"d".repeat(64), e.duration_seconds).is_err());
        e.frames.push(frame(14, &p, 0.).unwrap());
        assert!(validate(&e, &e.source_sha256, e.duration_seconds).is_err());
        e.frames.pop();
        e.runtime_publisher_sha256 = "c".repeat(64);
        assert!(validate(&e, &e.source_sha256, e.duration_seconds).is_err());
    }
}
