//! Native singing-note observations, independently of lyrics and YACE's F0 choices.
use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_runtime};
use clap::Parser;
use ort::{session::Session, value::Tensor};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::PathBuf, time::Instant};

const MODEL_SHA: &str = "9d55f9d239e9d70d1bf039506cf1c428b657df7bbb8b94104ca882af6b721e06";
const ARCHIVE_SHA: &str = "b98a350b60740e3a153b15e137ed025832956d918c44853048bec93af657f478";
const RATE: usize = 44_100;
const HOP: usize = 512;

#[derive(Parser)]
#[command(
    about = "Experimental local SOME note-boundary/rest evidence; no lyrics, chart edits, model downloads or quality promotion"
)]
struct Args {
    /// Isolated vocal audio or an original-clock excerpt. Never silently trimmed.
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    model_file: PathBuf,
    #[arg(long)]
    runtime: Option<PathBuf>,
    /// New JSON file; includes rests and continuous model note pitches.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct Interval {
    start_seconds: f64,
    end_seconds: f64,
    unbounded_end_seconds: f64,
    presence: bool,
    /// A model note estimate, not an RMVPE observation or an accepted MIDI note.
    native_midi: f32,
}
fn intervals(
    pitches: &[f32],
    rests: &[bool],
    durations: &[f32],
    duration: f64,
) -> Result<Vec<Interval>> {
    ensure!(
        duration.is_finite() && duration > 0.,
        "Invalid source duration"
    );
    ensure!(
        !pitches.is_empty() && pitches.len() == rests.len() && rests.len() == durations.len(),
        "Native note shape mismatch"
    );
    let tolerance = (HOP + 1) as f64 / RATE as f64;
    let mut at = 0.;
    let mut notes = Vec::with_capacity(pitches.len());
    for ((&pitch, &rest), &length) in pitches.iter().zip(rests).zip(durations) {
        ensure!(
            pitch.is_finite() && length.is_finite() && length >= 0.,
            "Invalid native note value"
        );
        ensure!(
            rest || (0. ..=127.).contains(&pitch),
            "Native pitch outside MIDI; no clamping or octave folding"
        );
        let end = at + length as f64;
        ensure!(
            end <= duration + tolerance,
            "Native note clock extends past recording"
        );
        // Keep every native interval, including zero-length intervals and rests.
        // Only the possible final model frame is bounded to the source duration.
        notes.push(Interval {
            start_seconds: at.min(duration),
            end_seconds: end.min(duration),
            unbounded_end_seconds: end,
            presence: !rest,
            native_midi: pitch,
        });
        at = end;
    }
    ensure!(
        (at - duration).abs() <= tolerance,
        "Native durations do not cover the source clock"
    );
    Ok(notes)
}
fn implementation_hash() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("main.rs"));
    h.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", h.finalize())
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new note evidence output");
    ensure!(
        models::hash_file(&a.model_file)? == MODEL_SHA,
        "SOME model checksum differs; another model was not substituted"
    );
    let source = models::hash_file(&a.input)?;
    let pcm = models::decode(&a.input, RATE as u32, 1)?;
    let duration = pcm.len() as f64 / RATE as f64;
    ensure!(
        (0.1..=120.).contains(&duration) && pcm.iter().all(|p| p.is_finite()),
        "Use finite isolated vocal audio between0.1and120seconds; no automatic trim or retiming"
    );
    let mut hash = Sha256::new();
    for p in &pcm {
        hash.update(p.to_le_bytes());
    }
    let pcm_sha = format!("{:x}", hash.finalize());
    let executable = std::env::current_exe()?;
    let runtime = vocal_runtime::resolve(
        executable.parent().context("Probe directory")?,
        a.runtime
            .map(Into::into)
            .or_else(|| std::env::var_os("ORT_DYLIB_PATH")),
    )?
    .context(
        "Set --runtime or ORT_DYLIB_PATH to a compatible explicitly identified native runtime",
    )?;
    ort::init_from(runtime.path.to_str().context("Runtime path is not UTF-8")?).commit()?;
    let started = Instant::now();
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_file(&a.model_file)?;
    ensure!(
        session.inputs.len() == 1 && session.inputs[0].name == "waveform",
        "Unexpected native model input contract"
    );
    ensure!(
        session.outputs.len() == 3
            && ["note_midi", "note_rest", "note_dur"]
                .iter()
                .all(|name| session.outputs.iter().any(|o| o.name == *name)),
        "Unexpected native model output contract"
    );
    let result = session.run(ort::inputs!["waveform"=>Tensor::from_array(([1,pcm.len()],pcm))?])?;
    let (pitch_shape, pitches) = result["note_midi"].try_extract_tensor::<f32>()?;
    let (rest_shape, rests) = result["note_rest"].try_extract_tensor::<bool>()?;
    let (duration_shape, durations) = result["note_dur"].try_extract_tensor::<f32>()?;
    ensure!(
        pitch_shape == rest_shape
            && rest_shape == duration_shape
            && pitch_shape.len() == 2
            && pitch_shape[0] == 1,
        "Expected matching single-source native note tensors"
    );
    let notes = intervals(pitches, rests, durations, duration)?;
    ensure!(
        models::hash_file(&a.input)? == source
            && models::hash_file(&a.model_file)? == MODEL_SHA
            && models::hash_file(&runtime.path)? == runtime.sha256,
        "Source/model/runtime changed during analysis"
    );
    let evidence = serde_json::json!({"version":1,"status":"EXPERIMENTAL_NOTE_OBSERVATIONS_NOT_PROMOTED","promotion_eligible":false,"backend":"OpenUtau-SOME0.0.1-export","source":a.input,"source_sha256":source,"pcm_sha256":pcm_sha,"sample_rate":RATE,"duration_seconds":duration,"model_sha256":MODEL_SHA,"publisher_package_sha256":ARCHIVE_SHA,"publisher_package":"https://github.com/xunmengshe/OpenUtau/releases/download/0.0.0.0/some-0.0.1.oudep","implementation_sha256":implementation_hash(),"helper_sha256":models::hash_file(&executable)?,"runtime":{"version":runtime.version,"library_sha256":runtime.sha256,"publisher_source_sha256":runtime.source_sha256},"preprocessing":"Shared YACE decode44.1kHz arithmetic mono mean; no gain normalization; complete original-clock waveform; publisher graph contains its own spectral frontend; batch1, native defaults, two intra-op threads, one inter-op thread","evidence_boundary":"Independent note hypotheses, including rests. Native MIDI values remain continuous and unaccepted. No RMVPE values, lyrics, key, singer identity, reference timings or charts enter inference. No existing project or pitch evidence is changed. Native-only research, not a desktop/browser quality option.","weight_terms":"SOME source MIT; exported weight-specific terms not separately established. No weights bundled or automatic downloads.","elapsed_seconds":started.elapsed().as_secs_f64(),"intervals":notes});
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&a.output)?
        .write_all(&serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_short_singing_rests_and_continuous_pitches() {
        let v = intervals(
            &[60.25, 0., 62.1],
            &[false, true, false],
            &[0.01, 0.02, 0.03],
            0.06,
        )
        .unwrap();
        assert_eq!(v.len(), 3);
        assert!(v[0].presence);
        assert!(!v[1].presence);
        assert_eq!(v[0].native_midi, 60.25);
        assert!((v[2].start_seconds - 0.03).abs() < 1e-8);
    }
    #[test]
    fn rejects_bad_shapes_values_and_clock() {
        assert!(intervals(&[60.], &[], &[0.1], 0.1).is_err());
        assert!(intervals(&[f32::NAN], &[false], &[0.1], 0.1).is_err());
        assert!(intervals(&[60.], &[false], &[-0.1], 0.1).is_err());
        assert!(intervals(&[128.], &[false], &[0.1], 0.1).is_err());
        assert!(intervals(&[60.], &[false], &[0.5], 0.1).is_err());
        assert!(intervals(&[60.], &[false], &[0.1], 0.5).is_err());
    }
    #[test]
    fn final_model_cell_is_bounded_without_retiming_or_erasing_native_values() {
        let v = intervals(&[60., 62.], &[false, false], &[0.05, 0.05], 0.095).unwrap();
        assert_eq!(v[1].end_seconds, 0.095);
        assert!(v[1].unbounded_end_seconds > 0.095);
        assert!((v[1].start_seconds - 0.05).abs() < 1e-8);
    }
}
