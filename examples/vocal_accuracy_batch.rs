//! Private source-bound regression runs. No reference notes enter inference.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean_recognition::Control, models, vocal_benchmark::Corpus, vocal_generation as vg,
};
use clap::{Parser, ValueEnum};
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Analyze,
    Reassemble,
    Seed,
    ShortConsensus,
}
#[derive(Parser)]
struct Args {
    #[arg(long, value_enum)]
    mode: Mode,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    model_dir: Option<PathBuf>,
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Reuse already verified RMVPE salience from raw-rmvpe-CASE directories.
    #[arg(long, required_if_eq("mode", "seed"))]
    raw_rmvpe: Option<PathBuf>,
    #[arg(long)]
    held_out: bool,
    /// Explicit frozen candidate helper; does not change the production profile.
    #[arg(long, required_if_eq("mode", "short-consensus"))]
    predictor: Option<PathBuf>,
}
fn main() -> Result<()> {
    let mut a = Args::parse();
    a.corpus = fs::canonicalize(&a.corpus)?;
    ensure!(!a.output.exists(), "Use a new output directory");
    let input = fs::read(&a.corpus)?;
    let corpus: Corpus = serde_json::from_slice(&input)?;
    ensure!(
        corpus.version == 1 && !corpus.recordings.is_empty() && corpus.recordings.len() <= 12,
        "Batch budget/version"
    );
    let mut ids = std::collections::BTreeSet::new();
    ensure!(
        corpus.recordings.iter().all(|r| !r.id.is_empty()
            && r.id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && ids.insert(r.id.clone())
            && matches!(r.split.as_str(), "development" | "held_out")),
        "Invalid/duplicate case ID or split"
    );
    fs::create_dir(&a.output)?;
    a.output = fs::canonicalize(&a.output)?;
    fs::write(
        a.output.join("recipe.json"),
        serde_json::to_vec_pretty(&json!({
            "source_corpus_sha256":models::hash_file(&a.corpus)?,
            "executable_sha256":models::hash_file(&std::env::current_exe()?)?,
            "analysis_profile":if matches!(a.mode,Mode::ShortConsensus){chart_engine::vocal_posterior::FROZEN_SHORT_PROFILE}else{chart_engine::vocal_posterior::FROZEN_ENERGY_PROFILE},
            "assembly_algorithm":vg::ALGORITHM,
            "references_read":false,"held_out":a.held_out,
            "condition":match a.mode {Mode::Analyze=>"same production independent model analysis",Mode::Reassemble=>"note assembly only; raw detector frames unchanged",Mode::Seed=>"verified existing RMVPE salience hardlinked into source-bound caches",Mode::ShortConsensus=>"frozen development-selected candidate; no backend promotion"}
        }))?,
    )?;
    let mut outputs = vec![];
    for r in corpus
        .recordings
        .iter()
        .filter(|r| (r.split == "held_out") == a.held_out)
    {
        ensure!(
            r.id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Unsafe case ID"
        );
        let file = r.candidate.as_ref().context("Missing generation input")?;
        let file = if file.is_absolute() {
            file.clone()
        } else {
            a.corpus.parent().context("Corpus parent")?.join(file)
        };
        let old: vg::Generation = serde_json::from_slice(&fs::read(&file)?)?;
        let audio = if r.audio.is_absolute() {
            r.audio.clone()
        } else {
            a.corpus.parent().unwrap().join(&r.audio)
        };
        ensure!(
            models::hash_file(&audio)? == r.audio_sha256 && old.audio_sha256 == r.audio_sha256,
            "Input audio does not match the saved report's source identity"
        );
        if matches!(a.mode, Mode::Seed) {
            let source = a
                .raw_rmvpe
                .as_ref()
                .context("Raw source required")?
                .join(format!("raw-rmvpe-{}", r.id));
            let bytes = fs::read(source.join("evidence.json"))?;
            let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
            let primary = old
                .rmvpe_pitch
                .as_ref()
                .context("Missing bound RMVPE contour")?;
            ensure!(
                raw["model_sha256"] == chart_engine::vocal_rmvpe::SHA256
                    && raw["implementation_sha256"]
                        == chart_engine::vocal_posterior::spectral_probe_implementation_hash()
                    && raw["source"]["sha256"] == old.vocal_stem_sha256
                    && raw["pcm_sha256"] == primary.pcm_sha256
                    && raw["sample_rate"] == 16000
                    && raw["hop_samples"] == 160,
                "Source/model/implementation/PCM differs"
            );
            let blob = raw["salience_file"]
                .as_str()
                .context("Missing salience path")?;
            ensure!(
                std::path::Path::new(blob)
                    .file_name()
                    .and_then(|n| n.to_str())
                    == Some(blob),
                "Salience must be a local basename"
            );
            ensure!(
                raw["salience_sha256"] == models::hash_file(&source.join(blob))?,
                "Salience changed"
            );
            let key = a
                .cache
                .as_ref()
                .context("Cache required")?
                .join(chart_engine::vocal_independent::cache_key(&old)?);
            fs::create_dir_all(&key)?;
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(key.join("analysis.lock"))?;
            lock.try_lock()
                .context("Analysis is already using this cache")?;
            let target = key.join("raw-rmvpe-source");
            if target.exists() {
                ensure!(
                    fs::read(target.join("evidence.json"))? == bytes
                        && raw["salience_sha256"] == models::hash_file(&target.join(blob))?,
                    "Existing cache differs; left unchanged"
                );
            } else {
                let staging = tempfile::tempdir_in(&key)?;
                fs::hard_link(source.join(blob), staging.path().join(blob))?;
                fs::write(staging.path().join("evidence.json"), bytes)?;
                fs::rename(staging.path(), target)?;
            }
            let mut row = r.clone();
            row.audio = audio;
            row.candidate = Some(file);
            row.reference = r.reference.as_ref().map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    a.corpus.parent().unwrap().join(p)
                }
            });
            outputs.push(row);
            eprintln!(
                "{}: source-bound RMVPE cache reused; no inference or targets read",
                r.id
            );
            continue;
        }
        let mut report = match a.mode {
            Mode::Analyze => {
                let stem = file
                    .parent()
                    .context("Generation parent")?
                    .parent()
                    .context("Case parent")?
                    .join("vocals.wav");
                chart_engine::vocal_independent::analyze(
                    &old,
                    &audio,
                    &stem,
                    a.model_dir.as_deref().context("Model directory required")?,
                    a.cache.as_deref().context("Cache directory required")?,
                    Arc::new(Control::default()),
                )?
            }
            Mode::ShortConsensus => {
                let cache = a
                    .cache
                    .as_ref()
                    .context("Cache required")?
                    .join(chart_engine::vocal_independent::cache_key(&old)?);
                let helper = a
                    .predictor
                    .as_ref()
                    .context("Explicit candidate helper required")?;
                let helper_sha = models::hash_file(helper)?;
                let work = tempfile::tempdir_in(&a.output)?;
                let predicted = work.path().join("predicted");
                let status = std::process::Command::new(helper)
                    .arg(cache.join("input.json"))
                    .arg(&cache)
                    .arg(&cache)
                    .arg(&predicted)
                    .arg("--notes-root")
                    .arg(&cache)
                    .arg("--swift-root")
                    .arg(&cache)
                    .args([
                        "--predict-only",
                        "--predict-recovery",
                        "--predict-energy",
                        "--predict-short-consensus",
                        "--held-out",
                        "--crepe-prefix",
                        "fresh-full-",
                    ])
                    .status()?;
                ensure!(status.success(), "Frozen candidate prediction failed");
                let recipe: serde_json::Value =
                    serde_json::from_slice(&fs::read(predicted.join("frozen-profile.json"))?)?;
                ensure!(
                    recipe["profile"] == chart_engine::vocal_posterior::FROZEN_SHORT_PROFILE
                        && recipe["executable_sha256"] == helper_sha,
                    "Candidate helper/profile differs"
                );
                let result: vg::Generation =
                    serde_json::from_slice(&fs::read(predicted.join("source.generation.json"))?)?;
                ensure!(
                    result.audio_sha256 == old.audio_sha256
                        && result.vocal_stem_sha256 == old.vocal_stem_sha256
                        && result.duration_seconds == old.duration_seconds,
                    "Candidate source binding differs"
                );
                fs::write(
                    a.output.join(format!("{}.prediction-recipe.json", r.id)),
                    serde_json::to_vec_pretty(&recipe)?,
                )?;
                result
            }
            Mode::Reassemble => {
                let mut result = old.clone();
                let mut issues = vec![];
                let asm = vg::assemble_notes(
                    &old.pitch_frames,
                    old.duration_seconds,
                    old.timing.as_ref().context("Timing required")?,
                    old.words.as_ref(),
                    &old.policy,
                    &mut issues,
                )?;
                result.chart = asm.chart;
                result.pitch_candidates = asm.pitch_candidates;
                result.note_evidence = asm.note_evidence;
                result.issues = issues;
                result.warnings.push(format!("Acoustic note assembly replayed using {}; source/model pitch evidence is cached and unchanged from {}.",vg::ALGORITHM,old.algorithm));
                result
            }
            Mode::Seed => anyhow::bail!("Cache seeding must complete before inference"),
        };
        if matches!(a.mode, Mode::Reassemble) {
            ensure!(
                serde_json::to_vec(&report.pitch_frames)? == serde_json::to_vec(&old.pitch_frames)?,
                "Reassembly changed pitch frames"
            );
            // Never copy old per-note counts onto different intervals.
            for note in &mut report.note_evidence {
                note.uncertain = true;
                note.reasons.push("Reassembled acoustic boundaries; model counts require regeneration for these exact intervals.".into());
            }
        }
        let output = a.output.join(format!("{}.generation.json", r.id));
        fs::write(&output, serde_json::to_vec(&report)?)?;
        if let Some(chart) = &report.chart {
            let timing = report.timing.as_ref().context("Missing timing")?;
            let midi = chart_engine::vocals::append_midi(
                &chart_engine::midi::new_midi(timing, &Default::default())?,
                chart,
            )?;
            let roundtrip = chart_engine::vocals::import_midi(&midi)?;
            ensure!(
                roundtrip.notes == chart.notes && roundtrip.phrases == chart.phrases,
                "MIDI round trip changed notes"
            );
            fs::write(a.output.join(format!("{}.mid", r.id)), midi)?;
        }
        let mut row = r.clone();
        row.audio = audio;
        row.reference = r.reference.as_ref().map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                a.corpus.parent().unwrap().join(p)
            }
        });
        row.baseline = Some(file);
        row.candidate = Some(output);
        outputs.push(row);
        eprintln!(
            "{}: {} notes; predictions saved without reading reference targets",
            r.id,
            report.chart.as_ref().map_or(0, |c| c.notes.len())
        );
    }
    ensure!(!outputs.is_empty(), "No selected recordings");
    fs::write(
        a.output.join("outputs.json"),
        serde_json::to_vec_pretty(&Corpus {
            version: 1,
            recordings: outputs,
        })?,
    )?;
    Ok(())
}
