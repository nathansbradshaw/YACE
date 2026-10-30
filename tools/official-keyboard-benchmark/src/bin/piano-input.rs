use anyhow::{Context, Result, ensure};
use chart_engine::{
    keyboard_audio,
    keyboard_detection::{self, KeyboardSound},
    mapping::Transcription,
    models,
    stem_review::StemManifest,
};
use serde_json::json;
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a = std::env::args().collect::<Vec<_>>();
    let source = Path::new(a.get(1).context("source")?);
    let manifest = StemManifest::read(Path::new(a.get(2).context("manifest")?))?;
    let model_dir = Path::new(a.get(3).context("model dir")?);
    let output = Path::new(a.get(4).context("new output")?);
    ensure!(!output.exists(), "output exists");
    let evidence = keyboard_detection::detect(source, &manifest, model_dir, KeyboardSound::Piano)?;
    let t = Transcription {
        piano: evidence.raw_notes.clone(),
        pro_piano: evidence.raw_notes.clone(),
        keyboard_detection: Some(evidence.clone()),
        isolation: Some(manifest),
        ..Default::default()
    };
    t.validate()?;
    if let Some(receipt) = &evidence.piano_audio {
        let prepared = keyboard_audio::prepare(source, t.isolation.as_ref().unwrap())?;
        ensure!(
            prepared.receipt == *receipt,
            "nonrepeatable input preparation"
        );
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(
            &json!({"kind":"piano","audio_sha256":models::hash_file(source)?,"notes":evidence.raw_notes,"evidence":evidence,"transcription":t}),
        )?,
    )?;
    Ok(())
}
