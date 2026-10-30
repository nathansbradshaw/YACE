# Native Rust LarsNet mask conversion

ZIP 9.1.3 is used directly and through the local Candle 0.9.2 archive patch.
Pinned checkpoint hashes are checked before parsing. These archives start at
byte zero, which is supplied explicitly to avoid offset-detection errors.

To check all five exact output hashes with installed official checkpoints:

```sh
YACE_LARSNET_MODELS=/path/to/pinned/checkpoints \
  cargo test --locked --manifest-path tools/larsnet-convert/Cargo.toml \
  --test cli pinned_models_keep_their_exact_onnx_outputs -- --ignored
```

This local research tool converts the five checksum-pinned LARS TorchScript files into ONNX magnitude-mask graphs without Python, LibTorch, downloads or arbitrary TorchScript execution. The converter/front-end source is independent MIT Rust; the resulting model remains **CC-BY-NC4.0** under the authors' [LarsNet](https://github.com/polimi-ispl/larsnet/blob/17d631fd18e77ee2f1d23ee7b3fc0fb46ae629e2/README.md) and [LARS](https://github.com/EdoardoMor/LARS/blob/872991160e7c26b1d9be76ddc625d01373727200/README.md#license) weight statements. No weights are bundled.

Attribution: LarsNet by Alessandro Ilic Mezza, Riccardo Giampiccolo, Alberto Bernardini and Augusto Sarti, [Toward Deep Drum Source Separation](https://doi.org/10.1016/j.patrec.2024.04.026), and the official LARS contributors. The source mathematical architecture is documented in [LARS `unet.py`](https://github.com/EdoardoMor/LARS/blob/872991160e7c26b1d9be76ddc625d01373727200/drums_demix/net/unet.py). No upstream Python/plugin source is copied or assumed MIT. Candle's tensor-data parser is used as a normal Rust dependency; no pickle callable is executed.

Obtain the original files from the official repository at revision `872991160e7c26b1d9be76ddc625d01373727200`, directory `drums_demix/DrumsDemixUtils/DrumsDemixModels/`, filenames `my_scripted_module_PART.pt`. Retain that revision, file name, attribution and [CC-BY-NC4.0 legal terms](https://creativecommons.org/licenses/by-nc/4.0/legalcode) with any local converted copy; conversion is a format modification, not a new weight-license grant.

| PART | Original SHA256 | ONNX SHA256 |
|---|---|---|
| kick | `15d40aa9760e67305a31be4053a87e2b208ee4c8718357691e3b49f80aff0e46` | `15f1dcd32d883a7c76827d374a56e960b87679fddfe9e35527811fedeb3c3351` |
| snare | `166eaa52d05ccdb3842f3bf79d3afefa354757d366db0438f9128df75575a6f8` | `16b8d4c7882473f3cccfdfad36583aa0cdbcb0c782a2db3f0ed4b7b870bf8555` |
| toms | `1eddcd147b1d771e45c3a443a94f332e2a1ecc5854cb11aa93459cecb1c63ca2` | `f8f279925383d539d3fef7ab43ed024be2ca483ad0e5e855484b045e31ea0e00` |
| hihat | `1b5d46d05841fb663a77aa8b037847f4f158e916806177ff1a5443724b3f73f3` | `972d6bfe012436156f9fc66dc062bc774fb620fcad3f895c35f2acedd5499b32` |
| cymbals | `40b2dc81213729148188806a89abde53e3a90aee909f1a0cfbe1efbf4cf98b89` | `69110dd7705a09935590ef9a22dfecd5e8d1af5656890b2d3dbd764015f9a727` |

```sh
CARGO_TARGET_DIR=/path/to/private/converter-target \
  cargo run --release --manifest-path tools/larsnet-convert/Cargo.toml -- \
  kick /path/to/my_scripted_module_kick.pt /path/to/new/kick.onnx
```

Repeat with each explicit part. Existing output or receipt is refused. All five graphs are39,341,756 bytes and use opset17. ONNX input `magnitude` and output `mask` are float32 `[1,2,2048,512]`. This tool writes masks only: the waveform frontend must implement the external centered4096 periodic-Hann FFT/hop1024 contract, final-frame block padding and zero Nyquist mask. It must not reinterpret the tensor as waveform audio.

Each conversion checks the pinned source bytes, tensor layout/finiteness, and a real CPU ORT graph execution. Its companion `*.onnx.receipt.json` retains original/converted hashes, source revision/URL, conversion-source identity, terms, tensor contract and native smoke measurements. These checks establish reproducibility and runtime compatibility. They do not establish upstream LibTorch numerical parity, clean instrument isolation, listening quality or charting improvement. See [DRUM_PARTS.md](../../docs/DRUM_PARTS.md) for the native audio path, controls and limits.

```sh
CARGO_TARGET_DIR=/path/to/private/converter-target \
  cargo test --offline --manifest-path tools/larsnet-convert/Cargo.toml
CARGO_TARGET_DIR=/path/to/private/converter-target \
  cargo clippy --offline --manifest-path tools/larsnet-convert/Cargo.toml --all-targets -- -D warnings
```
