YACE's optional English lyric aligner uses Meta/Facebook's
`facebook/wav2vec2-base-960h`, revision
`22aad52d435eb6dbaf354bdad9b0da84ce7d6156`, under Apache-2.0.

Official model card and authorship:
https://huggingface.co/facebook/wav2vec2-base-960h

The original model is by Alexei Baevski, Henry Zhou, Abdelrahman Mohamed and
Michael Auli. The model weights are not retrained or modified. The bundled
config's token identifiers are normalized to the original vocabulary and the
JSON is formatted deterministically; exact runtime asset hashes are retained
in `src/vocal_alignment.rs`.

The native Rust model implementation is separately MPL-2.0, adapted from
EntreNota's `practice-acoustic` and Djoe-Denne/wav2vec2-rs. Its license, notices
and corresponding modified source are in `vendor/practice-acoustic`.
No Python runtime is included or invoked. Spanish noncommercial model weights
are not included in YACE.
