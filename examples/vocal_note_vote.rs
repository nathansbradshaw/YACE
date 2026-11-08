//! Development-only note-interval voting. Authored targets never select a pitch.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::{Corpus, Reference, score},
    vocal_generation::{Generation, PitchSupport},
    vocal_pitch::Evidence,
    vocals::VocalTone,
};
use clap::Parser;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
#[derive(Parser)]
struct Args {
    corpus: PathBuf,
    root: PathBuf,
    notes: PathBuf,
    output: PathBuf,
}
fn midi(h: Option<f64>) -> Option<f64> {
    h.filter(|h| *h > 0.)
        .map(|h| 69. + 12. * (h / 440.).log2())
        .filter(|p| (35.5..84.5).contains(p))
}
fn view(mut r: Generation) -> Generation {
    let t = r.timing.as_ref().unwrap();
    for f in &mut r.pitch_frames {
        f.midi = r
            .chart
            .as_ref()
            .and_then(|ch| {
                ch.notes.iter().find(|n| {
                    t.seconds(n.start) <= f.time_seconds && f.time_seconds < t.seconds(n.end)
                })
            })
            .and_then(|n| match n.tone {
                VocalTone::Pitched { midi } => Some(midi as f64),
                _ => None,
            })
    }
    r
}
fn vote(pitches: &[Option<f64>], fraction: f64) -> Option<u8> {
    let mut hist = [0usize; 49];
    for p in pitches.iter().flatten() {
        let p = p.round() as isize - 36;
        if (0..49).contains(&p) {
            hist[p as usize] += 1
        }
    }
    let at = (0..49).max_by_key(|&i| hist[i])?;
    (hist[at] >= 3 && hist[at] as f64 >= pitches.len() as f64 * fraction).then_some(at as u8 + 36)
}
fn main() -> Result<()> {
    let a = Args::parse();
    ensure!(!a.output.exists(), "Output exists");
    let corpus: Corpus = serde_json::from_slice(&fs::read(&a.corpus)?)?;
    let mut predictions = vec![];
    for r in corpus
        .recordings
        .into_iter()
        .filter(|r| r.split == "development")
    {
        let original: Generation =
            serde_json::from_slice(&fs::read(r.candidate.context("High")?)?)?;
        let p = original.rmvpe_pitch.as_ref().context("RMVPE")?;
        let s: Evidence =
            serde_json::from_slice(&fs::read(a.root.join(format!("{}-swift.json", r.id)))?)?;
        let n: Value =
            serde_json::from_slice(&fs::read(a.notes.join(format!("{}.notes.json", r.id)))?)?;
        let c: Value =
            serde_json::from_slice(&fs::read(a.root.join(format!("{}-fcpe-raw.json", r.id)))?)?;
        ensure!(
            s.vocal_stem_sha256 == original.vocal_stem_sha256
                && s.pcm_sha256 == p.pcm_sha256
                && n["stem_sha256"].as_str() == Some(&original.vocal_stem_sha256)
                && c["pcm_sha256"].as_str() == Some(&p.pcm_sha256),
            "Source/PCM differs"
        );
        let t = original.timing.as_ref().context("Timing")?;
        for method in [
            "ros_swift",
            "ros_fcpe",
            "swift_rmvpe",
            "three_f0",
            "four_vote",
        ] {
            for fraction in [0.5, 0.7, 0.9] {
                let mut candidate = original.clone();
                let mut changes = vec![];
                for (index, note) in candidate
                    .chart
                    .as_mut()
                    .context("Chart")?
                    .notes
                    .iter_mut()
                    .enumerate()
                {
                    let VocalTone::Pitched { midi: old } = note.tone else {
                        continue;
                    };
                    let start = t.seconds(note.start);
                    let end = t.seconds(note.end);
                    let rows: Vec<_> = p
                        .frames
                        .iter()
                        .filter(|f| start <= f.time_seconds && f.time_seconds < end)
                        .collect();
                    let mut rp = vec![];
                    let mut sp = vec![];
                    let mut np = vec![];
                    let mut fp = vec![];
                    for f in rows {
                        rp.push(midi(f.frequency_hz.filter(|_| f.confidence > 0.03)));
                        let sf = &s.frames
                            [((f.time_seconds / 0.016).round() as usize).min(s.frames.len() - 1)];
                        sp.push(midi(sf.frequency_hz.filter(|_| sf.confidence >= 0.5)));
                        np.push(
                            n["notes"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|n| {
                                    n["start_seconds"].as_f64().unwrap() <= f.time_seconds
                                        && f.time_seconds < n["end_seconds"].as_f64().unwrap()
                                })
                                .and_then(|n| n["midi"].as_f64()),
                        );
                        let fs = c["frames"].as_array().unwrap();
                        let ff = &fs[(((f.time_seconds - 0.005) / 0.01).round() as usize)
                            .min(fs.len() - 1)];
                        fp.push(midi(
                            ff["frequency_hz"]
                                .as_f64()
                                .filter(|_| ff["confidence"].as_f64().unwrap_or(0.) > 0.006),
                        ))
                    }
                    let rv = vote(&rp, fraction);
                    let sv = vote(&sp, fraction);
                    let nv = vote(&np, fraction);
                    let fv = vote(&fp, fraction);
                    let agree = |a: Option<u8>, b: Option<u8>| {
                        a.zip(b).filter(|(a, b)| a == b).map(|(p, _)| p)
                    };
                    let selected = match method {
                        "ros_swift" => agree(nv, sv),
                        "ros_fcpe" => agree(nv, fv),
                        "swift_rmvpe" => agree(sv, rv),
                        "three_f0" => agree(sv, rv)
                            .or_else(|| agree(fv, rv))
                            .or_else(|| agree(sv, fv)),
                        _ => {
                            let v = [rv, sv, nv, fv];
                            let mut h = [0; 49];
                            for p in v.into_iter().flatten() {
                                h[(p - 36) as usize] += 1
                            }
                            let i = (0..49).max_by_key(|&i| h[i]).unwrap();
                            (h[i] >= 3).then_some(i as u8 + 36)
                        }
                    };
                    if let Some(selected) = selected.filter(|s| *s != old) {
                        note.tone = VocalTone::Pitched { midi: selected };
                        changes.push(json!({"note":index,"before":old,"after":selected,"start":start,"end":end,"votes":[rv,sv,nv,fv]}));
                        for f in &mut candidate.pitch_frames {
                            if start <= f.time_seconds && f.time_seconds < end && f.midi.is_some() {
                                f.midi = Some(selected as f64);
                                f.support = PitchSupport::AcousticAlternative;
                                f.clarity = f.clarity.min(0.5)
                            }
                        }
                    }
                }
                predictions.push((
                    r.id.clone(),
                    method,
                    fraction,
                    original.clone(),
                    candidate,
                    changes,
                    r.reference.clone().context("Reference")?,
                ));
            }
        }
    }
    let mut rows = vec![];
    for (id, method, fraction, baseline, candidate, changes, path) in predictions {
        let reference: Reference = serde_json::from_slice(&fs::read(path)?)?;
        let br = score(&reference, &baseline)?;
        let cr = score(&reference, &candidate)?;
        let bc = score(&reference, &view(baseline))?;
        let cc = score(&reference, &view(candidate))?;
        rows.push(json!({"id":id,"method":method,"fraction":fraction,"baseline_raw":br,"candidate_raw":cr,"baseline_chart":bc,"candidate_chart":cc,"changes":changes}));
    }
    fs::write(a.output, serde_json::to_vec(&rows)?)?;
    Ok(())
}
