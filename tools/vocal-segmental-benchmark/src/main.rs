//! Offline A/B replay. Prediction consumes no references; scoring is a second pass.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::{self as vb, Metrics, Reference},
    vocal_generation::Generation,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use vocal_segmental_benchmark::{Config, Method, Note, Sample, decode, fidelity};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    recordings: Vec<Recording>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recording {
    id: String,
    group: String,
    generation: PathBuf,
    jdc: PathBuf,
    references: Vec<PathBuf>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    confidence: f32,
    singing: f32,
    penalty: f64,
    articulation_db: Option<f64>,
    #[serde(default)]
    baseline_mask: bool,
    #[serde(default)]
    method: Method,
}
#[derive(Deserialize)]
struct Singing {
    source_sha256: String,
    frames: Vec<SingingFrame>,
}
#[derive(Deserialize)]
struct SingingFrame {
    time_seconds: f64,
    non_singing_probability: Option<f32>,
    #[serde(default)]
    note_band_mass: Vec<f64>,
}
impl SingingFrame {
    fn probability(&self) -> f32 {
        self.non_singing_probability
            .map(|v| 1.0 - v)
            .unwrap_or_else(|| self.note_band_mass.iter().sum::<f64>() as f32)
    }
}
#[derive(Serialize, Default)]
struct Summary {
    count: usize,
    baseline: [f64; 7],
    frame_rounding: [f64; 7],
    segmental: [f64; 7],
}
fn rates(m: &Metrics) -> [f64; 7] {
    let precision = m.note_matches as f64 / m.predicted_notes.max(1) as f64;
    let recall = m.note_matches as f64 / m.reference_notes.max(1) as f64;
    [
        m.correct_pitch_frames as f64 / m.reference_voiced_frames.max(1) as f64,
        m.detected_voiced_frames as f64 / m.reference_voiced_frames.max(1) as f64,
        m.incorrect_pitch_frames as f64 / m.detected_voiced_frames.max(1) as f64,
        m.false_voiced_frames as f64 / m.reference_unvoiced_frames.max(1) as f64,
        precision,
        recall,
        if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        },
    ]
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    ensure!(
        fs::metadata(p)?.len() <= 128 * 1024 * 1024,
        "JSON too large"
    );
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
fn nearest<T>(items: &[T], at: f64, time: impl Fn(&T) -> f64) -> usize {
    let i = items.partition_point(|v| time(v) < at);
    [i.saturating_sub(1), i.min(items.len() - 1)]
        .into_iter()
        .min_by(|&a, &b| {
            (time(&items[a]) - at)
                .abs()
                .total_cmp(&(time(&items[b]) - at).abs())
        })
        .unwrap()
}

