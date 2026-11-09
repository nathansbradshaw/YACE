# PESTO vocal-pitch research probe

Experimental Rust/ONNX inference, isolated from YACE chart generation. It never
reads reference pitches, lyrics, a song key or existing notes, and cannot edit a
project. PESTO is an additional estimator to evaluate; detector agreement is
not proof of the correct lead singer or pitch. No backend is promoted here.

## Pinned model and preprocessing

- Upstream: [Sony CSL PESTO](https://github.com/SonyCSLParis/pesto), reviewed at
  `62bc0c9702558f19af4593752947fb9db1eadac9`.
- ONNX publisher: [Qosmo pesto_tilde](https://github.com/QosmoInc/pesto_tilde),
  `9fc9108a3578cfb3e842a44a35b8aecccc4b8d0a`,
  `misc/mir-1k_g7_44100_512.onnx`, 17,019,271 bytes.
- SHA-256: `8048fd691f9018d86977bc3cb745e02f265d800df55513b921c64e464dee43e5`.
- 44,100 Hz mono float32 decoded by local FFmpeg, no normalization; hop 512.
  The model contains its CQT and pitch decoder. Prediction units are fractional
  MIDI semitones, **not Hz**. Confidence is periodicity salience, not calibrated
  probability of correct lead singing. Inputs must be finite, nonempty and at
  most one hour; oversized input fails rather than being truncated.
- The published one-hop streaming interface provides 3,840 past samples and
  zero future samples. Default offline processing supplies eight hops (4,096
  samples) of actual future audio, discards those output frames, and rebuilds
  past context directly from source samples for the next core. Core size
  changes batching, not the original recording clock. No reference time fitting.
- Frame centre: `(frame_index * 512 + 256) / 44100`. The final partial hop is
  zero padded and retained explicitly, so its centre can fall past the recording
  end. This model clock is not a claim that note attacks are correctly aligned.
- Two native ONNX Runtime threads. `ort` is locked to 2.0.0-rc.10. Supply a
  compatible local native runtime through `ORT_DYLIB_PATH`; no Python runtime,
  model downloader or automatic quality fallback is used.

See [THIRD_PARTY.md](THIRD_PARTY.md) for source and weight terms. Weights and
private audio/reports must stay outside Git. No model is bundled or installed.

## Run

Use an existing compatible Cargo target directory to avoid duplicate builds.
`ORT_DYLIB_PATH` must point to an installed native ONNX Runtime shared library.
Set `YACE_PESTO_MODEL_FILE` to the exact pinned local model above.

```sh
cargo run --locked --manifest-path tools/vocal-pesto-probe/Cargo.toml -- \
  --model "$YACE_PESTO_MODEL_FILE" --controls --output /private/tmp/new-controls
cargo run --locked --manifest-path tools/vocal-pesto-probe/Cargo.toml -- \
  --model "$YACE_PESTO_MODEL_FILE" --input /path/to/vocals.wav \
  --output /private/tmp/new-evidence
```

`--streaming` explicitly reproduces the single-hop, zero-future interface.
`--core-frames 1..256` changes offline batching (default 128). These flags cannot
be combined. Output must not exist. A temporary sibling directory is removed on
failure; completed output is renamed into place after source/model checks.

Output retains unfiltered pitch/confidence/RMS frames, the full 384-bin
activation grid in little-endian float32, grid dimensions and SHA-256, model and
implementation identity, source-byte and decoded-PCM hashes, processing mode,
clock and elapsed runtime. `profile.json` records parameters before inference.
No low-confidence pitch is silently discarded. Controls use a provisional 0.5
salience threshold only for reporting, not as a production voice threshold.

## Verification and acceptance

```sh
cargo fmt --manifest-path tools/vocal-pesto-probe/Cargo.toml -- --check
cargo test --locked --manifest-path tools/vocal-pesto-probe/Cargo.toml
cargo clippy --locked --manifest-path tools/vocal-pesto-probe/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path tools/vocal-pesto-probe/Cargo.toml \
  pinned_model_offline_pitch_is_invariant_to_core_length -- --ignored --nocapture
```

Normal tests require no model, FFmpeg, network or native runtime. The opt-in test
checks actual model output with one versus 128 core frames across a short pitch
change, silence, chunk seams and an unaligned tail. Synthetic controls cover
110/220/440 Hz, a harmonic-rich 110 Hz signal and silence. Passing these only
validates inference and clock mechanics.

Evaluate isolated vocal audio bound to original authored game MIDI, keeping
originals and provenance. Record absolute pitch, missing pitches, wrong pitches,
octave errors, false voicing, note precision/recall, boundaries, singer switches,
runtime and memory separately. Chart targets describe authored game notes and
are not continuous laboratory F0 annotations. Never use generated notes as truth.

Promotion still requires the existing held-out quality gate: at least 20%
relative reduction in missing-or-wrong pitch frames, wrong-among-detected also
improving, voiced recall falling by no more than two percentage points, and no
language regression beyond two points. Note/export performance, listening,
native/browser parity and musical/game acceptance remain separate checks. Any
previously inspected artist is development material for subsequent tuning.
