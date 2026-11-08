//! Development-only detector/decoder calibration; never promotes a profile.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::{Corpus, Reference, score},
    vocal_generation::{self as vg, Generation},
    vocal_pitch::{self, Evidence},
    vocal_pitch_tracking,
};
use clap::Parser;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    corpus: PathBuf,
    evidence_root: PathBuf,
    output: PathBuf,
    #[arg(long)]
    notes_root: Option<PathBuf>,
    #[arg(long)]
    swift_root: Option<PathBuf>,
    #[arg(long)]
    fcpe_root: Option<PathBuf>,
}
fn midi(hz: f64) -> Option<f64> {
    (hz > 0.)
        .then(|| 69. + 12. * (hz / 440.).log2())
        .filter(|p| (35.5..84.5).contains(p))
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Use a new output");
    let corpus: Corpus = serde_json::from_slice(&fs::read(&a.corpus)?)?;
    let mut rows = vec![];
    for r in corpus
        .recordings
        .into_iter()
        .filter(|r| r.split == "development")
    {
        let report: Generation =
            serde_json::from_slice(&fs::read(r.candidate.context("Candidate")?)?)?;
        let reference: Reference =
            serde_json::from_slice(&fs::read(r.reference.context("Reference")?)?)?;
        let crepe: Value = serde_json::from_slice(&fs::read(
            a.evidence_root
                .join(&r.id)
                .join("independent/evidence.json"),
        )?)?;
        let primary = report.rmvpe_pitch.as_ref().context("RMVPE")?;
        ensure!(
            crepe["source"]["sha256"].as_str() == Some(&report.vocal_stem_sha256)
                && crepe["pcm_sha256"].as_str() == Some(&primary.pcm_sha256),
            "CREPE stem/PCM binding"
        );
        let learned: Option<Value> = a
            .notes_root
            .as_ref()
            .map(|p| -> Result<_> {
                Ok(serde_json::from_slice(&fs::read(
                    p.join(format!("{}.notes.json", r.id)),
                )?)?)
            })
            .transpose()?;
        if let Some(n) = &learned {
            ensure!(
                n["stem_sha256"].as_str() == Some(&report.vocal_stem_sha256),
                "Note model source differs"
            )
        }
        let swift: Option<Evidence> = a
            .swift_root
            .as_ref()
            .map(|p| -> Result<_> {
                Ok(serde_json::from_slice(&fs::read(
                    p.join(format!("{}-swift.json", r.id)),
                )?)?)
            })
            .transpose()?;
        if let Some(s) = &swift {
            ensure!(
                s.model_sha256 == vocal_pitch::MODEL_SHA256
                    && s.implementation_sha256 == vocal_pitch::implementation_hash()
                    && s.audio_sha256 == report.audio_sha256
                    && s.vocal_stem_sha256 == report.vocal_stem_sha256
                    && (s.duration_seconds - report.duration_seconds).abs() < 1. / 16000.
                    && s.sample_rate == 16000
                    && s.hop_samples == 256
                    && !s.frames.is_empty()
                    && s.frames
                        .iter()
                        .enumerate()
                        .all(|(i, f)| (f.time_seconds - i as f64 * 0.016).abs() < 1e-9),
                "Swift source/model/grid differs"
            );
            ensure!(s.pcm_sha256 == primary.pcm_sha256, "Swift PCM differs")
        }
        let other = crepe["frames"].as_array().context("CREPE")?;
        let fcpe: Option<Value> = a
            .fcpe_root
            .as_ref()
            .map(|p| -> Result<_> {
                Ok(serde_json::from_slice(&fs::read(
                    p.join(format!("{}-fcpe-raw.json", r.id)),
                )?)?)
            })
            .transpose()?;
        if let Some(c) = &fcpe {
            ensure!(
                c["model_sha256"]
                    == "0467e0a627c4dd7e774c2d3a05fb5ca1830af7491451ca0158e62cb2621e0908"
                    && c["source_sha256"].as_str() == Some(&report.vocal_stem_sha256)
                    && c["pcm_sha256"].as_str() == Some(&primary.pcm_sha256),
                "FCPE source/PCM/model differs"
            )
        }
        ensure!(other.len() == primary.frames.len(), "Frame grid differs");
        for choice in [
            "original",
            "rmvpe",
            "crepe",
            "rosvot",
            "neural_agree",
            "swift",
            "swift_agree",
            "three_vote",
            "ros_swift",
            "fcpe",
            "fcpe_swift",
            "fcpe_note",
            "ros_swift_fcpe",
            "fcpe_vote",
        ] {
            if choice.contains("fcpe") && fcpe.is_none() {
                continue;
            }
            if swift.is_none()
                && matches!(choice, "swift" | "swift_agree" | "three_vote" | "ros_swift")
            {
                continue;
            }
            if learned.is_none() && matches!(choice, "rosvot" | "neural_agree" | "ros_swift") {
                continue;
            }
            for threshold in [0.5, 0.7, 0.9] {
                if choice == "original" && threshold != 0.5 {
                    continue;
                }
                for cost in [0., 0.5, 1., 3.2, 8.] {
                    let mut candidate = report.clone();
                    let mut weights = vec![1.; candidate.pitch_frames.len()];
                    for (i, f) in candidate.pitch_frames.iter_mut().enumerate() {
                        if f.midi.is_none() {
                            continue;
                        }
                        let at = ((f.time_seconds / 0.01).round() as usize)
                            .min(primary.frames.len() - 1);
                        let p = &primary.frames[at];
                        let q = &other[at];
                        let c = q["confidence"].as_f64().unwrap_or(0.);
                        let cp = (c >= threshold)
                            .then(|| midi(q["frequency_hz"].as_f64().unwrap_or(0.)))
                            .flatten();
                        let rp = p
                            .frequency_hz
                            .filter(|_| p.confidence > 0.03)
                            .and_then(midi);
                        let np = learned
                            .as_ref()
                            .and_then(|v| v["notes"].as_array())
                            .and_then(|ns| {
                                ns.iter().find(|n| {
                                    n["start_seconds"].as_f64().unwrap_or(f64::MAX)
                                        <= f.time_seconds
                                        && f.time_seconds < n["end_seconds"].as_f64().unwrap_or(0.)
                                })
                            })
                            .and_then(|n| n["midi"].as_f64())
                            .filter(|p| (35.5..84.5).contains(p));
                        let sf = swift.as_ref().and_then(|s| {
                            s.frames.get(
                                ((f.time_seconds / 0.016).round() as usize).min(s.frames.len() - 1),
                            )
                        });
                        let sp = sf
                            .filter(|s| s.confidence as f64 >= threshold)
                            .and_then(|s| s.frequency_hz)
                            .and_then(midi);
                        let agrees = |a: Option<f64>, b: Option<f64>| {
                            a.zip(b).is_some_and(|(a, b)| (a - b).abs() < 0.75)
                        };
                        let ff =
                            fcpe.as_ref()
                                .and_then(|f| f["frames"].as_array())
                                .and_then(|fs| {
                                    fs.get(
                                        (((f.time_seconds - 0.005) / 0.01).round() as usize)
                                            .min(fs.len() - 1),
                                    )
                                });
                        let fp = ff
                            .filter(|f| f["confidence"].as_f64().unwrap_or(0.) > 0.006)
                            .and_then(|f| f["frequency_hz"].as_f64())
                            .and_then(midi);
                        let chosen = match choice {
                            "fcpe" => fp.or(f.midi),
                            "fcpe_swift" => {
                                if agrees(fp, sp) {
                                    fp
                                } else {
                                    f.midi
                                }
                            }
                            "fcpe_note" => {
                                if agrees(fp, np) {
                                    np
                                } else {
                                    f.midi
                                }
                            }
                            "ros_swift_fcpe" => {
                                if agrees(np, sp) && agrees(np, fp) {
                                    np
                                } else {
                                    f.midi
                                }
                            }
                            "fcpe_vote" => {
                                if agrees(fp, sp) || agrees(fp, rp) {
                                    fp
                                } else if agrees(sp, rp) {
                                    sp
                                } else {
                                    f.midi
                                }
                            }
                            "rosvot" => np,
                            "neural_agree" => {
                                if agrees(np, cp) {
                                    np
                                } else {
                                    f.midi
                                }
                            }
                            "rmvpe" => rp,
                            "crepe" => cp.or(rp),
                            "swift" => sp.or(rp),
                            "swift_agree" => {
                                if agrees(sp, cp) {
                                    sp
                                } else {
                                    f.midi
                                }
                            }
                            "ros_swift" => {
                                if agrees(sp, np) {
                                    np
                                } else {
                                    f.midi
                                }
                            }
                            "three_vote" => match (rp, cp, sp) {
                                (Some(r), Some(c), Some(s)) => {
                                    let mut v = [r, c, s];
                                    v.sort_by(f64::total_cmp);
                                    Some(v[1])
                                }
                                _ => f.midi,
                            },
                            _ => f.midi,
                        };
                        if let Some(pitch) = chosen {
                            f.midi = Some(pitch)
                        }
                        weights[i] = if choice == "crepe" && cp.is_some() {
                            c.max(0.1)
                        } else {
                            (p.confidence as f64).max(threshold)
                        };
                    }
                    vocal_pitch_tracking::weighted_note_path(
                        &mut candidate.pitch_frames,
                        cost,
                        &weights,
                        &|| false,
                    )?;
                    let assembly = vg::assemble_notes(
                        &candidate.pitch_frames,
                        candidate.duration_seconds,
                        candidate.timing.as_ref().context("Timing")?,
                        None,
                        &candidate.policy,
                        &mut vec![],
                    )?;
                    candidate.chart = assembly.chart;
                    let raw = score(&reference, &candidate)?;
                    let timing = candidate.timing.as_ref().unwrap();
                    for f in &mut candidate.pitch_frames {
                        f.midi = candidate
                            .chart
                            .as_ref()
                            .and_then(|ch| {
                                ch.notes.iter().find(|n| {
                                    timing.seconds(n.start) <= f.time_seconds
                                        && f.time_seconds < timing.seconds(n.end)
                                })
                            })
                            .and_then(|n| match n.tone {
                                chart_engine::vocals::VocalTone::Pitched { midi } => {
                                    Some(midi as f64)
                                }
                                _ => None,
                            });
                    }
                    let notes = score(&reference, &candidate)?;
                    rows.push(json!({"id":r.id,"choice":choice,"threshold":threshold,"cost":cost,"raw":raw,"notes":notes}));
                }
            }
        }
    }
    fs::write(a.output, serde_json::to_vec(&rows)?)?;
    Ok(())
}
