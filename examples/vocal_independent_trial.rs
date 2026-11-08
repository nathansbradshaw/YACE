//! Exercise the desktop workflow on a project, reusing only verified stem-bound
//! research salience. Authoritative recording/tempo come from the actual project.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment, models, vocal_generation as vg, vocal_independent as independent,
};
use clap::Parser;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Parser)]
struct Args {
    project: PathBuf,
    models: PathBuf,
    cache: PathBuf,
    output: PathBuf,
    #[arg(long)]
    seed_root: Option<PathBuf>,
    #[arg(long, default_value = "song")]
    source_id: String,
    #[arg(long, default_value = "crepe-full")]
    crepe_folder: String,
    #[arg(long)]
    seed_only: bool,
}
fn seed_raw(
    source: &Path,
    destination: &Path,
    stem: &str,
    pcm: &str,
    model: &str,
    implementation: &str,
) -> Result<()> {
    let mut v: Value = serde_json::from_slice(&fs::read(source)?)?;
    let identity = v
        .get("source_sha256")
        .or_else(|| v["source"].get("sha256"))
        .and_then(Value::as_str);
    ensure!(
        identity == Some(stem)
            && v["pcm_sha256"] == pcm
            && v["model_sha256"] == model
            && v["implementation_sha256"] == implementation,
        "Seed model/source/PCM/recipe differs"
    );
    let old = PathBuf::from(v["salience_file"].as_str().context("Seed blob")?);
    let blob = source.parent().context("Seed parent")?.join(&old);
    ensure!(
        models::hash_file(&blob)? == v["salience_sha256"].as_str().context("Seed SHA")?,
        "Seed blob changed"
    );
    let name = old.file_name().context("Seed filename")?;
    fs::create_dir_all(destination.parent().context("Cache parent")?)?;
    let target = destination.parent().unwrap().join(name);
    if !target.exists() && fs::hard_link(&blob, &target).is_err() {
        fs::copy(&blob, &target)?;
    }
    ensure!(
        models::hash_file(&target)? == models::hash_file(&blob)?,
        "Existing cache blob differs"
    );
    v["salience_file"] = name.to_string_lossy().into_owned().into();
    chart_engine::generation::write_json(destination, &v)?;
    Ok(())
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Output exists");
    fs::create_dir_all(&a.output)?;
    let store = chart_engine::editor::Store::new(&a.project);
    let project = store.load()?;
    let audio = store.audio_path(&project)?;
    let timing = project.timing(&store)?;
    let manifest = project
        .content
        .stems
        .as_ref()
        .context("Project has no source-bound stems")?;
    manifest.validate_binding(&project.audio_sha256, project.duration)?;
    let stem = manifest
        .stems
        .iter()
        .find(|s| s.id == chart_engine::stem_review::StemId::Vocals)
        .context("Project has no vocal stem")?;
    let binding = StemAlignment {
        audio_sha256: project.audio_sha256.clone(),
        stem_sha256: stem.sha256.clone(),
        evidence: format!(
            "{}: {}",
            manifest.alignment.method, manifest.alignment.details
        ),
    };
    ensure!(
        models::hash_file(&audio)? == binding.audio_sha256
            && models::hash_file(&stem.path)? == binding.stem_sha256,
        "Project source or stem changed"
    );
    let control = Arc::new(chart_engine::clean_recognition::Control::default());
    let rmvpe = chart_engine::vocal_rmvpe::prepare(&a.models, false, &|| false)?;
    let primary =
        chart_engine::vocal_rmvpe::analyze(&audio, &stem.path, &binding, &rmvpe, &|| false)?;
    let baseline = vg::generate(vg::Request {
        audio: &audio,
        vocal_stem: &stem.path,
        alignment: &binding,
        timing: &timing,
        words: None,
        lyrics: None,
        policy: &vg::Policy {
            analysis_mode: vg::AnalysisMode::HighAccuracy,
            ..Default::default()
        },
        neural_pitch: None,
        vocal_pitch: None,
        rmvpe_pitch: Some(&primary),
    })?;
    chart_engine::generation::write_json(&a.output.join("baseline.json"), &baseline)?;
    let cache = a.cache.join(independent::cache_key(&baseline)?);
    fs::create_dir_all(&cache)?;
    if let Some(root) = a.seed_root {
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(cache.join("analysis.lock"))?;
        lock.try_lock()?;
        for (source, dest, sha, implementation) in [
            (
                root.join(format!("raw-rmvpe-{}/evidence.json", a.source_id)),
                cache.join("raw-rmvpe-source/evidence.json"),
                chart_engine::vocal_rmvpe::SHA256,
                chart_engine::vocal_posterior::spectral_probe_implementation_hash(),
            ),
            (
                root.join(&a.crepe_folder).join("evidence.json"),
                cache.join("fresh-full-source/evidence.json"),
                independent::ASSETS[0].sha256,
                chart_engine::vocal_posterior::spectral_probe_implementation_hash(),
            ),
            (
                root.join(format!("{}-fcpe-posterior.json", a.source_id)),
                cache.join("source-fcpe-posterior.json"),
                independent::ASSETS[1].sha256,
                chart_engine::vocal_posterior::fcpe_implementation_hash(),
            ),
        ] {
            seed_raw(
                &source,
                &dest,
                &binding.stem_sha256,
                &primary.pcm_sha256,
                sha,
                &implementation,
            )?;
        }
        let notes: chart_engine::vocal_note_consensus::Evidence =
            serde_json::from_slice(&fs::read(
                root.join("fresh-rosvot")
                    .join(format!("{}.notes.json", a.source_id)),
            )?)?;
        chart_engine::vocal_note_consensus::validate(
            &notes,
            &binding.stem_sha256,
            baseline.duration_seconds,
        )?;
        // Swift's evidence also binds the original recording. Re-run this small
        // model instead of rewriting an old report's authoritative source hash.
        let swift = chart_engine::vocal_pitch::analyze(
            &audio,
            &stem.path,
            &binding,
            &models::verified(&a.models, "vocal-pitch")?,
            &|| false,
        )?;
        chart_engine::vocal_note_consensus::validate_inputs(&primary, &swift, &notes)?;
        chart_engine::generation::write_json(&cache.join("source-swift.json"), &swift)?;
        fs::copy(
            root.join("fresh-rosvot")
                .join(format!("{}.notes.json", a.source_id)),
            cache.join("source.notes.json"),
        )?;
    }
    if !a.seed_only {
        let result =
            independent::analyze(&baseline, &audio, &stem.path, &a.models, &a.cache, control)?;
        chart_engine::generation::write_json(&a.output.join("generation.json"), &result)?;
        let chart = result.chart.as_ref().context("No vocal notes")?;
        chart_engine::generation::write_json(&a.output.join("vocals.json"), chart)?;
        if timing.offset == 0. {
            let base = chart_engine::midi::new_midi(&timing, &Default::default())?;
            fs::write(
                a.output.join("vocals.mid"),
                chart_engine::vocals::append_midi(&base, chart)?,
            )?;
        }
    }
    println!("Source-bound cache prepared: {}", cache.display());
    Ok(())
}
