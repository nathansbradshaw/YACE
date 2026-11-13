//! Lead/backing proposals used as evidence, never as singer-identity truth.
use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_generation::Generation};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use vocal_boundary_benchmark::read;
#[path = "../../../../src/vocal_separation.rs"]
mod separation;
fn mono(pcm: Vec<f32>) -> Vec<f32> {
    pcm.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect()
}
fn prefix(p: &[f32]) -> Vec<f64> {
    let mut sum = 0.;
    let mut x = vec![0.];
    for &p in p {
        sum += (p as f64).powi(2);
        x.push(sum);
    }
    x
}
fn rms(p: &[f64], i: usize) -> f64 {
    let a = i.saturating_sub(882).min(p.len() - 1);
    let b = (i + 883).min(p.len() - 1);
    if a < b {
        ((p[b] - p[a]).max(0.) / (b - a) as f64).sqrt()
    } else {
        0.
    }
}
fn corr(p: &[f32], center: usize, lag: f64) -> f64 {
    let a = center.saturating_sub(1024);
    let b = (center + 1024).min(p.len().saturating_sub(lag.ceil() as usize + 1));
    if a + 16 >= b {
        return 0.;
    }
    let mut n = 0.;
    let (mut x, mut y, mut xx, mut yy, mut xy) = (0., 0., 0., 0., 0.);
    for i in (a..b).step_by(4) {
        let at = i as f64 + lag;
        let j = at.floor() as usize;
        let v = p[j] as f64 + (p[j + 1] - p[j]) as f64 * at.fract();
        let u = p[i] as f64;
        n += 1.;
        x += u;
        y += v;
        xx += u * u;
        yy += v * v;
        xy += u * v;
    }
    let denom = ((xx - x * x / n).max(0.) * (yy - y * y / n).max(0.)).sqrt();
    if denom > 1e-12 {
        ((xy - x * y / n) / denom).clamp(-1., 1.)
    } else {
        0.
    }
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().collect();
    ensure!(
        a.len() == 4,
        "vocal-lead-evidence MANIFEST MODEL NEW_OUTPUT"
    );
    let manifest: Value = read(&a[1])?;
    let model = Path::new(&a[2]);
    ensure!(
        models::hash_file(model)? == separation::MODEL_SHA256,
        "Model checksum"
    );
    let out = Path::new(&a[3]);
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let runtime = std::env::var_os("ORT_DYLIB_PATH").context("Explicit native runtime")?;
    ort::init_from(runtime.to_string_lossy()).commit()?;
    let mut pins = vec![];
    let mut ids = std::collections::BTreeSet::new();
    let entries = manifest["recordings"].as_array().context("recordings")?;
    ensure!(
        (1..=64).contains(&entries.len()),
        "Recording budget exceeded"
    );
    for e in entries {
        let id = e["id"].as_str().context("id")?;
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "Invalid ID"
        );
        ensure!(ids.insert(id), "Duplicate ID");
        let g: Generation = read(e["generation"].as_str().context("generation")?)?;
        let raw = g.rmvpe_pitch.as_ref().context("Raw source")?;
        ensure!(
            (0.1..=360.).contains(&g.duration_seconds)
                && raw.frames.len() >= 2
                && raw.frames.len() <= 36_001
                && raw.frames.iter().all(|f| f.time_seconds.is_finite()
                    && f.time_seconds >= 0.
                    && f.time_seconds <= g.duration_seconds + 0.01),
            "Source frame/duration budget exceeded"
        );
        ensure!(
            raw.audio_sha256 == g.audio_sha256 && raw.vocal_stem_sha256 == g.vocal_stem_sha256,
            "Raw source mismatch"
        );
        let (lead, backing, origin) = if let Some(cache) = e["separation_cache"].as_str() {
            let root = Path::new(cache);
            let s: Value = read(root.join("separation.json"))?;
            let lp = root.join("lead.wav");
            let bp = root.join("backing.wav");
            ensure!(
                s["vocal_stem_sha256"] == g.vocal_stem_sha256
                    && s["model_sha256"] == separation::MODEL_SHA256
                    && s["lead_sha256"] == models::hash_file(&lp)?
                    && s["backing_sha256"] == models::hash_file(&bp)?
                    && s["sample_rate"] == 44100
                    && s["channels"] == 2,
                "Cached separation lineage mismatch"
            );
            let l = mono(models::decode(&lp, 44100, 2)?);
            let b = mono(models::decode(&bp, 44100, 2)?);
            (
                l,
                b,
                json!({"method":"verified_cached_separation","separation_receipt_sha256":models::hash_file(&root.join("separation.json"))?,"lead_sha256":s["lead_sha256"],"backing_sha256":s["backing_sha256"]}),
            )
        } else {
            let source = e["vocal_stem"].as_str().context("Missing vocal_stem")?;
            ensure!(
                models::hash_file(Path::new(source))? == g.vocal_stem_sha256,
                "Vocal stem mismatch"
            );
            let pcm = models::decode(Path::new(source), 44100, 2)?;
            let (l, b) = separation::separate(&pcm, model)?;
            (
                mono(l),
                mono(b),
                json!({"method":"same_pinned_native_separator_in_memory","source_sha256":g.vocal_stem_sha256}),
            )
        };
        ensure!(
            lead.len() == backing.len()
                && (lead.len() as f64 / 44100. - g.duration_seconds).abs() <= 1. / 16000.
                && lead.iter().chain(&backing).all(|v| v.is_finite()),
            "Separated PCM clock/values differ"
        );
        let lp = prefix(&lead);
        let bp = prefix(&backing);
        let mut rows = vec![];
        for f in &raw.frames {
            let at = (f.time_seconds * 44100.).round() as usize;
            let l = rms(&lp, at);
            let b = rms(&bp, at);
            let pitch = f
                .frequency_hz
                .filter(|_| f.confidence >= 0.03 && f.rms >= 0.0015)
                .filter(|h| h.is_finite() && *h > 0.)
                .filter(|h| (35.5..84.5).contains(&(69. + 12. * (*h / 440.).log2())));
            let mut x = vec![
                20. * l.max(1e-8).log10(),
                20. * b.max(1e-8).log10(),
                (20. * (l.max(1e-8) / b.max(1e-8)).log10()).clamp(-60., 60.),
                l / (l + b).max(1e-8),
            ];
            for scale in [1., 2., 0.5] {
                let (c, d) = pitch.map_or((0., 0.), |h| {
                    (
                        corr(&lead, at, 44100. / h * scale),
                        corr(&backing, at, 44100. / h * scale),
                    )
                });
                x.extend([c, d]);
            }
            x.extend([x[4] - x[5], f64::from(pitch.is_some())]);
            ensure!(
                x.len() == 12 && x.iter().all(|v| v.is_finite()),
                "Feature values"
            );
            rows.push(json!({"time_seconds":f.time_seconds,"x":x}));
        }
        let file = format!("{id}.json");
        let bytes = serde_json::to_vec(
            &json!({"status":"EXPERIMENTAL_LEAD_BACKING_EVIDENCE_NOT_IDENTITY_TRUTH_NOT_PROMOTION","audio_sha256":g.audio_sha256,"vocal_stem_sha256":g.vocal_stem_sha256,"duration_seconds":g.duration_seconds,"model_sha256":separation::MODEL_SHA256,"origin":origin,"sample_rate":44100,"rms_radius_samples":882,"correlation_window_samples":2048,"correlation_stride":4,"lag_multipliers":[1.,2.,0.5],"frames":rows,"implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("vocal-lead-evidence.rs").as_slice(),include_bytes!("../../../../src/vocal_separation.rs").as_slice()].concat())),"executable_sha256":models::hash_file(&std::env::current_exe()?)?}),
        )?;
        fs::write(out.join(&file), &bytes)?;
        pins.push(json!({"id":id,"file":file,"sha256":format!("{:x}",Sha256::digest(&bytes)),"generation_sha256":models::hash_file(Path::new(e["generation"].as_str().unwrap()))?}));
        println!("{id} lead/backing evidence complete");
    }
    fs::write(
        out.join("inference-receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"reads_reference_labels":false,"model_sha256":separation::MODEL_SHA256,"inputs":pins}),
        )?,
    )?;
    let mut next = manifest.clone();
    for e in next["recordings"].as_array_mut().unwrap() {
        let f = out
            .join(format!("{}.json", e["id"].as_str().unwrap()))
            .canonicalize()?;
        let m = e.as_object_mut().unwrap();
        m.remove("acoustic_features");
        m.remove("boundary_features");
        m.insert("role_features".into(), json!(f));
    }
    fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&next)?)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlation_at_the_candidate_pitch_distinguishes_a_different_source() {
        let lead: Vec<_> = (0..44100)
            .map(|i| (std::f64::consts::TAU * 440. * i as f64 / 44100.).sin() as f32)
            .collect();
        let backing: Vec<_> = (0..44100)
            .map(|i| (std::f64::consts::TAU * 660. * i as f64 / 44100.).sin() as f32)
            .collect();
        assert!(corr(&lead, 22050, 44100. / 440.) > 0.999);
        assert!(corr(&backing, 22050, 44100. / 440.) < -0.999);
        assert!(corr(&backing, 22050, 2. * 44100. / 440.) > 0.999);
    }
    #[test]
    fn rms_and_silence_are_finite_at_the_recording_edges() {
        let p = prefix(&vec![0.2; 4410]);
        for i in [0, 2205, 4410] {
            assert!((rms(&p, i) - 0.2).abs() < 1e-7);
        }
        assert_eq!(corr(&vec![0.; 4410], 0, 100.), 0.);
        assert_eq!(corr(&vec![0.; 4410], 4410, 100.), 0.);
    }
}
