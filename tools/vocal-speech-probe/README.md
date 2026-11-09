# Local speech activity research

This Rust probe records source-bound activity scores independently of pitch and
lyrics. It does not select pitches, modify charts, download models, or enable a
new analysis quality mode. Speech scores are not calibrated singing activity,
lead-singer identity, or pitch correctness probabilities. The separate
`vocal-activity-probe` retains the existing PANNs experiment.

```sh
cargo build --release --locked --manifest-path tools/vocal-speech-probe/Cargo.toml
tools/vocal-speech-probe/target/release/yace-vocal-speech-probe \
  --input /private/vocals.wav \
  --model-file /private/silero-vad-066409f3-op15.onnx \
  --runtime /private/libonnxruntime.1.24.4.dylib \
  --output /private/new-activity.json
```

An explicitly selected compatible native runtime is required; installed sealed
runtime resolution follows YACE's existing helper rules. The output records its
identity. The tool refuses a different model and existing output files. Keep
recordings and evidence outside public Git. No model is bundled by this tool.

Model: [Silero VAD](https://github.com/snakers4/silero-vad), revision
`066409f3d9db45a30d21dfcbf9147826ffdd7052`,
`src/silero_vad/data/silero_vad_16k_op15.onnx`.
SHA-256: `7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49`.
MIT license, Silero Team; see `LICENSE-Silero.txt`. License SHA-256:
`2e63e9a38b6e8fc0c7bc37ce174caca1862870856c6daf5697cfb785e925520b`.

The shared Rust implementation decodes stereo 16 kHz PCM and takes the arithmetic
channel mean without gain normalization. It supplies 512 samples plus 64 samples
of previous context, a zero-initialized `[2,1,128]` recurrent state per recording,
and scalar int64 sample rate 16000. Only the final block is zero-padded. Input and
output shapes, finite values, source/model checksums, duration and block counts
are checked. Evidence retains the original recording clock and source, PCM,
model, implementation and runtime identities. Counterphase stereo can cancel in
this publisher-style mono input; a low activity score does not prove silence.

The core framing and evidence checks compile for native and browser targets;
this CLI supplies native inference. Browser inference and use as a generation
filter remain unimplemented. Four development recordings produced bit-identical
scores to the independent local protocol implementation. That verifies this
implementation comparison, not publisher numerical parity or musical accuracy.

A development-selected veto protecting existing acoustic voicing failed the full
pitch/false-voicing checks on exposed regression material. No filter is promoted.
Any future filter must measure missing and wrong pitches, voiced recall,
exported-note frames and false voicing separately on unseen artists.
