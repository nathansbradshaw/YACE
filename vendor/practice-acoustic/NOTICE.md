The Rust model files in src/model are derived from Djoe-Denne/wav2vec2-rs,
commit 62aec29d45926f4d8cb626550b80bcda58cbc301, licensed under MPL-2.0:
https://github.com/Djoe-Denne/wav2vec2-rs

EntreNota corrects standard encoder normalization to run before the encoder
layers, while retaining final normalization for the stable pre-norm architecture.
Reference: Hugging Face Transformers' Wav2Vec2Encoder and
Wav2Vec2EncoderStableLayerNorm implementations.

Only the native Rust model is included. Alignment, model installation, and
application integration were implemented by EntreNota. YACE reuses this
native engine and adds cancellation/progress callbacks between bounded
inference windows in src/lib.rs. These modifications remain MPL-2.0; the
corresponding source is included in this directory. No Python is used.
