//! Existing MuScriptor native decoding with browser-local ONNX sessions.
use crate::{muscriptor::*, onnx::Runtime};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path};
pub async fn transcribe(audio: &Path, progress: &mut impl FnMut(String)) -> Result<Output> {
    let samples = crate::models::decode(audio, SAMPLE_RATE as u32, 1)?;
    ensure!(
        !samples.is_empty() && samples.iter().all(|s| s.is_finite()),
        "MuScriptor requires finite nonempty audio"
    );
    let options = Options {
        instrument_groups: vec![32],
        ..Default::default()
    };
    let ids = vec![34i64];
    let conditioner = Runtime::load("muscriptor-cond", progress, false).await?;
    let lm = Runtime::load("muscriptor-lm", progress, false).await?;
    let duration = samples.len() as f64 / SAMPLE_RATE as f64;
    let mut decoder = NoteDecoder::new();
    let mut chunks = Vec::new();
    let instrument = Runtime::int_tensor(&ids, &[1, ids.len()])?;
    let dataset = Runtime::int_tensor(&[1], &[1, 1])?;
    let empty_prefix = Runtime::float_tensor(&[], &[1, 0, 768])?;
    for (index, part) in samples.chunks(CHUNK_SAMPLES).enumerate() {
        let timer = js_sys::Date::now();
        let start = index as f64 * 5.;
        let end = (start + 5.).min(duration);
        progress(format!(
            "Detecting Synth · {:.0}/{duration:.0} seconds",
            end
        ));
        decoder.begin_chunk(start, end)?;
        let forced = if options.force_ties && index > 0 {
            decoder.tie_tokens()
        } else {
            vec![]
        };
        for &token in &forced {
            decoder.feed(token);
        }
        let mut padded = part.to_vec();
        padded.resize(CHUNK_SAMPLES, 0.);
        let audio = Runtime::float_tensor(&padded, &[1, CHUNK_SAMPLES])?;
        let outputs = conditioner
            .run_tensors(&[
                ("audio", &audio),
                ("instrument", &instrument),
                ("dataset", &dataset),
            ])
            .await?;
        let prefix = Runtime::tensor(&outputs, "prefix", &[1, 502 + ids.len(), 768])?;
        Runtime::output(&outputs, "prefix")?; // native finite-prefix check
        let mut past_k = Runtime::float_tensor(&[], &[14, 1, 0, 12, 64])?;
        let mut past_v = Runtime::float_tensor(&[], &[14, 1, 0, 12, 64])?;
        let mut input = vec![BOS];
        input.extend_from_slice(&forced);
        let mut generated = Vec::new();
        let mut eos = false;
        let mut cached = 0;
        for step in 0..options.max_tokens {
            if step % 16 == 0 {
                progress(format!(
                    "Detecting Synth · chunk {} · token {}",
                    index + 1,
                    step + 1
                ));
            }
            let tokens = Runtime::int_tensor(&input, &[1, input.len()])?;
            let outputs = lm
                .run_tensors(&[
                    ("tokens", &tokens),
                    ("prefix", if step == 0 { &prefix } else { &empty_prefix }),
                    ("past_k", &past_k),
                    ("past_v", &past_v),
                ])
                .await?;
            let logits = Runtime::output(&outputs, "logits")?;
            ensure!(
                logits.shape == [1, VOCAB_SIZE],
                "Unexpected MuScriptor logits shape"
            );
            let token = logits
                .values
                .iter()
                .enumerate()
                .max_by(|(ai, a), (bi, b)| a.total_cmp(b).then_with(|| bi.cmp(ai)))
                .context("Empty MuScriptor logits")?
                .0 as i64;
            generated.push(token);
            if token == EOS {
                eos = true;
                break;
            }
            decoder.feed(token);
            cached += input.len() + if step == 0 { 502 + ids.len() } else { 0 };
            past_k = Runtime::tensor(&outputs, "present_k", &[14, 1, cached, 12, 64])?;
            past_v = Runtime::tensor(&outputs, "present_v", &[14, 1, cached, 12, 64])?;
            input = vec![token];
        }
        chunks.push(ChunkTrace {
            start,
            end,
            instrument_ids: ids.clone(),
            forced_tokens: forced,
            generated_tokens: generated,
            reached_token_limit: !eos,
            emitted_eos: eos,
            elapsed_seconds: (js_sys::Date::now() - timer) / 1000.,
        });
    }
    let (notes, diagnostics) = decoder.finish(duration);
    Ok(Output {
        method: METHOD.into(),
        duration_seconds: duration,
        options,
        model_assets: assets()
            .into_iter()
            .map(|a| (a.filename, a.sha256))
            .collect::<BTreeMap<_, _>>(),
        notes,
        chunks,
        diagnostics,
    })
}
