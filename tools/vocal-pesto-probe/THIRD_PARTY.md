# Research provenance and terms

This tool implements the input/output and context contract of a separately
obtained PESTO ONNX graph. It does not contain model weights or upstream source.

PESTO: Alain Riou and colleagues, Sony Computer Science Laboratories Paris.
[Research paper](https://arxiv.org/abs/2508.01488).
[Source](https://github.com/SonyCSLParis/pesto/tree/62bc0c9702558f19af4593752947fb9db1eadac9).
The reviewed repository publishes GNU LGPL v3 terms in
[LICENSE.md](https://github.com/SonyCSLParis/pesto/blob/62bc0c9702558f19af4593752947fb9db1eadac9/LICENSE.md).

The pinned streaming ONNX conversion is published by Qosmo, Inc. in
[pesto_tilde](https://github.com/QosmoInc/pesto_tilde/tree/9fc9108a3578cfb3e842a44a35b8aecccc4b8d0a).
The publisher's [License.md](https://github.com/QosmoInc/pesto_tilde/blob/9fc9108a3578cfb3e842a44a35b8aecccc4b8d0a/License.md)
states GNU LGPL v3. These repository notices do not establish an independent
unrestricted weight grant or rights in training recordings. Distribution,
embedding or production promotion requires reviewing the exact weight terms
and satisfying applicable notices/source obligations first. This probe only
opens an explicitly supplied, checksum-verified local file.

ONNX Runtime is separately installed; its Microsoft MIT license applies to that
runtime. Rust dependency versions and checksums are recorded in Cargo.lock.
