use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_generation::Generation};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::{fs, path::Path};
use vocal_boundary_benchmark::{read, spectral};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().collect();
    ensure!(
        a.len() == 3,
        "vocal-spectral-evidence SOURCE_INVENTORY NEW_ROOT"
    );
    let manifest: Value = read(&a[1])?;
    let out = Path::new(&a[2]);
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    let mut pins = vec![];
    let mut ids = BTreeSet::new();
    for e in manifest["recordings"].as_array().context("recordings")? {
        let id = e["id"].as_str().context("id")?;
        ensure!(
            !id.is_empty()
                && id.len() <= 80
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(id.to_owned()),
            "Invalid/duplicate ID"
        );
        let gp = Path::new(e["generation"].as_str().context("generation")?);
        let g: Generation = read(gp)?;
        let audio = e["vocal_stem"]
            .as_str()
            .context("Explicit vocal_stem path required")?;
        ensure!(
            models::hash_file(Path::new(&audio))? == g.vocal_stem_sha256,
            "Stem source mismatch"
        );
        let cue_path = Path::new(e["game_features"].as_str().context("GAME")?);
        let cue: Value = read(cue_path)?;
        ensure!(
            cue["source_sha256"] == g.vocal_stem_sha256 && cue["timestep"] == 0.01,
            "GAME source/clock"
        );
        let raw = g.rmvpe_pitch.as_ref().context("Raw pitch")?;
        ensure!(
            raw.audio_sha256 == g.audio_sha256 && raw.vocal_stem_sha256 == g.vocal_stem_sha256,
            "Raw source"
        );
        let times: Vec<_> = raw.frames.iter().map(|f| f.time_seconds).collect();
        let pitches: Vec<_> = raw
            .frames
            .iter()
            .map(|f| {
                f.frequency_hz
                    .filter(|_| f.confidence >= 0.03 && f.rms >= 0.0015)
            })
            .collect();
        let confidence: Vec<_> = raw.frames.iter().map(|f| f.confidence as f64).collect();
        let voice: Vec<_> = (0..times.len())
            .map(|i| cue["rows"][i]["presence_confidence"].as_f64().unwrap_or(0.))
            .collect();
        let game_rows = cue["rows"].as_array().context("GAME rows")?;
        ensure!(
            cue["frame_kind"] == "interval_start"
                && game_rows.len().abs_diff(times.len()) <= 1
                && game_rows.iter().all(|r| r["presence_confidence"]
                    .as_f64()
                    .is_some_and(|v| v.is_finite() && (0.0..=1.).contains(&v))),
            "GAME values/clock coverage"
        );
        let pcm = models::decode(Path::new(&audio), 16000, 1)?;
        ensure!(
            (pcm.len() as f64 / 16000. - g.duration_seconds).abs() <= 1. / 16000. + 1e-8,
            "Decoded source clock"
        );
        let frames = spectral::extract(&pcm, &times, &pitches, &voice, &confidence)?;
        let file = format!("{id}.json");
        let bytes = serde_json::to_vec(
            &json!({"audio_sha256":g.audio_sha256,"vocal_stem_sha256":g.vocal_stem_sha256,"generation_sha256":models::hash_file(gp)?,"game_sha256":models::hash_file(cue_path)?,"times":times,"frames":frames}),
        )?;
        fs::write(out.join(&file), &bytes)?;
        pins.push(json!({"id":id,"file":file,"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        eprintln!("{id}: spectral features complete");
    }
    fs::write(
        out.join("inference-receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"reads_reference_labels":false,"schema":"spectral-v1-20-16khz-1024-hann-32mel-80-6000-source-template","implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("vocal-spectral-evidence.rs").as_slice(),include_bytes!("../spectral.rs").as_slice()].concat())),"executable_sha256":models::hash_file(&std::env::current_exe()?)?,"inputs":pins}),
        )?,
    )?;
    Ok(())
}
