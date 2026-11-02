//! The desktop's Rust separator, executed locally on a browser WebGPU worker.
use anyhow::{Context, Result, ensure};
use burn::backend::{
    Wgpu,
    wgpu::{WgpuDevice, graphics::WebGpu, init_setup_async},
};
use demucs_core::{Demucs, ModelOptions, listener::ForwardListener};
use sha2::{Digest, Sha256};
use std::path::Path;

pub async fn run(
    audio: &Path,
    output: &Path,
    progress: &mut impl FnMut(String),
) -> Result<crate::stem_review::StemManifest> {
    struct Progress<'a, F>(&'a mut F);
    impl<F: FnMut(String)> ForwardListener for Progress<'_, F> {
        fn on_event(&mut self, event: demucs_core::listener::ForwardEvent) {
            use demucs_core::listener::ForwardEvent;
            match event {
                ForwardEvent::ChunkStarted { index, total } => (self.0)(format!(
                    "Separating vocals and instruments · chunk {} of {total}",
                    index + 1
                )),
                ForwardEvent::ChunkDone { index, total } => {
                    (self.0)(format!("Separated {} of {total} chunks", index + 1))
                }
                _ => {}
            }
        }
    }
    let weights = crate::model_assets::load("separation", progress).await?;
    let model = Path::new("/models/htdemucs_6s.safetensors");
    crate::storage::write_owned(model, weights)?;
    progress("Preparing the browser GPU and native separation model".into());
    separate(audio, model, output, &mut Progress(progress)).await
}

/// Validate the exact existing checkpoint before allocating GPU model tensors.
pub fn validate_model(bytes: &[u8]) -> Result<()> {
    let expected = crate::models::assets()
        .into_iter()
        .find(|asset| asset.id == "separation")
        .context("Missing separation checkpoint identity")?;
    ensure!(
        format!("{:x}", Sha256::digest(bytes)) == expected.sha256,
        "The separation model does not match the pinned desktop checkpoint"
    );
    Ok(())
}

