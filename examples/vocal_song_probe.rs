//! Compare an existing song's vocal MIDI with lyric-free acoustic generation.
//! Model agreement is diagnostic evidence, never a substitute for an authored reference.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment, midi, models, vocal_generation as vg, vocal_rmvpe, vocals,
};
use clap::Parser;
use serde_json::json;
use std::{fs, path::PathBuf};

#[derive(Parser)]
struct Args {
    song: PathBuf,
    output: PathBuf,
    model: PathBuf,
    /// Reuse pinned, source/PCM-bound raw model evidence without another inference.
    #[arg(long)]
    evidence: Option<PathBuf>,
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        !args.output.exists(),
        "Choose a new diagnostic output directory"
    );
    let audio = args.song.join("song.wav");
    let stem = args.song.join("vocals.wav");
    let bytes = fs::read(args.song.join("notes.mid"))?;
    let timing = midi::timing(&bytes)?;
    let original = vocals::import_midi(&bytes)?;
    let vocal_base = midi::new_midi(&timing, &Default::default())?;
    let alignment = StemAlignment {
        audio_sha256: models::hash_file(&audio)?,
        stem_sha256: models::hash_file(&stem)?,
        evidence:
            "Supplied full-clock song/vocal stem pair; direct acoustic comparison; lyrics unused"
                .into(),
    };
    ensure!(
        models::hash_file(&args.model)? == vocal_rmvpe::SHA256,
        "Wrong model"
    );
    let evidence = if let Some(path) = args.evidence {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        eprintln!("Analyzing RMVPE contour on the supplied vocal stem");
        vocal_rmvpe::analyze(&audio, &stem, &alignment, &args.model, &|| false)?
    };
    fs::create_dir_all(&args.output)?;
    fs::write(
        args.output.join("rmvpe.json"),
        serde_json::to_vec(&evidence)?,
    )?;
    let mut rows = Vec::new();
    for (name, analysis_mode) in [
        ("standard", vg::AnalysisMode::Standard),
        ("high", vg::AnalysisMode::HighAccuracy),
    ] {
        eprintln!("Generating {name}; no words or lyrics enter inference");
        let report = vg::generate(vg::Request {
            audio: &audio,
            vocal_stem: &stem,
            alignment: &alignment,
            timing: &timing,
            words: None,
            lyrics: None,
            policy: &vg::Policy {
                analysis_mode,
                ..Default::default()
            },
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: (analysis_mode == vg::AnalysisMode::HighAccuracy).then_some(&evidence),
        })?;
        fs::write(
            args.output.join(format!("{name}.json")),
            serde_json::to_vec(&report)?,
        )?;
        let chart = report.chart.as_ref().context("No vocal notes detected")?;
        fs::write(
            args.output.join(format!("{name}-vocals.json")),
            serde_json::to_vec_pretty(chart)?,
        )?;
        fs::write(
            args.output.join(format!("{name}-vocals.mid")),
            vocals::append_midi(&vocal_base, chart)?,
        )?;
        let mut histogram = std::collections::BTreeMap::<u8, usize>::new();
        for note in &chart.notes {
            if let vocals::VocalTone::Pitched { midi } = note.tone {
                *histogram.entry(midi).or_default() += 1;
            }
        }
        let intervals: Vec<_> = chart
            .notes
            .iter()
            .map(|n| {
                (
                    timing.seconds(n.start),
                    timing.seconds(n.end),
                    n.tone.clone(),
                )
            })
            .collect();
        let old: Vec<_> = original
            .notes
            .iter()
            .map(|n| {
                (
                    timing.seconds(n.start),
                    timing.seconds(n.end),
                    n.tone.clone(),
                )
            })
            .collect();
        let mut seconds = Vec::new();
        for second in 0..report.duration_seconds.ceil() as usize {
            let frames: Vec<_> = report
                .pitch_frames
                .iter()
                .filter(|f| (second as f64..second as f64 + 1.).contains(&f.time_seconds))
                .collect();
            let raw: Vec<_> = evidence
                .frames
                .iter()
                .filter(|f| {
                    (second as f64..second as f64 + 1.).contains(&f.time_seconds)
                        && f.confidence >= 0.1
                        && f.frequency_hz.is_some()
                })
                .collect();
            let covered = |time, notes: &[(f64, f64, vocals::VocalTone)]| {
                notes.iter().any(|(a, b, _)| *a <= time && time < *b)
            };
            seconds.push(json!({"second":second,"dsp_frames":frames.iter().filter(|f|f.midi.is_some()).count(),"model_frames":raw.len(),"model_frames_without_chart_note":raw.iter().filter(|f|!covered(f.time_seconds,&intervals)).count(),"model_frames_without_original_note":raw.iter().filter(|f|!covered(f.time_seconds,&old)).count(),"detector_agreement_frames":frames.iter().filter(|f|f.yin_midi.zip(f.mcleod_midi).is_some_and(|(a,b)|(a-b).abs()<=0.55)).count()}));
        }
        rows.push(json!({"mode":name,"notes":chart.notes.len(),"phrases":chart.phrases.len(),"voiced_frames":report.pitch_frames.iter().filter(|f|f.midi.is_some()).count(),"charted_seconds":intervals.iter().map(|(a,b,_)|b-a).sum::<f64>(),"pitch_histogram":histogram,"seconds":seconds}));
    }
    let summary = json!({"audio_sha256":alignment.audio_sha256,"stem_sha256":alignment.stem_sha256,"midi_sha256":models::hash_file(&args.song.join("notes.mid"))?,"original_notes":original.notes.len(),"original_charted_seconds":original.notes.iter().map(|n|timing.seconds(n.end)-timing.seconds(n.start)).sum::<f64>(),"algorithm":vg::COMPARISON_ALGORITHM,"reference_accuracy":"not_measured; supplied MIDI is the chart under inspection, model agreement is not ground truth","lyrics_used":false,"rows":rows});
    fs::write(
        args.output.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"output":args.output,"original_notes":original.notes.len(),"modes":summary["rows"].as_array().unwrap().iter().map(|r|json!({"mode":r["mode"],"notes":r["notes"],"voiced_frames":r["voiced_frames"],"charted_seconds":r["charted_seconds"]})).collect::<Vec<_>>()})
        )?
    );
    Ok(())
}
