//! Native Transkun 2.0 ONNX candidate (piano-specific, not a mixed-song oracle).
//!
//! Frozen export: https://huggingface.co/TuesdayCrowd/transkun-onnx/tree/b44598166715a50d863973a8ecbbdbbb392f08d6
//! Algorithm reference: manh9011/TranskunCli commit 62aa0406e944b2b1a7276c7f4bc6e8e16e452d1d,
//! and the export's DECODE_SPEC.md. No Python or JavaScript runtime is used.
//! Model/weights: MIT, Copyright (c) 2021 Yujia Yan. See upstream license before redistribution.
use crate::{
    mapping::PitchNote,
    models::{self, ModelAsset},
};
use anyhow::{Context, Result, ensure};
use ort::{session::Session, value::Tensor};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const MODEL_REVISION: &str = "b44598166715a50d863973a8ecbbdbbb392f08d6";
pub const SAMPLE_RATE: usize = 44100;
const WIN: usize = 4096;
const HOP: usize = 1024;
const MELS: usize = 229;
const WINDOWS: usize = 6;
const BINS: usize = 2049;
const SYMBOLS: usize = 90;
const CTX: usize = 256;
const SEGMENT: usize = 16 * SAMPLE_RATE;
const PAD: usize = 8 * SAMPLE_RATE;
const STEP: usize = PAD.div_ceil(HOP) * HOP;

/// Can be appended to the application's model manifest. All six assets are required.
pub fn assets() -> Vec<ModelAsset> {
    [
        (
            "params.json",
            "4d52da9e78782329be6119a68fe4f0a2f98561d2874782e2bb17426620aed065",
        ),
        (
            "freq2mels.f32",
            "449f2a5269cde41f6705cfde7157240ce7c8930f20b50909d57e633395bbf0f4",
        ),
        (
            "windows.f32",
            "b059ebf1e5550a8186b120ddf3f068dd9075e72338051bcd0349c1f436ddb14f",
        ),
        (
            "symbols.i32",
            "cd2920adac71c7495a8fbf9586d04c2737a9440d280fdc4a619f636837b98fee",
        ),
        (
            "transkun.onnx",
            "6d6960ecbce5c3d51051ebc1ebdeec4d3018ea6cfd360fb1ae96359aa7a075a3",
        ),
        (
            "transkun-heads.onnx",
            "f609f53fbb4df0a2165f941aff1d5a1c1063ba40aaa699365448dca5d9f3544d",
        ),
    ]
    .into_iter()
    .map(|(file, sha)| ModelAsset {
        id: format!("transkun-{file}"),
        filename: format!("transkun/{file}"),
        url: format!(
            "https://huggingface.co/TuesdayCrowd/transkun-onnx/resolve/{MODEL_REVISION}/{file}"
        ),
        sha256: sha.into(),
        license: "MIT (Yujia Yan, 2021)".into(),
    })
    .collect()
}