pub async fn separate(
    audio: &Path,
    model: &Path,
    output: &Path,
    listener: &mut impl ForwardListener,
) -> Result<crate::stem_review::StemManifest> {
    let weights = crate::storage::shared(model)?;
    validate_model(&weights)?;
    let stereo = crate::models::decode(audio, 44100, 2)?;
    ensure!(
        !stereo.is_empty()
            && stereo.len().is_multiple_of(2)
            && stereo.iter().all(|sample| sample.is_finite()),
        "Invalid source recording"
    );
    let left: Vec<_> = stereo.iter().step_by(2).copied().collect();
    let right: Vec<_> = stereo.iter().skip(1).step_by(2).copied().collect();
    drop(stereo);
    let device = WgpuDevice::default();
    // Synchronous device setup is unsupported on WASM. Initialize explicitly
    // before loading the same model implementation used by the native app.
    init_setup_async::<WebGpu>(&device, Default::default()).await;
    let runner = Demucs::<Wgpu>::from_bytes(ModelOptions::SixStem, &weights, device)
        .map_err(|error| anyhow::anyhow!("Cannot load separation model: {error}"))?;
    drop(weights);
    crate::storage::clear(model);
    let stems = runner
        .separate_with_listener(&left, &right, 44100, listener)
        .await
        .map_err(|error| anyhow::anyhow!("Separation failed: {error}"))?;
    let names: std::collections::BTreeSet<_> = stems.iter().map(|stem| stem.id.as_str()).collect();
    ensure!(
        names
            == ["drums", "bass", "other", "vocals", "guitar", "piano"]
                .into_iter()
                .collect(),
        "The separator did not return all six distinct parts"
    );
    ensure!(
        stems.iter().all(|stem| stem.left.len() == left.len()
            && stem.right.len() == right.len()
            && stem
                .left
                .iter()
                .chain(&stem.right)
                .all(|sample| sample.is_finite())),
        "The separator returned incomplete or nonfinite audio"
    );
    for stem in stems {
        crate::models::write_audio(
            &output.join(format!("{}.wav", stem.id.as_str())),
            &stem.left,
            &stem.right,
        )?;
    }
    let frames = left.len() as u64;
    drop(left);
    drop(right);
    let audio_hash = crate::models::hash_file(audio)?;
    let mut identity = crate::stem_review::SeparationIdentity {
        model_id: "separation".into(),
        model_sha256: crate::models::assets()
            .into_iter()
            .find(|asset| asset.id == "separation")
            .context("Missing separation identity")?
            .sha256,
        backend: "demucs-rs-six-stem-webgpu-v1".into(),
        parameters: std::collections::BTreeMap::from([
            ("variant".into(), "htdemucs_6s".into()),
            ("sample_rate".into(), "44100".into()),
            ("channels".into(), "2".into()),
            ("input".into(), "entire_source_no_trim_or_shift".into()),
            ("export".into(), "pcm_f32le_wav_no_normalization".into()),
            ("burn".into(), "0.20.1".into()),
            ("browser_code_sha256".into(), implementation_hash()),
            (
                "training_length".into(),
                demucs_core::TRAINING_LENGTH.to_string(),
            ),
            ("decoder".into(), "yace-browser-source-clock-v1".into()),
        ]),
        cache_key: String::new(),
    };
    identity.cache_key = identity.calculated_cache_key(&audio_hash)?;
    let assets = crate::stem_review::StemId::ALL
        .into_iter()
        .map(|id| {
            let path = output.join(format!("{}.wav", id.as_str()));
            Ok(crate::stem_review::StemAsset {
                id,
                sha256: crate::models::hash_file(&path)?,
                path,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let manifest = crate::stem_review::StemManifest {
        version:1, source_audio_sha256:audio_hash, sample_rate:44100, channels:2, frames,
        alignment:crate::stem_review::AlignmentEvidence {
            method:"Native Rust six-stem model on browser WebGPU".into(),
            details:"Same checkpoint and chunk/overlap implementation as desktop; all six finite outputs retain exactly the decoded source frame count".into(),
        },
        stems:assets, generation:Some(identity), conventional:None,
    };
    manifest.validate_binding(&manifest.source_audio_sha256, frames as f64 / 44100.)?;
    crate::models::remember_stems(&manifest)?;
    manifest.validate_source(audio, &manifest.source_audio_sha256, frames as f64 / 44100.)?;
    let mut portable = manifest.clone();
    for stem in &mut portable.stems {
        stem.path = stem.path.strip_prefix(output)?.to_owned();
    }
    crate::storage::write(
        output.join("stem-manifest.json"),
        serde_json::to_vec_pretty(&portable)?,
    )?;
    Ok(manifest)
}

pub fn implementation_hash() -> String {
    let mut hash = Sha256::new();
    let sources: &[&[u8]] = &[
        include_bytes!("separation.rs"),
        include_bytes!("audio_decode.rs"),
        include_bytes!("../Cargo.lock"),
        include_bytes!("../../vendor/demucs-core/src/lib.rs"),
        include_bytes!("../../vendor/demucs-core/src/dsp/stft.rs"),
        include_bytes!("../../vendor/demucs-core/src/dsp/spectrogram.rs"),
        include_bytes!("../../vendor/demucs-core/src/dsp/resample.rs"),
        include_bytes!("../../vendor/demucs-core/src/dsp/cac.rs"),
        include_bytes!("../../vendor/demucs-core/src/model/htdemucs.rs"),
        include_bytes!("../../vendor/demucs-core/src/model/metadata.rs"),
        include_bytes!("../../vendor/demucs-core/src/model/conv.rs"),
        include_bytes!("../../vendor/demucs-core/src/model/transformer.rs"),
        include_bytes!("../../vendor/demucs-core/src/weights/load.rs"),
        include_bytes!("../../vendor/demucs-core/src/weights/tensor_store.rs"),
    ];
    for source in sources {
        hash.update((source.len() as u64).to_le_bytes());
        hash.update(source);
    }
    format!("{:x}", hash.finalize())
}
