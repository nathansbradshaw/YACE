# Singing pitch probe

A standalone MIT Rust research CLI for reproducible native CREPE evidence. It produces raw pitch/salience, **not chart notes or musical approvals**. It has no generator hook, provider, lyrics dependency, Python runtime or model downloader. YACE's production vocal defaults remain unchanged.

## Run

Requires Rust, FFmpeg on PATH, and an explicitly supplied pinned local ONNX file. Build dependencies use Cargo; runtime does not contact the network.

```sh
cargo run --release --manifest-path tools/singing-pitch-probe/Cargo.toml -- \
  --model tiny --model-file /absolute/path/tiny.onnx \
  --input /absolute/path/vocals.opus --output .runtime/pitch-probe/new-song

cargo run --release --manifest-path tools/singing-pitch-probe/Cargo.toml -- \
  --model tiny --model-file /absolute/path/tiny.onnx \
  --controls --output .runtime/pitch-probe/new-controls
```

`--model full|tiny` and the model path are both mandatory; a mismatched checksum fails before inference. Output must be a new directory. Files remain in a temporary directory until complete; existing output is not intentionally overwritten. Neither option changes the app or supplies a production cache.

## Pinned models and sources

The public [onnxcrepe v1.1.0 release](https://github.com/yqzhishen/onnxcrepe/releases/tag/v1.1.0) provides these models, converted from torchcrepe. Source revision: `946f80e7f61fb78c72a037beac46b8b2dc3892bc`. Its MIT notice is retained in [licenses/onnxcrepe-MIT.txt](licenses/onnxcrepe-MIT.txt). Weights are not committed or packaged here. Acquire the chosen release manually and verify its hash.

| Choice | Release asset | SHA256 | Approximate file size |
|---|---|---|---|
| tiny | [tiny.onnx](https://github.com/yqzhishen/onnxcrepe/releases/download/v1.1.0/tiny.onnx) | `91fc2a0fd10f965dbf7775995daf50e99273caedd7efd00001f23be649da1bc3` | 1.9 MiB |
| full | [full.onnx](https://github.com/yqzhishen/onnxcrepe/releases/download/v1.1.0/full.onnx) | `119845c72c702e052e5262430f9d120bce46176689aa226c39d09dea5cc3a610` | 85 MiB |

The [original CREPE paper](https://arxiv.org/abs/1802.06182), [original decoder](https://github.com/marl/crepe/blob/master/crepe/core.py), and [torchcrepe model](https://github.com/maxrmorrison/torchcrepe/blob/master/torchcrepe/model.py) define the numerical conventions. The Rust implementation uses the original local weighted mean over the maximum bin and its four neighbors on each side: `cents[i] = 20*i + 1997.3794084376191`, then `Hz = 10*2^(weighted_cents/1200)`. It does not apply the converter's extra sigmoid, optional dither, Viterbi or unvalidated confidence calibration. The pinned graph already embeds normalization and sigmoid. Code licensing and model provenance are recorded independently from any song/audio rights; no audio or professional charts are distributed by this tool.

## Evidence contract

- `evidence.json`: original file path/hash, decoded mono 44.1 kHz sample count/hash, model identity/hash, implementation hash (Rust source + manifest + lockfile), decoded 16 kHz PCM hash, clock and padding conventions, raw frequency/confidence/local energy, inference duration, tensor shape and salience hash. Source and model are hashed before and after inference.
- `salience.f32le`: complete frame-major `[frames,360]` sigmoid probabilities, little-endian float32. No probabilities are discarded by a voicing gate. Small optimized-ORT roundoff down to approximately -6e-8 is retained verbatim; finite values within a 1e-6 tolerance are clamped only for weighted decoding/confidence.
- Controls additionally retain a separate evidence/salience pair for each known harmonic tone, silence and deterministic noise, plus a compact `controls.json` summary. A harmonic instrument can produce the same waveform as a voice control; this is not a lead-singer classifier.

Input is decoded mono at 16 kHz. Centered 1024-sample frames use zero padding, a 160-sample/10 ms hop and no hidden timing offset. A frame exactly at EOF is retained when the sample count is divisible by the hop. Frame values are raw proposals; silence still has an arbitrary decoded frequency. Confidence is maximum neural salience, **not a calibrated probability of correct lead singing**. `local_rms` covers the centered source-hop cell and does not gate or certify voice.

A second streaming 44.1 kHz decode verifies the actual source clock against resampling, within one sample at each rate. The original file hash and both decoded PCM hashes remain separate. Stereo is downmixed by FFmpeg; separate voices can overlap or cancel, and raw inference does not establish voice identity. A separately trimmed input has its own zero-based clock; do not treat those times as full-song coordinates without a separately verified offset.

Inference uses two ORT CPU threads and batches of 32 frames, with finite PCM/tensor checks and a one-hour input budget. Oversized audio is rejected rather than silently truncated. A one-hour recording produces about 518 MB of raw salience, plus frame JSON and a 16 kHz PCM buffer of about 230 MB; model activations add memory. Peak native memory was not measured. Use compact metadata or selected frames when inspecting large results.

## Verification

```sh
cargo test --manifest-path tools/singing-pitch-probe/Cargo.toml
cargo clippy --manifest-path tools/singing-pitch-probe/Cargo.toml --all-targets -- -D warnings

YACE_CREPE_MODEL=tiny YACE_CREPE_MODEL_FILE=/absolute/path/tiny.onnx \
  cargo test --manifest-path tools/singing-pitch-probe/Cargo.toml \
  pinned_native_controls_and_salience_receipts -- --ignored --nocapture
```

The opt-in native control never downloads models. It checks 110/220/440 Hz pitch and fixed confidence >=0.5, silence/noise below that threshold, the 101-frame source clock and complete raw salience hashes. Unit tests cover probability decoding, no second sigmoid, nonfinite/invalid values, edge padding, explicit model/input selection, checksum rejection and failure before output. These checks establish implementation behavior, not singing quality or production acceptance.
