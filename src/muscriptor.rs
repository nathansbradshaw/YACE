//! Native MuScriptor-small ONNX research candidate. Source algorithm: MIT,
//! Kyutai x Mirelo, muscriptor/muscriptor revision8a1ef88672cc23c26619adaff37ec48d82c0b906.
//! Weights: CC BY-NC4.0, derivative unifolia/muscriptor-small-onnx revision
//!15dd43dd431b7b12c0fb2f771bb57dea0cc2d7fa. Commercial redistribution is not authorized.
//! Uses the checkpoint's own embedded spectral front end and a cached greedy LM.
use crate::models::ModelAsset;
use anyhow::{Result, ensure};
#[cfg(not(target_arch = "wasm32"))]
use ort::{memory::Allocator, session::Session, value::Tensor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use {
    crate::models,
    anyhow::Context,
    std::path::{Path, PathBuf},
};

pub const MODEL_REVISION: &str = "15dd43dd431b7b12c0fb2f771bb57dea0cc2d7fa";
pub const METHOD: &str = "muscriptor-small-onnx-rust-v1;16kHz;5s;cached-greedy;absolute100Hz;forced-tie-preludes;source-bounded-missing-release-evidence";
pub const SAMPLE_RATE: usize = 16000;
pub const CHUNK_SAMPLES: usize = 80000;
pub const VOCAB_SIZE: usize = 1393;
pub const BOS: i64 = 1393;
pub const EOS: i64 = 1;
pub const TIE: i64 = 1134;
const PITCH: i64 = 1004;
const VELOCITY: i64 = 1132;
const PROGRAM: i64 = 1135;
const DRUM: i64 = 1265;
#[cfg(not(target_arch = "wasm32"))]
const LAYERS: usize = 14;
#[cfg(not(target_arch = "wasm32"))]
const HEADS: usize = 12;
#[cfg(not(target_arch = "wasm32"))]
const HEAD_DIM: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const DIM: usize = 768;

pub fn assets() -> Vec<ModelAsset> {
    [
        (
            "muscriptor-small-cond.onnx",
            "bfdd8edb30affe9f085cf145400cbd3b320822946468a42d88664d2cd1d91d6b",
        ),
        (
            "muscriptor-small-lm.onnx",
            "88231c3343ecb11f8e3cba4dbf22835e7398122425025a5c00c60e4dfd2e1c66",
        ),
    ]
    .into_iter()
    .map(|(file, sha)| ModelAsset {
        id: format!("muscriptor-{file}"),
        filename: format!("muscriptor/{file}"),
        url: format!(
            "https://huggingface.co/unifolia/muscriptor-small-onnx/resolve/{MODEL_REVISION}/{file}"
        ),
        sha256: sha.into(),
        license: "CC BY-NC4.0 (original MuScriptor weights)".into(),
    })
    .collect()
}
#[cfg(not(target_arch = "wasm32"))]
fn verified_dir(dir: &Path) -> Result<PathBuf> {
    let dir = if dir.join("muscriptor-small-cond.onnx").is_file() {
        dir.to_owned()
    } else {
        dir.join("muscriptor")
    };
    for a in assets() {
        let p = dir.join(
            Path::new(&a.filename)
                .file_name()
                .context("Invalid model name")?,
        );
        ensure!(p.is_file(), "Missing MuScriptor asset: {}", p.display());
        ensure!(
            models::hash_file(&p)? == a.sha256,
            "MuScriptor checksum mismatch: {}",
            p.display()
        );
    }
    Ok(dir)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    /// Learned MT3_FULL_PLUS group IDs; empty means unconditional. Advisory only:
    /// all decoded instrument programs are preserved regardless of conditioning.
    pub instrument_groups: Vec<u16>,
    pub force_ties: bool,
    pub max_tokens: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            instrument_groups: vec![],
            force_ties: true,
            max_tokens: 2048,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Release {
    Explicit,
    Retrigger,
    MissingTieAtBoundary,
    MalformedTieBoundary,
    SourceEndUnknown,
    DrumConvention,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub pitch: u8,
    pub program: u16,
    pub instrument: String,
    pub is_drum: bool,
    pub release: Release,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Diagnostics {
    pub ignored_special_tokens: usize,
    pub invalid_tokens: usize,
    pub missing_state_pitches: usize,
    pub unmatched_note_offs: usize,
    pub malformed_tie_chunks: usize,
    pub backwards_shifts: usize,
    pub out_of_window_events: usize,
    pub collapsed_intervals: usize,
    pub missing_release_at_eof: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkTrace {
    pub start: f64,
    pub end: f64,
    pub instrument_ids: Vec<i64>,
    pub forced_tokens: Vec<i64>,
    pub generated_tokens: Vec<i64>,
    pub emitted_eos: bool,
    pub reached_token_limit: bool,
    pub elapsed_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub method: String,
    pub duration_seconds: f64,
    pub options: Options,
    pub model_assets: BTreeMap<String, String>,
    pub notes: Vec<Note>,
    pub chunks: Vec<ChunkTrace>,
    pub diagnostics: Diagnostics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Special(i64),
    Shift(u16),
    Pitch(u8),
    Velocity(u8),
    Tie,
    Program(u16),
    Drum(u8),
}
pub fn decode_token(token: i64) -> Option<Event> {
    Some(match token {
        0..=2 => Event::Special(token),
        3..=1003 => Event::Shift((token - 3) as u16),
        1004..=1131 => Event::Pitch((token - PITCH) as u8),
        1132..=1133 => Event::Velocity((token - VELOCITY) as u8),
        1134 => Event::Tie,
        1135..=1264 => Event::Program((token - PROGRAM) as u16),
        1265..=1392 => Event::Drum((token - DRUM) as u8),
        _ => return None,
    })
}
pub fn instrument_name(program: u16) -> String {
    let name = match program {
        0 => "acoustic_piano",
        2 => "electric_piano",
        8 => "chromatic_percussion",
        16 => "organ",
        24 => "acoustic_guitar",
        26 => "clean_electric_guitar",
        29 => "distorted_electric_guitar",
        32 => "acoustic_bass",
        33 => "electric_bass",
        40 => "violin",
        41 => "viola",
        42 => "cello",
        43 => "contrabass",
        46 => "orchestral_harp",
        47 => "timpani",
        48 => "string_ensemble",
        50 => "synth_strings",
        52 => "voice",
        55 => "orchestra_hit",
        56 => "trumpet",
        57 => "trombone",
        58 => "tuba",
        60 => "french_horn",
        61 => "brass_section",
        64 => "soprano_and_alto_sax",
        66 => "tenor_sax",
        67 => "baritone_sax",
        68 => "oboe",
        69 => "english_horn",
        70 => "bassoon",
        71 => "clarinet",
        72 => "flutes",
        80 => "synth_lead",
        88 => "synth_pad",
        100 => "atmosphere_fx",
        101 => "brightness_fx",
        128 => "drums",
        _ => return format!("program_{program}"),
    };
    name.into()
}

/// Exact source token-state rules, except EOF releases are bounded by the
/// recording and explicitly marked unknown rather than fabricated 10ms holds.
pub struct NoteDecoder {
    open: BTreeMap<(u16, u8), f64>,
    notes: Vec<Note>,
    tied: BTreeSet<(u16, u8)>,
    seek: f64,
    end: f64,
    tick: u64,
    start_tick: u64,
    program: Option<u16>,
    velocity: Option<u8>,
    prologue: bool,
    skip: bool,
    started: bool,
    pub diagnostics: Diagnostics,
}
impl Default for NoteDecoder {
    fn default() -> Self {
        Self::new()
    }
}
impl NoteDecoder {
    pub fn new() -> Self {
        Self {
            open: BTreeMap::new(),
            notes: vec![],
            tied: BTreeSet::new(),
            seek: 0.,
            end: 0.,
            tick: 0,
            start_tick: 0,
            program: None,
            velocity: None,
            prologue: true,
            skip: false,
            started: false,
            diagnostics: Diagnostics::default(),
        }
    }
    pub fn begin_chunk(&mut self, start: f64, end: f64) -> Result<()> {
        ensure!(
            start.is_finite() && end.is_finite() && start >= 0. && end > start,
            "Invalid MuScriptor source window"
        );
        if self.started && self.prologue {
            self.close_all(self.seek, Release::MalformedTieBoundary);
            self.diagnostics.malformed_tie_chunks += 1;
        }
        self.seek = start;
        self.end = end;
        self.start_tick = (start * 100.).round() as u64;
        self.tick = self.start_tick;
        self.program = None;
        self.velocity = None;
        self.prologue = true;
        self.skip = false;
        self.tied.clear();
        self.started = true;
        Ok(())
    }
    pub fn tie_tokens(&self) -> Vec<i64> {
        let mut tokens = vec![];
        let mut program = None;
        for &(p, k) in self.open.keys() {
            if program != Some(p) {
                tokens.push(PROGRAM + p as i64);
                program = Some(p);
            }
            tokens.push(PITCH + k as i64);
        }
        tokens.push(TIE);
        tokens
    }
    pub fn feed(&mut self, token: i64) {
        let Some(event) = decode_token(token) else {
            self.diagnostics.invalid_tokens += 1;
            return;
        };
        if let Event::Special(_) = event {
            self.diagnostics.ignored_special_tokens += 1;
            return;
        }
        if self.prologue {
            match event {
                Event::Tie => {
                    self.prologue = false;
                    self.velocity = None;
                    let ending: Vec<_> = self
                        .open
                        .keys()
                        .filter(|k| !self.tied.contains(k))
                        .copied()
                        .collect();
                    for k in ending {
                        self.close(k, self.seek, Release::MissingTieAtBoundary);
                    }
                }
                Event::Shift(_) => {
                    self.prologue = false;
                    self.skip = true;
                    self.diagnostics.malformed_tie_chunks += 1;
                    self.close_all(self.seek, Release::MalformedTieBoundary);
                }
                Event::Program(p) => self.program = Some(p),
                Event::Pitch(k) => {
                    if let Some(p) = self.program {
                        self.tied.insert((p, k));
                    }
                }
                _ => {}
            }
            return;
        }
        if self.skip {
            return;
        }
        match event {
            Event::Shift(s) => {
                if s > 0 {
                    let next = self.start_tick + s as u64;
                    if next < self.tick {
                        self.diagnostics.backwards_shifts += 1;
                    }
                    self.tick = next;
                }
            }
            Event::Program(p) => self.program = Some(p),
            Event::Velocity(v) => self.velocity = Some(v),
            Event::Pitch(k) => {
                let (Some(p), Some(v)) = (self.program, self.velocity) else {
                    self.diagnostics.missing_state_pitches += 1;
                    return;
                };
                let t = self.tick as f64 / 100.;
                if t < self.seek || t >= self.end {
                    self.diagnostics.out_of_window_events += 1;
                    return;
                }
                let key = (p, k);
                if self.open.contains_key(&key) {
                    self.close(
                        key,
                        t,
                        if v > 0 {
                            Release::Retrigger
                        } else {
                            Release::Explicit
                        },
                    );
                } else if v == 0 {
                    self.diagnostics.unmatched_note_offs += 1;
                }
                if v > 0 {
                    self.open.insert(key, t);
                }
            }
            Event::Drum(k) => {
                let t = self.tick as f64 / 100.;
                if t >= self.seek && t < self.end {
                    self.notes.push(Note {
                        start: t,
                        end: (t + 0.01).min(self.end),
                        pitch: k,
                        program: 128,
                        instrument: "drums".into(),
                        is_drum: true,
                        release: Release::DrumConvention,
                    });
                } else {
                    self.diagnostics.out_of_window_events += 1;
                }
            }
            _ => {}
        }
    }
    fn close(&mut self, key: (u16, u8), end: f64, release: Release) {
        if let Some(start) = self.open.remove(&key) {
            if end > start {
                self.notes.push(Note {
                    start,
                    end,
                    pitch: key.1,
                    program: key.0,
                    instrument: instrument_name(key.0),
                    is_drum: false,
                    release,
                });
            } else {
                self.diagnostics.collapsed_intervals += 1;
            }
        }
    }
    fn close_all(&mut self, end: f64, release: Release) {
        let keys: Vec<_> = self.open.keys().copied().collect();
        for k in keys {
            self.close(k, end, release.clone());
        }
    }
    pub fn finish(mut self, duration: f64) -> (Vec<Note>, Diagnostics) {
        if self.started && self.prologue {
            self.diagnostics.malformed_tie_chunks += 1;
            self.close_all(self.seek, Release::MalformedTieBoundary);
        } else {
            self.diagnostics.missing_release_at_eof = self.open.len();
            self.close_all(duration, Release::SourceEndUnknown);
        }
        self.notes
            .retain(|n| n.start >= 0. && n.start < duration && n.end > n.start);
        for n in &mut self.notes {
            n.end = n.end.min(duration);
        }
        self.notes.sort_by(|a, b| {
            a.start
                .total_cmp(&b.start)
                .then(a.program.cmp(&b.program))
                .then(a.pitch.cmp(&b.pitch))
        });
        (self.notes, self.diagnostics)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Transcriber {
    conditioner: Session,
    lm: Session,
    model_assets: BTreeMap<String, String>,
}
#[cfg(not(target_arch = "wasm32"))]
impl Transcriber {
    pub fn load(dir: &Path) -> Result<Self> {
        let dir = verified_dir(dir)?;
        let conditioner = Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(dir.join("muscriptor-small-cond.onnx"))?;
        let lm = Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(dir.join("muscriptor-small-lm.onnx"))?;
        for name in ["audio", "instrument", "dataset"] {
            ensure!(
                conditioner.inputs.iter().any(|i| i.name == name),
                "MuScriptor conditioner input missing: {name}"
            );
        }
        for name in ["tokens", "prefix", "past_k", "past_v"] {
            ensure!(
                lm.inputs.iter().any(|i| i.name == name),
                "MuScriptor LM input missing: {name}"
            );
        }
        Ok(Self {
            conditioner,
            lm,
            model_assets: assets()
                .into_iter()
                .map(|a| (a.filename, a.sha256))
                .collect(),
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn transcribe(&mut self, audio: &Path, options: Options) -> Result<Output> {
        self.transcribe_samples(&models::decode(audio, SAMPLE_RATE as u32, 1)?, options)
    }
    pub fn transcribe_samples(&mut self, audio: &[f32], options: Options) -> Result<Output> {
        ensure!(
            !audio.is_empty() && audio.iter().all(|s| s.is_finite()),
            "MuScriptor requires finite nonempty16kHz mono audio"
        );
        ensure!(
            options.max_tokens > 0 && options.max_tokens <= 8192,
            "Invalid MuScriptor token limit"
        );
        ensure!(
            options.instrument_groups.iter().all(|g| *g <= 999 - 2),
            "Invalid MuScriptor conditioning group"
        );
        let ids: Vec<i64> = if options.instrument_groups.is_empty() {
            vec![1]
        } else {
            options
                .instrument_groups
                .iter()
                .map(|g| *g as i64 + 2)
                .collect()
        };
        let duration = audio.len() as f64 / SAMPLE_RATE as f64;
        let mut decoder = NoteDecoder::new();
        let mut chunks = vec![];
        for (index, part) in audio.chunks(CHUNK_SAMPLES).enumerate() {
            let timer = Instant::now();
            let start = index as f64 * 5.;
            let end = (start + 5.).min(duration);
            decoder.begin_chunk(start, end)?;
            let forced = if options.force_ties && index > 0 {
                decoder.tie_tokens()
            } else {
                vec![]
            };
            for &t in &forced {
                decoder.feed(t);
            }
            let mut padded = part.to_vec();
            padded.resize(CHUNK_SAMPLES, 0.);
            let mut out=self.conditioner.run(ort::inputs!["audio"=>Tensor::from_array(([1,CHUNK_SAMPLES],padded.into_boxed_slice()))?,"instrument"=>Tensor::from_array(([1,ids.len()],ids.clone().into_boxed_slice()))?,"dataset"=>Tensor::from_array(([1,1],vec![1i64].into_boxed_slice()))?])?;
            let prefix = out.remove("prefix").context("MuScriptor prefix missing")?;
            let (ps, pd) = prefix.try_extract_tensor::<f32>()?;
            ensure!(
                ps.as_ref() == [1, (502 + ids.len()) as i64, DIM as i64]
                    && pd.iter().all(|x| x.is_finite()),
                "Invalid MuScriptor prefix shape/numerics: {ps:?}"
            );
            let prefix_len = ps[1] as usize;
            // ort's raw-array constructor rejects zero dimensions. Allocator
            // tensors support the genuine empty prefix/cache required by the
            // graph; they contain zero elements, so no data is uninitialized.
            let allocator = Allocator::default();
            let empty_prefix = Tensor::<f32>::new(&allocator, [1, 0, DIM])?.into_dyn();
            let mut past_k =
                Tensor::<f32>::new(&allocator, [LAYERS, 1, 0, HEADS, HEAD_DIM])?.into_dyn();
            let mut past_v =
                Tensor::<f32>::new(&allocator, [LAYERS, 1, 0, HEADS, HEAD_DIM])?.into_dyn();
            let mut input = vec![BOS];
            input.extend_from_slice(&forced);
            let mut generated = vec![];
            let mut eos = false;
            let mut cached = 0;
            for step in 0..options.max_tokens {
                let first = step == 0;
                let prefix_input = if first { &prefix } else { &empty_prefix };
                let n = input.len();
                let mut output=self.lm.run(ort::inputs!["tokens"=>Tensor::from_array(([1,n],input.into_boxed_slice()))?,"prefix"=>prefix_input,"past_k"=>&past_k,"past_v"=>&past_v])?;
                let (shape, logits) = output["logits"].try_extract_tensor::<f32>()?;
                ensure!(
                    shape.as_ref() == [1, VOCAB_SIZE as i64]
                        && logits.iter().all(|x| x.is_finite()),
                    "Invalid MuScriptor logits shape/numerics: {shape:?}"
                );
                let token = logits
                    .iter()
                    .enumerate()
                    .max_by(|(ai, a), (bi, b)| a.total_cmp(b).then_with(|| bi.cmp(ai)))
                    .context("Empty MuScriptor logits")?
                    .0 as i64;
                generated.push(token);
                if token == EOS {
                    eos = true;
                    break;
                }
                decoder.feed(token);
                cached += n + if first { prefix_len } else { 0 };
                past_k = output
                    .remove("present_k")
                    .context("MuScriptor present_k missing")?;
                past_v = output
                    .remove("present_v")
                    .context("MuScriptor present_v missing")?;
                for (name, value) in [("K", &past_k), ("V", &past_v)] {
                    let (sh, _) = value.try_extract_tensor::<f32>()?;
                    ensure!(
                        sh.as_ref()
                            == [
                                LAYERS as i64,
                                1,
                                cached as i64,
                                HEADS as i64,
                                HEAD_DIM as i64
                            ],
                        "Unexpected MuScriptor {name} cache shape: {sh:?}"
                    );
                }
                input = vec![token];
            }
            eprintln!(
                "MuScriptor chunk{} {:.1}/{duration:.1}s: {}generatedtokens EOS={eos}",
                index + 1,
                end,
                generated.len()
            );
            chunks.push(ChunkTrace {
                start,
                end,
                instrument_ids: ids.clone(),
                forced_tokens: forced,
                generated_tokens: generated,
                reached_token_limit: !eos,
                emitted_eos: eos,
                elapsed_seconds: timer.elapsed().as_secs_f64(),
            });
        }
        let (notes, diagnostics) = decoder.finish(duration);
        Ok(Output {
            method: METHOD.into(),
            duration_seconds: duration,
            options,
            model_assets: self.model_assets.clone(),
            notes,
            chunks,
            diagnostics,
        })
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn transcribe(audio: &Path, model_dir: &Path, options: Options) -> Result<Output> {
    Transcriber::load(model_dir)?.transcribe(audio, options)
}
