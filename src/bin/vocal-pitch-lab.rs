//! Local diagnostics and development-only ablations. No targets enter inference.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    clean::StemAlignment,
    models,
    vocal_benchmark::{Corpus, Reference, score},
    vocal_generation::Generation,
    vocal_pitch,
};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Task,
}
#[derive(Subcommand)]
enum Task {
    /// Run the same source-bound comparison as the editor, without changing notes.
    CheckProject {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        model_dir: PathBuf,
        #[arg(long)]
        output: PathBuf,
        /// Experimental recursive audio/MIDI review; never applies notes.
        #[arg(long)]
        adaptive: bool,
        /// Restrict review to original recording seconds, keeping full audio context.
        #[arg(long, requires = "end")]
        start: Option<f64>,
        #[arg(long, requires = "start")]
        end: Option<f64>,
    },
    /// Evaluate the section checker on generated notes, never authored intervals.
    Repair {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        project_template: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        held_out: bool,
        /// Compare current High notes, including all proposed structural repairs.
        /// Reports are diagnostic; previously exposed material is not fresh held-out.
        #[arg(long)]
        sections: bool,
    },
    Weighted {
        #[arg(long)]
        corpus: Vec<PathBuf>,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Generate {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        held_out: bool,
        /// Reuse source/model/implementation-bound raw F0 and require identical output.
        #[arg(long)]
        replay: bool,
    },
    Assemble {
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long)]
        batch: PathBuf,
        #[arg(long)]
        runner: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Diagnose {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Infer {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        swift_model: PathBuf,
        #[arg(long)]
        probe: Option<PathBuf>,
        #[arg(long)]
        rmvpe_model: Option<PathBuf>,
        #[arg(long)]
        crepe_model: Option<PathBuf>,
        #[arg(long)]
        held_out: bool,
    },
    Sweep {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        smooth: bool,
        #[arg(long)]
        viterbi: bool,
    },
}
fn weighted(corpora: &[PathBuf], evidence: &Path, output: &Path) -> Result<()> {
    use chart_engine::vocal_generation::PitchSupport;
    let mut data = Vec::new();
    for corpus in corpora {
        let c: Corpus = read(corpus)?;
        let base = corpus.parent().unwrap();
        for r in c
            .recordings
            .into_iter()
            .filter(|r| r.split == "development")
        {
            let selected: Generation = read(
                &base.join(
                    r.candidate
                        .as_ref()
                        .context("Missing development candidate")?,
                ),
            )?;
            let path = base.join(r.baseline.as_ref().context("Missing baseline")?);
            let old: Generation = read(&path.parent().unwrap().join("high_accuracy.json"))?;
            let reference: Reference = read(
                &base.join(
                    r.reference
                        .as_ref()
                        .context("Missing development reference")?,
                ),
            )?;
            let raw = if let Some(e) = selected.rmvpe_pitch {
                ensure!(
                    e.model_sha256 == chart_engine::vocal_rmvpe::SHA256
                        && e.vocal_stem_sha256 == old.vocal_stem_sha256,
                    "Comparison source differs"
                );
                Raw {
                    model_sha256: e.model_sha256,
                    source: RawSource {
                        sha256: e.vocal_stem_sha256,
                    },
                    frames: e
                        .frames
                        .into_iter()
                        .map(|f| RawFrame {
                            time_seconds: f.time_seconds,
                            frequency_hz: f.frequency_hz.unwrap_or(0.),
                            confidence: f.confidence as f64,
                        })
                        .collect(),
                }
            } else {
                read(&evidence.join(&r.id).join("rmvpe/evidence.json"))?
            };
            ensure!(
                raw.source.sha256 == old.vocal_stem_sha256
                    && raw.model_sha256 == chart_engine::vocal_rmvpe::SHA256,
                "Comparison source differs"
            );
            data.push((r.id, old, reference, raw));
        }
    }
    let mut rows = Vec::new();
    for family in 7..13 {
        for cost in [0.8, 1.6, 2.4, 3.2, 4., 4.8, 6.4] {
            let mut songs = Vec::new();
            for (id, old, reference, raw) in &data {
                let mut new = old.clone();
                let mut weights = vec![1.; new.pitch_frames.len()];
                let mut anchors = vec![(0., 0.); new.pitch_frames.len()];
                let mut context = vec![false; old.pitch_frames.len()];
                let mut scan = 0;
                while scan < old.pitch_frames.len() {
                    if old.pitch_frames[scan].midi.is_some() {
                        context[scan] = true;
                        scan += 1;
                        continue;
                    }
                    let start = scan;
                    while scan < old.pitch_frames.len() && old.pitch_frames[scan].midi.is_none() {
                        scan += 1;
                    }
                    if start > 0 && scan < old.pitch_frames.len() && scan - start <= 8 {
                        context[start..scan].fill(true);
                    }
                }
                for (i, f) in new.pitch_frames.iter_mut().enumerate() {
                    let original = f.midi.unwrap_or(0.);
                    if f.midi.is_none() && (family < 7 || (family >= 10 && !context[i])) {
                        continue;
                    }
                    let model = nearest(&raw.frames, f.time_seconds, |v| v.time_seconds);
                    let pitch = model
                        .filter(|m| m.confidence > 0.03 && m.frequency_hz > 0.)
                        .map(|m| 69. + 12. * (m.frequency_hz / 440.).log2())
                        .filter(|p| (35.5..84.5).contains(p));
                    let confidence = if pitch.is_some() {
                        model.unwrap().confidence
                    } else {
                        0.
                    };
                    if let Some(p) = pitch {
                        f.midi = Some(p);
                    }
                    weights[i] = match family {
                        0 => 1.,
                        1 | 3 | 4 | 6 | 8 | 9 | 11 | 12 => confidence.max(0.1),
                        2 => confidence.sqrt().max(0.1),
                        5 => {
                            if pitch.is_some() {
                                1.
                            } else {
                                0.1
                            }
                        }
                        _ => 1.,
                    };
                    let anchor = match family {
                        3 | 9 | 12 => 0.1,
                        4 => 0.25,
                        6 => 0.5,
                        _ => 0.,
                    };
                    let anchor = if f.support == PitchSupport::Consensus {
                        anchor
                    } else {
                        anchor * 0.2
                    };
                    anchors[i] = (
                        original,
                        if old.pitch_frames[i].midi.is_some() {
                            anchor
                        } else {
                            0.
                        },
                    );
                }
                chart_engine::vocal_pitch_tracking::anchored_note_path(
                    &mut new.pitch_frames,
                    cost,
                    &weights,
                    &anchors,
                    &|| false,
                )?;
                for (f, original) in new.pitch_frames.iter_mut().zip(&old.pitch_frames) {
                    if original.midi.is_none() {
                        f.midi = None;
                    }
                }
                songs.push(json!({"id":id,"baseline":score(reference,old)?,"candidate":score(reference,&new)?}));
            }
            rows.push(json!({"family":family,"cost":cost,"songs":songs}));
        }
    }
    write(
        output,
        &json!({"status":"development_only_weighted_note_path_ablations_no_promotion","families":["unweighted","salience","sqrt_salience","salience_with_dsp0.1","salience_with_dsp0.25","dropout0.1","salience_with_dsp0.5","all_context","all_context_salience","all_context_salience_dsp0.1","bounded_context","bounded_context_salience","bounded_context_salience_dsp0.1"],"evaluated_families":[7,8,9,10,11,12],"rows":rows}),
    )
}
fn generate(corpus: &Path, model: &Path, output: &Path, held: bool, replay: bool) -> Result<()> {
    use chart_engine::{vocal_generation as vg, vocal_rmvpe};
    ensure!(!output.exists(), "Output exists");
    let mut c: Corpus = read(corpus)?;
    let base = corpus.parent().unwrap();
    fs::create_dir_all(output)?;
    write(
        &output.join("frozen-run.json"),
        &json!({"corpus_sha256":models::hash_file(corpus)?,"model_sha256":models::hash_file(model)?,"algorithm":vg::COMPARISON_ALGORITHM,"implementation_sha256":vocal_rmvpe::implementation_hash(),"executable_sha256":models::hash_file(&std::env::current_exe()?)?,"replay_requires_exact_generation_equality":replay,"target_values_read":false,"split":if held{"held_out"}else{"development"}}),
    )?;
    for r in &mut c.recordings {
        if (r.split == "held_out") != held {
            r.candidate = None;
            continue;
        }
        let old: Generation =
            read(&base.join(r.candidate.as_ref().context("Missing acoustic baseline")?))?;
        let audio = base.join(&r.audio);
        let stem = base
            .join(r.baseline.as_ref().context("Missing source layout")?)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("vocals.wav");
        let binding=StemAlignment{audio_sha256:old.audio_sha256.clone(),stem_sha256:old.vocal_stem_sha256.clone(),evidence:"Frozen professional-reference batch; source-clock direct vocals; no target values enter inference".into()};
        eprintln!("{} RMVPE + temporal note centres", r.id);
        let result = output.join(&r.id);
        fs::create_dir(&result)?;
        let e = if replay {
            old.rmvpe_pitch
                .clone()
                .context("Replay needs raw independent contour")?
        } else {
            vocal_rmvpe::analyze(&audio, &stem, &binding, model, &|| false)?
        };
        let timing = old.timing.as_ref().context("Missing timing")?;
        let report = vg::generate(vg::Request {
            audio: &audio,
            vocal_stem: &stem,
            alignment: &binding,
            timing,
            words: None,
            lyrics: None,
            policy: &vg::Policy {
                analysis_mode: vg::AnalysisMode::HighAccuracy,
                ..Default::default()
            },
            neural_pitch: None,
            vocal_pitch: None,
            rmvpe_pitch: Some(&e),
        })?;
        ensure!(
            report.pitch_frames.len() == old.pitch_frames.len()
                && report
                    .pitch_frames
                    .iter()
                    .zip(&old.pitch_frames)
                    .all(|(a, b)| a.time_seconds == b.time_seconds
                        && a.midi.is_some() == b.midi.is_some()),
            "Candidate changed the acoustic voicing mask"
        );
        if replay {
            ensure!(
                serde_json::to_value(&old)? == serde_json::to_value(&report)?,
                "Frozen generation changed during replay: {}",
                r.id
            );
        }
        let path = result.join("generation.json");
        write(&path, &report)?;
        r.candidate = Some(path);
    }
    write(&output.join("corpus.json"), &c)
}
fn repair(corpus: &Path, template: &Path, output: &Path, held: bool, sections: bool) -> Result<()> {
    let c: Corpus = read(corpus)?;
    let base = corpus.parent().unwrap();
    let template: chart_engine::editor::Project = read(template)?;
    let mut rows = Vec::new();
    for r in c
        .recordings
        .iter()
        .filter(|r| (r.split == "held_out") == held)
    {
        let generation: Generation = read(
            &base.join(
                r.candidate
                    .as_ref()
                    .context("Missing independent contour")?,
            ),
        )?;
        let baseline_path = base.join(r.baseline.as_ref().context("Missing baseline")?);
        let old: Generation = if sections {
            generation.clone()
        } else {
            read(&baseline_path.parent().unwrap().join("high_accuracy.json"))?
        };
        let Some(chart) = old.chart.as_ref() else {
            continue;
        };
        let timing = old.timing.as_ref().context("Missing source timing")?;
        let mut project = template.clone();
        project.audio_sha256 = old.audio_sha256.clone();
        project.duration = old.duration_seconds;
        project.content.vocals = Some(chart.clone());
        let stem = project
            .content
            .stems
            .as_mut()
            .context("Template needs a vocal stem")?
            .stems
            .iter_mut()
            .find(|s| s.id == chart_engine::stem_review::StemId::Vocals)
            .context("Missing template vocal stem")?;
        stem.sha256 = old.vocal_stem_sha256.clone();
        // No reference is read until the same production proposal has finished.
        let proposal =
            chart_engine::vocal_pitch_analysis::propose(&project, timing, &generation, None)?;
        let mut corrected = chart.clone();
        for correction in &proposal.corrections {
            if sections
                && proposal
                    .section
                    .repairs
                    .iter()
                    .any(|r| r.before.contains(&chart.notes[correction.index]))
            {
                continue;
            }
            corrected.notes[correction.index].tone = chart_engine::vocals::VocalTone::Pitched {
                midi: correction.after,
            };
        }
        if sections {
            corrected =
                chart_engine::vocal_section_analysis::apply(&corrected, &proposal.section.repairs)?;
        }
        let view = |chart: &chart_engine::vocals::VocalChart| {
            let mut report = old.clone();
            for f in &mut report.pitch_frames {
                f.midi = chart
                    .notes
                    .iter()
                    .find(|n| {
                        timing.seconds(n.start) <= f.time_seconds
                            && f.time_seconds < timing.seconds(n.end)
                    })
                    .and_then(|n| match n.tone {
                        chart_engine::vocals::VocalTone::Pitched { midi } => Some(midi as f64),
                        _ => None,
                    });
            }
            report.chart = Some(chart.clone());
            report
        };
        let reference: Reference = read(
            &base.join(
                r.reference
                    .as_ref()
                    .context("Missing evaluation reference")?,
            ),
        )?;
        let checks: Vec<_> = proposal.corrections.iter().map(|c| {
            let note = &chart.notes[c.index];
            let (mut gained, mut lost, mut reference_voiced) = (0usize, 0usize, 0usize);
            for f in reference.frames.iter().filter(|f| timing.seconds(note.start) <= f.time_seconds && f.time_seconds < timing.seconds(note.end)) {
                if let Some(p) = f.midi {
                    reference_voiced += 1;
                    let before = (p - c.before as f64).abs() <= 0.5;
                    let after = (p - c.after as f64).abs() <= 0.5;
                    gained += usize::from(after && !before);
                    lost += usize::from(before && !after);
                }
            }
            json!({"index":c.index,"reference_voiced_frames":reference_voiced,"correct_frames_gained":gained,"correct_frames_lost":lost})
        }).collect();
        let structural_only = if sections {
            Some(score(
                &reference,
                &view(&chart_engine::vocal_section_analysis::apply(
                    chart,
                    &proposal.section.repairs,
                )?),
            )?)
        } else {
            None
        };
        let standard_note_view = if sections {
            let standard: Generation = read(&baseline_path)?;
            Some(score(
                &reference,
                &view(standard.chart.as_ref().context("Missing Standard chart")?),
            )?)
        } else {
            None
        };
        rows.push(json!({"id":r.id,"split":r.split,"proposal":proposal,"posthoc_correction_checks":checks,"standard_generated_note_view":standard_note_view,"baseline_generated_note_view":score(&reference,&view(chart))?,"structural_only_note_view":structural_only,"corrected_note_view":score(&reference,&view(&corrected))?}));
    }
    write(
        output,
        &json!({"algorithm":chart_engine::vocal_pitch_analysis::ANALYSIS_ALGORITHM,"section_algorithm":if sections{Some(chart_engine::vocal_section_analysis::ALGORITHM)}else{None},"input":if sections{"current High generated notes; all repair proposals selected; previously exposed diagnostic corpus"}else{"legacy High generated notes; no authored intervals or lyrics enter comparison"},"rows":rows}),
    )
}
fn assemble(
    selection: &Path,
    prepared: &Path,
    batch: &Path,
    runner: &Path,
    output: &Path,
) -> Result<()> {
    let manifest: serde_json::Value = read(selection)?;
    let songs = manifest["songs"].as_array().context("Missing selection")?;
    ensure!(songs.len() == 6, "Expected six-song frozen batch");
    // Validate every completed inference before reading any authored targets.
    let mut reports = Vec::new();
    for (i, song) in songs.iter().enumerate() {
        let case = prepared.join(format!("case-{:02}", i + 1));
        let result = batch.join(format!("case-{:02}", i + 1));
        let receipt: serde_json::Value = read(&case.join("reference-source.json"))?;
        ensure!(
            receipt["source_archive_sha256"] == song["archive_sha256"],
            "Prepared source differs"
        );
        let standard: Generation = read(&result.join("paired/standard.json"))?;
        let high: Generation = read(&result.join("paired/high_accuracy.json"))?;
        ensure!(
            models::hash_file(&case.join("source.wav"))? == standard.audio_sha256
                && models::hash_file(&result.join("vocals.wav"))? == standard.vocal_stem_sha256
                && standard.audio_sha256 == high.audio_sha256
                && standard.vocal_stem_sha256 == high.vocal_stem_sha256,
            "Inference binding differs"
        );
        reports.push(standard);
    }
    let mut recordings = Vec::new();
    for (i, song) in songs.iter().enumerate() {
        let case = prepared.join(format!("case-{:02}", i + 1));
        let result = batch.join(format!("case-{:02}", i + 1));
        let reference = result.join("reference.json");
        if !reference.exists() {
            ensure!(
                Command::new(runner)
                    .args(["reference", "--audio"])
                    .arg(case.join("source.wav"))
                    .arg("--midi")
                    .arg(case.join("notes.mid"))
                    .arg("--ini-delay-ms")
                    .arg(song["metadata"]["delay"].as_str().unwrap_or("0"))
                    .arg("--output")
                    .arg(&reference)
                    .status()?
                    .success(),
                "Reference conversion failed"
            );
        }
        let r: Reference = read(&reference)?;
        ensure!(
            r.audio_sha256 == reports[i].audio_sha256,
            "Reference source differs"
        );
        recordings.push(chart_engine::vocal_benchmark::Recording {
            id: format!("pitch-{:02}", i + 1),
            artist: song["metadata"]["artist"]
                .as_str()
                .context("Missing artist")?
                .into(),
            language: if song["vocal_language_reliable"] == true {
                song["language"].as_str().unwrap_or("und")
            } else {
                "und"
            }
            .into(),
            split: song["split"].as_str().context("Missing split")?.into(),
            categories: vec![],
            audio: case.join("source.wav"),
            audio_sha256: reports[i].audio_sha256.clone(),
            reference: Some(reference),
            baseline: Some(result.join("paired/standard.json")),
            candidate: Some(result.join("paired/high_accuracy.json")),
        });
    }
    write(
        output,
        &Corpus {
            version: 1,
            recordings,
        },
    )
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    ensure!(fs::metadata(p)?.len() < 128 * 1024 * 1024, "Oversized JSON");
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn write(p: &Path, v: &impl Serialize) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)?;
    Ok(f.write_all(&serde_json::to_vec_pretty(v)?)?)
}
fn nearest<T>(values: &[T], time: f64, get: impl Fn(&T) -> f64) -> Option<&T> {
    if values.is_empty() {
        return None;
    }
    let i = values.partition_point(|v| get(v) < time);
    let at = [i.saturating_sub(1), i.min(values.len() - 1)]
        .into_iter()
        .min_by(|&a, &b| {
            (get(&values[a]) - time)
                .abs()
                .total_cmp(&(get(&values[b]) - time).abs())
        })?;
    ((get(&values[at]) - time).abs() <= 0.020).then_some(&values[at])
}
#[derive(Default, Serialize)]
struct Counts {
    voiced: usize,
    correct: usize,
    wrong: usize,
    octave: usize,
    semitone: usize,
    false_voiced: usize,
}
fn diagnose(path: &Path, output: &Path) -> Result<()> {
    let corpus: Corpus = read(path)?;
    let base = path.parent().unwrap();
    let mut rows = Vec::new();
    for r in corpus.recordings {
        let reference: Reference = read(&base.join(r.reference.context("Missing reference")?))?;
        let standard: Generation = read(&base.join(r.baseline.context("Missing baseline")?))?;
        let high: Generation = read(&base.join(r.candidate.context("Missing candidate")?))?;
        score(&reference, &high)?;
        score(&reference, &standard)?;
        let mut counts: BTreeMap<String, Counts> = BTreeMap::new();
        for truth in &reference.frames {
            let Some(f) = nearest(&high.pitch_frames, truth.time_seconds, |f| f.time_seconds)
            else {
                continue;
            };
            let Some(p) = f.midi else { continue };
            let old = nearest(&standard.pitch_frames, truth.time_seconds, |f| {
                f.time_seconds
            })
            .and_then(|f| f.midi);
            let group = format!(
                "{:?}/{}",
                f.support,
                if old.is_some() {
                    "previously_voiced"
                } else {
                    "new_voicing"
                }
            );
            let row = counts.entry(group).or_default();
            row.voiced += 1;
            if let Some(t) = truth.midi {
                let delta = (p - t).abs();
                if delta <= 0.5 {
                    row.correct += 1;
                } else {
                    row.wrong += 1;
                    if (delta - 12.).abs() <= 0.5 {
                        row.octave += 1;
                    }
                    if delta <= 1.5 {
                        row.semitone += 1;
                    }
                }
            } else {
                row.false_voiced += 1;
            }
        }
        rows.push(json!({"id":r.id,"split":r.split,"counts":counts}));
    }
    write(
        output,
        &json!({"status":"post_hoc_diagnostic_not_tuning_or_promotion","rows":rows}),
    )
}
fn infer(
    corpus: &Path,
    output: &Path,
    swift: &Path,
    probe: Option<&Path>,
    rmvpe: Option<&Path>,
    crepe: Option<&Path>,
    held: bool,
) -> Result<()> {
    ensure!(!output.exists(), "Output exists");
    let c: Corpus = read(corpus)?;
    let base = corpus.parent().unwrap();
    fs::create_dir_all(output)?;
    for r in c
        .recordings
        .iter()
        .filter(|r| (r.split == "held_out") == held)
    {
        let report: Generation =
            read(&base.join(r.baseline.as_ref().context("Missing baseline")?))?;
        let stem = base
            .join(r.baseline.as_ref().unwrap())
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("vocals.wav");
        ensure!(
            models::hash_file(&stem)? == report.vocal_stem_sha256
                && models::hash_file(&base.join(&r.audio))? == report.audio_sha256,
            "Source changed"
        );
        let folder = output.join(&r.id);
        fs::create_dir(&folder)?;
        eprintln!("{} SwiftF0", r.id);
        let result = vocal_pitch::analyze(
            &base.join(&r.audio),
            &stem,
            &StemAlignment {
                audio_sha256: report.audio_sha256,
                stem_sha256: report.vocal_stem_sha256,
                evidence: "Same source-bound evaluator vocal stem; no target pitches/timings used"
                    .into(),
            },
            swift,
            &|| false,
        )?;
        write(&folder.join("swift.json"), &result)?;
        for (name, model) in [("rmvpe", rmvpe), ("tiny", crepe)] {
            if let Some(model) = model {
                eprintln!("{} {name}", r.id);
                ensure!(
                    Command::new(probe.context("Need --probe for neural candidates")?)
                        .args(["--model", name, "--model-file"])
                        .arg(model)
                        .arg("--input")
                        .arg(&stem)
                        .arg("--output")
                        .arg(folder.join(name))
                        .status()?
                        .success(),
                    "Probe failed"
                );
            }
        }
    }
    write(
        &output.join("run.json"),
        &json!({"corpus_sha256":models::hash_file(corpus)?,"swift_sha256":models::hash_file(swift)?,"probe_sha256":probe.map(models::hash_file).transpose()?,"target_values_read":false,"split":if held{"held_out"}else{"development"}}),
    )
}
#[derive(Deserialize)]
struct RawFrame {
    time_seconds: f64,
    frequency_hz: f64,
    confidence: f64,
}
#[derive(Deserialize)]
struct RawSource {
    sha256: String,
}
#[derive(Deserialize)]
struct Raw {
    model_sha256: String,
    source: RawSource,
    frames: Vec<RawFrame>,
}
#[derive(Clone, Copy, Serialize)]
struct Config {
    method: u8,
    confidence: f64,
    stable: usize,
    min_difference: f64,
    smoothing: usize,
    change_cost: f64,
}
fn sweep(corpus: &Path, evidence: &Path, output: &Path, smooth: bool, viterbi: bool) -> Result<()> {
    let c: Corpus = read(corpus)?;
    let base = corpus.parent().unwrap();
    let mut data = Vec::new();
    for r in c.recordings.iter().filter(|r| r.split == "development") {
        let report: Generation =
            read(&base.join(r.candidate.as_ref().context("Missing candidate")?))?;
        let reference: Reference =
            read(&base.join(r.reference.as_ref().context("Missing reference")?))?;
        let swift: vocal_pitch::Evidence = read(&evidence.join(&r.id).join("swift.json"))?;
        ensure!(
            swift.audio_sha256 == report.audio_sha256
                && swift.vocal_stem_sha256 == report.vocal_stem_sha256
                && swift.model_sha256 == vocal_pitch::MODEL_SHA256,
            "Swift binding differs"
        );
        let raw = |name: &str, sha: &str| -> Result<Option<Raw>> {
            let path = evidence.join(&r.id).join(name).join("evidence.json");
            if !path.exists() {
                return Ok(None);
            }
            let raw: Raw = read(&path)?;
            ensure!(
                raw.source.sha256 == report.vocal_stem_sha256 && raw.model_sha256 == sha,
                "Probe binding differs"
            );
            Ok(Some(raw))
        };
        let rmvpe = raw(
            "rmvpe",
            "5370e71ac80af8b4b7c793d27efd51fd8bf962de3a7ede0766dac0befa3660fd",
        )?;
        let crepe = raw(
            "tiny",
            "91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3",
        )?;
        data.push((r.id.clone(), report, reference, swift, rmvpe, crepe));
    }
    let mut rows = Vec::new();
    for method in if viterbi {
        vec![1]
    } else if smooth {
        vec![0, 1, 2, 3, 4]
    } else {
        (0..5).collect()
    } {
        for confidence in if viterbi {
            vec![0.03]
        } else if smooth {
            vec![0.03, 0.5]
        } else {
            vec![0.03, 0.1, 0.2, 0.3, 0.4, 0.5, 0.7, 0.8, 0.9, 0.95, 0.98]
        } {
            for stable in if smooth || viterbi {
                vec![1]
            } else {
                vec![1, 3, 5]
            } {
                for min_difference in if smooth || viterbi {
                    vec![0.]
                } else {
                    vec![0., 0.25, 0.5, 1., 6.]
                } {
                    for smoothing in if viterbi {
                        vec![0, 5, 11]
                    } else if smooth {
                        vec![5, 11, 21, 31]
                    } else {
                        vec![0]
                    } {
                        for change_cost in if viterbi {
                            vec![0.2, 0.4, 0.8, 1.6, 3.2, 6.4]
                        } else {
                            vec![0.]
                        } {
                            let config = Config {
                                method,
                                confidence,
                                stable,
                                min_difference,
                                smoothing,
                                change_cost,
                            };
                            let mut songs = Vec::new();
                            for (id, old, reference, swift, rmvpe, crepe) in &data {
                                let mut new = old.clone();
                                let mut changed = 0;
                                let raw_at =
                                    |raw: &Option<Raw>, time: f64, threshold: f64| -> Option<f64> {
                                        let f = nearest(&raw.as_ref()?.frames, time, |f| {
                                            f.time_seconds
                                        })?;
                                        (f.frequency_hz > 0. && f.confidence >= threshold)
                                            .then(|| 69. + 12. * (f.frequency_hz / 440.).log2())
                                    };
                                let at = |time: f64| -> Option<f64> {
                                    let s = nearest(&swift.frames, time, |f| f.time_seconds)
                                        .filter(|f| {
                                            f.confidence as f64
                                                >= if method == 2 {
                                                    confidence.max(0.7)
                                                } else {
                                                    confidence
                                                }
                                        })
                                        .and_then(|f| f.frequency_hz)
                                        .map(|f| 69. + 12. * (f / 440.).log2());
                                    let r = raw_at(rmvpe, time, confidence);
                                    let c = raw_at(
                                        crepe,
                                        time,
                                        if method == 4 {
                                            confidence.max(0.6)
                                        } else {
                                            confidence
                                        },
                                    );
                                    match method {
                                        0 => s,
                                        1 => r,
                                        2 => s
                                            .zip(r)
                                            .filter(|(s, r)| (s - r).abs() <= 0.5)
                                            .map(|(s, r)| (s + r) * 0.5),
                                        3 => c,
                                        4 => r
                                            .zip(c)
                                            .filter(|(r, c)| (r - c).abs() <= 0.5)
                                            .map(|(r, c)| (r + c) * 0.5),
                                        _ => None,
                                    }
                                };
                                let smoothed = |time: f64| -> Option<f64> {
                                    if smoothing == 0 {
                                        return at(time);
                                    }
                                    let mut values: Vec<_> = (-(smoothing as isize / 2)
                                        ..=smoothing as isize / 2)
                                        .filter_map(|delta| {
                                            let t = time + delta as f64 * 0.01;
                                            nearest(&old.pitch_frames, t, |f| f.time_seconds)
                                                .and_then(|f| f.midi)?;
                                            at(t)
                                        })
                                        .collect();
                                    if values.len() < smoothing / 2 {
                                        return at(time);
                                    }
                                    values.sort_by(f64::total_cmp);
                                    Some(values[values.len() / 2])
                                };
                                for f in &mut new.pitch_frames {
                                    if f.midi.is_none() {
                                        continue;
                                    }
                                    let Some(p) = smoothed(f.time_seconds)
                                        .filter(|p| (35.5..84.5).contains(p))
                                    else {
                                        continue;
                                    };
                                    if (-(stable as isize / 2)..=stable as isize / 2).all(|delta| {
                                        smoothed(f.time_seconds + delta as f64 * 0.016)
                                            .is_some_and(|v| (v - p).abs() <= 0.5)
                                    }) && f
                                        .midi
                                        .is_some_and(|old| (old - p).abs() > min_difference)
                                    {
                                        f.midi = Some(p);
                                        changed += 1;
                                    }
                                }
                                if viterbi {
                                    chart_engine::vocal_pitch_tracking::note_path(
                                        &mut new.pitch_frames,
                                        change_cost,
                                        &|| false,
                                    )?;
                                }
                                songs.push(json!({"id":id,"baseline":score(reference,old)?,"candidate":score(reference,&new)?,"changed":changed}));
                            }
                            rows.push(json!({"config":config,"songs":songs}));
                        }
                    }
                }
            }
        }
    }
    write(
        output,
        &json!({"status":"development_frame_ablations_not_chart_generation_or_promotion","methods":["swift","rmvpe","swift_rmvpe_agreement","crepe_tiny","rmvpe_crepe_agreement"],"rows":rows}),
    )
}
fn main() -> Result<()> {
    match Args::parse().command {
        Task::CheckProject {
            project,
            model_dir,
            output,
            adaptive,
            start,
            end,
        } => {
            let store = chart_engine::editor::Store::new(&project);
            let p = store.load()?;
            let analysis = chart_engine::vocal_pitch_analysis::analyze_configured(
                &p,
                &store.audio_path(&p)?,
                &p.timing(&store)?,
                start
                    .zip(end)
                    .map(|(start, end)| chart_engine::editor::TimeRange { start, end }),
                &model_dir,
                std::sync::Arc::default(),
                adaptive,
            )?;
            write(&output, &analysis)
        }
        Task::Repair {
            corpus,
            project_template,
            output,
            held_out,
            sections,
        } => repair(&corpus, &project_template, &output, held_out, sections),
        Task::Weighted {
            corpus,
            evidence,
            output,
        } => weighted(&corpus, &evidence, &output),
        Task::Generate {
            corpus,
            model,
            output,
            held_out,
            replay,
        } => generate(&corpus, &model, &output, held_out, replay),
        Task::Assemble {
            selection,
            prepared,
            batch,
            runner,
            output,
        } => assemble(&selection, &prepared, &batch, &runner, &output),
        Task::Diagnose { corpus, output } => diagnose(&corpus, &output),
        Task::Infer {
            corpus,
            output,
            swift_model,
            probe,
            rmvpe_model,
            crepe_model,
            held_out,
        } => infer(
            &corpus,
            &output,
            &swift_model,
            probe.as_deref(),
            rmvpe_model.as_deref(),
            crepe_model.as_deref(),
            held_out,
        ),
        Task::Sweep {
            corpus,
            evidence,
            output,
            smooth,
            viterbi,
        } => sweep(&corpus, &evidence, &output, smooth, viterbi),
    }
}