fn observations(g: &Generation, jdc: &Singing, r: Recipe) -> Result<Vec<Sample>> {
    let raw = g.rmvpe_pitch.as_ref().context("Missing RMVPE evidence")?;
    ensure!(
        raw.audio_sha256 == g.audio_sha256
            && raw.vocal_stem_sha256 == g.vocal_stem_sha256
            && jdc.source_sha256 == g.vocal_stem_sha256,
        "Observation source differs"
    );
    ensure!(
        !raw.frames.is_empty() && !jdc.frames.is_empty() && g.pitch_frames.len() >= 2,
        "Missing observation frames"
    );
    ensure!(
        g.duration_seconds.is_finite()
            && g.duration_seconds > 0.0
            && g.duration_seconds <= 3600.0
            && raw.frames.len() <= 400_000
            && jdc.frames.len() <= 400_000
            && g.pitch_frames.len() <= 400_000,
        "Observation budget exceeded"
    );
    ensure!(
        raw.frames.iter().all(|p| p.time_seconds.is_finite()
            && p.time_seconds >= 0.0
            && p.confidence.is_finite()
            && (0.0..=1.0).contains(&p.confidence)
            && p.rms.is_finite()
            && p.rms >= 0.0
            && p.frequency_hz.is_none_or(|hz| hz.is_finite() && hz > 0.0)),
        "Invalid RMVPE observation"
    );
    ensure!(
        jdc.frames.iter().all(|j| j.time_seconds.is_finite()
            && j.time_seconds >= 0.0
            && j.probability().is_finite()
            && (-0.00001..=1.00001).contains(&j.probability())
            && j.note_band_mass.iter().all(|v| v.is_finite() && *v >= 0.0)),
        "Invalid JDC observation"
    );
    ensure!(
        raw.frames
            .windows(2)
            .all(|v| v[0].time_seconds < v[1].time_seconds)
            && jdc
                .frames
                .windows(2)
                .all(|v| v[0].time_seconds < v[1].time_seconds),
        "Observation clock not ordered"
    );
    ensure!(
        jdc.frames
            .iter()
            .all(|j| j.non_singing_probability.is_some() || !j.note_band_mass.is_empty()),
        "Missing JDC class probability"
    );
    let step = g.pitch_frames[1].time_seconds - g.pitch_frames[0].time_seconds;
    ensure!(
        step > 0.0
            && step <= 0.02
            && g.pitch_frames
                .windows(2)
                .all(|v| ((v[1].time_seconds - v[0].time_seconds) - step).abs() < 1e-8),
        "Expected regular source clock"
    );
    let mut result = vec![];
    for f in &g.pitch_frames {
        let p = &raw.frames[nearest(&raw.frames, f.time_seconds, |v| v.time_seconds)];
        let j = &jdc.frames[nearest(&jdc.frames, f.time_seconds, |v| v.time_seconds)];
        ensure!(
            (p.time_seconds - f.time_seconds).abs() <= 0.020
                && (j.time_seconds - f.time_seconds).abs() <= 0.020,
            "Observation does not cover source frame"
        );
        let pitch = p
            .frequency_hz
            .filter(|_| {
                p.confidence >= r.confidence
                    && p.rms >= 0.0015
                    && j.probability() >= r.singing
                    && (!r.baseline_mask || f.midi.is_some())
            })
            .map(|h| 69.0 + 12.0 * (h / 440.0).log2())
            .filter(|p| (35.5..84.5).contains(p));
        let start = (f.time_seconds - step / 2.0).max(0.0);
        let end = (f.time_seconds + step / 2.0).min(g.duration_seconds);
        if end > start {
            result.push(Sample {
                start,
                end,
                pitch,
                rms: p.rms,
            });
        }
    }
    Ok(result)
}
fn rounded(samples: &[Sample]) -> Vec<Note> {
    let mut notes: Vec<Note> = vec![];
    for s in samples {
        if let Some(p) = s.pitch {
            let midi = p.round() as u8;
            if let Some(last) = notes
                .last_mut()
                .filter(|n| n.midi == midi && (n.end - s.start).abs() < 1e-8)
            {
                last.end = s.end;
            } else {
                notes.push(Note {
                    start: s.start,
                    end: s.end,
                    continuous_pitch: p,
                    midi,
                });
            }
        }
    }
    notes
}
fn with_notes(original: &Generation, notes: &[Note]) -> Result<Generation> {
    let mut g = original.clone();
    let timing = g.timing.as_ref().context("Missing tempo map")?;
    let mut output: Vec<VocalNote> = vec![];
    for n in notes {
        let start = timing.tick(n.start).context("Invalid onset")?;
        let end = timing.tick(n.end).context("Invalid offset")?;
        ensure!(start < end, "MIDI clock cannot represent source note");
        output.push(VocalNote {
            midi: None,
            start,
            end,
            tone: VocalTone::Pitched { midi: n.midi },
            lyric: Syllable::Missing,
        });
    }
    g.chart = output
        .first()
        .zip(output.last())
        .map(|(a, b)| (a.start, b.end))
        .map(|(start, end)| VocalChart {
            version: 1,
            resolution: timing.resolution,
            source: "Continuous-pitch segmental research comparison; not approved".into(),
            phrases: vec![VocalPhrase {
                midi: None,
                start,
                end,
                player: 1,
            }],
            notes: output,
        });
    if let Some(chart) = &g.chart {
        chart.validate()?;
    }
    Ok(g)
}
fn roundtrip(g: &Generation) -> Result<()> {
    let Some(chart) = &g.chart else {
        return Ok(());
    };
    // Source resolution is inserted into a minimal valid MIDI before export.
    let mut empty = vec![
        77, 84, 104, 100, 0, 0, 0, 6, 0, 1, 0, 1, 1, 224, 77, 84, 114, 107, 0, 0, 0, 4, 0, 255, 47,
        0,
    ];
    let division = u16::try_from(chart.resolution)?;
    empty[12..14].copy_from_slice(&division.to_be_bytes());
    let bytes = chart_engine::vocals::append_midi(&empty, chart)?;
    let imported = chart_engine::vocals::import_midi(&bytes)?;
    ensure!(
        chart.notes.len() == imported.notes.len()
            && chart
                .notes
                .iter()
                .zip(&imported.notes)
                .all(|(a, b)| a.start == b.start && a.end == b.end && a.tone == b.tone),
        "MIDI roundtrip changed notes"
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4,
        "vocal-segmental-benchmark MANIFEST RECIPES NEW_OUTPUT_DIRECTORY"
    );
    let manifest: Manifest = read(Path::new(&args[1]))?;
    let recipes: Vec<Recipe> = read(Path::new(&args[2]))?;
    ensure!(
        !manifest.recordings.is_empty()
            && manifest.recordings.len() <= 64
            && !recipes.is_empty()
            && recipes.len() <= 256,
        "Invalid experiment size"
    );
    let mut ids = std::collections::BTreeSet::new();
    for entry in &manifest.recordings {
        ensure!(ids.insert(&entry.id), "Duplicate recording ID");
        ensure!(!entry.references.is_empty(), "Missing reference paths");
    }
    let output = Path::new(&args[3]);
    ensure!(!output.exists(), "Output already exists");
    fs::create_dir(output)?;
    let mut predictions = vec![];
    let mut witnesses = vec![];
    let mut inputs = vec![];
    // All predictions are written and hashed before any target file is opened.
    for entry in &manifest.recordings {
        ensure!(
            entry
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && !entry.id.is_empty(),
            "Invalid recording ID"
        );
        let g_bytes = fs::read(&entry.generation)?;
        let j_bytes = fs::read(&entry.jdc)?;
        ensure!(
            g_bytes.len() <= 128 * 1024 * 1024 && j_bytes.len() <= 128 * 1024 * 1024,
            "Input JSON exceeds budget"
        );
        let g: Generation = serde_json::from_slice(&g_bytes)?;
        let jdc: Singing = serde_json::from_slice(&j_bytes)?;
        inputs.push(json!({"id":entry.id,"generation_sha256":format!("{:x}",Sha256::digest(&g_bytes)),"jdc_sha256":format!("{:x}",Sha256::digest(&j_bytes))}));
        let mut outputs = vec![];
        for (i, &r) in recipes.iter().enumerate() {
            ensure!(
                r.confidence.is_finite()
                    && (0.0..=1.0).contains(&r.confidence)
                    && r.singing.is_finite()
                    && (0.0..=1.0).contains(&r.singing),
                "Invalid gate"
            );
            let samples = observations(&g, &jdc, r)?;
            let decoded = decode(
                &samples,
                Config {
                    penalty: r.penalty,
                    articulation_db: r.articulation_db,
                    ..Config::default()
                },
                r.method,
            )?;
            let audit = fidelity(&samples, &decoded.notes)?;
            let frame = rounded(&samples);
            let bytes = serde_json::to_vec(
                &json!({"id":entry.id,"recipe":r,"notes":decoded.notes,"frame_rounding":frame,"accepted_seconds":decoded.accepted_seconds,"cost_evaluations":decoded.cost_evaluations,"pitch_fidelity":audit}),
            )?;
            let path = output.join(format!("{}-{i:03}.json", entry.id));
            fs::write(&path, &bytes)?;
            witnesses.push(json!({"path":path,"sha256":format!("{:x}",Sha256::digest(&bytes))}));
            outputs.push((frame, decoded.notes));
        }
        predictions.push((g, outputs));
    }
    fs::write(
        output.join("prediction-receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"status":"predictions_complete_before_reference_reads","manifest_sha256":format!("{:x}",Sha256::digest(fs::read(&args[1])?)),"recipes_sha256":format!("{:x}",Sha256::digest(fs::read(&args[2])?)),"implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("lib.rs").as_slice(),include_bytes!("main.rs").as_slice()].concat())),"executable_sha256":format!("{:x}",Sha256::digest(fs::read(std::env::current_exe()?)?)),"inputs":inputs,"outputs":witnesses}),
        )?,
    )?;
    let mut summaries = vec![];
    let mut details = vec![];
    for (i, &recipe) in recipes.iter().enumerate() {
        let mut groups = std::collections::BTreeMap::<String, Summary>::new();
        for (entry, (g, outputs)) in manifest.recordings.iter().zip(&predictions) {
            let (frame, notes) = &outputs[i];
            let a = with_notes(g, frame)?;
            let b = with_notes(g, notes)?;
            roundtrip(&b)?;
            for (annotation, path) in entry.references.iter().enumerate() {
                let reference: Reference = read(path)?;
                let baseline = vb::score_exported_notes(&reference, g)?;
                let rounded = vb::score_exported_notes(&reference, &a)?;
                let candidate = vb::score_exported_notes(&reference, &b)?;
                ensure!(
                    rounded.detected_voiced_frames == candidate.detected_voiced_frames
                        && rounded.false_voiced_frames == candidate.false_voiced_frames,
                    "Segmentation changed accepted voicing coverage"
                );
                let key = format!("{}/A{}", entry.group, annotation + 1);
                let s = groups.entry(key).or_default();
                s.count += 1;
                for k in 0..7 {
                    s.baseline[k] += rates(&baseline)[k];
                    s.frame_rounding[k] += rates(&rounded)[k];
                    s.segmental[k] += rates(&candidate)[k];
                }
                details.push(json!({"id":entry.id,"annotation":annotation+1,"recipe_index":i,"baseline":baseline,"frame_rounding":rounded,"segmental":candidate}));
            }
        }
        for s in groups.values_mut() {
            for k in 0..7 {
                s.baseline[k] /= s.count as f64;
                s.frame_rounding[k] /= s.count as f64;
                s.segmental[k] /= s.count as f64;
            }
        }
        summaries.push(json!({"recipe":recipe,"groups":groups}));
    }
    let report = json!({"status":"exposed_development_comparison_not_held_out_promotion","rates_order":["correct/reference_voiced","voiced_recall","wrong/detected_voiced","false/reference_unvoiced","event_precision","event_recall","event_F1"],"midi_roundtrip_verified":true,"summary":summaries,"details":details});
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output.join("evaluation.json"))?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report["summary"])?);
    Ok(())
}
