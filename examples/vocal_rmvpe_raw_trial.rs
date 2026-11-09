//! Diagnose detector evidence lost to voicing guards, independently of lyrics.
//! All predictions finish before reference targets are loaded.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models,
    vocal_benchmark::{Reference, score, score_exported_notes},
    vocal_generation::{self as vg, Generation, PitchFrame, PitchSupport},
    vocal_pitch_tracking, vocal_posterior as vp, vocal_rmvpe,
};
use clap::Parser;
use serde_json::{Value, json};
use sha2::Digest;
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    baseline: PathBuf,
    #[arg(long)]
    audio: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    reference: Option<PathBuf>,
    /// Derived lead probe; requires the source-bound separation receipt.
    #[arg(long, requires = "separation_receipt")]
    probe: Option<PathBuf>,
    #[arg(long)]
    separation_receipt: Option<PathBuf>,
    #[arg(long, requires = "pesto_stem", conflicts_with = "probe")]
    pesto: Option<PathBuf>,
    #[arg(long, requires = "pesto")]
    pesto_stem: Option<PathBuf>,
    #[arg(long,requires="crepe_stem",conflicts_with_all=["probe","pesto"])]
    crepe: Option<PathBuf>,
    #[arg(long, requires = "crepe")]
    crepe_stem: Option<PathBuf>,
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "0.03,0.1,0.3,0.5,0.7,0.9"
    )]
    thresholds: Vec<f64>,
    #[arg(long, default_value_t = 0.08)]
    note_minimum: f64,
    #[arg(long, default_value_t = 0.)]
    relative_energy: f64,
    /// Minimum energy share of derived lead relative to lead plus backing.
    #[arg(long, requires = "separation_receipt")]
    lead_dominance: Option<f64>,
}
fn read(path: &std::path::Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(
        a.separation_receipt.is_none() || a.probe.is_some() || a.pesto.is_some(),
        "A separation receipt requires a derived model input"
    );
    ensure!(!a.output.exists(), "Use a new output directory");
    ensure!(
        !a.thresholds.is_empty()
            && a.thresholds.len() <= 12
            && a.thresholds
                .iter()
                .all(|t| t.is_finite() && (0.03..=1.).contains(t)),
        "Invalid threshold grid"
    );
    let old: Generation = serde_json::from_slice(&fs::read(&a.baseline)?)?;
    ensure!(
        models::hash_file(&a.audio)? == old.audio_sha256,
        "Original recording differs"
    );
    let mut evidence = old
        .rmvpe_pitch
        .clone()
        .context("Raw RMVPE evidence required")?;
    vocal_rmvpe::validate(
        &evidence,
        &old.audio_sha256,
        &old.vocal_stem_sha256,
        old.duration_seconds,
    )?;
    let mut derived = None;
    let mut selected_path = None;
    if let Some(path) = &a.probe {
        let p = read(path)?;
        let receipt = read(a.separation_receipt.as_ref().unwrap())?;
        ensure!(
            receipt["sample_rate"] == 44100
                && receipt["channels"] == 2
                && receipt["frames"]
                    .as_f64()
                    .is_some_and(|n| (n / 44100. - old.duration_seconds).abs() <= 1. / 44100.),
            "Derived recording clock differs"
        );
        ensure!(
            receipt["vocal_stem_sha256"] == old.vocal_stem_sha256
                && receipt["model_sha256"]
                    == "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4"
                && receipt["lead_sha256"] == p["source"]["sha256"],
            "Derived stem parent/model/child differs"
        );
        ensure!(
            p["model_sha256"] == vocal_rmvpe::SHA256
                && p["implementation_sha256"] == vp::spectral_probe_implementation_hash()
                && p["sample_rate"] == 16000
                && p["hop_samples"] == 160,
            "Probe model/preprocessing differs"
        );
        let stem = PathBuf::from(p["source"]["path"].as_str().context("Stem path")?);
        selected_path = Some(stem.clone());
        ensure!(
            models::hash_file(&stem)? == p["source"]["sha256"].as_str().context("Stem hash")?,
            "Derived audio changed"
        );
        let pcm = models::decode(&stem, 16000, 1)?;
        let mut hash = sha2::Sha256::new();
        for v in &pcm {
            hash.update(v.to_le_bytes());
        }
        ensure!(
            format!("{:x}", hash.finalize()) == p["pcm_sha256"].as_str().context("PCM hash")?
                && (pcm.len() as f64 / 16000. - old.duration_seconds).abs() <= 1. / 16000.,
            "Derived PCM/clock differs"
        );
        let blob_name = p["salience_file"].as_str().context("Salience file")?;
        ensure!(
            std::path::Path::new(blob_name)
                .file_name()
                .and_then(|p| p.to_str())
                == Some(blob_name),
            "Salience must be local"
        );
        let blob = path.parent().unwrap().join(blob_name);
        ensure!(
            models::hash_file(&blob)? == p["salience_sha256"].as_str().context("Salience hash")?
                && p["salience_shape"] == json!([pcm.len() / 160 + 1, 360]),
            "Raw salience differs"
        );
        let bytes = fs::read(blob)?;
        ensure!(
            bytes.len() == (pcm.len() / 160 + 1) * 360 * 4,
            "Salience budget"
        );
        let values = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect::<Vec<_>>();
        evidence = vocal_rmvpe::finish_pcm(&pcm, vocal_rmvpe::decode_chunk(&pcm, 0, &values)?)?;
        evidence.audio_sha256 = old.audio_sha256.clone();
        evidence.vocal_stem_sha256 = p["source"]["sha256"].as_str().unwrap().into();
        evidence.duration_seconds = old.duration_seconds;
        derived = Some(receipt);
    }
    vocal_rmvpe::validate(
        &evidence,
        &old.audio_sha256,
        &evidence.vocal_stem_sha256,
        old.duration_seconds,
    )?;
    let mut observations = evidence.frames.clone();
    let mut selected_stem = evidence.vocal_stem_sha256.clone();
    let mut model_id = vocal_rmvpe::MODEL_ID.to_owned();
    let mut model_sha = vocal_rmvpe::SHA256.to_owned();
    let mut implementation_sha = evidence.implementation_sha256.clone();
    if let Some(path) = &a.pesto {
        let p = read(path)?;
        let profile = &p["profile"];
        let implementation = format!(
            "{:x}",
            sha2::Sha256::digest(
                [
                    include_bytes!("../tools/vocal-pesto-probe/src/main.rs").as_slice(),
                    include_bytes!("../tools/vocal-pesto-probe/Cargo.toml").as_slice(),
                    include_bytes!("../tools/vocal-pesto-probe/Cargo.lock").as_slice()
                ]
                .concat()
            )
        );
        ensure!(
            profile["model_sha256"]
                == "8048fd691f9018d86977bc3cb745e02f265d800df55513b921c64e464dee43e5"
                && profile["implementation_sha256"] == implementation
                && profile["sample_rate"] == 44100
                && profile["hop_samples"] == 512
                && profile["mode"] == "offline-original-future"
                && profile["lookahead_samples"] == 4096
                && profile["target_values_read"] == false,
            "PESTO model/preprocessing differs"
        );
        let stem = a.pesto_stem.as_ref().unwrap();
        selected_path = Some(stem.clone());
        let stem_sha = models::hash_file(stem)?;
        ensure!(p["source_sha256"] == stem_sha, "PESTO stem differs");
        if stem_sha != old.vocal_stem_sha256 {
            let r = read(
                a.separation_receipt
                    .as_ref()
                    .context("Derived PESTO requires its parent receipt")?,
            )?;
            ensure!(
                r["model_sha256"]
                    == "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4"
                    && r["vocal_stem_sha256"] == old.vocal_stem_sha256
                    && r["lead_sha256"] == stem_sha
                    && r["sample_rate"] == 44100
                    && r["frames"]
                        .as_f64()
                        .is_some_and(|n| (n / 44100. - old.duration_seconds).abs() <= 1. / 44100.),
                "Derived PESTO parent/clock differs"
            );
            derived = Some(r);
        }
        let pcm = models::decode(stem, 44100, 1)?;
        let mut hash = sha2::Sha256::new();
        for v in &pcm {
            hash.update(v.to_le_bytes());
        }
        ensure!(
            p["pcm_sha256"] == format!("{:x}", hash.finalize())
                && (pcm.len() as f64 / 44100. - old.duration_seconds).abs() <= 1. / 16000.,
            "PESTO PCM/clock differs"
        );
        let values = p["frames"].as_array().context("PESTO frames")?;
        ensure!(
            values.len() == pcm.len().div_ceil(512) && values.len() <= 400000,
            "PESTO frame budget"
        );
        let grid = read(&path.parent().unwrap().join("activation-grid.json"))?;
        let blob = path.parent().unwrap().join("activations.f32le");
        ensure!(
            grid["frames"] == values.len()
                && grid["bins"] == 384
                && models::hash_file(&blob)?
                    == grid["sha256"].as_str().context("Activation hash")?
                && fs::metadata(blob)?.len() == values.len() as u64 * 384 * 4,
            "PESTO activation integrity differs"
        );
        observations = values
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let t = f["time_seconds"].as_f64().context("Frame clock")?;
                let midi = f["midi"].as_f64().context("MIDI pitch")?;
                let confidence = f["confidence"].as_f64().context("Confidence")?;
                let rms = f["rms"].as_f64().context("RMS")?;
                ensure!(
                    t.is_finite()
                        && (t - (i * 512 + 256) as f64 / 44100.).abs() < 1e-9
                        && midi.is_finite()
                        && (0.0..=128.).contains(&midi)
                        && confidence.is_finite()
                        && (0.0..=1.).contains(&confidence)
                        && rms.is_finite()
                        && rms >= 0.,
                    "Invalid PESTO frame"
                );
                Ok(chart_engine::vocal_pitch::Frame {
                    time_seconds: t,
                    frequency_hz: Some(440. * 2f64.powf((midi - 69.) / 12.)),
                    confidence: confidence as f32,
                    rms,
                })
            })
            .collect::<Result<_>>()?;
        selected_stem = stem_sha;
        model_id = "pesto-mir1k-offline-hop512".into();
        model_sha = profile["model_sha256"].as_str().unwrap().into();
        implementation_sha = implementation;
    }
    if let Some(path) = &a.crepe {
        let p = read(path)?;
        ensure!(
            p["model_id"] == "crepe-full-onnx-v1.1.0"
                && p["model_sha256"]
                    == "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610"
                && p["implementation_sha256"] == vp::spectral_probe_implementation_hash()
                && p["sample_rate"] == 16000
                && p["hop_samples"] == 160,
            "CREPE model/frontend differs"
        );
        let stem = a.crepe_stem.as_ref().unwrap();
        let stem_sha = models::hash_file(stem)?;
        ensure!(
            stem_sha == old.vocal_stem_sha256 && p["source"]["sha256"] == stem_sha,
            "CREPE stem differs"
        );
        let pcm = models::decode(stem, 16000, 1)?;
        let mut hash = sha2::Sha256::new();
        for v in &pcm {
            hash.update(v.to_le_bytes());
        }
        ensure!(
            p["pcm_sha256"] == format!("{:x}", hash.finalize())
                && (pcm.len() as f64 / 16000. - old.duration_seconds).abs() <= 1. / 16000.,
            "CREPE PCM/clock differs"
        );
        let name = p["salience_file"].as_str().context("Salience file")?;
        ensure!(
            std::path::Path::new(name)
                .file_name()
                .and_then(|s| s.to_str())
                == Some(name),
            "Salience must be local"
        );
        let blob = path.parent().unwrap().join(name);
        ensure!(
            models::hash_file(&blob)? == p["salience_sha256"].as_str().context("Salience hash")?
                && fs::metadata(blob)?.len() == (pcm.len() / 160 + 1) as u64 * 360 * 4,
            "CREPE salience differs"
        );
        let rows = p["frames"].as_array().context("CREPE frames")?;
        ensure!(
            rows.len() == pcm.len() / 160 + 1 && rows.len() <= 400000,
            "CREPE frame budget"
        );
        observations = rows
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let t = f["time_seconds"].as_f64().context("Frame clock")?;
                let hz = f["frequency_hz"].as_f64().context("Frequency")?;
                let confidence = f["confidence"].as_f64().context("Confidence")?;
                let rms = f["local_rms"].as_f64().context("RMS")?;
                ensure!(
                    t.is_finite()
                        && (t - i as f64 * 0.01).abs() < 1e-9
                        && hz.is_finite()
                        && hz >= 0.
                        && confidence.is_finite()
                        && (0.0..=1.).contains(&confidence)
                        && rms.is_finite()
                        && rms >= 0.,
                    "Invalid CREPE frame"
                );
                Ok(chart_engine::vocal_pitch::Frame {
                    time_seconds: t,
                    frequency_hz: (hz > 0.).then_some(hz),
                    confidence: confidence as f32,
                    rms,
                })
            })
            .collect::<Result<_>>()?;
        model_id = "crepe-full-onnx-v1.1.0".into();
        model_sha = p["model_sha256"].as_str().unwrap().into();
        implementation_sha = p["implementation_sha256"].as_str().unwrap().into();
    }
    ensure!(
        a.relative_energy.is_finite() && (0.0..=1.).contains(&a.relative_energy),
        "Invalid relative energy"
    );
    ensure!(!old.pitch_frames.is_empty(), "Missing baseline frame clock");
    let dominance = if let Some(minimum) = a.lead_dominance {
        ensure!(
            minimum.is_finite() && (0.0..=1.).contains(&minimum),
            "Invalid lead dominance"
        );
        let receipt = derived
            .as_ref()
            .context("Lead dominance needs a verified derived stem")?;
        let backing = a
            .separation_receipt
            .as_ref()
            .unwrap()
            .parent()
            .unwrap()
            .join("backing.wav");
        ensure!(
            models::hash_file(&backing)?
                == receipt["backing_sha256"].as_str().context("Backing hash")?,
            "Backing stem changed"
        );
        let lead = models::decode(selected_path.as_ref().context("Lead path")?, 16000, 2)?;
        let backing = models::decode(&backing, 16000, 2)?;
        ensure!(
            lead.len() == backing.len()
                && (lead.len() as f64 / 32000. - old.duration_seconds).abs() <= 1. / 16000.,
            "Lead/backing clocks differ"
        );
        Some(
            observations
                .iter()
                .map(|f| {
                    let centre = (f.time_seconds * 16000.).round() as usize;
                    let a = centre.saturating_sub(80).min(lead.len() / 2) * 2;
                    let b = (centre + 80).min(lead.len() / 2) * 2;
                    let power =
                        |pcm: &[f32]| pcm[a..b].iter().map(|s| f64::from(*s).powi(2)).sum::<f64>();
                    let l = power(&lead);
                    let r = power(&backing);
                    if l + r > 1e-16 { l / (l + r) } else { 0. }
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut energies: Vec<_> = observations
        .iter()
        .filter(|f| {
            let at = old
                .pitch_frames
                .partition_point(|o| o.time_seconds < f.time_seconds);
            [at.saturating_sub(1), at.min(old.pitch_frames.len() - 1)]
                .into_iter()
                .any(|i| {
                    (old.pitch_frames[i].time_seconds - f.time_seconds).abs() <= 0.0051
                        && old.pitch_frames[i].midi.is_some()
                })
        })
        .map(|f| f.rms)
        .filter(|r| r.is_finite() && *r > 0.)
        .collect();
    energies.sort_by(f64::total_cmp);
    let energy_floor = energies
        .get(energies.len() / 2)
        .map_or(0.0015, |r| (r * a.relative_energy).max(0.0015));
    fs::create_dir(&a.output)?;
    if let Some(minimum) = a.lead_dominance {
        fs::write(
            a.output.join("dominance.json"),
            serde_json::to_vec_pretty(
                &json!({"lead_power_share_minimum":minimum,"stereo_sample_rate":16000,"window_frames":160,"separation":derived,"meaning":"source energy contribution, not calibrated singer identity"}),
            )?,
        )?;
    }
    fs::write(
        a.output.join("recipe.json"),
        serde_json::to_vec_pretty(
            &json!({"executable_sha256":models::hash_file(&std::env::current_exe()?)?,"baseline_sha256":models::hash_file(&a.baseline)?,"thresholds":a.thresholds,"minimum_note_seconds":a.note_minimum,"energy_floor":energy_floor,"relative_energy":a.relative_energy,"energy_context":"source RMS at original accepted voice times; no target annotations","model_id":model_id,"model_sha256":model_sha,"implementation_sha256":implementation_sha,"vocal_stem_sha256":selected_stem,"change_cost":3.2,"reference_values_read":false,"derived_stem":derived,"promotion_eligible":false}),
        )?,
    )?;
    let mut predictions = vec![];
    for (index, threshold) in a.thresholds.iter().enumerate() {
        let mut report = old.clone();
        report.vocal_stem_sha256 = selected_stem.clone();
        report.rmvpe_pitch = (a.pesto.is_none() && a.crepe.is_none()).then(|| evidence.clone());
        report.neural_pitch = None;
        report.vocal_pitch = None;
        report.words = None;
        report.chart = None;
        report.note_evidence.clear();
        report.issues.clear();
        report.warnings = vec![
            "Experimental raw-model voicing diagnostic; no quality promotion or lyric input."
                .into(),
        ];
        report.policy.analysis_mode = vg::AnalysisMode::HighAccuracy;
        report.policy.minimum_note_seconds = a.note_minimum;
        report.policy.validate()?;
        report.algorithm = format!(
            "research-only-raw-{model_id}-voicing;salience>{threshold};RMS>={energy_floor};change3.2;source-time;no-lyrics-reference-or-note-model-gate"
        );
        if let Some(minimum) = a.lead_dominance {
            report
                .algorithm
                .push_str(&format!(";derived-lead-stereo-power-share>={minimum}"));
        }
        report.pitch_frames = observations
            .iter()
            .enumerate()
            .filter(|(_, f)| f.time_seconds <= report.duration_seconds)
            .map(|(i, f)| PitchFrame {
                time_seconds: f.time_seconds,
                midi: f
                    .frequency_hz
                    .filter(|_| {
                        f64::from(f.confidence) > *threshold
                            && f.rms >= energy_floor
                            && dominance
                                .as_ref()
                                .is_none_or(|d| d[i] >= a.lead_dominance.unwrap())
                    })
                    .map(|hz| 69. + 12. * (hz / 440.).log2())
                    .filter(|p| (35.5..84.5).contains(p)),
                yin_midi: None,
                mcleod_midi: None,
                clarity: f64::from(f.confidence),
                rms: f.rms,
                local_rms: Some(f.rms),
                local_energy_active: Some(f.rms >= energy_floor),
                support: PitchSupport::AcousticAlternative,
            })
            .collect();
        let raw = report.clone();
        let weights = report
            .pitch_frames
            .iter()
            .map(|f| f.clarity.max(0.1))
            .collect::<Vec<_>>();
        vocal_pitch_tracking::weighted_note_path(&mut report.pitch_frames, 3.2, &weights, &|| {
            false
        })?;
        let tracked = report.clone();
        let assembly = vg::assemble_notes(
            &report.pitch_frames,
            report.duration_seconds,
            report.timing.as_ref().context("Timing required")?,
            None,
            &report.policy,
            &mut report.issues,
        )?;
        report.chart = assembly.chart;
        report.status = if report.chart.is_some() {
            vg::Status::NeedsLyrics
        } else {
            vg::Status::NoVoicedPitch
        };
        report.note_evidence = assembly.note_evidence;
        report.pitch_candidates = assembly.pitch_candidates;
        vp::annotate(
            &mut report,
            &[vp::ModelFrames {
                id: model_id.clone(),
                model_sha256: model_sha.clone(),
                implementation_sha256: implementation_sha.clone(),
                midi: raw.pitch_frames.iter().map(|f| f.midi).collect(),
            }],
        )?;
        for n in &mut report.note_evidence {
            n.uncertain = true;
            n.reasons
                .push("Raw-model voicing experiment; review against the recording.".into());
        }
        let file = a
            .output
            .join(format!("threshold-{index:02}.generation.json"));
        fs::write(&file, serde_json::to_vec(&report)?)?;
        if let Some(chart) = &report.chart {
            let bytes = chart_engine::vocals::append_midi(
                &chart_engine::midi::new_midi(
                    report.timing.as_ref().unwrap(),
                    &Default::default(),
                )?,
                chart,
            )?;
            let back = chart_engine::vocals::import_midi(&bytes)?;
            ensure!(
                back.notes == chart.notes && back.phrases == chart.phrases,
                "MIDI round trip differs"
            );
            fs::write(a.output.join(format!("threshold-{index:02}.mid")), bytes)?;
        }
        predictions.push((*threshold, file, raw, tracked, report));
    }
    if let Some(path) = a.reference {
        let reference: Reference = serde_json::from_slice(&fs::read(path)?)?;
        let rows=predictions.iter().map(|(threshold,file,raw,tracked,report)|Ok(json!({"threshold":threshold,"generation":file,"raw":score(&reference,raw)?,"tracked":score(&reference,tracked)?,"notes":score_exported_notes(&reference,report)?}))).collect::<Result<Vec<_>>>()?;
        fs::write(
            a.output.join("scores.json"),
            serde_json::to_vec_pretty(
                &json!({"baseline_raw":score(&reference,&old)?,"baseline_notes":score_exported_notes(&reference,&old)?,"rows":rows,"promotion_eligible":false}),
            )?,
        )?;
    }
    Ok(())
}
