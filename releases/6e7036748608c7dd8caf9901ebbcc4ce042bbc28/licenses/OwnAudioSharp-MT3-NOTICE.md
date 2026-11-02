# OwnAudioSharp native MT3 source attribution

`src/mt3.rs` adapts native session/cache driving from **ModernMube/OwnAudioSharp**,
revision `4b7cc4f1ea32acb4f0f23723e0fa4508bafa488d`, specifically
`OwnAudio/Source/Mt3/rust-mt3/ownaudio-mt3-core/src/`.

- [Pinned source](https://github.com/ModernMube/OwnAudioSharp/tree/4b7cc4f1ea32acb4f0f23723e0fa4508bafa488d/OwnAudio/Source/Mt3/rust-mt3/ownaudio-mt3-core/src)
- Source license: **MIT**, Copyright (c)2025 Modernmube. The complete copyright,
  permission and warranty notice is retained in `OwnAudioSharp-MIT.txt` and in
  the adapted source header. The source is licensed; checkpoint provenance is
  a separate question.

YACE changes: existing Rust ORT integration; strict four-asset hashes; exact
596-token codec; author-documented YourMT3 absolute100Hz shifts and required tie
prologues; source sample boundaries; retained program/drum labels and raw tokens;
explicit source-end bounds without a fixed10s note cap. No model correctness
probability is invented. No Python program is bundled or executed.

This research module is outside production Piano/Synth routing. Its four MT3
checkpoint files are excluded from the application model catalog and packaged
model assets. The ModernMube weight mirror labels the repository MIT, while
OwnAudioSharp's documentation warns about YourMT3 checkpoint GPL terms; exact
checkpoint lineage and weight-specific redistribution terms remain unresolved.
This uncertainty does not negate the MIT license of the adapted OwnAudioSharp
source. No endorsement by ModernMube or the model authors is implied.