fn verified_dir(dir: &Path) -> Result<PathBuf> {
    let dir = if dir.join("transkun.onnx").is_file() {
        dir.to_owned()
    } else {
        dir.join("transkun")
    };
    for asset in assets() {
        let file = Path::new(&asset.filename)
            .file_name()
            .context("Invalid model filename")?;
        let path = dir.join(file);
        ensure!(path.is_file(), "Transkun asset missing: {}", path.display());
        ensure!(
            models::hash_file(&path)? == asset.sha256,
            "Transkun checksum mismatch: {}",
            path.display()
        );
    }
    Ok(dir)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PianoNote {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: u8,
    /// Uncalibrated semi-CRF interval score, not a correctness probability.
    pub interval_score: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PedalInterval {
    pub start: f64,
    pub end: f64,
    pub controller: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transcription {
    pub notes: Vec<PianoNote>,
    pub pedals: Vec<PedalInterval>,
}

/// `model_dir` may be the common model root or its `transkun` subdirectory.
/// PitchNote confidence is sigmoid(interval score), an uncalibrated ranking only.
pub fn pitch(audio: &Path, model_dir: &Path) -> Result<Vec<PitchNote>> {
    Ok(transcribe(audio, model_dir)?
        .notes
        .into_iter()
        .map(|n| PitchNote {
            start: n.start,
            end: n.end,
            pitch: n.pitch,
            confidence: 1.0 / (1.0 + (-n.interval_score).exp()),
        })
        .collect())
}

pub fn transcribe(audio: &Path, model_dir: &Path) -> Result<Transcription> {
    let dir = verified_dir(model_dir)?;
    let samples = models::decode(audio, SAMPLE_RATE as u32, 1)?;
    transcribe_samples(&samples, &dir)
}

/// Native model inference on mono 44.1 kHz samples. Verifies all pinned assets.
pub fn transcribe_samples(audio: &[f32], model_dir: &Path) -> Result<Transcription> {
    ensure!(
        !audio.is_empty() && audio.iter().all(|x| x.is_finite()),
        "Empty or nonfinite Transkun audio"
    );
    let dir = verified_dir(model_dir)?;
    let frontend = FrontEnd::load(&dir)?;
    let mut backbone = Session::builder()?
        .with_intra_threads(4)?
        .commit_from_file(dir.join("transkun.onnx"))?;
    let mut heads = Session::builder()?
        .with_intra_threads(4)?
        .commit_from_file(dir.join("transkun-heads.onnx"))?;
    let duration = audio.len() as f64 / SAMPLE_RATE as f64;
    let mut padded = vec![0.0; PAD];
    padded.extend_from_slice(audio);
    padded.resize(padded.len() + PAD, 0.0);
    let mut start_positions = vec![PAD / HOP; SYMBOLS];
    let mut by_pitch: BTreeMap<i32, Vec<Event>> = BTreeMap::new();
    for offset in (0..padded.len()).step_by(STEP) {
        let begin_time = offset as f64 / SAMPLE_RATE as f64 - 8.0;
        // A wholly post-recording padding window cannot contain a source event.
        if begin_time >= duration {
            break;
        }
        let mut segment = padded[offset..(offset + SEGMENT).min(padded.len())].to_vec();
        segment.resize(SEGMENT, 0.0);
        let (features, frames) = frontend.compute(&segment)?;
        let outputs = backbone.run(ort::inputs!["featuresBatch" => Tensor::from_array(([1, frames, MELS, WINDOWS], features.into_boxed_slice()))?])?;
        let (shape, scores) = outputs["S"].try_extract_tensor::<f32>()?;
        ensure!(
            shape.len() == 3 && shape[0] > 0 && shape[0] == shape[1] && shape[2] == SYMBOLS as i64,
            "Unexpected Transkun score shape: {shape:?}"
        );
        let t = shape[0] as usize;
        let (ctx_shape, ctx) = outputs["ctx"].try_extract_tensor::<f32>()?;
        ensure!(
            ctx_shape.as_ref() == [SYMBOLS as i64, t as i64, CTX as i64],
            "Unexpected Transkun context shape: {ctx_shape:?}"
        );
        ensure!(
            ctx.iter().all(|v| v.is_finite()),
            "Nonfinite Transkun context"
        );
        let intervals = semi_crf_decode(scores, t, SYMBOLS, Some(&start_positions))?;
        let count: usize = intervals.iter().map(Vec::len).sum();
        let mut attr = Vec::with_capacity(count * CTX * 3);
        for (track, list) in intervals.iter().enumerate() {
            for iv in list {
                let a = &ctx[(track * t + iv.begin) * CTX..(track * t + iv.begin + 1) * CTX];
                let b = &ctx[(track * t + iv.end) * CTX..(track * t + iv.end + 1) * CTX];
                attr.extend_from_slice(a);
                attr.extend_from_slice(b);
                attr.extend(a.iter().zip(b).map(|(a, b)| a * b));
            }
        }
        let (velocities, offsets) = if count == 0 {
            (Vec::new(), Vec::new())
        } else {
            let out = heads.run(ort::inputs!["attr" => Tensor::from_array(([count, CTX * 3], attr.into_boxed_slice()))?])?;
            let (vs, vd) = out["velLogits"].try_extract_tensor::<f32>()?;
            let (os, od) = out["ofRaw"].try_extract_tensor::<f32>()?;
            ensure!(
                vs.as_ref() == [count as i64, 128] && os.as_ref() == [count as i64, 4],
                "Unexpected Transkun heads shape"
            );
            ensure!(
                vd.iter().chain(od).all(|v| v.is_finite()),
                "Nonfinite Transkun attributes"
            );
            (vd.to_vec(), od.to_vec())
        };
        let mut cursor = 0;
        let mut events = Vec::new();
        for (track, list) in intervals.iter().enumerate() {
            let mut last_end: f64 = 0.0;
            let mut last_closed: usize = 0;
            for iv in list {
                let velocity = velocities[cursor * 128..(cursor + 1) * 128]
                    .iter()
                    .enumerate()
                    .max_by(|(ai, a), (bi, b)| a.total_cmp(b).then_with(|| bi.cmp(ai)))
                    .unwrap()
                    .0
                    .clamp(1, 127) as u8;
                let raw = &offsets[cursor * 4..cursor * 4 + 4];
                let start = ((iv.begin as f64 + subframe_offset(raw[0] as f64)) * HOP as f64
                    / SAMPLE_RATE as f64)
                    .max(last_end);
                let end = ((iv.end as f64 + subframe_offset(raw[1] as f64)) * HOP as f64
                    / SAMPLE_RATE as f64)
                    .max(start + 1e-8);
                last_end = end;
                let has_offset =
                    iv.end < (SEGMENT as f64 / HOP as f64).round() as usize || raw[3] > 0.0;
                if has_offset {
                    last_closed = iv.end;
                }
                let shifted_start = (start + begin_time).max(0.0);
                events.push(Event {
                    pitch: frontend.symbols[track],
                    start: shifted_start,
                    end: (end + begin_time).max(shifted_start),
                    has_onset: iv.begin > 0 || raw[2] > 0.0,
                    has_offset,
                    velocity,
                    score: scores[(iv.end * t + iv.begin) * SYMBOLS + track],
                });
                cursor += 1;
            }
            start_positions[track] = last_closed.saturating_sub(STEP / HOP);
        }
        events.sort_by(event_order);
        for event in events {
            merge_event(&mut by_pitch, event);
        }
        eprintln!(
            "Transkun: {:.1}/{duration:.1} seconds",
            (begin_time + 16.0).min(duration).max(0.0)
        );
    }
    let mut events = Vec::new();
    for list in by_pitch.values_mut() {
        if let Some(last) = list.last_mut() {
            last.has_offset = true;
        }
        events.extend(list.iter().filter(|e| e.has_offset).cloned());
    }
    events.sort_by(event_order);
    let mut previous = BTreeMap::new();
    for i in 0..events.len() {
        if let Some(j) = previous.insert(events[i].pitch, i) {
            events[j].end = events[j].end.min(events[i].start);
        }
    }
    let mut result = Transcription {
        notes: Vec::new(),
        pedals: Vec::new(),
    };
    for e in events {
        let end = e.end.min(duration);
        if e.start >= end {
            continue;
        }
        if (21..=108).contains(&e.pitch) {
            result.notes.push(PianoNote {
                start: e.start,
                end,
                pitch: e.pitch as u8,
                velocity: e.velocity,
                interval_score: e.score,
            });
        } else if e.pitch == -64 || e.pitch == -67 {
            result.pedals.push(PedalInterval {
                start: e.start,
                end,
                controller: (-e.pitch) as u8,
            });
        }
    }
    result
        .notes
        .sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    Ok(result)
}

#[derive(Clone, Debug)]
struct Event {
    pitch: i32,
    start: f64,
    end: f64,
    has_onset: bool,
    has_offset: bool,
    velocity: u8,
    score: f32,
}
fn event_order(a: &Event, b: &Event) -> std::cmp::Ordering {
    a.start
        .total_cmp(&b.start)
        .then(a.end.total_cmp(&b.end))
        .then(a.pitch.cmp(&b.pitch))
}
fn merge_event(by_pitch: &mut BTreeMap<i32, Vec<Event>>, event: Event) {
    let list = by_pitch.entry(event.pitch).or_default();
    if let Some(last) = list.last_mut().filter(|last| event.start < last.end) {
        if event.has_onset {
            *last = event;
        } else {
            last.end = last.end.max(event.end);
            last.has_offset = event.has_offset;
        }
    } else if event.has_onset {
        list.push(event);
    }
}

/// Continuous-Bernoulli mean, recentered to +/- half a frame. Uses the equivalent
/// logit-domain formula to avoid sigmoid overflow at extreme finite logits.
pub fn subframe_offset(logit: f64) -> f64 {
    let mean = if logit.abs() < 0.004 {
        0.5 + logit / 12.0 - logit.powi(3) / 720.0
    } else {
        1.0 / -(-logit).exp_m1() - 1.0 / logit
    };
    ((mean - 0.5) / 0.99).clamp(-0.5, 0.5)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval {
    pub begin: usize,
    pub end: usize,
}
/// Backward Viterbi for S[end, begin, track], including positive diagonal events.
pub fn semi_crf_decode(
    score: &[f32],
    t: usize,
    tracks: usize,
    forced: Option<&[usize]>,
) -> Result<Vec<Vec<Interval>>> {
    ensure!(
        t > 0 && tracks > 0,
        "Transkun decoder needs nonempty dimensions"
    );
    let expected = t
        .checked_mul(t)
        .and_then(|x| x.checked_mul(tracks))
        .context("Transkun score dimensions overflow")?;
    ensure!(score.len() == expected, "Invalid Transkun score length");
    ensure!(
        score.iter().all(|x| x.is_finite()),
        "Nonfinite Transkun scores"
    );
    if let Some(start) = forced {
        ensure!(
            start.len() == tracks && start.iter().all(|s| *s <= t),
            "Invalid forced Transkun start positions"
        );
    }
    let at = |end: usize, begin: usize, track: usize| score[(end * t + begin) * tracks + track];
    let mut q = vec![0.0f32; t * tracks];
    let mut ptr = vec![None; (t - 1) * tracks];
    for k in 0..tracks {
        q[(t - 1) * tracks + k] = at(t - 1, t - 1, k).max(0.0);
    }
    for begin in (0..t - 1).rev() {
        for k in 0..tracks {
            let mut best = q[(begin + 1) * tracks + k];
            for end in begin + 1..t {
                let candidate = q[end * tracks + k] + at(end, begin, k);
                if candidate > best {
                    best = candidate;
                    ptr[begin * tracks + k] = Some(end);
                }
            }
            q[begin * tracks + k] = best + at(begin, begin, k).max(0.0);
        }
    }
    let mut result = Vec::with_capacity(tracks);
    for k in 0..tracks {
        let mut intervals = Vec::new();
        let mut j = forced.map_or(0, |v| v[k]);
        while j < t - 1 {
            if at(j, j, k) > 0.0 {
                intervals.push(Interval { begin: j, end: j });
            }
            if let Some(end) = ptr[j * tracks + k] {
                intervals.push(Interval { begin: j, end });
                j = end;
            } else {
                j += 1;
            }
        }
        if j < t && at(t - 1, t - 1, k) > 0.0 {
            intervals.push(Interval {
                begin: t - 1,
                end: t - 1,
            });
        }
        result.push(intervals);
    }
    Ok(result)
}

fn read_f32(path: &Path) -> Result<Vec<f32>> {
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() % 4 == 0,
        "Invalid float buffer: {}",
        path.display()
    );
    let data: Vec<_> = bytes
        .chunks_exact(4)
        .map(|x| f32::from_le_bytes(x.try_into().unwrap()))
        .collect();
    ensure!(
        data.iter().all(|x| x.is_finite()),
        "Nonfinite float buffer: {}",
        path.display()
    );
    Ok(data)
}

/// Frozen multi-window log-mel front end; public for numeric reference validation.
pub struct FrontEnd {
    filters: Vec<f32>,
    windows: Vec<f32>,
    symbols: Vec<i32>,
    ranges: Vec<std::ops::RangeInclusive<usize>>,
}
impl FrontEnd {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let dir = verified_dir(model_dir)?;
        let p: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("params.json"))?)?;
        for (key, value) in [
            ("fs", SAMPLE_RATE),
            ("windowSize", WIN),
            ("hopSize", HOP),
            ("nMels", MELS),
            ("nWindows", WINDOWS),
            ("rfftBins", BINS),
            ("nSymbols", SYMBOLS),
        ] {
            ensure!(
                p[key].as_u64() == Some(value as u64),
                "Unsupported Transkun parameter {key}"
            );
        }
        ensure!(
            p["eps"].as_f64() == Some(1e-5)
                && p["segmentSizeSeconds"].as_f64() == Some(16.0)
                && p["segmentHopSeconds"].as_f64() == Some(8.0),
            "Unsupported Transkun scaling"
        );
        let filters = read_f32(&dir.join("freq2mels.f32"))?;
        let windows = read_f32(&dir.join("windows.f32"))?;
        let bytes = fs::read(dir.join("symbols.i32"))?;
        ensure!(
            filters.len() == BINS * MELS
                && windows.len() == WINDOWS * WIN
                && bytes.len() == SYMBOLS * 4,
            "Invalid Transkun buffer sizes"
        );
        let symbols: Vec<_> = bytes
            .chunks_exact(4)
            .map(|x| i32::from_le_bytes(x.try_into().unwrap()))
            .collect();
        let expected: Vec<_> = [-64, -67].into_iter().chain(21..=108).collect();
        ensure!(symbols == expected, "Unsupported Transkun symbol order");
        let ranges = (0..MELS)
            .map(|m| {
                let first = (0..BINS)
                    .find(|k| filters[k * MELS + m] != 0.0)
                    .unwrap_or(1);
                let last = (0..BINS)
                    .rfind(|k| filters[k * MELS + m] != 0.0)
                    .unwrap_or(0);
                first..=last
            })
            .collect();
        Ok(Self {
            filters,
            windows,
            symbols,
            ranges,
        })
    }
    pub fn compute(&self, audio: &[f32]) -> Result<(Vec<f32>, usize)> {
        ensure!(
            !audio.is_empty() && audio.iter().all(|x| x.is_finite()),
            "Empty or invalid Transkun segment"
        );
        let frames = audio.len().div_ceil(HOP) + 1;
        let mut padded = vec![0.0f64; (frames - 1) * HOP + WIN];
        for (target, sample) in padded[WIN / 2..].iter_mut().zip(audio) {
            *target = *sample as f64;
        }
        let count = frames * WIN;
        let sum: f64 = (0..frames)
            .map(|f| padded[f * HOP..f * HOP + WIN].iter().sum::<f64>())
            .sum();
        let mean = sum / count as f64;
        let sq_sum: f64 = (0..frames)
            .map(|f| {
                padded[f * HOP..f * HOP + WIN]
                    .iter()
                    .map(|v| (v - mean).powi(2))
                    .sum::<f64>()
            })
            .sum();
        let inv_std = 1.0 / ((sq_sum / (count - 1) as f64).sqrt() + 1e-8);
        let fft = FftPlanner::<f64>::new().plan_fft_forward(WIN);
        let mut buf = vec![Complex::new(0.0, 0.0); WIN];
        let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let mut power = vec![0.0f64; BINS];
        let mut features = vec![0.0; frames * MELS * WINDOWS];
        let log_eps = 1e-5f64.ln();
        for frame in 0..frames {
            for w in 0..WINDOWS {
                for j in 0..WIN {
                    buf[j] = Complex::new(
                        (padded[frame * HOP + j] - mean)
                            * inv_std
                            * self.windows[w * WIN + j] as f64,
                        0.0,
                    );
                }
                fft.process_with_scratch(&mut buf, &mut scratch);
                for k in 0..BINS {
                    power[k] = buf[k].norm_sqr() / WIN as f64;
                }
                for m in 0..MELS {
                    let acc: f64 = self.ranges[m]
                        .clone()
                        .map(|k| power[k] * self.filters[k * MELS + m] as f64)
                        .sum();
                    features[(frame * MELS + m) * WINDOWS + w] =
                        ((acc + 1e-5).ln() / (-log_eps) + 1.0) as f32;
                }
            }
        }
        ensure!(
            features.iter().all(|x| x.is_finite()),
            "Nonfinite Transkun features"
        );
        Ok((features, frames))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(start: f64, end: f64, onset: bool, offset: bool) -> Event {
        Event {
            pitch: 60,
            start,
            end,
            has_onset: onset,
            has_offset: offset,
            velocity: 80,
            score: 2.0,
        }
    }
    #[test]
    fn boundary_continuation_extends_without_duplicate_attack() {
        let mut tracks = BTreeMap::new();
        merge_event(&mut tracks, event(7.8, 8.1, true, false));
        merge_event(&mut tracks, event(8.0, 8.8, false, true));
        let notes = &tracks[&60];
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start, 7.8);
        assert_eq!(notes[0].end, 8.8);
        assert!(notes[0].has_offset);
    }
    #[test]
    fn orphan_continuation_is_discarded_and_repeated_attack_replaces_overlap() {
        let mut tracks = BTreeMap::new();
        merge_event(&mut tracks, event(0.0, 0.3, false, true));
        assert!(tracks[&60].is_empty());
        merge_event(&mut tracks, event(7.8, 8.1, true, false));
        merge_event(&mut tracks, event(7.9, 8.9, true, true));
        assert_eq!(tracks[&60].len(), 1);
        assert_eq!(tracks[&60][0].start, 7.9);
        merge_event(&mut tracks, event(9.0, 9.4, true, true));
        assert_eq!(tracks[&60].len(), 2);
    }
}
