//! The desktop Rust acoustic engine and word decoder, on a bounded worker.
use anyhow::{Context, Result, ensure};
use std::{cell::RefCell, path::Path};

pub const MODEL_ID: &str = "vocal-alignment-english";
pub fn asset() -> crate::models::ModelAsset {
    crate::models::ModelAsset {
        id: MODEL_ID.into(),
        filename: "model.safetensors".into(),
        url: format!(
            "https://huggingface.co/facebook/wav2vec2-base-960h/resolve/{}/model.safetensors",
            crate::vocal_alignment::REVISION
        ),
        sha256: crate::vocal_alignment::assets()[2].sha256.into(),
        license: "Apache-2.0; facebook/wav2vec2-base-960h".into(),
    }
}
pub async fn align(
    audio: &Path,
    stem: &Path,
    binding: &crate::clean::StemAlignment,
    lyrics: &crate::vocal_generation::LyricSource,
    progress: &mut impl FnMut(String),
) -> Result<crate::vocal_alignment::Alignment> {
    let source_hash = crate::models::hash_file(audio)?;
    let stem_hash = crate::models::hash_file(stem)?;
    let duration = crate::models::duration(audio)?;
    ensure!(
        binding.audio_sha256 == source_hash
            && binding.stem_sha256 == stem_hash
            && !binding.evidence.trim().is_empty(),
        "Acoustic alignment source changed"
    );
    ensure!(
        crate::models::format(stem)? == (44100, 2)
            && (crate::models::duration(stem)? - duration).abs() <= 1. / 44100.,
        "Vocal stem clock differs"
    );
    ensure!(
        !lyrics.source.trim().is_empty(),
        "Supplied lyrics need provenance"
    );
    let config = include_bytes!("../../assets/vocal-alignment/config.json");
    let vocab = include_bytes!("../../assets/vocal-alignment/vocab.json");
    use sha2::{Digest, Sha256};
    for (bytes, asset) in [config.as_slice(), vocab.as_slice()]
        .into_iter()
        .zip(crate::vocal_alignment::assets())
    {
        ensure!(
            format!("{:x}", Sha256::digest(bytes)) == asset.sha256,
            "Acoustic configuration changed"
        );
    }
    let vocabulary = serde_json::from_slice(vocab)?;
    let samples = crate::models::decode(stem, 16000, 1)?;
    let weights = crate::model_assets::load(MODEL_ID, progress).await?;
    // A worker has one thread. Rayon must use it rather than trying to spawn
    // OS threads, which static wasm32 does not provide.
    thread_local! { static INITIALIZED: std::cell::Cell<bool> = const {std::cell::Cell::new(false)}; }
    INITIALIZED.with(|initialized| -> Result<()> {
        if !initialized.get() {
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .use_current_thread()
                .build_global()
                .context("Initializing acoustic CPU worker")?;
            initialized.set(true);
        }
        Ok(())
    })?;
    progress("Loading local English acoustic alignment…".into());
    let model = practice_acoustic::AcousticModel::from_bytes(config, weights)
        .map_err(|error| anyhow::anyhow!("Loading acoustic model: {error}"))?;
    let progress = RefCell::new(progress);
    let emissions = model
        .emissions_cancellable(&samples, &|| false, &|position, length| {
            progress.borrow_mut()(format!(
                "Analyzing sung-word timing: {}%…",
                position.saturating_mul(100) / length.max(1)
            ))
        })
        .map_err(|error| anyhow::anyhow!("Acoustic lyric alignment: {error}"))?;
    progress.borrow_mut()("Checking acoustic word boundaries…".into());
    let (words, diagnostics, selection) = crate::vocal_alignment::align_emissions_with_selection(
        &emissions,
        &vocabulary,
        lyrics,
        &source_hash,
        duration,
        &|| false,
    )?;
    ensure!(
        crate::models::hash_file(audio)? == source_hash
            && crate::models::hash_file(stem)? == stem_hash,
        "Acoustic source changed during inference"
    );
    let mut warnings=vec!["Acoustic word timing is model-estimated. The English speech model is not singing-specific; repeated choruses, long vowels, missing lines and backing vocals require review. Uncertain words retain proposed bounds only in diagnostics and are excluded from playable timing.".into()];
    if lyrics.synced_lyrics.is_some() && !selection.used_guide {
        warnings.push(format!(
            "Provider line timing was not selected: {}",
            selection.reason
        ));
    }
    Ok(crate::vocal_alignment::Alignment {
        version: 1,
        language: "en".into(),
        model_revision: crate::vocal_alignment::REVISION.into(),
        model_sha256: asset().sha256,
        vocal_stem_sha256: stem_hash,
        words,
        diagnostics,
        warnings,
        selection: Some(selection),
    })
}
