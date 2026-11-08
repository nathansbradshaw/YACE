//! Frozen, lyric-free recovery experiments. References are read only after
//! every prediction has been produced; recordings/reports stay private.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment,
    models,
    vocal_benchmark::{Corpus, Reference, score},
    vocal_consensus,
    vocal_generation::{self as vg, Generation},
    vocal_pitch::Frame,
    vocal_pitch_tracking,
};
use clap::{Parser, ValueEnum};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone, Copy, ValueEnum)]
enum Model {
    Tiny,
    Full,
}
impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Full => "full",
        }
    }
    fn sha(self) -> &'static str {
        match self {
            Self::Tiny => "91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3",
            Self::Full => "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610",
        }
    }
}
#[derive(Parser)]
struct Args {
    #[arg(long, conflicts_with = "report", required_unless_present = "report")]
    corpus: Option<PathBuf>,
    #[arg(long, requires = "audio", requires = "stem")]
    report: Option<PathBuf>,
    #[arg(long)]
    audio: Option<PathBuf>,
    #[arg(long)]
    stem: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_enum)]
    model: Model,
    #[arg(long)]
    model_file: PathBuf,
    #[arg(long)]
    probe: PathBuf,
    #[arg(long)]
    evidence: Option<PathBuf>,
    #[arg(long)]
    held_out: bool,
}
#[derive(Deserialize)]
struct RawSource {
    sha256: String,
}
#[derive(Deserialize)]
struct Raw {
    model_sha256: String,
    source: RawSource,
    pcm_sha256: String,
    duration_seconds: f64,
    implementation_sha256: String,
    sample_rate: usize,
    hop_samples: usize,
    frames: Vec<RawFrame>,
}
fn probe_implementation() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("../tools/singing-pitch-probe/src/main.rs"));
    hash.update(include_bytes!("../tools/singing-pitch-probe/src/rmvpe.rs"));
    hash.update(include_bytes!("../src/rmvpe_frontend.rs"));
    hash.update(include_bytes!("../tools/singing-pitch-probe/Cargo.toml"));
    hash.update(include_bytes!("../tools/singing-pitch-probe/Cargo.lock"));
    format!("{:x}", hash.finalize())
}
#[derive(Deserialize)]
struct RawFrame {
    time_seconds: f64,
    frequency_hz: f64,
    confidence: f32,
    local_rms: f64,
}
struct Input {
    id: String,
    audio: PathBuf,
    stem: PathBuf,
    baseline: PathBuf,
    raw: Option<PathBuf>,
    reference: Option<PathBuf>,
    standard: Option<PathBuf>,
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn write(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    ensure!(!path.exists(), "Output exists: {}", path.display());
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn note_view(report: &Generation) -> Generation {
    let mut view = report.clone();
    if let Some(timing) = &report.timing {
        for frame in &mut view.pitch_frames {
            frame.midi = report
                .chart
                .as_ref()
                .and_then(|c| {
                    c.notes.iter().find(|n| {
                        timing.seconds(n.start) <= frame.time_seconds
                            && frame.time_seconds < timing.seconds(n.end)
                    })
                })
                .and_then(|n| match n.tone {
                    chart_engine::vocals::VocalTone::Pitched { midi } => Some(midi as f64),
                    _ => None,
                });
        }
    }
    view
}
fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(!args.output.exists(), "Use a new output directory");
    ensure!(
        models::hash_file(&args.model_file)? == args.model.sha(),
        "Wrong independent model"
    );
    let inputs = if let Some(path) = &args.corpus {
        let corpus: Corpus = read(path)?;
        let base = path.parent().context("Corpus root")?;
        corpus
            .recordings
            .into_iter()
            .filter(|r| (r.split == "held_out") == args.held_out)
            .map(|r| {
                let baseline = base.join(r.candidate.context("Missing current High baseline")?);
                let standard = base.join(r.baseline.context("Missing Standard baseline")?);
                let stem = standard
                    .parent()
                    .context("Paired reports")?
                    .parent()
                    .context("Case root")?
                    .join("vocals.wav");
                let raw = standard.parent().unwrap().join("high_accuracy.json");
                Ok(Input {
                    id: r.id,
                    audio: base.join(r.audio),
                    stem,
                    baseline,
                    raw: Some(raw),
                    reference: r.reference.map(|p| base.join(p)),
                    standard: Some(standard),
                })
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![Input {
            id: "song".into(),
            audio: args.audio.clone().unwrap(),
            stem: args.stem.clone().unwrap(),
            baseline: args.report.clone().context("Report")?,
            raw: None,
            reference: None,
            standard: None,
        }]
    };
    fs::create_dir_all(&args.output)?;
    write(
        &args.output.join("frozen-run.json"),
        &json!({"algorithm":vocal_consensus::ALGORITHM,"model":args.model.name(),"model_sha256":args.model.sha(),"probe_sha256":models::hash_file(&args.probe)?,"replay_executable_sha256":models::hash_file(&std::env::current_exe()?)?,"references_enter_inference":false,"held_out":args.held_out}),
    )?;
    let mut predictions = Vec::new();
    for input in inputs {
        let baseline: Generation = read(&input.baseline)?;
        ensure!(
            baseline.words.is_none() && baseline.lyrics.is_none(),
            "Use the lyric-free baseline"
        );
        let primary = baseline
            .rmvpe_pitch
            .as_ref()
            .context("Missing primary contour")?;
        chart_engine::vocal_rmvpe::validate(
            primary,
            &baseline.audio_sha256,
            &baseline.vocal_stem_sha256,
            baseline.duration_seconds,
        )?;
        ensure!(
            models::hash_file(&input.audio)? == baseline.audio_sha256
                && models::hash_file(&input.stem)? == baseline.vocal_stem_sha256,
            "Source changed"
        );
        let folder = args.output.join(&input.id);
        fs::create_dir(&folder)?;
        let evidence = if let Some(path) = &args.evidence {
            path.clone()
        } else {
            eprintln!("{} CREPE {}", input.id, args.model.name());
            ensure!(
                Command::new(&args.probe)
                    .args(["--model", args.model.name(), "--model-file"])
                    .arg(&args.model_file)
                    .arg("--input")
                    .arg(&input.stem)
                    .arg("--output")
                    .arg(folder.join("independent"))
                    .status()?
                    .success(),
                "Independent inference failed"
            );
            folder.join("independent/evidence.json")
        };
        let raw: Raw = read(&evidence)?;
        ensure!(
            raw.model_sha256 == args.model.sha()
                && raw.implementation_sha256 == probe_implementation()
                && raw.sample_rate == 16000
                && raw.hop_samples == 160
                && raw.frames.len() == primary.frames.len()
                && raw
                    .frames
                    .iter()
                    .enumerate()
                    .all(|(i, f)| (f.time_seconds - i as f64 * 0.01).abs() < 1e-9)
                && raw.source.sha256 == baseline.vocal_stem_sha256
                && raw.pcm_sha256 == primary.pcm_sha256
                && (raw.duration_seconds - primary.duration_seconds).abs() <= 1. / 16000.,
            "Independent source/model/PCM differs"
        );
        let independent: Vec<_> = raw
            .frames
            .into_iter()
            .map(|f| Frame {
                time_seconds: f.time_seconds,
                frequency_hz: (f.frequency_hz > 0.).then_some(f.frequency_hz),
                confidence: f.confidence,
                rms: f.local_rms,
            })
            .collect();
        let mut candidate: Generation = if let Some(path) = &input.raw {
            read(path)?
        } else {
            eprintln!("{} acoustic-only baseline", input.id);
            vg::generate(vg::Request{audio:&input.audio,vocal_stem:&input.stem,alignment:&StemAlignment{audio_sha256:baseline.audio_sha256.clone(),stem_sha256:baseline.vocal_stem_sha256.clone(),evidence:"Explicit supplied source-clock pair; reference notes are not an inference input".into()},timing:baseline.timing.as_ref().context("Timing")?,words:None,lyrics:None,policy:&baseline.policy,neural_pitch:None,vocal_pitch:None,rmvpe_pitch:None})?
        };
        ensure!(
            candidate.audio_sha256 == baseline.audio_sha256
                && candidate.vocal_stem_sha256 == baseline.vocal_stem_sha256
                && candidate.pitch_frames.len() == baseline.pitch_frames.len()
                && candidate
                    .pitch_frames
                    .iter()
                    .zip(&baseline.pitch_frames)
                    .all(|(a, b)| a.time_seconds == b.time_seconds
                        && a.midi.is_some() == b.midi.is_some()),
            "Acoustic baseline changed"
        );
        let signal = models::decode(&input.stem, 22050, 1)?;
        let recovered = vocal_consensus::recover(
            &mut candidate.pitch_frames,
            &signal,
            primary,
            &independent,
            &|| false,
        )?;
        vocal_pitch_tracking::refine(&mut candidate.pitch_frames, primary, &|| false)?;
        for &i in &recovered {
            candidate.pitch_frames[i].support = vg::PitchSupport::AcousticAlternative;
            candidate.pitch_frames[i].clarity = 0.5;
        }
        let mut issues = Vec::new();
        let assembly = vg::assemble_notes(
            &candidate.pitch_frames,
            candidate.duration_seconds,
            candidate.timing.as_ref().context("Timing")?,
            None,
            &candidate.policy,
            &mut issues,
        )?;
        candidate.chart = assembly.chart;
        candidate.pitch_candidates = assembly.pitch_candidates;
        candidate.note_evidence = assembly.note_evidence;
        candidate.issues = issues;
        candidate.rmvpe_pitch = Some(primary.clone());
        candidate.algorithm = format!(
            "research-only {}; {}",
            vg::COMPARISON_ALGORITHM,
            vocal_consensus::ALGORITHM
        );
        write(&folder.join("candidate.json"), &candidate)?;
        eprintln!(
            "{} recovered {} frames; {} -> {} notes",
            input.id,
            recovered.len(),
            baseline.chart.as_ref().map_or(0, |c| c.notes.len()),
            candidate.chart.as_ref().map_or(0, |c| c.notes.len())
        );
        predictions.push((input, baseline, candidate, recovered.len()));
    }
    // Keep evaluation targets completely outside inference and policy selection.
    let mut rows = Vec::new();
    for (input, baseline, candidate, recovered) in predictions {
        if let Some(reference) = input.reference {
            let reference: Reference = read(&reference)?;
            let standard: Generation = read(&input.standard.context("Missing Standard")?)?;
            rows.push(json!({"id":input.id,"recovered_frames":recovered,"standard_frames":score(&reference,&standard)?,"baseline_frames":score(&reference,&baseline)?,"candidate_frames":score(&reference,&candidate)?,"standard_notes":score(&reference,&note_view(&standard))?,"baseline_notes":score(&reference,&note_view(&baseline))?,"candidate_notes":score(&reference,&note_view(&candidate))?}));
        } else {
            rows.push(json!({"id":input.id,"recovered_frames":recovered,"baseline_notes":baseline.chart.as_ref().map_or(0,|c|c.notes.len()),"candidate_notes":candidate.chart.as_ref().map_or(0,|c|c.notes.len()),"reference_accuracy":"not measured; diagnostic model support only"}));
        }
    }
    write(
        &args.output.join("summary.json"),
        &json!({"algorithm":vocal_consensus::ALGORITHM,"model":args.model.name(),"rows":rows}),
    )?;
    Ok(())
}
