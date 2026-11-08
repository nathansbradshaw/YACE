//! Evaluate learned acoustic note events against private authored game charts.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models,
    vocal_benchmark::{Corpus, Reference, score},
    vocal_generation::{Generation, PitchSupport},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use clap::Parser;
use serde::Deserialize;
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};
#[derive(Parser)]
struct Args {
    corpus: PathBuf,
    output: PathBuf,
    probe: PathBuf,
    model: PathBuf,
    word_model: PathBuf,
    #[arg(long)]
    held_out: bool,
    #[arg(long)]
    swift_model: Option<PathBuf>,
}
#[derive(Deserialize)]
struct Events {
    stem_sha256: String,
    duration_seconds: f64,
    notes: Vec<Event>,
}
#[derive(Deserialize)]
struct Event {
    start_seconds: f64,
    end_seconds: f64,
    midi: u8,
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new output directory");
    let corpus: Corpus = serde_json::from_slice(&fs::read(&a.corpus)?)?;
    fs::create_dir(&a.output)?;
    fs::write(
        a.output.join("frozen-profile.json"),
        serde_json::to_vec_pretty(
            &json!({"profile":chart_engine::vocal_note_consensus::ALGORITHM,"probe_sha256":models::hash_file(&a.probe)?,"replay_sha256":models::hash_file(&std::env::current_exe()?)?,"corpus_sha256":models::hash_file(&a.corpus)?,"reference_notes_enter_inference":false,"held_out":a.held_out}),
        )?,
    )?;
    let mut predictions = vec![];
    for r in corpus
        .recordings
        .into_iter()
        .filter(|r| (r.split == "held_out") == a.held_out)
    {
        let base = a.corpus.parent().context("Root")?;
        let baseline = base.join(r.candidate.context("High report")?);
        let standard = base.join(r.baseline.context("Standard report")?);
        let stem = standard
            .parent()
            .and_then(|p| p.parent())
            .context("Stem root")?
            .join("vocals.wav");
        let output = a.output.join(format!("{}.notes.json", r.id));
        ensure!(
            Command::new(&a.probe)
                .arg("infer")
                .arg(&stem)
                .arg(&baseline)
                .arg(&a.model)
                .arg(&a.word_model)
                .arg(&output)
                .status()?
                .success(),
            "Note inference failed"
        );
        let events: Events = serde_json::from_slice(&fs::read(&output)?)?;
        let original: Generation = serde_json::from_slice(&fs::read(&baseline)?)?;
        ensure!(
            events.stem_sha256 == original.vocal_stem_sha256
                && models::hash_file(&stem)? == events.stem_sha256
                && (events.duration_seconds - original.duration_seconds).abs() <= 1. / 16000.,
            "Clock/source differs"
        );
        let mut candidate = original.clone();
        let timing = candidate.timing.as_ref().context("Timing")?;
        let mut notes = vec![];
        let mut last_end = 0;
        for e in &events.notes {
            ensure!(
                e.start_seconds.is_finite()
                    && e.end_seconds.is_finite()
                    && e.start_seconds >= 0.
                    && e.start_seconds < e.end_seconds
                    && e.end_seconds <= events.duration_seconds + 0.01,
                "Invalid event"
            );
            if !(36..=84).contains(&e.midi) {
                continue;
            }
            let start = timing
                .tick(e.start_seconds)
                .context("Note starts before chart clock")?
                .max(last_end);
            let end = timing
                .tick(e.end_seconds)
                .context("Note ends before chart clock")?;
            if end > start {
                notes.push(VocalNote {
                    start,
                    end,
                    tone: VocalTone::Pitched { midi: e.midi },
                    lyric: Syllable::Missing,
                    midi: None,
                });
                last_end = end
            }
        }
        let chart = if notes.is_empty() {
            None
        } else {
            let chart = VocalChart {
                version: 1,
                resolution: timing.resolution,
                source: "ROSVOT Rust note-event experiment; no reference input".into(),
                phrases: vec![VocalPhrase {
                    start: notes[0].start,
                    end: notes.last().unwrap().end,
                    player: 1,
                }],
                notes,
            };
            chart.validate()?;
            Some(chart)
        };
        candidate.chart = chart;
        candidate.algorithm = "rosvot-rust-onnx-v1-experimental".into();
        candidate.note_evidence.clear();
        candidate.pitch_candidates.clear();
        candidate.words = None;
        candidate.lyrics = None;
        for f in &mut candidate.pitch_frames {
            f.midi = candidate
                .chart
                .as_ref()
                .and_then(|c| {
                    c.notes.iter().find(|n| {
                        timing.seconds(n.start) <= f.time_seconds
                            && f.time_seconds < timing.seconds(n.end)
                    })
                })
                .map(|n| {
                    if let VocalTone::Pitched { midi } = n.tone {
                        midi as f64
                    } else {
                        unreachable!()
                    }
                });
            f.support = PitchSupport::AcousticAlternative;
            f.clarity = 0.5;
        }
        if let Some(swift_model) = &a.swift_model {
            candidate = original.clone();
            let audio = base.join(&r.audio);
            let alignment = chart_engine::clean::StemAlignment {
                audio_sha256: original.audio_sha256.clone(),
                stem_sha256: original.vocal_stem_sha256.clone(),
                evidence: "Source-bound fixed-profile evaluation; reference events excluded".into(),
            };
            let swift = chart_engine::vocal_pitch::analyze(
                &audio,
                &stem,
                &alignment,
                swift_model,
                &|| false,
            )?;
            fs::write(
                a.output.join(format!("{}.swift.json", r.id)),
                serde_json::to_vec(&swift)?,
            )?;
            let notes = serde_json::from_slice(&fs::read(&output)?)?;
            chart_engine::vocal_note_consensus::refine(
                &mut candidate.pitch_frames,
                candidate.rmvpe_pitch.as_ref().context("RMVPE")?,
                &swift,
                &notes,
                &|| false,
            )?;
            let assembly = chart_engine::vocal_generation::assemble_notes(
                &candidate.pitch_frames,
                candidate.duration_seconds,
                candidate.timing.as_ref().context("Timing")?,
                None,
                &candidate.policy,
                &mut candidate.issues,
            )?;
            candidate.chart = assembly.chart;
            candidate.pitch_candidates = assembly.pitch_candidates;
            candidate.note_evidence = assembly.note_evidence;
            candidate.vocal_pitch = Some(swift);
            candidate.algorithm = chart_engine::vocal_note_consensus::ALGORITHM.into();
        }
        fs::write(
            a.output.join(format!("{}.generation.json", r.id)),
            serde_json::to_vec(&candidate)?,
        )?;
        predictions.push((
            r.id,
            original,
            candidate,
            base.join(r.reference.context("Reference")?),
        ));
    }
    let mut rows = vec![];
    for (id, mut baseline, mut candidate, path) in predictions {
        let reference: Reference = serde_json::from_slice(&fs::read(path)?)?;
        let baseline_raw = score(&reference, &baseline)?;
        let candidate_raw = score(&reference, &candidate)?;
        let ct = candidate.timing.as_ref().context("Timing")?;
        for f in &mut candidate.pitch_frames {
            f.midi = candidate
                .chart
                .as_ref()
                .and_then(|c| {
                    c.notes.iter().find(|n| {
                        ct.seconds(n.start) <= f.time_seconds && f.time_seconds < ct.seconds(n.end)
                    })
                })
                .and_then(|n| match n.tone {
                    VocalTone::Pitched { midi } => Some(midi as f64),
                    _ => None,
                });
        }
        let timing = baseline.timing.as_ref().context("Timing")?;
        for f in &mut baseline.pitch_frames {
            f.midi = baseline
                .chart
                .as_ref()
                .and_then(|c| {
                    c.notes.iter().find(|n| {
                        timing.seconds(n.start) <= f.time_seconds
                            && f.time_seconds < timing.seconds(n.end)
                    })
                })
                .and_then(|n| match n.tone {
                    VocalTone::Pitched { midi } => Some(midi as f64),
                    _ => None,
                });
        }
        rows.push(json!({"id":id,"baseline_raw":baseline_raw,"candidate_raw":candidate_raw,"baseline":score(&reference,&baseline)?,"candidate":score(&reference,&candidate)?}));
    }
    fs::write(
        a.output.join("summary.json"),
        serde_json::to_vec_pretty(
            &json!({"experimental":true,"reference_notes_enter_inference":false,"rows":rows}),
        )?,
    )?;
    Ok(())
}
