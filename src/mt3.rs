//! Experimental pinned MT3/YourMT3-family ONNX candidate, outside production routing.
//! Session/cache driving adapted from ModernMube OwnAudioSharp (MIT), revision
//! 4b7cc4f1ea32acb4f0f23723e0fa4508bafa488d. Models are downloaded separately;
//! their checkpoint-specific licensing/provenance remains unresolved. No Python.
//!
//! MIT License — Copyright (c) 2025 Modernmube
//! Permission is hereby granted, free of charge, to any person obtaining a copy
//! of this software and associated documentation files (the "Software"), to deal
//! in the Software without restriction, including without limitation the rights
//! to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//! copies of the Software, and to permit persons to whom the Software is
//! furnished to do so, subject to the following conditions:
//! The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software.
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//! IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//! FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//! LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
//! SOFTWARE.
use crate::models;
use anyhow::{Context, Result, ensure};
use ort::{
    session::{Session, SessionInputValue, builder::GraphOptimizationLevel},
    value::{DynValue, Tensor},
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub const REVISION: &str = "3c057f86ffa760025f7ed30091b221fae1302226";
pub const METHOD: &str = "mt3-onnx-native-experimental-v2;16kHz;32767-sample segments;embedded-mel;greedy-KV-cache;YourMT3-absolute100Hz-shifts+tie-prologue;program+drums;source-end-bound-without10s-cap;binary-velocity-not-confidence";
const SIZE: usize = 32767;
const RATE: usize = 16000;
pub fn assets() -> Vec<models::ModelAsset> {
    [("mt3_encoder.onnx","0804f42fdf8b3d072d0b4807a42f281dac62fb78ea6559e37bc35b980cb3040f"),("mt3_decoder_init.onnx","9622bfa3091c5af620e61a209cc543726a6da0c9ae70f693d1725a3e566922c4"),("mt3_decoder_step.onnx","55be61e757febf44605998d1c8092b11bde858bd4fec9b9f4597a2273270d3ed"),("vocab.json","48db4bb722d6ccd085904ed50dd52ebb5d69a0f4ba49d9aa3af2c60dc7975bdf")].into_iter().map(|(file,hash)|models::ModelAsset{id:format!("mt3-{file}"),filename:file.into(),url:format!("https://huggingface.co/ModernMube/HTDemucs_onnx/resolve/{REVISION}/mt3-onnx/{file}"),sha256:hash.into(),license:"Checkpoint-specific license unresolved; mirror labels MIT, OwnAudio docs warn YourMT3 GPL3. Experimental evaluation only.".into()}).collect()
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub program: u8,
    pub is_drum: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub tokens: Vec<u32>,
    pub eos: bool,
    pub token_limit_reached: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub method: String,
    pub implementation_sha256: String,
    pub source_audio_sha256: Option<String>,
    pub duration_seconds: f64,
    pub model_sha256: BTreeMap<String, String>,
    pub notes: Vec<Note>,
    pub segments: Vec<Segment>,
    pub discarded_events: usize,
    pub notes_clipped_at_source_end: usize,
    pub score_semantics: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Shift(u32),
    Pitch(u8),
    Velocity(u8),
    Tie,
    Program(u8),
    Drum(u8),
    Eos,
}
/// Strictly matches the exported vocabulary: special0..2, shift3..208,
/// pitch209..336, velocity337..338, tie339, program340..467, drum468..595.
/// A pinned vocabulary is checked before model loading; arbitrary exports are refused.
pub fn event(token: u32) -> Option<Event> {
    Some(match token {
        1 => Event::Eos,
        3..=208 => Event::Shift(token - 3),
        209..=336 => Event::Pitch((token - 209) as u8),
        337..=338 => Event::Velocity((token - 337) as u8),
        339 => Event::Tie,
        340..=467 => Event::Program((token - 340) as u8),
        468..=595 => Event::Drum((token - 468) as u8),
        _ => return None,
    })
}

#[derive(Default)]
struct NoteDecoder {
    active: BTreeMap<(u8, u8), (f64, u8)>,
    notes: Vec<Note>,
    dropped: usize,
}
impl NoteDecoder {
    fn end(&mut self, key: (u8, u8), time: f64) {
        if let Some((start, velocity)) = self.active.remove(&key) {
            if time > start {
                self.notes.push(Note {
                    start,
                    end: time,
                    pitch: key.0,
                    program: key.1,
                    velocity,
                    is_drum: false,
                });
            }
        } else {
            self.dropped += 1;
        }
    }
    fn ties(&mut self, tied: &BTreeSet<(u8, u8)>, time: f64) {
        let ending: Vec<_> = self
            .active
            .keys()
            .copied()
            .filter(|k| !tied.contains(k))
            .collect();
        for key in ending {
            self.end(key, time);
        }
    }
    fn push(&mut self, tokens: &[u32], start: f64) {
        // This export has YourMT3's absolute shifts and binary velocities,
        // not Magenta's consecutive run-length shift groups. Round the source
        // segment origin once to the checkpoint's100Hz codec grid.
        let mut time = (start * 100.).round() / 100.;
        let mut velocity = None;
        let mut program = None;
        let mut tied = BTreeSet::new();
        let prologue = tokens
            .iter()
            .position(|t| matches!(event(*t), Some(Event::Tie | Event::Shift(_) | Event::Eos)));
        let Some(tie_index) = prologue.filter(|i| event(tokens[*i]) == Some(Event::Tie)) else {
            self.dropped += tokens.len().max(1);
            self.ties(&tied, start);
            return;
        };
        for &token in &tokens[..tie_index] {
            match event(token) {
                Some(Event::Program(p)) => program = Some(p),
                Some(Event::Pitch(p)) => {
                    if let Some(program) = program {
                        if self.active.contains_key(&(p, program)) {
                            tied.insert((p, program));
                        } else {
                            self.dropped += 1;
                        }
                    } else {
                        self.dropped += 1;
                    }
                }
                _ => {}
            }
        }
        self.ties(&tied, start);
        let mut pending = Vec::new();
        for &token in &tokens[tie_index + 1..] {
            match event(token) {
                Some(Event::Eos) => break,
                Some(Event::Program(p)) => program = Some(p),
                Some(Event::Velocity(v)) => velocity = Some(v),
                Some(Event::Shift(step)) => {
                    if step > 0 {
                        time = ((start * 100.).round() + step as f64) / 100.;
                    } else {
                        self.dropped += 1;
                    }
                }
                Some(Event::Pitch(p)) => {
                    if let (Some(program), Some(velocity)) = (program, velocity) {
                        pending.push((time, p, program, velocity, false));
                    } else {
                        self.dropped += 1;
                    }
                }
                Some(Event::Drum(p)) => pending.push((time, p, 0, 1, true)),
                None => {}
                _ => self.dropped += 1,
            }
        }
        // Author codec sorts note events; offsets precede new onsets of the
        // same key at the same tick, so a true retrigger cannot be erased.
        pending.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.4.cmp(&b.4))
                .then(a.1.cmp(&b.1))
                .then(a.3.cmp(&b.3))
                .then(a.2.cmp(&b.2))
        });
        for (time, pitch, program, velocity, drum) in pending {
            let key = (pitch, program);
            if drum {
                self.notes.push(Note {
                    start: time,
                    end: time + 0.01,
                    pitch,
                    program,
                    velocity: 127,
                    is_drum: true,
                });
            } else if velocity == 0 {
                self.end(key, time);
            } else {
                if self.active.contains_key(&key) {
                    self.end(key, time);
                }
                self.active.insert(key, (time, 127));
            }
        }
    }
    fn finish(mut self, end: f64) -> (Vec<Note>, usize, usize) {
        let keys: Vec<_> = self.active.keys().copied().collect();
        for key in keys {
            self.end(key, end);
        }
        let mut clipped = 0;
        self.notes.retain_mut(|n| {
            if n.end > end {
                n.end = end;
                clipped += 1;
            }
            n.start >= 0. && n.start < end && n.end > n.start
        });
        self.notes.sort_by(|a, b| {
            a.start
                .total_cmp(&b.start)
                .then(a.program.cmp(&b.program))
                .then(a.pitch.cmp(&b.pitch))
        });
        (self.notes, self.dropped, clipped)
    }
}
pub fn decode_segments(segments: &[Segment], duration: f64) -> Result<(Vec<Note>, usize, usize)> {
    ensure!(
        duration.is_finite() && duration > 0.,
        "Invalid MT3 source duration"
    );
    let mut state = NoteDecoder::default();
    for (i, segment) in segments.iter().enumerate() {
        ensure!(
            (segment.start - i as f64 * SIZE as f64 / RATE as f64).abs() < 1e-9,
            "MT3 segments are not on the source sample clock"
        );
        ensure!(segment.tokens.iter().all(|t| *t < 596), "Unknown MT3 token");
        state.push(&segment.tokens, segment.start);
    }
    Ok(state.finish(duration))
}
#[derive(Clone)]
struct Floats {
    shape: Vec<usize>,
    data: Vec<f32>,
}
impl Floats {
    fn value(self) -> Result<DynValue> {
        Ok(Tensor::from_array((self.shape, self.data.into_boxed_slice()))?.into_dyn())
    }
}
fn extract(outputs: &ort::session::SessionOutputs<'_>, name: &str) -> Result<Floats> {
    let (shape, data) = outputs
        .get(name)
        .with_context(|| format!("MT3 missing {name}"))?
        .try_extract_tensor::<f32>()?;
    ensure!(
        shape.iter().all(|n| *n >= 0) && data.iter().all(|n| n.is_finite()),
        "Invalid MT3 tensor{name}"
    );
    Ok(Floats {
        shape: shape.iter().map(|n| *n as usize).collect(),
        data: data.to_vec(),
    })
}
fn token_value(token: u32) -> Result<DynValue> {
    Ok(Tensor::from_array(([1, 1], vec![token as i64].into_boxed_slice()))?.into_dyn())
}
fn best(outputs: &ort::session::SessionOutputs<'_>) -> Result<u32> {
    let logits = extract(outputs, "logits")?;
    ensure!(
        logits.shape.last() == Some(&596) && logits.data.len() >= 596,
        "MT3 logits vocabulary mismatch"
    );
    Ok(logits.data[logits.data.len() - 596..]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0 as u32)
}
fn open(path: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_intra_threads(4)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(path)?)
}
pub struct Transcriber {
    encoder: Session,
    init: Session,
    step: Session,
    cache: Vec<(String, String)>,
    model_hashes: BTreeMap<String, String>,
}
impl Transcriber {
    pub fn load(dir: &Path) -> Result<Self> {
        let directory = if dir.join("mt3_encoder.onnx").is_file() {
            dir.to_owned()
        } else {
            dir.join("mt3")
        };
        let mut hashes = BTreeMap::new();
        for asset in assets() {
            let hash = models::hash_file(&directory.join(&asset.filename))?;
            ensure!(
                hash == asset.sha256,
                "MT3 asset checksum mismatch:{}",
                asset.filename
            );
            hashes.insert(asset.filename, hash);
        }
        let encoder = open(&directory.join("mt3_encoder.onnx"))?;
        let init = open(&directory.join("mt3_decoder_init.onnx"))?;
        let step = open(&directory.join("mt3_decoder_step.onnx"))?;
        ensure!(
            encoder.inputs.len() == 1 && encoder.outputs.len() == 1,
            "Unexpected MT3 encoder graph"
        );
        let mut cache = vec![];
        for input in step.inputs.iter().filter(|i| i.name.starts_with("past")) {
            let suffix = input.name.trim_start_matches("past_key_values");
            let present = step
                .outputs
                .iter()
                .find(|o| o.name.starts_with("present") && o.name.ends_with(suffix))
                .context("MT3 cache tensor has no paired output")?;
            ensure!(
                init.outputs.iter().any(|o| o.name == present.name),
                "MT3 initial cache missing{}",
                present.name
            );
            cache.push((input.name.clone(), present.name.clone()));
        }
        cache.sort();
        ensure!(!cache.is_empty(), "MT3 decoder has no cache");
        Ok(Self {
            encoder,
            init,
            step,
            cache,
            model_hashes: hashes,
        })
    }
    pub fn infer_samples(&mut self, audio: &[f32]) -> Result<Output> {
        ensure!(
            !audio.is_empty() && audio.iter().all(|s| s.is_finite()),
            "Invalid16kHz mono MT3 input"
        );
        let mut segments = vec![];
        for (i, chunk) in audio.chunks(SIZE).enumerate() {
            let mut padded = vec![0.; SIZE];
            padded[..chunk.len()].copy_from_slice(chunk);
            let input_name = self.encoder.inputs[0].name.clone();
            let output_name = self.encoder.outputs[0].name.clone();
            let hidden = {
                let output=self.encoder.run(ort::inputs![input_name.as_str()=>Tensor::from_array(([1,SIZE],padded.into_boxed_slice()))?])?;
                extract(&output, &output_name)?
            };
            let hidden = hidden.value()?;
            let (mut token, mut cache) = {
                let output = self.init.run(
                    ort::inputs!["input_ids"=>token_value(0)?,"encoder_hidden_states"=>&hidden],
                )?;
                (
                    best(&output)?,
                    self.cache
                        .iter()
                        .map(|(_, p)| extract(&output, p))
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            let mut tokens = vec![];
            for _ in 0..1024 {
                if token == 1 {
                    break;
                }
                tokens.push(token);
                let mut inputs: Vec<(Cow<'static, str>, SessionInputValue<'_>)> = vec![
                    ("input_ids".into(), token_value(token)?.into()),
                    ("encoder_hidden_states".into(), (&hidden).into()),
                ];
                for ((past, _), tensor) in self.cache.iter().zip(cache) {
                    inputs.push((past.clone().into(), tensor.value()?.into()));
                }
                let output = self.step.run(inputs)?;
                token = best(&output)?;
                cache = self
                    .cache
                    .iter()
                    .map(|(_, p)| extract(&output, p))
                    .collect::<Result<Vec<_>>>()?;
            }
            let segment = Segment {
                start: (i * SIZE) as f64 / RATE as f64,
                token_limit_reached: tokens.len() == 1024 && token != 1,
                eos: token == 1,
                tokens,
            };
            eprintln!(
                "MT3 segment{}: {}tokens, eos{}",
                i + 1,
                segment.tokens.len(),
                segment.eos
            );
            segments.push(segment);
        }
        let duration = audio.len() as f64 / RATE as f64;
        let (notes, dropped, clipped) = decode_segments(&segments, duration)?;
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for source in [
            include_bytes!("mt3.rs").as_slice(),
            include_bytes!("models.rs").as_slice(),
        ] {
            hash.update((source.len() as u64).to_le_bytes());
            hash.update(source);
        }
        Ok(Output {
            method: METHOD.into(),
            implementation_sha256: format!("{:x}", hash.finalize()),
            source_audio_sha256: None,
            duration_seconds: duration,
            model_sha256: self.model_hashes.clone(),
            notes, segments,
            discarded_events: dropped,
            notes_clipped_at_source_end: clipped,
            score_semantics: "No note confidence is produced. Velocity is the checkpoint's binary on/off bin mapped to127; greedy token logits are not correctness probabilities.".into(),
        })
    }
    pub fn transcribe(&mut self, audio: &Path) -> Result<Output> {
        let hash = models::hash_file(audio)?;
        let samples = models::decode(audio, RATE as u32, 1)?;
        let mut output = self.infer_samples(&samples)?;
        ensure!(
            models::hash_file(audio)? == hash,
            "MT3 source changed during inference"
        );
        output.source_audio_sha256 = Some(hash);
        Ok(output)
    }
}
