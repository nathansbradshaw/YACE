# Singing-note model research

This Rust probe runs the **OpenUtau SOME 0.0.1 ONNX export** on isolated vocal
audio. It records note boundaries, rests and continuous model note pitches.
It does not change a project, replace RMVPE observations, accept MIDI notes,
recognize lyrics, download weights or enable a new quality mode.

```sh
CARGO_TARGET_DIR=target cargo build --release --locked \
  --manifest-path tools/vocal-note-probe/Cargo.toml
target/release/yace-vocal-note-probe \
  --input /private/vocals-excerpt.wav \
  --model-file /private/0918_continuous128_clean_3spk.onnx \
  --runtime /private/libonnxruntime.1.24.4.dylib \
  --output /private/new-note-observations.json
```

Use an original-clock excerpt between 0.1 and 120 seconds. The tool does not
silently trim, shift, normalize or split the source. Only the final model frame
may be bounded to the recording endpoint; its original endpoint is also saved.
Rests, short intervals and unrounded pitches remain visible. Existing outputs
are refused. Source, decoded PCM, model, helper, implementation and runtime
hashes are recorded. A different or missing resource fails explicitly.

This is a native research runner. It does not establish browser inference,
publisher PyTorch/ONNX numerical parity, full-song accuracy, singer identity or
musical acceptance. Compare note events and reviewed musical intervals as well
as separately retained continuous-F0 measurements. Keep audio, references and
reports outside public Git. Predictions must precede reference reads.

## Resources and terms

- [SOME source](https://github.com/openvpi/SOME), MIT. This older export is not
  the current GAME model and does not represent all SOME versions.
- [Publisher export package](https://github.com/xunmengshe/OpenUtau/releases/tag/0.0.0.0),
  `some-0.0.1.oudep`; archive SHA-256
  `b98a350b60740e3a153b15e137ed025832956d918c44853048bec93af657f478`.
- Enforced model SHA-256:
  `9d55f9d239e9d70d1bf039506cf1c428b657df7bbb8b94104ca882af6b721e06`.
  The package's `some.yaml` declares 44.1 kHz. Input is `[1,samples]` waveform;
  outputs are matching `[1,notes]` `note_midi`, `note_rest` and `note_dur`.
- [OpenUtau interface reference](https://github.com/stakira/OpenUtau/blob/2005b9670ac312aaf0bbf2b42767fb70fe9185dd/OpenUtau.Core/Analysis/Some.cs).
- Weight-specific terms for this exported package are not separately
  established. No weights are bundled; this runner is not app integration.

Preprocessing uses YACE's 44.1 kHz arithmetic mono decode and the complete
publisher graph, including its spectral frontend. It preserves amplitude and
records the actual PCM checksum. Inference uses native defaults, two intra-op
threads and one inter-op thread. No lyrics, note reference, key or RMVPE values
enter the model. Its output is independent evidence for comparison, not a
replacement for a trusted pitch stream.

## Measured limitation

On eight exposed solo development clips, using these boundaries with unchanged
RMVPE interval medians raised correct musical-note time from 66.19% to 79.39%
against annotator A1, but raised false-note time in reference gaps from 3.30% to
43.51%. Restricting the sounding portions to RMVPE confidence 0.8 produced
75.10% correct note time and 89.91% presence recall, but wrong pitches among
detected note time were 16.63%, above the baseline's 10.14%. These are paired
development diagnostics, not fresh held-out results or a passed quality gate.
The model remains experimental. Do not promote a boundary model because it
merely produces more notes.
