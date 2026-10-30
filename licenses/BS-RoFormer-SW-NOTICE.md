# Optional BS-RoFormer SW model and native frontend

YACE's optional higher-quality piano source uses an independent native Rust
implementation of the public BS-RoFormer SW ONNX contract. No Python runtime
or GPL INTERSECT implementation is copied, invoked, or bundled by this module.
Demucs remains the primary/default separator.

## Weight provenance and unresolved terms

- [Pinned ONNX conversion](https://huggingface.co/scragnog/HOT-Step-CPP-SuperSep/tree/440487b8300dcd61453cc52ec244a38150b03456)
- File: `bs_roformer_sw.onnx`, 704,536,949 bytes
- SHA-256: `d2a13ad34eb26a8aa3135314b2cf1efb41e70289ffb9d03124a4201736a56df3`
- [Checkpoint mirror and model card](https://huggingface.co/enerjazzer/BS-ROFO-SW-Fixed/tree/a443a2985534b3bc815ef54a5d446c6a0390f974)

The conversion repository grants MIT **only for conversion and packaging** and
explicitly leaves individual weight terms with their original licenses. The
checkpoint mirror attributes its origin to jarredou and declares the license
`unknown`. A license grant for these original weights has not been established.
The conversion's general attribution also mentions aufr33; this does not prove
cryptographic provenance against the original training checkpoint. Exact ONNX
bytes, configuration provenance, and actual tensor contracts are pinned.

These weights are not included with source code or the default model catalog.
An explicit optional download or private/local package may include the verified
asset under its own unresolved terms. YACE's MIT source license does not grant
rights to the weights or imply that public weight redistribution is permitted.
No new public redistribution is asserted by this notice.

## Independent frontend and attribution

The native implementation uses standard centered Hann STFT, complex masks,
length-preserving inverse STFT, and overlap-add. Factual tensor packing and
chunk parameters are documented by
[ZFTurbo/MSS_ONNX_TensorRT, revision43d939e7671d8ff6cf1922f98c2f2e4b56908e47](https://github.com/ZFTurbo/MSS_ONNX_TensorRT/tree/43d939e7671d8ff6cf1922f98c2f2e4b56908e47),
notably `models/preprocess.py`, `models_without_stft/bs_roformer_no_stft.py`,
`export_to_onnx.py`, and `utils.py`. Those files were read as source text only;
no Python was executed. The original architecture is described in
[Music Source Separation with Band-Split RoFormer](https://arxiv.org/abs/2309.02612).

YACE changes include native Rust/ORT/FFT inference, strict checksums and tensor
contracts, finite PCM/mask checks, disk-backed source PCM, bounded chunk and
overlap memory, streamed RF64 float outputs, original source-clock preservation,
exactly equivalent omission of chunks disjoint from the original-source crop,
and source/model/runtime/decoder/implementation provenance. Model labels are
estimates, not proof of instrument identity. Benchmark receipts do not establish
accuracy on every recording. Attribution implies no endorsement.

The upstream preprocessing/export framework declares:

MIT License

Copyright (c) 2024 Roman Solovyev (ZFTurbo)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
