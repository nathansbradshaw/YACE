//! Replay source-bound cached recognition/model evidence to attribute vocal losses.
//! Audio, lyrics and reports stay in the caller's private output directory.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment,
    clean_recognition::Recognition,
    editor::Store,
    stem_review::StemId,
    vocal_generation::{self, Generation},
};
use clap::Parser;
use serde_json::json;
use std::{fs, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    project: PathBuf,
    #[arg(long)]
    generation: PathBuf,
    #[arg(long)]
    recognition: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
fn metrics(report: &Generation, fallback: &chart_engine::timing::TempoMap) -> serde_json::Value {
    let timing = report.timing.as_ref().unwrap_or(fallback);
    let timed: Vec<_> = report
        .words
        .iter()
        .flat_map(|words| &words.words)
        .filter(|word| word.start.zip(word.end).is_some())
        .collect();
    let with_pitch = timed
        .iter()
        .filter(|word| {
            report.pitch_candidates.iter().any(|span| {
                word.end.unwrap().min(span.end_seconds)
                    - word.start.unwrap().max(span.start_seconds)
                    >= 0.030
            })
        })
        .count();
    json!({
        "timed_words":report.words.as_ref().map(|words|words.words.iter().filter(|w|w.start.zip(w.end).is_some()).count()),
        "total_words":report.words.as_ref().map(|words|words.words.len()),
        "voiced_frames":report.pitch_frames.iter().filter(|f|f.midi.is_some()).count(),
        "timed_words_intersecting_pitch":with_pitch,
        "timed_words_without_pitch_intersection":timed.len()-with_pitch,
        "frames_missing_both_dsp_estimates":report.pitch_frames.iter().filter(|frame|frame.yin_midi.is_none() && frame.mcleod_midi.is_none()).count(),
        "frames_yin_only_still_rejected":report.pitch_frames.iter().filter(|frame|frame.midi.is_none() && frame.yin_midi.is_some() && frame.mcleod_midi.is_none()).count(),
        "frames_dsp_disagreement_still_rejected":report.pitch_frames.iter().filter(|frame|frame.midi.is_none() && frame.yin_midi.zip(frame.mcleod_midi).is_some_and(|(yin,mcleod)|(yin-mcleod).abs()>0.55)).count(),
        "pitch_candidates":report.pitch_candidates.len(),
        "candidate_seconds":report.pitch_candidates.iter().map(|p|p.end_seconds-p.start_seconds).sum::<f64>(),
        "notes":report.chart.as_ref().map(|chart|chart.notes.len()),
        "notes_with_missing_lyrics":report.chart.as_ref().map(|chart|chart.notes.iter().filter(|note|note.lyric==chart_engine::vocals::Syllable::Missing).count()),
        "note_seconds":report.chart.as_ref().map(|chart|chart.notes.iter().map(|note|timing.seconds(note.end)-timing.seconds(note.start)).sum::<f64>()),
        "phrases":report.chart.as_ref().map(|chart|chart.phrases.len()),
        "issues":report.issues.iter().fold(std::collections::BTreeMap::<String,usize>::new(),|mut counts,issue|{*counts.entry(format!("{:?}",issue.reason)).or_default()+=1;counts}),
    })
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "Output must be new");
    let original: Generation = serde_json::from_slice(&fs::read(&args.generation)?)?;
    let recognition: Recognition = serde_json::from_slice(&fs::read(&args.recognition)?)?;
    let store = Store::new(&args.project);
    let doc = store.load()?;
    let audio = store.audio_path(&doc)?;
    let timing = doc.timing(&store)?;
    ensure!(
        original.audio_sha256 == doc.audio_sha256,
        "Cached generation belongs to another recording"
    );
    if let Some(original_timing) = &original.timing {
        ensure!(
            serde_json::to_value(original_timing)? == serde_json::to_value(&timing)?,
            "Cached generation uses another tempo map"
        );
    }
    let stem = doc
        .content
        .stems
        .as_ref()
        .context("No stems")?
        .stems
        .iter()
        .find(|s| s.id == StemId::Vocals)
        .context("No vocal stem")?;
    ensure!(
        recognition.recognition_audio_sha256 == stem.sha256,
        "Recognition belongs to another stem"
    );
    let alignment = StemAlignment {
        audio_sha256: original.audio_sha256.clone(),
        stem_sha256: original.vocal_stem_sha256.clone(),
        evidence: "Replay of cached source-bound generation".into(),
    };
    let stage = tempfile::tempdir_in(args.output.parent().context("Output needs a parent")?)?;
    let mut summary = json!({"baseline":metrics(&original,&timing),"variants":{},"audio_sha256":original.audio_sha256,"vocal_stem_sha256":original.vocal_stem_sha256,"interpretation":"Evidence retention and coverage only; increased counts do not establish correct pitches or lyrics."});
    for (label, words, lyrics) in [
        (
            "cached_alignment",
            original
                .words
                .as_ref()
                .context("Cached report has no words")?,
            None,
        ),
        (
            "cached_alignment_with_lyrics",
            original
                .words
                .as_ref()
                .context("Cached report has no words")?,
            original.lyrics.as_ref(),
        ),
        (
            "recognized_with_lyrics",
            &recognition.words,
            original.lyrics.as_ref(),
        ),
        ("recognized_only", &recognition.words, None),
    ] {
        eprintln!("Replaying {label}");
        let report = vocal_generation::generate_cancellable(
            vocal_generation::Request {
                audio: &audio,
                vocal_stem: &stem.path,
                alignment: &alignment,
                timing: &timing,
                words: Some(words),
                lyrics,
                policy: &original.policy,
                neural_pitch: original.neural_pitch.as_ref(),
                vocal_pitch: original.vocal_pitch.as_ref(),
            },
            &|| false,
        )?;
        summary["variants"][label] = metrics(&report, &timing);
        fs::write(
            stage.path().join(format!("{label}.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        if let Some(chart) = report.chart {
            fs::write(
                stage.path().join(format!("{label}-vocals.json")),
                serde_json::to_vec_pretty(&chart)?,
            )?;
        }
    }
    fs::write(
        stage.path().join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    chart_engine::pipeline::publish(stage.path(), &args.output)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
