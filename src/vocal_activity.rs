//! Pinned speech-activity research evidence. This model does not select pitches
//! or recognize lyrics; speech scores are not calibrated singing probabilities.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MODEL_ID: &str = "silero-vad-16k-op15";
pub const REVISION: &str = "066409f3d9db45a30d21dfcbf9147826ffdd7052";
pub const MODEL_SHA256: &str = "7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49";
pub const MODEL_URL: &str = "https://raw.githubusercontent.com/snakers4/silero-vad/066409f3d9db45a30d21dfcbf9147826ffdd7052/src/silero_vad/data/silero_vad_16k_op15.onnx";
pub const LICENSE_SHA256: &str = "2e63e9a38b6e8fc0c7bc37ce174caca1862870856c6daf5697cfb785e925520b";
pub const RATE: usize = 16_000;
pub const HOP: usize = 512;
pub const CONTEXT: usize = 64;
pub const STATE: usize = 256;
const MAX_SECONDS: usize = 3600;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub version: u32,
    pub model_id: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub implementation_sha256: String,
    pub source_sha256: String,
    pub pcm_sha256: String,
    pub sample_rate: usize,
    pub hop_samples: usize,
    pub context_samples: usize,
    pub sample_count: usize,
    pub duration_seconds: f64,
    /// One score for each source-clock block, including a padded final block.
    pub probabilities: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub version: String,
    pub library_sha256: String,
    pub publisher_source_sha256: String,
}

