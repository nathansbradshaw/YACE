//! Experimental chart/audio feedback pass. Scan only residual sections using
//! smaller RMVPE contexts; preserve original recording time and all raw evidence.
//! This is a diagnostic, never an instruction to add notes or select a singer.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models, rmvpe_frontend, vocal_generation::Generation, vocal_rmvpe, vocals::VocalTone,
};
use clap::Parser;
use ort::session::Session;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Instant};
#[derive(Parser)]
struct Args {
    audio: PathBuf,
    stem: PathBuf,
    generation: PathBuf,
    model: PathBuf,
    output: PathBuf,
    #[arg(long, default_value_t = 6)]
    chunk_seconds: usize,
    /// Evaluator only: loaded after all predictions, never used to find chunks.
    #[arg(long)]
    reference: Option<PathBuf>,
}
fn windows(marked: &[bool], size: usize) -> Vec<(usize, usize)> {
    // Identify residual runs before dividing time, so a short sound straddling
    // two cores is not lost to two independent duration thresholds.
    let mut supported = vec![false; marked.len()];
    let mut i = 0;
    while i < marked.len() {
        if !marked[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < marked.len() && marked[i] {
            i += 1;
        }
        if i - start >= 8 {
            supported[start..i].fill(true);
        }
    }
    (0..marked.len())
        .step_by(size)
        .filter_map(|start| {
            let end = (start + size).min(marked.len());
            // Eight frames of disagreement are a review trigger, not a pitch proof.
            supported[start..end]
                .iter()
                .any(|v| *v)
                .then_some((start, end))
        })
        .collect()
}
fn pitch(frame: &chart_engine::vocal_pitch::Frame) -> Option<f64> {
    frame
        .frequency_hz
        .filter(|_| frame.confidence >= 0.1 && frame.rms >= 0.0015)
        .map(|hz| 69. + 12. * (hz / 440.).log2())
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        (2..=30).contains(&args.chunk_seconds),
        "Choose chunks between 2 and 30 seconds"
    );
    ensure!(!args.output.exists(), "Output already exists");
    let report: Generation = serde_json::from_slice(&fs::read(&args.generation)?)?;
    let original = report
        .rmvpe_pitch
        .as_ref()
        .context("Missing original RMVPE observations")?;
    vocal_rmvpe::validate(
        original,
        &report.audio_sha256,
        &report.vocal_stem_sha256,
        report.duration_seconds,
    )?;
    ensure!(
        models::hash_file(&args.audio)? == report.audio_sha256
            && models::hash_file(&args.stem)? == report.vocal_stem_sha256,
        "Recording or stem binding differs"
    );
    ensure!(
        models::hash_file(&args.model)? == vocal_rmvpe::SHA256,
        "Pinned RMVPE model differs"
    );
    let timing = report.timing.as_ref().context("Missing tempo witness")?;
    let chart = report.chart.as_ref().context("Missing generated chart")?;
    let pcm = models::decode(&args.stem, 16000, 1)?;
    let mut digest = Sha256::new();
    for sample in &pcm {
        digest.update(sample.to_le_bytes());
    }
    ensure!(
        format!("{:x}", digest.finalize()) == original.pcm_sha256
            && original.frames.len() == pcm.len() / 160 + 1,
        "Original PCM or clock differs"
    );
    let mut marked = vec![false; original.frames.len()];
    let mut gap = vec![false; marked.len()];
    let mut cursor = 0;
    for (i, frame) in original.frames.iter().enumerate() {
        while cursor < chart.notes.len()
            && timing.seconds(chart.notes[cursor].end) <= frame.time_seconds
        {
            cursor += 1;
        }
        let note = chart
            .notes
            .get(cursor)
            .filter(|n| timing.seconds(n.start) <= frame.time_seconds)
            .and_then(|n| match n.tone {
                VocalTone::Pitched { midi } => Some(midi as f64),
                _ => None,
            });
        let observation = pitch(frame);
        gap[i] = note.is_none() && observation.is_some();
        marked[i] = match (note, observation) {
            (Some(a), Some(b)) => (a - b).abs() > 0.75,
            (None, None) => false,
            _ => true,
        };
    }
    let selected = windows(&marked, args.chunk_seconds * 100);
    let recipe = json!({"profile":"experimental-rmvpe-residual-chunks-v1", "chunk_seconds":args.chunk_seconds,"context_seconds_each_side":1,"trigger_frames":8,"raw_confidence_floor":0.1,"local_rms_floor":0.0015,"pitch_difference_semitones":0.75,"generation_sha256":models::hash_file(&args.generation)?,"implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("vocal_chunk_trial.rs").as_slice(),include_bytes!("../src/rmvpe_frontend.rs").as_slice(),include_bytes!("../src/vocal_rmvpe.rs").as_slice()].concat()))});
    // No target pitches, lyrics, reference timestamps, or key enter selection.
    let timer = Instant::now();
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .commit_from_file(&args.model)?;
    let mut chunks = vec![];
    let mut rescanned = original.frames.clone();
    for (index, &(start, end)) in selected.iter().enumerate() {
        eprintln!(
            "Rescanning residual section {}/{}",
            index + 1,
            selected.len()
        );
        let values = rmvpe_frontend::infer(&mut session, &pcm, start, end)?;
        let frames = vocal_rmvpe::decode_chunk(&pcm, start, &values)?;
        rescanned[start..end].clone_from_slice(&frames);
        let mut changed = 0;
        let mut lost = 0;
        let mut gained = 0;
        let mut gaps_supported = 0;
        let mut gaps_disputed = 0;
        for (offset, frame) in frames.iter().enumerate() {
            let i = start + offset;
            let a = pitch(&original.frames[i]);
            let b = pitch(frame);
            match (a, b) {
                (Some(a), Some(b)) if (a - b).abs() > 0.75 => changed += 1,
                (Some(_), None) => lost += 1,
                (None, Some(_)) => gained += 1,
                _ => {}
            }
            if gap[i] {
                if a.zip(b).is_some_and(|(a, b)| (a - b).abs() <= 0.75) {
                    gaps_supported += 1;
                } else {
                    gaps_disputed += 1;
                }
            }
        }
        chunks.push(json!({"start_seconds":start as f64 * 0.01,"end_seconds":(end as f64 * 0.01).min(report.duration_seconds),"uncovered_voice_frames":gap[start..end].iter().filter(|v|**v).count(),"gap_frames_agree_after_rescan":gaps_supported,"gap_frames_disputed_after_rescan":gaps_disputed,"changed_pitch_frames":changed,"lost_voiced_frames":lost,"gained_voiced_frames":gained,"frames":frames}));
    }
    ensure!(
        models::hash_file(&args.audio)? == report.audio_sha256
            && models::hash_file(&args.stem)? == report.vocal_stem_sha256,
        "Sources changed during rescan"
    );
    let evaluation = if let Some(path) = &args.reference {
        let reference: chart_engine::vocal_benchmark::Reference =
            serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            reference.reviewed
                && reference.audio_sha256 == report.audio_sha256
                && (reference.duration_seconds - report.duration_seconds).abs() <= 0.01,
            "Reference is not reviewed or source-bound"
        );
        let measure = |frames: &[chart_engine::vocal_pitch::Frame]| {
            let (mut voiced, mut correct, mut wrong, mut missing, mut false_voice) =
                (0usize, 0usize, 0usize, 0usize, 0usize);
            for frame in &reference.frames {
                let index = (frame.time_seconds * 100.).round() as usize;
                let detected = frames.get(index).and_then(pitch);
                match (frame.midi, detected) {
                    (Some(a), Some(b)) => {
                        voiced += 1;
                        if (a - b).abs() <= 0.5 {
                            correct += 1;
                        } else {
                            wrong += 1;
                        }
                    }
                    (Some(_), None) => {
                        voiced += 1;
                        missing += 1;
                    }
                    (None, Some(_)) => false_voice += 1,
                    _ => {}
                }
            }
            json!({"reference_voiced_frames":voiced,"correct_pitch_frames":correct,"wrong_pitch_frames":wrong,"missing_pitch_frames":missing,"false_voiced_frames":false_voice})
        };
        Some(
            json!({"reference_sha256":models::hash_file(path)?,"baseline_raw":measure(&original.frames),"rescanned_raw":measure(&rescanned),"limitation":"Raw contour diagnostic only; no note segmentation or quality-gate promotion."}),
        )
    } else {
        None
    };
    let result = json!({"status":"experimental_review_only", "recipe":recipe,"audio_sha256":report.audio_sha256,"vocal_stem_sha256":report.vocal_stem_sha256,"model_sha256":vocal_rmvpe::SHA256,"pcm_sha256":original.pcm_sha256,"duration_seconds":report.duration_seconds,"runtime_seconds":timer.elapsed().as_secs_f64(),"target_values_read_for_inference":false,"evaluation":evaluation,"limitations":["A repeated model is not independent agreement or correctness proof.","Voice can include backing singers or instrumental leakage; no chart is modified.","Voicing gains and losses require reference evaluation before integration."],"chunks":chunks});
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    serde_json::to_writer_pretty(&mut file, &result)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn residual_windows_are_bounded_on_original_clock_and_keep_tail() {
        let mut flags = vec![false; 1205];
        flags[597..605].fill(true);
        flags[1190..1205].fill(true);
        assert_eq!(
            windows(&flags, 600),
            vec![(0, 600), (600, 1200), (1200, 1205)]
        );
        flags[580..598].fill(true);
        assert_eq!(
            windows(&flags, 600),
            vec![(0, 600), (600, 1200), (1200, 1205)]
        );
        let flags = vec![true; 1210];
        assert_eq!(
            windows(&flags, 600),
            vec![(0, 600), (600, 1200), (1200, 1210)]
        );
    }
}
