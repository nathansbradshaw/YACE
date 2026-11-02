//! Explicit local comparisons. Never changes production model defaults.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::{CleanMode, CleanPolicy, StemAlignment},
    clean_recognition::{self, Control},
    lyric_language::{Language, RecognitionProfile},
    models,
    vocal_generation::{self, AnalysisMode, LyricSource, Policy},
};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
#[path = "../vocal_separation.rs"]
mod vocal_separation;
#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Attach a source-bound draft only to a project with no authored vocals.
    Attach {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        generation: PathBuf,
    },
    /// Compare raw neural F0 to the same reference; does not generate chart notes.
    CompareProbe {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Cut only after decoding to the declared 44.1kHz source clock.
    Excerpt {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        start_frame: usize,
        #[arg(long)]
        end_frame: usize,
        #[arg(long)]
        output: PathBuf,
    },
    /// Analyze a frozen six-song selection before reading target pitches.
    PairBatch {
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long)]
        baseline_binary: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
    /// Experimental second-stage lead/backing proposals, never singer identities.
    Separate {
        #[arg(long)]
        vocal_stem: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Preserve source time while padding a shorter game vocal channel's tail.
    PadStem {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        stem: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Evaluator only: convert authored PART VOCALS to chart-relative references.
    /// Never pass this output or MIDI timestamps to pitch/recognition commands.
    Reference {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        midi: PathBuf,
        #[arg(long, default_value_t = 0.)]
        ini_delay_ms: f64,
        #[arg(long)]
        output: PathBuf,
    },
    /// Compare small/large-v3/turbo; supply original audio or an aligned stem.
    Recognize {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        vocal_stem: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "auto")]
        language: Language,
        #[arg(long, value_enum, default_value = "small")]
        profile: RecognitionProfile,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        download_model: bool,
        #[arg(long)]
        phrase_segments: bool,
        #[arg(long)]
        output: PathBuf,
    },
    /// Native Spanish wav2vec2-CTC research; no English alphabet substitution.
    SpanishAlignment {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        vocal_stem: PathBuf,
        #[arg(long)]
        lyrics: PathBuf,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Model-free paired acoustic analysis on identical, explicitly aligned audio.
    Pitch {
        #[arg(long)]
        audio: PathBuf,
        #[arg(long)]
        vocal_stem: PathBuf,
        #[arg(long, default_value_t = 120.)]
        bpm: f64,
        #[arg(long)]
        output: PathBuf,
    },
}
fn write(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn binding(audio: &Path, stem: &Path) -> Result<StemAlignment> {
    let source = models::decode(audio, 44100, 2)?;
    let voice = models::decode(stem, 44100, 2)?;
    ensure!(
        !source.is_empty() && source.len() == voice.len(),
        "Research source and stem must have the identical full recording clock"
    );
    Ok(StemAlignment{audio_sha256:models::hash_file(audio)?,stem_sha256:models::hash_file(stem)?,evidence:"Explicit research input pair; equal decoded clocks checked; alignment assertion is caller-owned, not inferred from duration".into()})
}
fn main() -> Result<()> {
    match Args::parse().command {
        Command::Attach {
            project,
            generation,
        } => {
            let report: vocal_generation::Generation =
                serde_json::from_slice(&fs::read(generation)?)?;
            let store = chart_engine::editor::Store::new(&project);
            let doc = store.load()?;
            ensure!(
                doc.content.vocals.is_none(),
                "Research import refuses to replace existing authored vocals; use explicit editor review"
            );
            let document = report.chart.clone().context("No generated vocal chart")?;
            let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&report)?;
            review.validate_binding(&doc)?;
            review.validate_timing(&doc.timing(&store)?, true)?;
            store.apply(chart_engine::editor::Request {
                expected_revision: doc.revision,
                actor: chart_engine::editor::Actor {
                    kind: chart_engine::editor::ActorKind::Ai,
                    name: "Explicit research draft import; listening review required".into(),
                },
                action: chart_engine::editor::Action::SetVocalGeneration { document, review },
            })?;
        }
        Command::CompareProbe {
            reference,
            baseline,
            evidence,
            output,
        } => {
            let reference: chart_engine::vocal_benchmark::Reference =
                serde_json::from_slice(&fs::read(reference)?)?;
            let baseline: vocal_generation::Generation =
                serde_json::from_slice(&fs::read(baseline)?)?;
            let probe: serde_json::Value = serde_json::from_slice(&fs::read(&evidence)?)?;
            ensure!(
                probe["source"]["sha256"].as_str() == Some(baseline.vocal_stem_sha256.as_str()),
                "Probe input differs from the source-bound baseline vocal stem"
            );
            let source_path = probe["source"]["path"]
                .as_str()
                .context("Probe source path missing")?;
            ensure!(
                models::hash_file(Path::new(source_path))? == baseline.vocal_stem_sha256,
                "Probe source changed"
            );
            let model = probe["model_id"].as_str().context("Probe model missing")?;
            let (expected, threshold) = match model {
                "crepe-full-onnx-v1.1.0" => (
                    "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610",
                    0.5,
                ),
                "crepe-tiny-onnx-v1.1.0" => (
                    "91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3",
                    0.5,
                ),
                "rmvpe-rvc-onnx-experimental" => (
                    "5370e71ac80af8b4b7c793d27efd51fd8bf962de3a7ede0766dac0befa3660fd",
                    0.03,
                ),
                _ => anyhow::bail!("Unknown pinned probe model"),
            };
            ensure!(
                probe["model_sha256"].as_str() == Some(expected),
                "Probe model identity differs"
            );
            let mut frame_view = baseline.clone();
            frame_view.duration_seconds = probe["duration_seconds"]
                .as_f64()
                .context("Probe clock missing")?;
            frame_view.chart = None;
            frame_view.words = None;
            frame_view.note_evidence.clear();
            frame_view.pitch_frames = probe["frames"]
                .as_array()
                .context("Probe frames missing")?
                .iter()
                .map(|frame| {
                    let time = frame["time_seconds"]
                        .as_f64()
                        .context("Probe frame clock missing")?;
                    let frequency = frame["frequency_hz"]
                        .as_f64()
                        .context("Probe frequency missing")?;
                    let confidence = frame["confidence"]
                        .as_f64()
                        .context("Probe salience missing")?;
                    ensure!(
                        (0.0..=1.).contains(&confidence)
                            && frequency.is_finite()
                            && frequency >= 0.,
                        "Invalid probe values"
                    );
                    let midi = (frequency > 0. && confidence > threshold)
                        .then(|| 69. + 12. * (frequency / 440.).log2());
                    Ok(vocal_generation::PitchFrame {
                        time_seconds: time,
                        midi,
                        yin_midi: None,
                        mcleod_midi: None,
                        clarity: confidence,
                        rms: frame["local_rms"].as_f64().unwrap_or(0.),
                        local_rms: None,
                        local_energy_active: None,
                        support: vocal_generation::PitchSupport::Unspecified,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            write(
                &output,
                &json!({"model_id":model,"model_sha256":expected,"probe_evidence_sha256":models::hash_file(&evidence)?,"voicing_threshold":threshold,"reference_kind":reference.reference_kind,"baseline":chart_engine::vocal_benchmark::score(&reference,&baseline)?,"candidate":chart_engine::vocal_benchmark::score(&reference,&frame_view)?,"candidate_chart_formation_evaluated":false,"promotion_eligible":false,"meaning":"Raw neural F0 comparison only; no lyric, note-formation or voice-identity approval"}),
            )?;
        }
        Command::Excerpt {
            audio,
            start_frame,
            end_frame,
            output,
        } => {
            ensure!(
                !output.exists() && !output.with_extension("excerpt.json").exists(),
                "Excerpt output already exists"
            );
            let source_hash = models::hash_file(&audio)?;
            let pcm = models::decode(&audio, 44100, 2)?;
            ensure!(
                start_frame < end_frame && end_frame <= pcm.len() / 2,
                "Invalid excerpt on decoded 44.1kHz source clock"
            );
            let excerpt = &pcm[start_frame * 2..end_frame * 2];
            let left: Vec<_> = excerpt.chunks_exact(2).map(|s| s[0]).collect();
            let right: Vec<_> = excerpt.chunks_exact(2).map(|s| s[1]).collect();
            ensure!(
                models::hash_file(&audio)? == source_hash,
                "Source changed during excerpt preparation"
            );
            models::write_audio(&output, &left, &right)?;
            write(
                &output.with_extension("excerpt.json"),
                &json!({"source_sha256":source_hash,"output_sha256":models::hash_file(&output)?,"sample_rate":44100,"start_source_frame":start_frame,"end_source_frame":end_frame,"output_frames":end_frame-start_frame,"time_mapping":"excerpt seconds + start_source_frame/44100 = original source seconds; no inferred chart offset"}),
            )?;
        }
        Command::PairBatch {
            selection,
            prepared,
            baseline_binary,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let frozen: serde_json::Value = serde_json::from_slice(&fs::read(&selection)?)?;
            let songs = frozen["songs"]
                .as_array()
                .context("Missing frozen selection")?;
            ensure!(songs.len() == 6, "Expected frozen six-song selection");
            fs::create_dir_all(&output)?;
            // Cargo may replace its binary while other checks run. Every
            // child in this batch uses one frozen executable instead.
            let executable = output.join("frozen-analysis-runner");
            fs::copy(std::env::current_exe()?, &executable)?;
            let executable_hash = models::hash_file(&executable)?;
            let run = |args: Vec<std::ffi::OsString>| -> Result<()> {
                ensure!(
                    std::process::Command::new(&executable)
                        .args(args)
                        .status()?
                        .success(),
                    "Research batch stage failed; retain incomplete output and logs"
                );
                Ok(())
            };
            for (index, song) in songs.iter().enumerate() {
                let case = prepared.join(format!("case-{:02}", index + 1));
                let destination = output.join(format!("case-{:02}", index + 1));
                fs::create_dir(&destination)?;
                let receipt: serde_json::Value =
                    serde_json::from_slice(&fs::read(case.join("reference-source.json"))?)?;
                ensure!(
                    receipt["source_archive_sha256"] == song["archive_sha256"],
                    "Prepared archive differs from frozen selection"
                );
                let audio = case.join("source.wav");
                let stem = destination.join("vocals.wav");
                run(vec![
                    "pad-stem".into(),
                    "--audio".into(),
                    audio.clone().into_os_string(),
                    "--stem".into(),
                    case.join("vocals.opus").into_os_string(),
                    "--output".into(),
                    stem.clone().into_os_string(),
                ])?;
                if let Some(binary) = &baseline_binary {
                    ensure!(
                        std::process::Command::new(binary)
                            .arg(&audio)
                            .arg(&stem)
                            .arg(destination.join("baseline.json"))
                            .status()?
                            .success(),
                        "Frozen baseline failed"
                    );
                }
                run(vec![
                    "pitch".into(),
                    "--audio".into(),
                    audio.into_os_string(),
                    "--vocal-stem".into(),
                    stem.into_os_string(),
                    "--output".into(),
                    destination.join("paired").into_os_string(),
                ])?;
            }
            // Target note values are read only after every inference has finished.
            let mut recordings = Vec::new();
            for (index, song) in songs.iter().enumerate() {
                let case = prepared.join(format!("case-{:02}", index + 1));
                let destination = output.join(format!("case-{:02}", index + 1));
                let audio = case.join("source.wav");
                let reference = destination.join("reference.json");
                let delay = song["metadata"]["delay"].as_str().unwrap_or("0");
                run(vec![
                    "reference".into(),
                    "--audio".into(),
                    audio.clone().into_os_string(),
                    "--midi".into(),
                    case.join("notes.mid").into_os_string(),
                    "--ini-delay-ms".into(),
                    delay.into(),
                    "--output".into(),
                    reference.clone().into_os_string(),
                ])?;
                recordings.push(chart_engine::vocal_benchmark::Recording {
                    id: format!("pitch-{:02}", index + 1),
                    artist: song["metadata"]["artist"]
                        .as_str()
                        .context("Missing artist")?
                        .into(),
                    language: song["language"].as_str().unwrap_or("und").into(),
                    split: song["split"].as_str().context("Missing split")?.into(),
                    categories: vec![],
                    audio: audio.clone(),
                    audio_sha256: models::hash_file(&audio)?,
                    reference: Some(reference),
                    baseline: Some(if baseline_binary.is_some() {
                        destination.join("baseline.json")
                    } else {
                        destination.join("paired/standard.json")
                    }),
                    candidate: Some(destination.join("paired/high_accuracy.json")),
                });
            }
            let corpus = output.join("diagnostic-corpus.json");
            write(
                &corpus,
                &chart_engine::vocal_benchmark::Corpus {
                    version: 1,
                    recordings,
                },
            )?;
            write(
                &output.join("evaluation.json"),
                &chart_engine::vocal_benchmark::evaluate_partial(&corpus)?,
            )?;
            ensure!(
                models::hash_file(&executable)? == executable_hash,
                "Frozen analysis runner changed"
            );
            write(
                &output.join("run.json"),
                &json!({"selection_sha256":models::hash_file(&selection)?,"algorithm":vocal_generation::ALGORITHM,"baseline_binary_sha256":baseline_binary.as_ref().map(|p|models::hash_file(p)).transpose()?,"policy_sha256":executable_hash,"target_note_values_used_in_generation":false,"promotion_eligible":false}),
            )?;
        }
        Command::Separate {
            vocal_stem,
            model,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let timer = Instant::now();
            let source_hash = models::hash_file(&vocal_stem)?;
            let pcm = models::decode(&vocal_stem, 44100, 2)?;
            let (lead, backing) = vocal_separation::separate(&pcm, &model)?;
            ensure!(
                models::hash_file(&vocal_stem)? == source_hash,
                "Vocal source changed"
            );
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::create_dir_all(parent)?;
            let stage = tempfile::tempdir_in(parent)?;
            for (name, pcm) in [("lead.wav", &lead), ("backing.wav", &backing)] {
                let left: Vec<_> = pcm.chunks_exact(2).map(|s| s[0]).collect();
                let right: Vec<_> = pcm.chunks_exact(2).map(|s| s[1]).collect();
                models::write_audio(&stage.path().join(name), &left, &right)?;
            }
            write(
                &stage.path().join("separation.json"),
                &json!({"status":"experimental_lead_backing_proposals_not_singer_identity","vocal_stem_sha256":source_hash,"model_sha256":vocal_separation::MODEL_SHA256,"upstream":"https://github.com/Anjok07/ultimatevocalremovergui/tree/a5f88453bfb2b38b05a965bcf67727243e0cbf19","lead_sha256":models::hash_file(&stage.path().join("lead.wav"))?,"backing_sha256":models::hash_file(&stage.path().join("backing.wav"))?,"frames":pcm.len()/2,"sample_rate":44100,"channels":2,"preprocessing":"whole-stem peak normalization; centered reflect periodic-Hann STFT5120/hop1024; 2048 bins; bins0..2 zero; channel order Lre,Lim,Rre,Rim; 256-frame chunks, trim2560 samples on both sides; zero pad source edges; no denoise; compensated backing=model*1.065; lead=source-backing; no retiming","elapsed_seconds":timer.elapsed().as_secs_f64(),"peak_rss_bytes":peak_rss()}),
            )?;
            ensure!(!output.exists(), "Output appeared during inference");
            fs::rename(stage.path(), output)?;
        }
        Command::PadStem {
            audio,
            stem,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let source = models::decode(&audio, 44100, 2)?;
            let mut voice = models::decode(&stem, 44100, 2)?;
            ensure!(
                !source.is_empty() && !voice.is_empty() && voice.len() <= source.len(),
                "Vocal channel must fit the full source clock"
            );
            let original_frames = voice.len() / 2;
            voice.resize(source.len(), 0.);
            let left: Vec<_> = voice.chunks_exact(2).map(|s| s[0]).collect();
            let right: Vec<_> = voice.chunks_exact(2).map(|s| s[1]).collect();
            models::write_audio(&output, &left, &right)?;
            write(
                &output.with_extension("binding.json"),
                &json!({"source_audio_sha256":models::hash_file(&audio)?,"original_stem_sha256":models::hash_file(&stem)?,"output_sha256":models::hash_file(&output)?,"original_frames":original_frames,"source_frames":source.len()/2,"sample_rate":44100,"transformation":"source-frame zero preserved; silence padding only at tail; no normalization or retiming"}),
            )?;
        }
        Command::Reference {
            audio,
            midi,
            ini_delay_ms,
            output,
        } => {
            use chart_engine::{
                benchmark::{self, Instrument},
                vocal_benchmark::{Reference, ReferenceFrame, ReferenceNote},
            };
            let bytes = fs::read(&midi)?;
            let authored = benchmark::parse(&bytes, Instrument::Vocals, ini_delay_ms)?;
            let duration = models::decode(&audio, 16000, 1)?.len() as f64 / 16000.;
            let notes: Vec<_> = authored
                .notes
                .iter()
                .filter_map(|note| {
                    note.pitch.map(|pitch| ReferenceNote {
                        start_seconds: note.start,
                        end_seconds: note.end,
                        midi: pitch,
                    })
                })
                .collect();
            ensure!(
                !notes.is_empty()
                    && notes
                        .iter()
                        .all(|n| n.start_seconds >= 0. && n.end_seconds <= duration),
                "Authored notes must fit the verified source clock and INI delay"
            );
            let frames = (0..(duration * 100.).ceil() as usize)
                .map(|index| {
                    let time = index as f64 * 0.010;
                    let pitch = notes
                        .iter()
                        .find(|n| n.start_seconds <= time && time < n.end_seconds)
                        .map(|n| n.midi as f64);
                    ReferenceFrame {
                        time_seconds: time,
                        midi: pitch,
                        backing_midi: vec![],
                    }
                })
                .collect();
            let reference = Reference {
                reference_kind: "authored_chart".into(),
                audio_sha256: models::hash_file(&audio)?,
                duration_seconds: duration,
                provenance: format!(
                    "User-authorized professional YARG PART VOCALS; MIDI sha256={}; INI delay={ini_delay_ms}ms; chart-relative target pitches, not measured performed F0. Lyrics and singer identity require separate annotations.",
                    models::hash_file(&midi)?
                ),
                reviewed: true,
                frames,
                notes,
                words: vec![],
            };
            write(&output, &reference)?;
        }
        Command::Pitch {
            audio,
            vocal_stem,
            bpm,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let binding = binding(&audio, &vocal_stem)?;
            fs::create_dir_all(&output)?;
            let timing = chart_engine::timing::TempoMap::constant(bpm)?;
            for (name, mode) in [
                ("standard", AnalysisMode::Standard),
                ("high_accuracy", AnalysisMode::HighAccuracy),
            ] {
                let timer = Instant::now();
                let report = vocal_generation::generate(vocal_generation::Request {
                    audio: &audio,
                    vocal_stem: &vocal_stem,
                    alignment: &binding,
                    timing: &timing,
                    words: None,
                    lyrics: None,
                    policy: &Policy {
                        analysis_mode: mode,
                        ..Default::default()
                    },
                    neural_pitch: None,
                    vocal_pitch: None,
                    rmvpe_pitch: None,
                })?;
                write(&output.join(format!("{name}.json")), &report)?;
                write(
                    &output.join(format!("{name}-runtime.json")),
                    &json!({"elapsed_seconds":timer.elapsed().as_secs_f64(),"peak_rss_bytes":peak_rss(),"scope":"process high-water RSS; includes earlier stages in this process", "reference_notes_used":false}),
                )?;
            }
        }
        Command::Recognize {
            audio,
            vocal_stem,
            language,
            profile,
            model_dir,
            download_model,
            phrase_segments,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let timer = Instant::now();
            let identity = vocal_stem
                .as_ref()
                .map(|stem| binding(&audio, stem))
                .transpose()?;
            let control = Arc::new(Control::default());
            let mut receipts = vec![];
            let mut recognition = if !phrase_segments {
                clean_recognition::detect_profile(
                    clean_recognition::Request {
                        audio: &audio,
                        model_dir: &model_dir,
                        download_model,
                        vocal_stem: vocal_stem
                            .as_ref()
                            .zip(identity.as_ref())
                            .map(|(s, b)| (s.as_path(), b)),
                    },
                    &CleanPolicy::default(),
                    CleanMode::FullMix,
                    control.clone(),
                    language,
                    profile,
                )?
                .recognition
            } else {
                let input = vocal_stem.as_ref().unwrap_or(&audio);
                let samples = models::decode(input, 44100, 2)?;
                let regions = activity_regions(&samples);
                ensure!(
                    !regions.is_empty(),
                    "No active regions for segmented recognition"
                );
                let stage = tempfile::tempdir()?;
                let mut merged = None::<clean_recognition::Recognition>;
                for (index, (start, end)) in regions.into_iter().enumerate() {
                    let clip = stage.path().join(format!("region-{index}.wav"));
                    let left: Vec<_> = (start..end).map(|i| samples[i * 2]).collect();
                    let right: Vec<_> = (start..end).map(|i| samples[i * 2 + 1]).collect();
                    models::write_audio(&clip, &left, &right)?;
                    let mut r = clean_recognition::detect_profile(
                        clean_recognition::Request {
                            audio: &clip,
                            model_dir: &model_dir,
                            download_model,
                            vocal_stem: None,
                        },
                        &CleanPolicy::default(),
                        CleanMode::FullMix,
                        control.clone(),
                        language,
                        profile,
                    )?
                    .recognition;
                    let offset = start as f64 / 44100.;
                    for word in &mut r.words.words {
                        word.start = word.start.map(|v| v + offset);
                        word.end = word.end.map(|v| v + offset);
                    }
                    for segment in &mut r.segments {
                        if segment.start_seconds >= 0. {
                            segment.start_seconds += offset;
                            segment.end_seconds += offset;
                        }
                    }
                    receipts.push(json!({"start_source_frame":start,"end_source_frame":end,"sample_rate":44100,"chunk_sha256":models::hash_file(&clip)?,"detected_language":r.language}));
                    if let Some(merged) = &mut merged {
                        merged.words.words.extend(r.words.words);
                        merged.segments.extend(r.segments);
                    } else {
                        merged = Some(r);
                    }
                }
                let mut r = merged.context("No recognition results")?;
                r.words.audio_sha256 = models::hash_file(&audio)?;
                r.words.duration_seconds = samples.len() as f64 / 88200.;
                r.recognition_audio_sha256 = models::hash_file(input)?;
                r.words.source = format!(
                    "Research activity segmentation with exact source-frame rebasing; profile={profile:?}; requested_language={language:?}; multiple segment languages retained in receipt"
                );
                r.words.validate()?;
                r
            };
            let source_hash = models::hash_file(&audio)?;
            if let Some(identity) = identity {
                ensure!(
                    source_hash == identity.audio_sha256
                        && models::hash_file(vocal_stem.as_ref().unwrap())? == identity.stem_sha256,
                    "Research inputs changed"
                );
            }
            recognition.warnings.push("Research comparison; no production backend promotion or musical acceptance implied".into());
            write(
                &output,
                &json!({"source_audio_sha256":source_hash,"profile":profile,"phrase_segments":phrase_segments,"regions":receipts,"elapsed_seconds":timer.elapsed().as_secs_f64(),"peak_rss_bytes":peak_rss(),"recognition":recognition}),
            )?;
        }
        Command::SpanishAlignment {
            audio,
            vocal_stem,
            lyrics,
            model_dir,
            output,
        } => {
            ensure!(!output.exists(), "Output already exists");
            let timer = Instant::now();
            let identity = binding(&audio, &vocal_stem)?;
            ensure!(
                models::hash_file(&model_dir.join("pytorch_model.bin"))?
                    == "41c110e55d2eac8c79486ad87dbe8f9527ed034fe087a6adf03c891eeba914c1",
                "Pinned Spanish XLSR weights differ"
            );
            let config_hash = models::hash_file(&model_dir.join("config.json"))?;
            let vocab_hash = models::hash_file(&model_dir.join("vocab.json"))?;
            ensure!(
                config_hash == "39ffe954848d5161270eef4378a17a538bffeefc310482f409ec099f7fceaa5b"
                    && vocab_hash
                        == "7aea8bcd8ba1c176f12cc96c1e8f45247548f4de3218106f7352e3d833cfafdc",
                "Pinned Spanish model configuration/vocabulary differ"
            );
            let vocabulary: HashMap<String, usize> =
                serde_json::from_slice(&fs::read(model_dir.join("vocab.json"))?)?;
            ensure!(
                vocabulary.contains_key("ñ") || vocabulary.contains_key("Ñ"),
                "Spanish vocabulary must retain ñ"
            );
            let samples = models::decode(&vocal_stem, 16000, 1)?;
            let model = practice_acoustic::AcousticModel::load_pytorch(&model_dir)
                .map_err(|e| anyhow::anyhow!("Native Spanish model: {e}"))?;
            let emissions = model
                .emissions(&samples)
                .map_err(|e| anyhow::anyhow!("Spanish emissions: {e}"))?;
            let lyric_source = LyricSource {
                text: fs::read_to_string(&lyrics)?,
                source: format!("User supplied text {}", lyrics.display()),
                synced_lyrics: None,
            };
            let (mut words, diagnostics) = chart_engine::vocal_alignment::align_emissions(
                &emissions,
                &vocabulary,
                &lyric_source,
                &identity.audio_sha256,
                samples.len() as f64 / 16000.,
                &|| false,
            )?;
            words.source="Experimental native Spanish XLSR CTC alignment; vocabulary-preserving Unicode; unsupported words remain untimed; speech-trained model, singing acceptance pending".into();
            ensure!(
                models::hash_file(&model_dir.join("config.json"))? == config_hash
                    && models::hash_file(&model_dir.join("vocab.json"))? == vocab_hash
                    && models::hash_file(&model_dir.join("pytorch_model.bin"))?
                        == "41c110e55d2eac8c79486ad87dbe8f9527ed034fe087a6adf03c891eeba914c1",
                "Spanish model changed during alignment"
            );
            ensure!(
                models::hash_file(&audio)? == identity.audio_sha256
                    && models::hash_file(&vocal_stem)? == identity.stem_sha256,
                "Research sources changed"
            );
            write(
                &output,
                &json!({"language":"es","model_revision":"96d7e9b4e4a78af515a3c6d3cee7c0826045d276","model_sha256":"41c110e55d2eac8c79486ad87dbe8f9527ed034fe087a6adf03c891eeba914c1","config_sha256":config_hash,"vocab_sha256":vocab_hash,"elapsed_seconds":timer.elapsed().as_secs_f64(),"peak_rss_bytes":peak_rss(),"words":words,"diagnostics":diagnostics,"status":"research_only"}),
            )?;
        }
    }
    Ok(())
}
fn peak_rss() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the struct on success; no read on failure.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(usage.ru_maxrss as u64 * if cfg!(target_os = "macos") { 1 } else { 1024 })
}
fn activity_regions(stereo: &[f32]) -> Vec<(usize, usize)> {
    let frames = stereo.len() / 2;
    let hop = 882;
    let energies: Vec<_> = stereo
        .chunks(hop * 2)
        .map(|chunk| {
            (chunk.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / chunk.len() as f64).sqrt()
        })
        .collect();
    let gate = energies.iter().copied().fold(0., f64::max) * 0.02;
    let mut regions = Vec::new();
    let mut begin = None;
    let mut last = 0;
    for (index, &energy) in energies.iter().enumerate() {
        if energy >= gate.max(0.0015) {
            if begin.is_none() {
                begin = Some(
                    index.saturating_sub(10).max(
                        regions
                            .last()
                            .map(|r: &(usize, usize)| r.1.div_ceil(hop))
                            .unwrap_or(0),
                    ),
                );
            }
            last = index;
        }
        if let Some(start) = begin
            && (index - last >= 20 || index - start >= 1400 || index + 1 == energies.len())
        {
            let end = if index - start >= 1400 {
                index
            } else {
                (last + 11).min(energies.len())
            };
            if end > start {
                regions.push((start * hop, (end * hop).min(frames)));
            }
            begin = None;
        }
    }
    regions
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_and_long_activity_are_bounded_without_overlap() {
        assert!(activity_regions(&vec![0.; 88200]).is_empty());
        let regions = activity_regions(&vec![0.2; 88200 * 60]);
        assert!(regions.len() >= 2);
        assert!(regions.windows(2).all(|w| w[0].1 <= w[1].0));
        assert!(regions.iter().all(|&(a, b)| a < b && b - a <= 44100 * 29));
    }
}