pub fn implementation_hash() -> String {
    let mut hash = Sha256::new();
    hash.update(include_bytes!("vocal_activity.rs"));
    hash.update(include_bytes!("models.rs"));
    hash.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", hash.finalize())
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn validate(e: &Evidence, source_sha256: &str, duration_seconds: f64) -> Result<()> {
    if let Some(runtime) = &e.runtime {
        ensure!(
            !runtime.version.trim().is_empty()
                && runtime.version.len() <= 128
                && valid_sha(&runtime.library_sha256)
                && valid_sha(&runtime.publisher_source_sha256),
            "Invalid activity runtime identity"
        );
    }
    ensure!(
        e.version == 1
            && e.model_id == MODEL_ID
            && e.model_revision == REVISION
            && e.model_sha256 == MODEL_SHA256
            && e.implementation_sha256 == implementation_hash(),
        "Vocal activity model or implementation differs"
    );
    ensure!(
        valid_sha(&e.source_sha256)
            && valid_sha(&e.pcm_sha256)
            && e.source_sha256.eq_ignore_ascii_case(source_sha256)
            && e.sample_rate == RATE
            && e.hop_samples == HOP
            && e.context_samples == CONTEXT
            && (1..=RATE * MAX_SECONDS).contains(&e.sample_count)
            && duration_seconds.is_finite()
            && e.duration_seconds.is_finite()
            && (e.duration_seconds - e.sample_count as f64 / RATE as f64).abs() < 1e-9
            && (e.duration_seconds - duration_seconds).abs() <= 1. / RATE as f64 + 1e-9,
        "Vocal activity source or recording clock differs"
    );
    ensure!(
        e.probabilities.len() == e.sample_count.div_ceil(HOP)
            && e.probabilities
                .iter()
                .all(|p| p.is_finite() && (0. ..=1.).contains(p)),
        "Invalid vocal activity scores"
    );
    Ok(())
}

/// The caller supplies the same graph interface on native and browser runtimes.
/// State belongs to this recording; neither model state nor context crosses it.
pub fn infer_pcm(
    pcm: &[f32],
    source_sha256: &str,
    mut run: impl FnMut(&[f32], &[f32]) -> Result<(f32, Vec<f32>)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Evidence> {
    ensure!(
        !pcm.is_empty()
            && pcm.len() <= RATE * MAX_SECONDS
            && pcm.iter().all(|p| p.is_finite())
            && valid_sha(source_sha256),
        "Activity analysis requires finite source-bound PCM within one hour"
    );
    let mut hash = Sha256::new();
    for p in pcm {
        hash.update(p.to_le_bytes());
    }
    let mut state = vec![0.; STATE];
    let mut context = [0.; CONTEXT];
    let mut probabilities = Vec::with_capacity(pcm.len().div_ceil(HOP));
    for chunk in pcm.chunks(HOP) {
        ensure!(!cancelled(), "Vocal activity analysis cancelled");
        let mut input = [0.; CONTEXT + HOP];
        input[..CONTEXT].copy_from_slice(&context);
        input[CONTEXT..CONTEXT + chunk.len()].copy_from_slice(chunk);
        let (score, next_state) = run(&input, &state)?;
        ensure!(
            score.is_finite()
                && (0. ..=1.).contains(&score)
                && next_state.len() == STATE
                && next_state.iter().all(|v| v.is_finite()),
            "Invalid vocal activity graph output"
        );
        context.copy_from_slice(&input[HOP..]);
        state = next_state;
        probabilities.push(score);
    }
    ensure!(!cancelled(), "Vocal activity analysis cancelled");
    let e = Evidence {
        version: 1,
        model_id: MODEL_ID.into(),
        model_revision: REVISION.into(),
        model_sha256: MODEL_SHA256.into(),
        implementation_sha256: implementation_hash(),
        source_sha256: source_sha256.into(),
        pcm_sha256: format!("{:x}", hash.finalize()),
        sample_rate: RATE,
        hop_samples: HOP,
        context_samples: CONTEXT,
        sample_count: pcm.len(),
        duration_seconds: pcm.len() as f64 / RATE as f64,
        probabilities,
        runtime: None,
    };
    validate(&e, source_sha256, e.duration_seconds)?;
    Ok(e)
}

/// Original-clock support around each requested time; no reference alignment,
/// segmentation, pitch changes or lyric decisions take place here.
pub fn support(e: &Evidence, times: &[f64], padding_seconds: f64) -> Result<Vec<f32>> {
    validate(e, &e.source_sha256, e.duration_seconds)?;
    ensure!(
        padding_seconds.is_finite()
            && (0. ..=2.).contains(&padding_seconds)
            && times
                .iter()
                .all(|t| t.is_finite() && *t >= 0. && *t <= e.duration_seconds + 1. / RATE as f64),
        "Invalid activity comparison times or padding"
    );
    Ok(times
        .iter()
        .map(|&time| {
            let lo = (((time - padding_seconds).max(0.) * RATE as f64 / HOP as f64).floor()
                as usize)
                .min(e.probabilities.len());
            let hi = (((time + padding_seconds) * RATE as f64 / HOP as f64).floor() as usize + 1)
                .min(e.probabilities.len());
            e.probabilities[lo..hi.max(lo)]
                .iter()
                .copied()
                .fold(0., f32::max)
        })
        .collect())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn analyze(
    source: &std::path::Path,
    model: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Evidence> {
    use ort::{session::Session, value::Tensor};
    ensure!(!cancelled(), "Vocal activity analysis cancelled");
    ensure!(
        crate::models::hash_file(model)? == MODEL_SHA256,
        "Activity model checksum differs; another model was not substituted"
    );
    let metadata = crate::pipeline::metadata(source)?;
    ensure!(
        metadata.duration.is_finite()
            && metadata.duration > 0.
            && metadata.duration <= MAX_SECONDS as f64,
        "Activity analysis requires audio within one hour"
    );
    let source_sha256 = crate::models::hash_file(source)?;
    let stereo = crate::models::decode(source, RATE as u32, 2)?;
    ensure!(stereo.len().is_multiple_of(2), "Invalid stereo PCM");
    let pcm = stereo
        .chunks_exact(2)
        .map(|p| (p[0] + p[1]) * 0.5)
        .collect::<Vec<_>>();
    drop(stereo);
    ensure!(!cancelled(), "Vocal activity analysis cancelled");
    let mut session = Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_file(model)?;
    ensure!(
        session.inputs.len() == 3
            && session
                .inputs
                .iter()
                .all(|i| ["input", "state", "sr"].contains(&i.name.as_str()))
            && session.outputs.len() == 2,
        "Unexpected activity model interface"
    );
    let evidence = infer_pcm(
        &pcm,
        &source_sha256,
        |input, state| {
            let outputs = session.run(ort::inputs![
                "input" => Tensor::from_array(([1, CONTEXT + HOP], input.to_vec()))?,
                "state" => Tensor::from_array(([2, 1, 128], state.to_vec()))?,
                "sr" => Tensor::from_array((Vec::<i64>::new(), vec![RATE as i64]))?
            ])?;
            let (shape, p) = outputs[0].try_extract_tensor::<f32>()?;
            ensure!(
                shape.as_ref() == [1, 1] && p.len() == 1,
                "Invalid activity output shape"
            );
            let (shape, state) = outputs[1].try_extract_tensor::<f32>()?;
            ensure!(
                shape.as_ref() == [2, 1, 128],
                "Invalid activity state shape"
            );
            Ok((p[0], state.to_vec()))
        },
        cancelled,
    )?;
    ensure!(
        crate::models::hash_file(source)? == source_sha256
            && crate::models::hash_file(model)? == MODEL_SHA256,
        "Activity source or model changed during inference"
    );
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sha() -> String {
        "a".repeat(64)
    }

    #[test]
    fn context_state_and_partial_final_block_keep_the_source_clock() {
        let pcm = (0..700).map(|i| i as f32 / 1000.).collect::<Vec<_>>();
        let mut calls = 0;
        let e = infer_pcm(
            &pcm,
            &sha(),
            |input, state| {
                assert_eq!(input.len(), CONTEXT + HOP);
                assert_eq!(state.len(), STATE);
                if calls == 0 {
                    assert!(input[..CONTEXT].iter().all(|&p| p == 0.));
                    assert_eq!(&input[CONTEXT..], &pcm[..HOP]);
                    assert!(state.iter().all(|&p| p == 0.));
                } else {
                    assert_eq!(&input[..CONTEXT], &pcm[HOP - CONTEXT..HOP]);
                    assert_eq!(&input[CONTEXT..CONTEXT + 188], &pcm[HOP..]);
                    assert!(input[CONTEXT + 188..].iter().all(|&p| p == 0.));
                    assert!(state.iter().all(|&p| p == 1.));
                }
                calls += 1;
                Ok((0.75, vec![1.; STATE]))
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(e.probabilities, [0.75, 0.75]);
        assert_eq!(e.sample_count, 700);
        assert_eq!(e.duration_seconds, 700. / RATE as f64);
        let reread: Evidence = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
        validate(&reread, &sha(), e.duration_seconds).unwrap();
        assert!(validate(&reread, &"b".repeat(64), e.duration_seconds).is_err());
    }

    #[test]
    fn malformed_graph_output_and_cancellation_fail_without_evidence() {
        for pcm in [Vec::new(), vec![f32::NAN]] {
            assert!(
                infer_pcm(
                    &pcm,
                    &sha(),
                    |_, _| panic!("invalid audio before graph"),
                    &|| false
                )
                .is_err()
            );
        }
        for (p, state) in [
            (f32::NAN, vec![0.; STATE]),
            (0.5, vec![0.; STATE - 1]),
            (0.5, vec![f32::INFINITY; STATE]),
        ] {
            assert!(
                infer_pcm(&[0.; HOP], &sha(), |_, _| Ok((p, state.clone())), &|| false).is_err()
            );
        }
        assert!(
            infer_pcm(
                &[0.; HOP],
                &sha(),
                |_, _| panic!("cancelled before graph"),
                &|| true
            )
            .is_err()
        );
        let cancelled = std::cell::Cell::new(false);
        let calls = std::cell::Cell::new(0);
        assert!(
            infer_pcm(
                &[0.; HOP * 2],
                &sha(),
                |_, _| {
                    calls.set(calls.get() + 1);
                    cancelled.set(true);
                    Ok((0.5, vec![0.; STATE]))
                },
                &|| cancelled.get()
            )
            .is_err()
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn recordings_reset_state_and_support_never_retimes_or_fills_missing_blocks() {
        for _ in 0..2 {
            infer_pcm(
                &[0.; 16],
                &sha(),
                |input, state| {
                    assert!(input.iter().chain(state).all(|&p| p == 0.));
                    Ok((0., vec![42.; STATE]))
                },
                &|| false,
            )
            .unwrap();
        }
        let mut i = 0;
        let e = infer_pcm(
            &[0.; HOP * 3],
            &sha(),
            |_, _| {
                i += 1;
                Ok((if i == 2 { 1. } else { 0. }, vec![0.; STATE]))
            },
            &|| false,
        )
        .unwrap();
        assert_eq!(support(&e, &[0., 0.032, 0.064], 0.).unwrap(), [0., 1., 0.]);
        assert_eq!(support(&e, &[0.], 0.032).unwrap(), [1.]);
        assert!(support(&e, &[0.1], 0.).is_err());
        let mut corrupt = e.clone();
        corrupt.probabilities.pop();
        assert!(validate(&corrupt, &sha(), corrupt.duration_seconds).is_err());
    }
}
