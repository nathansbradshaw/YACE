//! Development-only joint use of complete pitch-strength outputs. No target
//! pitches/timestamps enter decoding. References are loaded after predictions.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    models,
    vocal_benchmark::{Corpus, Reference, score, score_exported_notes},
    vocal_generation::{self as vg, Generation},
    vocal_posterior as vp,
};
use clap::Parser;
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Parser)]
struct Args {
    corpus: PathBuf,
    root: PathBuf,
    crepe_root: PathBuf,
    output: PathBuf,
    #[arg(long, requires = "swift_root")]
    notes_root: Option<PathBuf>,
    #[arg(long, requires = "notes_root")]
    swift_root: Option<PathBuf>,
    #[arg(long,requires_all=["notes_root","swift_root"])]
    predict_only: bool,
    #[arg(long, requires = "predict_only")]
    predict_recovery: bool,
    #[arg(long, requires = "predict_recovery")]
    predict_energy: bool,
    /// Development experiment only; never identifies itself as a frozen app profile.
    #[arg(long, requires_all = ["predict_only", "predict_recovery"])]
    research_minimum_duration: Option<f64>,
    /// Retain the 180 ms two-vote recovery and add shorter three-vote contours.
    #[arg(long, requires = "research_minimum_duration")]
    research_consensus_extension: bool,
    /// Development-only assembly experiment, recorded in policy and identity.
    #[arg(long, requires = "predict_only")]
    research_note_minimum: Option<f64>,
    #[arg(long, requires = "predict_only")]
    held_out: bool,
    #[arg(long, default_value = "")]
    crepe_prefix: String,
    #[arg(long, requires_all=["notes_root","swift_root"], conflicts_with="predict_only")]
    recover: bool,
    #[arg(long, requires = "recover")]
    activity_root: Option<PathBuf>,
    #[arg(long, requires = "recover")]
    energy_recovery: bool,
}
fn json_read(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
struct Raw {
    values: Vec<f32>,
    rows: usize,
    start: f64,
    threshold: f32,
    bands: Vec<Vec<usize>>,
    model_sha256: String,
    implementation_sha256: String,
}
impl Raw {
    fn load(path: &Path, report: &Generation, kind: &str) -> Result<Self> {
        let v = json_read(path)?;
        let hash = match kind {
            "rmvpe" => chart_engine::vocal_rmvpe::SHA256,
            "fcpe" => "0467e0a627c4dd7e774c2d3a05fb5ca1830af7491451ca0158e62cb2621e0908",
            _ => "119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610",
        };
        ensure!(
            v["model_sha256"] == hash
                && v["implementation_sha256"]
                    == if kind == "fcpe" {
                        vp::fcpe_implementation_hash()
                    } else {
                        vp::spectral_probe_implementation_hash()
                    }
                && v["pcm_sha256"].as_str()
                    == Some(&report.rmvpe_pitch.as_ref().context("Primary")?.pcm_sha256)
                && v["sample_rate"] == 16000
                && v["hop_samples"] == 160,
            "Posterior model/PCM/grid differs"
        );
        let source = if kind == "fcpe" {
            &v["source_sha256"]
        } else {
            &v["source"]["sha256"]
        };
        ensure!(
            source.as_str() == Some(&report.vocal_stem_sha256),
            "Posterior source differs"
        );
        let data = path
            .parent()
            .context("Root")?
            .join(v["salience_file"].as_str().context("Raw file")?);
        ensure!(
            models::hash_file(&data)? == v["salience_sha256"].as_str().context("Raw SHA")?,
            "Posterior bytes changed"
        );
        let rows = v["salience_shape"][0].as_u64().context("Shape")? as usize;
        let duration = v["duration_seconds"]
            .as_f64()
            .context("Posterior duration")?;
        ensure!(
            duration.is_finite() && (duration - report.duration_seconds).abs() <= 1. / 16000.,
            "Posterior clock differs"
        );
        let samples = (duration * 16000.).round() as usize;
        ensure!(
            rows > 0
                && rows == samples / 160 + usize::from(kind != "fcpe")
                && rows <= 400001
                && v["salience_shape"][1] == 360
                && fs::metadata(&data)?.len() == rows as u64 * 360 * 4,
            "Posterior shape/budget differs"
        );
        let bytes = fs::read(data)?;
        let mut values = Vec::with_capacity(rows * 360);
        for c in bytes.chunks_exact(4) {
            let p = f32::from_le_bytes(c.try_into().unwrap());
            ensure!(
                p.is_finite() && (-1e-6..=1. + 1e-6).contains(&p),
                "Invalid posterior strength"
            );
            values.push(p.clamp(0., 1.))
        }
        let cents: Vec<f64> = if kind == "fcpe" {
            v["cents_table"]
                .as_array()
                .context("Cents")?
                .iter()
                .map(|v| v.as_f64().context("Cent value"))
                .collect::<Result<_>>()?
        } else {
            (0..360)
                .map(|i| 1_997.379_408_437_619 + 20. * i as f64)
                .collect()
        };
        ensure!(
            cents.len() == 360
                && cents.iter().all(|v| v.is_finite())
                && cents.windows(2).all(|w| w[0] < w[1]),
            "Invalid cents table"
        );
        let bands = (36..=84)
            .map(|m| {
                let c = (m as f64 - 69.) * 100. + 1200. * 44f64.log2();
                cents
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &v)| ((v - c).abs() <= 50.).then_some(i))
                    .collect()
            })
            .collect();
        Ok(Self {
            model_sha256: hash.into(),
            implementation_sha256: v["implementation_sha256"]
                .as_str()
                .context("Posterior implementation")?
                .into(),
            values,
            rows,
            start: if kind == "fcpe" { 0.005 } else { 0. },
            threshold: match kind {
                "rmvpe" => 0.03,
                "fcpe" => 0.006,
                _ => 0.5,
            },
            bands,
        })
    }
    fn observations(&self, report: &Generation, id: &str) -> vp::ModelFrames {
        let midi = self
            .costs(report, 2.)
            .iter()
            .map(|row| {
                let s = (0..49).min_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
                (row.iter().copied().fold(0., f64::max) - row[s] > 0.25).then_some((s + 36) as f64)
            })
            .collect();
        vp::ModelFrames {
            id: id.into(),
            model_sha256: self.model_sha256.clone(),
            implementation_sha256: self.implementation_sha256.clone(),
            midi,
        }
    }
    fn costs(&self, report: &Generation, cap: f64) -> Vec<[f64; 49]> {
        report
            .pitch_frames
            .iter()
            .map(|f| {
                let at =
                    (((f.time_seconds - self.start) / 0.01).round() as usize).min(self.rows - 1);
                if (f.time_seconds - (self.start + at as f64 * 0.01)).abs() > 0.00501 {
                    return [0.; 49];
                }
                let row = &self.values[at * 360..(at + 1) * 360];
                let peak = row.iter().copied().fold(0., f32::max);
                if peak <= self.threshold {
                    return [0.; 49];
                }
                std::array::from_fn(|m| {
                    let q = self.bands[m].iter().map(|&i| row[i]).fold(0., f32::max);
                    (-(q.max(1e-8) as f64 / peak.max(1e-8) as f64).ln()).min(cap)
                })
            })
            .collect()
    }
}
fn main() -> Result<()> {
    let a = Args::parse();
    if let Some(seconds) = a.research_minimum_duration {
        ensure!(
            seconds.is_finite() && (0.04..=0.3).contains(&seconds),
            "Invalid research recovery duration"
        );
    }
    if let Some(seconds) = a.research_note_minimum {
        ensure!(
            seconds.is_finite() && (0.03..=0.5).contains(&seconds),
            "Invalid research note minimum"
        );
    }
    let frozen_profile = if a.predict_energy {
        vp::FROZEN_ENERGY_PROFILE
    } else if a.predict_recovery {
        vp::FROZEN_RECOVERY_PROFILE
    } else {
        vp::FROZEN_PROFILE
    };
    let mut profile = if let Some(seconds) = a.research_minimum_duration {
        format!("{frozen_profile};research-only-recovery-minimum-seconds={seconds:.6}")
    } else {
        frozen_profile.to_owned()
    };
    if let Some(seconds) = a.research_note_minimum {
        profile.push_str(&format!(";research-only-note-minimum-seconds={seconds:.6}"));
    }
    if a.research_consensus_extension {
        profile.push_str(";research-only-retain180ms-two-votes-plus-short-three-votes");
    }
    ensure!(!a.output.exists(), "Output exists");
    let c: Corpus = serde_json::from_slice(&fs::read(&a.corpus)?)?;
    let mut generated = vec![];
    if a.predict_only {
        fs::create_dir(&a.output)?;
        fs::write(
            a.output.join("frozen-profile.json"),
            serde_json::to_vec_pretty(
                &json!({"profile":profile,"research_minimum_duration":a.research_minimum_duration,"research_note_minimum":a.research_note_minimum,"inputs_sha256":models::hash_file(&a.corpus)?,"executable_sha256":models::hash_file(&std::env::current_exe()?)?,"target_values_read":false,"held_out":a.held_out,"validation_notice":"After first evaluation, these artists are exposed regression material; parameters remain selected only on the four development recordings.","contour_implementation_sha256":format!("{:x}",sha2::Sha256::digest(include_bytes!("../src/vocal_posterior.rs")))}),
            )?,
        )?;
    }
    let mut predictions = vec![];
    for r in c
        .recordings
        .into_iter()
        .filter(|r| (r.split == "held_out") == a.held_out)
    {
        let mut recording = r.clone();
        let report: Generation =
            serde_json::from_slice(&fs::read(r.candidate.context("Report")?)?)?;
        let baseline = std::sync::Arc::new(report.clone());
        let mut observations = vec![];
        let targets = if let (Some(nr), Some(sr)) = (&a.notes_root, &a.swift_root) {
            let notes: chart_engine::vocal_note_consensus::Evidence =
                serde_json::from_slice(&fs::read(nr.join(format!("{}.notes.json", r.id)))?)?;
            let swift: chart_engine::vocal_pitch::Evidence =
                serde_json::from_slice(&fs::read(sr.join(format!("{}-swift.json", r.id)))?)?;
            chart_engine::vocal_note_consensus::validate_inputs(
                report.rmvpe_pitch.as_ref().context("Primary")?,
                &swift,
                &notes,
            )?;
            observations.push(vp::ModelFrames {
                id: "SwiftF0".into(),
                model_sha256: swift.model_sha256.clone(),
                implementation_sha256: swift.implementation_sha256.clone(),
                midi: report
                    .pitch_frames
                    .iter()
                    .map(|f| {
                        let s = &swift.frames[((f.time_seconds / 0.016).round() as usize)
                            .min(swift.frames.len() - 1)];
                        s.frequency_hz
                            .filter(|_| {
                                s.confidence >= 0.5
                                    && (s.time_seconds - f.time_seconds).abs() <= 0.008001
                            })
                            .map(|h| 69. + 12. * (h / 440.).log2())
                            .filter(|p| (35.5..84.5).contains(p))
                    })
                    .collect(),
            });
            observations.push(vp::ModelFrames {
                id: "ROSVOT with acoustic RWBD boundaries".into(),
                model_sha256: notes.encoder_sha256.clone(),
                implementation_sha256: notes.implementation_sha256.clone(),
                midi: report
                    .pitch_frames
                    .iter()
                    .map(|f| {
                        notes
                            .notes
                            .iter()
                            .find(|n| {
                                n.start_seconds <= f.time_seconds
                                    && f.time_seconds < n.end_seconds
                                    && (36..=84).contains(&n.midi)
                            })
                            .map(|n| n.midi as f64)
                    })
                    .collect(),
            });
            Some(
                report
                    .pitch_frames
                    .iter()
                    .map(|f| {
                        let s = &swift.frames[((f.time_seconds / 0.016).round() as usize)
                            .min(swift.frames.len() - 1)];
                        let p = s
                            .frequency_hz
                            .filter(|_| {
                                s.confidence >= 0.5
                                    && (s.time_seconds - f.time_seconds).abs() <= 0.008001
                            })
                            .map(|h| 69. + 12. * (h / 440.).log2());
                        notes
                            .notes
                            .iter()
                            .find(|n| {
                                n.start_seconds <= f.time_seconds && f.time_seconds < n.end_seconds
                            })
                            .filter(|n| {
                                (36..=84).contains(&n.midi)
                                    && p.is_some_and(|p| (p - n.midi as f64).abs() < 0.75)
                            })
                            .map(|n| n.midi as f64)
                    })
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let rm = Raw::load(
            &a.root.join(format!("raw-rmvpe-{}/evidence.json", r.id)),
            &report,
            "rmvpe",
        )?;
        let activity = if let Some(root) = &a.activity_root {
            let v = json_read(&root.join(format!("{}-activity.json", r.id)))?;
            ensure!(
                v["source_sha256"].as_str() == Some(&report.vocal_stem_sha256)
                    && v["model_sha256"]
                        == "a972a3f101b919e70f117135d8a968a785435d8f9748515f207a4ac3153e0718"
                    && v["checkpoint_sha256"]
                        == "8a17541944f2623a2a0164e072ac2f915dcedca99968083cc44bfbc7eb682275"
                    && v["sample_rate"] == 32000
                    && (v["duration_seconds"].as_f64().context("Activity clock")?
                        - report.duration_seconds)
                        .abs()
                        < 0.0001,
                "Activity model/source/clock differs"
            );
            let frames = v["frames"].as_array().context("Activity frames")?;
            ensure!(
                !frames.is_empty() && frames.len() <= 7201,
                "Activity budget"
            );
            let rows = frames
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let t = f["time_seconds"].as_f64().context("Activity time")?;
                    let p = f["voice"].as_f64().context("Activity score")?;
                    ensure!(
                        t == i as f64 * 0.5 && p.is_finite() && (0. ..=1.).contains(&p),
                        "Activity grid/score differs"
                    );
                    Ok(p)
                })
                .collect::<Result<Vec<_>>>()?;
            Some(
                report
                    .pitch_frames
                    .iter()
                    .map(|f| rows[((f.time_seconds / 0.5).round() as usize).min(rows.len() - 1)])
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let cr = Raw::load(
            &if a.crepe_prefix.is_empty() {
                a.crepe_root.join(&r.id).join("independent/evidence.json")
            } else {
                a.crepe_root
                    .join(format!("{}{}", a.crepe_prefix, r.id))
                    .join("evidence.json")
            },
            &report,
            "crepe",
        )?;
        let fc = Raw::load(
            &a.root.join(format!("{}-fcpe-posterior.json", r.id)),
            &report,
            "fcpe",
        )?;
        observations.extend([
            rm.observations(&report, "RMVPE pitch bands"),
            cr.observations(&report, "CREPE full pitch bands"),
            fc.observations(&report, "FCPE pitch bands"),
        ]);
        for cap in [2., 4., 8.] {
            if targets.is_some() && cap != 2. {
                continue;
            }
            let rc = rm.costs(&report, cap);
            let cc = cr.costs(&report, cap);
            let fc = fc.costs(&report, cap);
            for kind in ["rmvpe", "rmvpe_crepe", "rmvpe_fcpe", "all_three"] {
                if targets.is_some() && kind != "all_three" {
                    continue;
                }
                for anchor in [0., 0.25, 0.5] {
                    if targets.is_some() && anchor != 0.25 {
                        continue;
                    }
                    for note_weight in if targets.is_some() {
                        vec![0., 0.25, 0.5, 1., 2., 4.]
                    } else {
                        vec![0.]
                    } {
                        if a.recover && note_weight != 4. {
                            continue;
                        }
                        if a.predict_only && note_weight != 4. {
                            continue;
                        }
                        let costs: Vec<[f64; 49]> = (0..report.pitch_frames.len())
                            .map(|i| {
                                std::array::from_fn(|s| {
                                    let base = rc[i][s]
                                        + if kind == "rmvpe_crepe" || kind == "all_three" {
                                            cc[i][s]
                                        } else {
                                            0.
                                        }
                                        + if kind == "rmvpe_fcpe" || kind == "all_three" {
                                            fc[i][s]
                                        } else {
                                            0.
                                        };
                                    let prior = report.pitch_frames[i]
                                        .midi
                                        .map_or(0., |p| (p - (s + 36) as f64).powi(2).min(4.));
                                    base + anchor * prior
                                        + note_weight
                                            * targets.as_ref().and_then(|t| t[i]).map_or(0., |p| {
                                                (p - (s + 36) as f64).powi(2).min(4.)
                                            })
                                })
                            })
                            .collect();
                        for change in [0.5, 1., 3.2, 8.] {
                            if a.recover && change != 8. {
                                continue;
                            }
                            if a.predict_only && change != 8. {
                                continue;
                            }
                            if targets.is_some() && change != 3.2 && change != 8. {
                                continue;
                            }
                            for recovery_votes in if a.predict_recovery || a.energy_recovery {
                                vec![2]
                            } else if a.recover {
                                vec![2, 3]
                            } else {
                                vec![0]
                            } {
                                for recovery_duration in if a.predict_recovery || a.energy_recovery
                                {
                                    vec![a.research_minimum_duration.unwrap_or(0.18)]
                                } else if a.recover {
                                    if recovery_votes == 2 {
                                        vec![0.16, 0.18, 0.20, 0.24]
                                    } else {
                                        vec![0.12, 0.14, 0.16]
                                    }
                                } else {
                                    vec![0.08]
                                } {
                                    for activity_threshold in if activity.is_some() {
                                        vec![0., 0.05, 0.1, 0.2, 0.35, 0.5]
                                    } else {
                                        vec![0.]
                                    } {
                                        for energy_ratio in if a.predict_energy {
                                            vec![0.5]
                                        } else if a.energy_recovery {
                                            vec![0., 0.1, 0.2, 0.35, 0.5, 0.75]
                                        } else {
                                            vec![0.]
                                        } {
                                            let mut candidate = report.clone();
                                            if recovery_votes > 0 {
                                                let filtered: Vec<_> = targets
                                                    .as_ref()
                                                    .context("Targets")?
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(i, p)| {
                                                        p.filter(|_| {
                                                            activity.as_ref().is_none_or(|v| {
                                                                v[i] >= activity_threshold
                                                            })
                                                        })
                                                    })
                                                    .collect();
                                                let mut strong = candidate.pitch_frames.clone();
                                                if a.research_consensus_extension {
                                                    vp::recover_energy(
                                                        &mut strong,
                                                        [&rc, &cc, &fc],
                                                        &filtered,
                                                        3,
                                                        recovery_duration,
                                                        energy_ratio,
                                                        &|| false,
                                                    )?;
                                                }
                                                vp::recover_energy(
                                                    &mut candidate.pitch_frames,
                                                    [&rc, &cc, &fc],
                                                    &filtered,
                                                    recovery_votes,
                                                    if a.research_consensus_extension {
                                                        0.18
                                                    } else {
                                                        recovery_duration
                                                    },
                                                    energy_ratio,
                                                    &|| false,
                                                )?;
                                                if a.research_consensus_extension {
                                                    for (frame, supported) in candidate
                                                        .pitch_frames
                                                        .iter_mut()
                                                        .zip(strong)
                                                    {
                                                        if frame.midi.is_none()
                                                            && supported.midi.is_some()
                                                        {
                                                            *frame = supported;
                                                        }
                                                    }
                                                }
                                            }
                                            vp::decode(
                                                &mut candidate.pitch_frames,
                                                &costs,
                                                change,
                                                &|| false,
                                            )?;
                                            let mut issues = vec![];
                                            if let Some(seconds) = a.research_note_minimum {
                                                ensure!(
                                                    candidate.policy.analysis_mode
                                                        == vg::AnalysisMode::HighAccuracy,
                                                    "Short-note experiment requires explicit High accuracy"
                                                );
                                                candidate.policy.minimum_note_seconds = seconds;
                                                candidate.policy.validate()?;
                                            }
                                            let asm = vg::assemble_notes(
                                                &candidate.pitch_frames,
                                                candidate.duration_seconds,
                                                candidate.timing.as_ref().context("Timing")?,
                                                None,
                                                &candidate.policy,
                                                &mut issues,
                                            )?;
                                            candidate.chart = asm.chart;
                                            candidate.pitch_candidates = asm.pitch_candidates;
                                            candidate.note_evidence = asm.note_evidence;
                                            candidate.issues = issues;
                                            candidate.status = if candidate.chart.is_some() {
                                                vg::Status::ReviewRequired
                                            } else {
                                                vg::Status::NoVoicedPitch
                                            };
                                            vp::annotate(&mut candidate, &observations)?;
                                            if a.predict_only {
                                                candidate.algorithm = profile.clone();
                                                let path = a
                                                    .output
                                                    .join(format!("{}.generation.json", r.id));
                                                ensure!(
                                                    !path.exists(),
                                                    "Prediction already exists"
                                                );
                                                fs::write(&path, serde_json::to_vec(&candidate)?)?;
                                                recording.baseline = recording.candidate.take();
                                                recording.candidate = Some(path);
                                                generated.push(recording.clone());
                                                continue;
                                            }
                                            predictions.push((
                                                r.id.clone(),
                                                kind,
                                                cap,
                                                anchor,
                                                change,
                                                note_weight,
                                                recovery_votes,
                                                recovery_duration,
                                                activity_threshold,
                                                energy_ratio,
                                                baseline.clone(),
                                                candidate
                                                    .pitch_frames
                                                    .iter()
                                                    .map(|f| f.midi)
                                                    .collect::<Vec<_>>(),
                                                candidate.chart.take(),
                                                r.reference.clone().context("Reference")?,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        eprintln!("{} predictions complete", r.id);
    }
    if a.predict_only {
        fs::write(
            a.output.join("predicted-corpus.json"),
            serde_json::to_vec_pretty(&Corpus {
                version: 1,
                recordings: generated,
            })?,
        )?;
        return Ok(());
    }
    let mut rows = vec![];
    for (
        id,
        kind,
        cap,
        anchor,
        change,
        note_weight,
        recovery_votes,
        recovery_duration,
        activity_threshold,
        energy_ratio,
        b,
        pitches,
        chart,
        path,
    ) in predictions
    {
        let mut c = (*b).clone();
        for (f, p) in c.pitch_frames.iter_mut().zip(pitches) {
            f.midi = p
        }
        c.chart = chart;
        let r: Reference = serde_json::from_slice(&fs::read(path)?)?;
        let br = score(&r, &b)?;
        let cr = score(&r, &c)?;
        rows.push(json!({"id":id,"kind":kind,"cap":cap,"anchor":anchor,"change":change,"note_weight":note_weight,"recovery_votes":recovery_votes,"recovery_duration":recovery_duration,"activity_threshold":activity_threshold,"energy_ratio":energy_ratio,"baseline_raw":br,"candidate_raw":cr,"baseline_chart":score_exported_notes(&r,&b)?,"candidate_chart":score_exported_notes(&r,&c)?}));
    }
    fs::write(a.output, serde_json::to_vec(&rows)?)?;
    Ok(())
}
