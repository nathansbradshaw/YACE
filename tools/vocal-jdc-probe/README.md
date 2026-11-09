# JDC singing pitch research

This Rust tool reconstructs the original JDC network and analyzes an isolated
vocal stem without a lyric recognizer. It does not edit a project, download
weights, change a production quality mode, or certify a chart. Model output
and chart targets measure different things; manually authored game charts are
not continuous performed-frequency annotations.

The [publisher implementation](https://github.com/keums/melodyExtraction_JDC/tree/86e6d2b225c441cf5641ca74c342348646ff1bdc)
and [paper](https://www.mdpi.com/2076-3417/9/7/1324) describe joint singing
detection and pitch classification. Revision
`86e6d2b225c441cf5641ca74c342348646ff1bdc` includes its original model and
normalization files under the accompanying MIT license, reproduced in
[JDC-LICENSE.txt](JDC-LICENSE.txt). Copyright 2019 Sangeun Kum. Retain this
notice when redistributing the original or converted model. Audio and chart
rights are separate; no recordings, reference charts or private scores belong
in this tool's directory or public Git history.

| Resource | SHA256 |
| --- | --- |
| `weights/ResNet_joint_add_L(CE_G).hdf5` | `72c9c038a093031cfbfd2f61bb878e3351720e4230ab2a35596df1e5dd969005` |
| `x_data_mean_total_31.npy` (save as `mean.npy`) | `820df6eaa4da22c496e2299409d934eaebd77b89331d4618be2291abb6d7cf95` |
| `x_data_std_total_31.npy` (save as `std.npy`) | `cb8015fa1119f9b8a2a2018357c71bbb5674ff940cb356fcea23bc606fd26d83` |
| Converted `original-rust.onnx` | `7f6449c25f3c45e94eea6ce1403e6d40f6bfa9deac41621c5a13a97fe8757122` |
| ONNX Runtime 1.24.4, macOS arm64 publisher library | `f958c8c8f50a787f64cd13aa5f07710b97075d9869c6983dc0001d2452100eaa` |

```sh
cargo run --release --locked --manifest-path tools/vocal-jdc-probe/Cargo.toml -- \
  convert --checkpoint /private/models/model.hdf5 \
  --output /private/models/original-rust.onnx
cargo run --release --locked --manifest-path tools/vocal-jdc-probe/Cargo.toml -- \
  infer --input /private/audio/vocals.wav --model-dir /private/models \
  --runtime /private/runtime/libonnxruntime.1.24.4.dylib \
  --output /private/reports/jdc.json --posterior /private/reports/jdc.f32le
cargo run --release --locked --manifest-path tools/vocal-jdc-probe/Cargo.toml -- \
  analyze --baseline /private/reports/high/generation.json \
  --audio /private/audio/song.wav --input /private/audio/vocals.wav \
  --model-dir /private/models --rmvpe-model /private/models/rmvpe.onnx \
  --runtime /private/runtime/libonnxruntime.1.24.4.dylib \
  --output /private/reports/jdc-draft
```

Conversion reads original HDF5 tensors in pure Rust; it does not execute Keras
or deserialize executable framework objects. Weights remain unchanged. The
opset17 graph uses original HardSigmoid recurrent activations, Keras-to-ONNX
gate reordering, inference batch normalization, and both original output heads.
The frozen graph hash is checked before publication and inference.

The explicit frontend uses FFmpeg mono 8kHz; FFT1024/hop80; periodic Hann;
center reflection; magnitude then **10 log10**; whole-recording peak reference;
an 80dB floor; nonoverlapping 31-frame windows; zero-dB feature padding before
position-specific f64 normalization `(x - mean) / (std + 0.0001)`; then f32.
The last partial window does not extend the evidence clock. CPU inference
uses batches of16, two intra-op threads and one inter-op thread. Numerical
parity with the publisher's old madmom/Keras environment is **unestablished**.

The evidence retains fine-pitch argmax, singing score and f64 sums of the
original f32 probabilities into semitone bands, plus source/PCM/model/frontend/
runtime identities. Optional raw output is row-major little-endian f32 with
722 columns at 10ms intervals. JDC's modeled range is **MIDI38..83**; this
tool cannot establish reliable coverage outside it or identify individual
singers in harmonies. The shared Rust frontend and evidence format compile
for the browser; this CLI's pinned native runtime currently requires macOS
arm64. No different model/runtime is silently substituted.

`analyze` accepts a High accuracy generation report only as the acoustic frame
grid, local energy, source binding and tempo map. It recomputes pinned RMVPE
under the explicit runtime and selects pitches from JDC/RMVPE agreement. It
does not use recognized words, lyric text, reference notes, key or octave
folding. Strong seeds require JDC semitone mass0.9, singing score0.7,
RMVPE confidence0.03 and agreement within0.75 semitone. The source-clock note
path uses change cost3.2. Gaps up to180ms between equal accepted pitches need
JDC band mass0.03, singing score0.7, RMVPE confidence0.03 and agreement within
0.5 semitone throughout, with nonzero local mono RMS. Notes shorter than60ms
are omitted. Zero mono RMS is not proof of physical stereo silence.

The output includes raw singing evidence, a generation report, an editable
vocal chart, and playable MIDI when the source tempo map has zero offset.
Every note remains uncertain with per-model support and alternatives. Unknown
words remain typed missing syllables; expected absent lyrics do not flood
the pitch review with text issues. Existing outputs are never overwritten.
The parent worker can cancel this process group without applying a partial
draft; standalone process termination leaves no published output directory.

This frozen profile **failed** the20% held-out improvement gate. On eight
previously unexposed artists, exported-note missing-or-wrong errors fell14.23%
relative to the actual installed High accuracy app, while mean voiced recall
increased8.08 percentage points and false voicing fell2.54 points. It remains
an explicit research tool; no production backend or default is promoted.
