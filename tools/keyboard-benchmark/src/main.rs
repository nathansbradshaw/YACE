//! Reproducible raw keyboard transcription comparisons, before game arrangement.
//! Real recordings have no invented reference labels. Listening decides preference.
mod scoring;
use anyhow::{Context, Result, ensure};
use chart_engine::{mapping::PitchNote, models};
use clap::{Parser, Subcommand};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use scoring::metrics;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Instant,
};

const RATE: usize = 44100;
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Render supplied candidate notes for human review without fabricating scores.
    Audition {
        #[arg(long)]
        notes: PathBuf,
        #[arg(long)]
        soundfont: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        duration: f64,
        #[arg(long, value_delimiter = ',')]
        programs: Vec<u8>,
    },
    Prepare {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        soundfont: PathBuf,
    },
    AddSong {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        stems: PathBuf,
        #[arg(long, default_value = "0")]
        source_offset: f64,
    },
    AddControls {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        soundfont: PathBuf,
    },
    Run {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long, default_value = "basic")]
        detector: String,
        #[arg(long)]
        clip: Option<String>,
        #[arg(long)]
        input: Option<String>,
    },
    Report {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        soundfont: PathBuf,
        #[arg(long)]
        clip: Option<String>,
        #[arg(long, default_value = "listening")]
        name: String,
        #[arg(long)]
        primary: bool,
        #[arg(long)]
        synth: bool,
        #[arg(long)]
        game_range: bool,
    },
}
#[derive(Clone, Serialize, Deserialize)]
struct Clip {
    id: String,
    title: String,
    original: PathBuf,
    start: f64,
    duration: f64,
    inputs: BTreeMap<String, PathBuf>,
    reference: Option<Vec<PitchNote>>,
    reference_kind: String,
    source_sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Corpus {
    version: u32,
    clips: Vec<Clip>,
}
#[derive(Serialize, Deserialize)]
struct Run {
    clip: String,
    input: String,
    detector: String,
    input_sha256: String,
    model_sha256: String,
    implementation_sha256: String,
    #[serde(default)]
    shared_audio_sha256: String,
    #[serde(default)]
    model_assets: BTreeMap<String, String>,
    elapsed_seconds: f64,
    notes: Vec<PitchNote>,
    metrics: serde_json::Value,
}
fn implementation(detector: &str) -> &'static [u8] {
    match detector {
        "basic" => include_bytes!("../../../src/models.rs"),
        "kong" => include_bytes!("../../../src/piano_model.rs"),
        "transkun" => include_bytes!("../../../src/transkun.rs"),
        _ => unreachable!(),
    }
}
fn model_assets(detector: &str) -> Result<Vec<models::ModelAsset>> {
    Ok(match detector {
        "basic" => models::assets()
            .into_iter()
            .filter(|m| m.id == "pitch")
            .collect(),
        "kong" => vec![models::ModelAsset {
            id: "piano".into(),
            filename: "piano_kong.onnx".into(),
            url: chart_engine::piano_model::MODEL_URL.into(),
            sha256: chart_engine::piano_model::MODEL_SHA256.into(),
            license: "CC-BY-4.0".into(),
        }],
        "transkun" => chart_engine::transkun::assets(),
        _ => anyhow::bail!("Unknown detector {detector}"),
    })
}
fn snapshot(base: &Path, bytes: &[u8]) -> Result<String> {
    let hash = format!("{:x}", Sha256::digest(bytes));
    let dir = base.join("source-snapshots");
    fs::create_dir_all(&dir)?;
    let path = dir.join(&hash);
    if path.exists() {
        ensure!(fs::read(&path)? == bytes, "Source snapshot changed");
    } else {
        fs::write(&path, bytes)?;
    }
    Ok(hash)
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn command(command: &mut Command) -> Result<()> {
    let out = command.output()?;
    ensure!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}
fn cut(source: &Path, start: f64, duration: f64, out: &Path) -> Result<()> {
    command(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(source)
            .args([
                "-ss",
                &start.to_string(),
                "-t",
                &duration.to_string(),
                "-ac",
                "1",
                "-ar",
                "44100",
                "-c:a",
                "pcm_f32le",
                "-n",
            ])
            .arg(out),
    )
}
fn render(
    font: &Arc<SoundFont>,
    notes: &[PitchNote],
    duration: f64,
    program: i32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    let mut settings = SynthesizerSettings::new(RATE as i32);
    settings.enable_reverb_and_chorus = false;
    let mut synth = Synthesizer::new(font, &settings)?;
    synth.process_midi_message(0, 0xC0, program, 0);
    let mut events: Vec<_> = notes
        .iter()
        .flat_map(|n| {
            [
                ((n.start * RATE as f64).round() as usize, true, n.pitch),
                ((n.end * RATE as f64).round() as usize, false, n.pitch),
            ]
        })
        .collect();
    events.sort_by_key(|(at, on, p)| (*at, *on, *p));
    let count = (duration * RATE as f64).round() as usize;
    let (mut left, mut right) = (vec![0.; count], vec![0.; count]);
    let mut pos = 0;
    for (at, on, pitch) in events {
        let at = at.min(count);
        if at > pos {
            synth.render(&mut left[pos..at], &mut right[pos..at]);
            pos = at;
        }
        if on {
            synth.note_on(0, pitch as i32, 96);
        } else {
            synth.note_off(0, pitch as i32);
        }
    }
    if pos < count {
        synth.render(&mut left[pos..], &mut right[pos..]);
    }
    Ok((left, right))
}
fn fixture_notes() -> Vec<PitchNote> {
    let mut notes = vec![];
    // Original benchmark phrase, fixed before detector runs. Wide range, dyads,
    // repeated keys and a fast passage; this is a controlled fixture, not a song.
    for (i, pitch) in [
        60, 64, 67, 72, 71, 67, 64, 62, 60, 60, 60, 67, 69, 65, 62, 60, 48, 55, 60, 64, 72, 79, 84,
        76,
    ]
    .iter()
    .enumerate()
    {
        let start = 0.5 + i as f64 * 0.45;
        notes.push(PitchNote {
            start,
            end: start + 0.32,
            pitch: *pitch,
            confidence: 1.,
        });
    }
    for pitch in [53, 60, 65] {
        notes.push(PitchNote {
            start: 11.5,
            end: 12.8,
            pitch,
            confidence: 1.,
        });
    }
    for (i, pitch) in [67, 69, 71, 72, 74, 76, 77, 79].iter().enumerate() {
        let start = 13.1 + i as f64 * 0.15;
        notes.push(PitchNote {
            start,
            end: start + 0.1,
            pitch: *pitch,
            confidence: 1.,
        });
    }
    notes
}
fn prepare(out: &Path, soundfont: &Path) -> Result<()> {
    ensure!(!out.exists(), "Corpus output already exists");
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let font = Arc::new(SoundFont::new(&mut fs::File::open(soundfont)?)?);
    let mut clips = vec![];
    for (id, program) in [
        ("fixture-piano", 0),
        ("fixture-electric-piano", 4),
        ("fixture-square-lead", 80),
        ("fixture-saw-lead", 81),
        ("fixture-warm-pad", 89),
    ] {
        let dir = out.join(id);
        fs::create_dir(&dir)?;
        let notes = fixture_notes();
        let (left, right) = render(&font, &notes, 16., program)?;
        let audio = dir.join("mix.wav");
        models::write_audio(&audio, &left, &right)?;
        let hash = models::hash_file(&audio)?;
        clips.push(Clip{id:id.into(),title:format!("Controlled GM program {program}"),original:audio.clone(),start:0.,duration:16.,inputs:BTreeMap::from([("mix".into(),audio)]),reference:Some(notes),reference_kind:"Known authored events rendered with RustySynth / TimGM6mb. Controlled timbre fixture; not real-song accuracy.".into(),source_sha256:hash});
    }
    let dir = out.join("fixture-silence");
    fs::create_dir(&dir)?;
    let audio = dir.join("mix.wav");
    let samples = vec![0.; 16 * RATE];
    models::write_audio(&audio, &samples, &samples)?;
    let hash = models::hash_file(&audio)?;
    clips.push(Clip {
        id: "fixture-silence".into(),
        title: "Silence negative control".into(),
        original: audio.clone(),
        start: 0.,
        duration: 16.,
        inputs: BTreeMap::from([("mix".into(), audio)]),
        reference: Some(vec![]),
        reference_kind: "Known zero target notes".into(),
        source_sha256: hash,
    });
    let corpus = Corpus { version: 1, clips };
    save(&out.join("corpus.json"), &corpus)?;
    println!("{}", out.join("corpus.json").display());
    Ok(())
}
fn extend_song(
    corpus_path: &Path,
    id: &str,
    title: &str,
    audio: &Path,
    stems: &Path,
    source_offset: f64,
) -> Result<()> {
    ensure!(
        id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !id.is_empty(),
        "Invalid song ID"
    );
    ensure!(
        source_offset.is_finite() && source_offset >= 0.,
        "Invalid source offset"
    );
    let mut corpus: Corpus = load(corpus_path)?;
    let base = corpus_path.parent().unwrap().canonicalize()?;
    // Require the source-bound native separator receipt; do not infer alignment
    // from file lengths alone.
    let manifest: serde_json::Value = load(&stems.join("stem-manifest.json"))?;
    let hash = models::hash_file(audio)?;
    ensure!(
        manifest["source_audio_sha256"] == hash,
        "Stem manifest/source mismatch"
    );
    for (index, start) in [0., 16.].iter().enumerate() {
        let clip_id = format!("{id}-{index}");
        ensure!(!corpus.clips.iter().any(|c| c.id == clip_id), "Clip exists");
        let dir = base.join(&clip_id);
        fs::create_dir(&dir)?;
        let mut inputs = BTreeMap::new();
        for (input, path) in [
            ("mix", audio.to_owned()),
            ("piano", stems.join("piano.wav")),
            ("other", stems.join("other.wav")),
        ] {
            let output = dir.join(format!("{input}.wav"));
            cut(&path, *start, 16., &output)?;
            inputs.insert(input.into(), output);
        }
        let piano = models::decode(&inputs["piano"], RATE as u32, 1)?;
        let other = models::decode(&inputs["other"], RATE as u32, 1)?;
        ensure!(piano.len() == other.len(), "Stem clocks differ");
        let bed: Vec<_> = piano.iter().zip(&other).map(|(a, b)| a + b).collect();
        let output = dir.join("keyboard-bed.wav");
        models::write_audio(&output, &bed, &bed)?;
        inputs.insert("keyboard-bed".into(), output);
        corpus.clips.push(Clip{id:clip_id,title:title.into(),original:audio.canonicalize()?,start:source_offset+start,duration:16.,inputs,reference:None,reference_kind:format!("Unannotated real recording; source-bound separation of a 32-second excerpt starting {source_offset}s into the original. Listening review only."),source_sha256:hash.clone()});
    }
    fs::copy(corpus_path, base.join(format!("corpus-before-{id}.json")))?;
    corpus.version += 1;
    save(corpus_path, &corpus)
}
fn extend_controls(corpus_path: &Path, font_path: &Path) -> Result<()> {
    let mut corpus: Corpus = load(corpus_path)?;
    let base = corpus_path.parent().unwrap().canonicalize()?;
    let font = Arc::new(SoundFont::new(&mut fs::File::open(font_path)?)?);
    for (id, program, notes, duration) in [
        (
            "fixture-held-pad",
            89,
            vec![
                PitchNote {
                    start: 0.5,
                    end: 12.5,
                    pitch: 60,
                    confidence: 1.,
                },
                PitchNote {
                    start: 3.,
                    end: 13.5,
                    pitch: 67,
                    confidence: 1.,
                },
                PitchNote {
                    start: 8.,
                    end: 14.5,
                    pitch: 64,
                    confidence: 1.,
                },
            ],
            16.,
        ),
        (
            "fixture-seam-repeats",
            0,
            [6.8, 7.4, 8., 8.6, 9.2, 14.8, 15.4, 16., 16.6]
                .iter()
                .map(|&start| PitchNote {
                    start,
                    end: start + 0.4,
                    pitch: 60,
                    confidence: 1.,
                })
                .collect(),
            20.,
        ),
    ] {
        ensure!(!corpus.clips.iter().any(|c| c.id == id), "Control exists");
        let dir = base.join(id);
        fs::create_dir(&dir)?;
        let (left, right) = render(&font, &notes, duration, program)?;
        let audio = dir.join("mix.wav");
        models::write_audio(&audio, &left, &right)?;
        let hash = models::hash_file(&audio)?;
        corpus.clips.push(Clip{id:id.into(),title:id.into(),original:audio.clone(),start:0.,duration,inputs:BTreeMap::from([("mix".into(),audio)]),reference:Some(notes),reference_kind:"Known authored events rendered with RustySynth / TimGM6mb; long holds and repeated-key window seams. Controlled fixture, not real-song accuracy.".into(),source_sha256:hash});
    }
    fs::copy(corpus_path, base.join("corpus-before-long-controls.json"))?;
    corpus.version += 1;
    save(corpus_path, &corpus)
}
fn run(
    corpus_path: &Path,
    model_dir: &Path,
    detector: &str,
    filter_clip: Option<&str>,
    filter_input: Option<&str>,
) -> Result<()> {
    let corpus: Corpus = load(corpus_path)?;
    let output = corpus_path.parent().unwrap().join("runs");
    fs::create_dir_all(&output)?;
    let assets = model_assets(detector)?;
    let mut asset_hashes = BTreeMap::new();
    for asset in &assets {
        let hash = models::hash_file(&model_dir.join(&asset.filename))?;
        ensure!(
            hash == asset.sha256,
            "Model checksum mismatch: {}",
            asset.filename
        );
        asset_hashes.insert(asset.filename.clone(), hash);
    }
    let model = model_dir.join(match detector {
        "basic" => "basic_pitch.onnx",
        "kong" => "piano_kong.onnx",
        "transkun" => "transkun/transkun.onnx",
        _ => unreachable!(),
    });
    let base = corpus_path.parent().unwrap();
    let implementation_hash = snapshot(base, implementation(detector))?;
    let shared_hash = snapshot(base, include_bytes!("../../../src/models.rs"))?;
    snapshot(base, include_bytes!("../../../Cargo.lock"))?;
    snapshot(base, include_bytes!("../Cargo.lock"))?;
    for clip in &corpus.clips {
        if filter_clip.is_some_and(|id| id != clip.id) {
            continue;
        }
        for (input, path) in &clip.inputs {
            if filter_input.is_some_and(|id| id != input) {
                continue;
            }
            let out = output.join(format!("{}--{}--{}.json", clip.id, input, detector));
            let input_hash = models::hash_file(path)?;
            let model_hash = models::hash_file(&model)?;
            if out.exists() {
                let old: Run = load(&out)?;
                ensure!(
                    old.input_sha256 == input_hash
                        && old.model_sha256 == model_hash
                        && old.implementation_sha256 == implementation_hash
                        && old.shared_audio_sha256 == shared_hash
                        && old.model_assets == asset_hashes,
                    "Stale benchmark result; preserve old run and rerun after decoder changes"
                );
                continue;
            }
            let start = Instant::now();
            let notes = match detector {
                "basic" => models::pitch(path, &model)?,
                "kong" => chart_engine::piano_model::pitch(path, &model)?,
                "transkun" => chart_engine::transkun::pitch(path, &model_dir.join("transkun"))?,
                _ => unreachable!(),
            };
            let result = Run {
                clip: clip.id.clone(),
                input: input.clone(),
                detector: detector.into(),
                input_sha256: input_hash,
                model_sha256: model_hash,
                implementation_sha256: implementation_hash.clone(),
                shared_audio_sha256: shared_hash.clone(),
                model_assets: asset_hashes.clone(),
                elapsed_seconds: start.elapsed().as_secs_f64(),
                metrics: metrics(&notes, clip.reference.as_deref()),
                notes,
            };
            save(&out, &result)?;
            println!(
                "{} {} {}: {} notes",
                clip.id,
                input,
                detector,
                result.notes.len()
            );
        }
    }
    Ok(())
}
fn encode(path: &Path, left: &[f32], right: &[f32]) -> Result<()> {
    let wav = path.with_extension("wav");
    models::write_audio(&wav, left, right)?;
    command(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&wav)
            .args(["-c:a", "libopus", "-b:a", "128k", "-n"])
            .arg(path),
    )
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn report(
    corpus_path: &Path,
    font_path: &Path,
    only: Option<&str>,
    name: &str,
    primary: bool,
    synth: bool,
    game_range: bool,
) -> Result<()> {
    ensure!(!(primary && synth), "Choose piano or synth audition routes");
    let corpus: Corpus = load(corpus_path)?;
    let base = corpus_path.parent().unwrap();
    let font = Arc::new(SoundFont::new(&mut fs::File::open(font_path)?)?);
    ensure!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid report name"
    );
    let root = base.join(name);
    ensure!(
        !root.exists(),
        "Listening report already exists; use a new report directory after changes"
    );
    let mut all = vec![];
    for file in fs::read_dir(base.join("runs"))? {
        let file = file?;
        if file.path().extension().is_some_and(|e| e == "json") {
            all.push(load::<Run>(&file.path())?);
        }
    }
    let mut versions = BTreeMap::<String, (String, String)>::new();
    let mut seen = std::collections::BTreeSet::new();
    let selected = |clip: &Clip, input: &str, detector: &str| {
        only.is_none_or(|id| id == clip.id)
            && (!synth || input == "keyboard-bed" || input == "piano" && detector == "basic")
            && (!primary
                || input == "piano"
                || input == "mix" && (detector == "kong" || clip.reference.is_some()))
    };
    all.retain(|r| {
        corpus
            .clips
            .iter()
            .find(|c| c.id == r.clip)
            .is_some_and(|c| selected(c, &r.input, &r.detector))
    });
    for run in &mut all {
        let clip = corpus
            .clips
            .iter()
            .find(|c| c.id == run.clip)
            .ok_or_else(|| anyhow::anyhow!("Unknown run clip {}", run.clip))?;
        let input = clip
            .inputs
            .get(&run.input)
            .ok_or_else(|| anyhow::anyhow!("Unknown input {}", run.input))?;
        ensure!(
            seen.insert((run.clip.clone(), run.input.clone(), run.detector.clone())),
            "Duplicate run"
        );
        ensure!(
            models::hash_file(input)? == run.input_sha256,
            "Stale run input"
        );
        let pinned: BTreeMap<_, _> = model_assets(&run.detector)?
            .into_iter()
            .map(|a| (a.filename, a.sha256))
            .collect();
        ensure!(
            run.model_assets == pinned,
            "Incomplete or stale model provenance"
        );
        ensure!(
            pinned.values().any(|h| h == &run.model_sha256),
            "Primary model mismatch"
        );
        for hash in [&run.implementation_sha256, &run.shared_audio_sha256] {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid source hash"
            );
            ensure!(
                models::hash_file(&base.join("source-snapshots").join(hash))? == *hash,
                "Missing or changed implementation snapshot"
            );
        }
        let version = (
            run.implementation_sha256.clone(),
            run.shared_audio_sha256.clone(),
        );
        if let Some(old) = versions.insert(run.detector.clone(), version.clone()) {
            ensure!(
                old == version,
                "Mixed decoder versions for {}",
                run.detector
            );
        }
        // References/scorer can improve without repeating expensive neural inference.
        run.metrics = metrics(&run.notes, clip.reference.as_deref());
        ensure!(
            run.metrics.get("scoring_error").is_none(),
            "Invalid note interval in {}",
            run.clip
        );
        ensure!(
            run.notes
                .iter()
                .all(|n| n.start < clip.duration && n.end <= clip.duration + 1e-6),
            "Run exceeds source bounds"
        );
    }
    let expected: usize = corpus
        .clips
        .iter()
        .map(|c| {
            c.inputs
                .keys()
                .map(|input| {
                    ["basic", "kong", "transkun"]
                        .iter()
                        .filter(|d| selected(c, input, d))
                        .count()
                })
                .sum::<usize>()
        })
        .sum();
    ensure!(expected > 0, "Unknown or empty clip selection");
    ensure!(
        seen.len() == expected,
        "Incomplete benchmark: {} of {expected} cases",
        seen.len()
    );
    for clip in &corpus.clips {
        if only.is_some_and(|id| id != clip.id) {
            continue;
        }
        ensure!(
            models::hash_file(&clip.original)? == clip.source_sha256,
            "Source recording changed: {}",
            clip.id
        );
        for input in clip.inputs.keys() {
            for detector in ["basic", "kong", "transkun"] {
                if !selected(clip, input, detector) {
                    continue;
                }
                ensure!(
                    seen.contains(&(clip.id.clone(), input.clone(), detector.into())),
                    "Missing case {} / {input} / {detector}",
                    clip.id
                );
            }
        }
    }
    fs::create_dir(&root)?;
    save(
        &root.join("provenance.json"),
        &json!({
            "corpus_sha256":models::hash_file(corpus_path)?,
            "soundfont_sha256":models::hash_file(font_path)?,
            "scoring_source_sha256":snapshot(base, include_bytes!("scoring.rs"))?,
            "harness_source_sha256":snapshot(base, include_bytes!("main.rs"))?,
        "root_dependency_lock_sha256":snapshot(base,include_bytes!("../../../Cargo.lock"))?,
        "harness_dependency_lock_sha256":snapshot(base,include_bytes!("../Cargo.lock"))?,
        "game_range":game_range,
            "versions":versions,"validated_cases":expected,
            "note":"Real songs are unlabeled; reference scores apply only to authored controls. Landing legacy stem alignment lacks a source-bound receipt and is exploratory only."
        }),
    )?;
    let mut key = BTreeMap::new();
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>YACE keyboard listening benchmark</title><style>body{background:#101820;color:#eef4f7;font:17px system-ui;max-width:1100px;margin:35px auto;padding:20px}section{padding:20px;background:#1d2a35;margin:20px 0;border-radius:12px}audio{width:100%}details{margin:16px 0}p{line-height:1.5}button{padding:8px}a{color:#9ddeff}</style><h1>YACE keyboard benchmark</h1><p>Compare notes and timing against the original. Every candidate uses the same piano patch and fixed velocity for audition, so judge note selection, octaves, attacks and releases rather than resemblance of tone. These are raw detections before game-range folding. Model identities are hidden until you open the answer key. Real songs have no fabricated accuracy scores.</p><p><a href='blind-key.json'>Reveal identities</a> · <a href='results.json'>Measurements and provenance</a></p>",
    );
    if game_range {
        html = html.replace("These are raw detections before game-range folding.",
            "These candidates use the Pro Keys game-range arrangement; octave moves and merges are recorded in each arrangement JSON. Source times are preserved with controller tick rounding.");
    }
    for clip in &corpus.clips {
        if only.is_some_and(|id| id != clip.id) {
            continue;
        }
        let source = models::decode(&clip.inputs["mix"], RATE as u32, 1)?;
        let filename = format!("{}--source.ogg", clip.id);
        encode(&root.join(&filename), &source, &source)?;
        html.push_str(&format!("<section><h2>{}</h2><p>Passage starts {:.0}s into the recording. {}</p><h3>Original</h3><audio controls preload=none src='{}'></audio>",escape(&clip.title),clip.start,escape(&clip.reference_kind),filename));
        let mut candidates: Vec<_> = all
            .iter()
            .filter(|r| {
                r.clip == clip.id
                    && (!primary
                        || r.input == "piano"
                        || r.input == "mix" && (r.detector == "kong" || clip.reference.is_some()))
            })
            .collect();
        candidates.sort_by_key(|r| {
            format!("{}:{}:{}", r.detector, r.input, clip.id)
                .bytes()
                .fold(0u64, |a, b| a.wrapping_mul(131).wrapping_add(b as u64))
        });
        for (index, r) in candidates.iter().enumerate() {
            let label = format!("{}-{}", clip.id, (b'A' + index as u8) as char);
            let filename = format!("{label}.ogg");
            let arranged;
            let notes = if game_range {
                let timing = chart_engine::timing::TempoMap::constant(120.)?;
                let plan = chart_engine::pro_keys::arrange(&r.notes, &timing)?;
                arranged = plan
                    .notes
                    .iter()
                    .map(|n| PitchNote {
                        start: timing.seconds(n.tick),
                        end: timing.seconds(n.tick + n.sustain),
                        pitch: n.lane,
                        confidence: 1.,
                    })
                    .collect::<Vec<_>>();
                save(&root.join(format!("{label}-arrangement.json")), &plan)?;
                &arranged
            } else {
                &r.notes
            };
            let (left, right) = render(&font, notes, clip.duration, 0)?;
            encode(&root.join(&filename), &left, &right)?;
            key.insert(label.clone(),json!({"detector":r.detector,"input":r.input,"input_sha256":r.input_sha256,"model_sha256":r.model_sha256}));
            html.push_str(&format!("<details><summary>Candidate {}</summary><audio controls preload=none src='{}'></audio></details>",(b'A'+index as u8)as char,filename));
        }
        html.push_str("</section>");
    }
    fs::write(root.join("index.html"), html)?;
    save(&root.join("results.json"), &all)?;
    save(&root.join("blind-key.json"), &key)?;
    println!("{}", root.join("index.html").display());
    Ok(())
}
fn main() -> Result<()> {
    match Args::parse().command {
        Action::Audition {
            notes,
            soundfont,
            output,
            duration,
            programs,
        } => {
            ensure!(
                duration.is_finite() && duration > 0. && duration <= 1200.,
                "Invalid audition duration"
            );
            ensure!(!output.exists(), "Audition output already exists");
            let value: serde_json::Value = load(&notes)?;
            let array = value
                .as_array()
                .or_else(|| value.get("notes").and_then(|v| v.as_array()))
                .ok_or_else(|| anyhow::anyhow!("Expected notes array"))?;
            let notes: Vec<PitchNote> = array
                .iter()
                .filter(|v| !v["is_drum"].as_bool().unwrap_or(false))
                .filter(|v| {
                    programs.is_empty()
                        || v["program"]
                            .as_u64()
                            .is_some_and(|p| programs.iter().any(|id| u64::from(*id) == p))
                })
                .map(|v| -> Result<_> {
                    Ok(PitchNote {
                        start: v["start"].as_f64().context("Missing onset")?,
                        end: v["end"].as_f64().context("Missing release")?,
                        pitch: v["pitch"]
                            .as_u64()
                            .filter(|p| *p <= 127)
                            .context("Invalid pitch")? as u8,
                        // Rendering uses neither model confidence nor velocity.
                        confidence: 1.,
                    })
                })
                .collect::<Result<_>>()?;
            ensure!(
                notes.iter().all(|n| n.start.is_finite()
                    && n.end.is_finite()
                    && n.start >= 0.
                    && n.end > n.start
                    && n.end <= duration + 1e-6),
                "Notes exceed source bounds"
            );
            let font = Arc::new(SoundFont::new(&mut fs::File::open(&soundfont)?)?);
            let (left, right) = render(&font, &notes, duration, 0)?;
            fs::create_dir(&output)?;
            encode(&output.join("candidate.ogg"), &left, &right)?;
            save(&output.join("source-events.json"), &value)?;
            save(
                &output.join("render.json"),
                &json!({"source_events_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&value)?)),"soundfont_sha256":models::hash_file(&soundfont)?,"source_program_selection":programs,"program":0,"fixed_velocity":96,"duration":duration,"note":"Audition only; no generated confidence is a claim of correctness."}),
            )?;
            println!("{}", output.join("candidate.wav").display());
            Ok(())
        }
        Action::Prepare { output, soundfont } => prepare(&output, &soundfont),
        Action::AddSong {
            corpus,
            id,
            title,
            audio,
            stems,
            source_offset,
        } => extend_song(&corpus, &id, &title, &audio, &stems, source_offset),
        Action::AddControls { corpus, soundfont } => extend_controls(&corpus, &soundfont),
        Action::Run {
            corpus,
            model_dir,
            detector,
            clip,
            input,
        } => run(
            &corpus,
            &model_dir,
            &detector,
            clip.as_deref(),
            input.as_deref(),
        ),
        Action::Report {
            corpus,
            soundfont,
            clip,
            name,
            primary,
            synth,
            game_range,
        } => report(
            &corpus,
            &soundfont,
            clip.as_deref(),
            &name,
            primary,
            synth,
            game_range,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(t: f64, p: u8) -> PitchNote {
        PitchNote {
            start: t,
            end: t + 0.3,
            pitch: p,
            confidence: 1.,
        }
    }
    #[test]
    fn one_reference_cannot_match_two_detections() {
        let m = metrics(&[n(0., 60), n(0.01, 60)], Some(&[n(0., 60)]));
        assert_eq!(m["reference"]["matched_exact_pitch_onset_50ms"], 1);
        assert_eq!(m["reference"]["false_positives"], 1);
    }
    #[test]
    fn maximum_matching_recovers_competing_pairs() {
        let m = metrics(&[n(0.04, 60), n(0., 60)], Some(&[n(0., 60), n(0.08, 60)]));
        assert_eq!(m["reference"]["matched_exact_pitch_onset_50ms"], 2);
    }
    #[test]
    fn octaves_and_large_offsets_do_not_get_credit() {
        let m = metrics(&[n(0., 72), n(0.06, 60)], Some(&[n(0., 60)]));
        assert_eq!(m["reference"]["matched_exact_pitch_onset_50ms"], 0);
    }
}
