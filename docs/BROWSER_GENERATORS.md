# Browser generator acceptance — in progress

This extends the manual parity run. It does not mark the complete generator
workflow or full-song generation accepted. The development branch is rebased
frequently onto shared `main`; desktop export and source-import fixes remain
part of the browser gate.

## Architecture

The static Dioxus client runs the existing Rust Demucs implementation and pinned
six-stem checkpoint on worker WebGPU. The resulting six full-source-clock float
WAVs are checked for finite samples, complete frame counts and exact hashes.
No input audio leaves the browser. Model downloads are content verified before
use or CacheStorage insertion. A native file picker accepts existing local
models only when their complete SHA-256 matches a catalogue entry.

Instrument inference uses the official ONNX Runtime Web 1.22.0 distribution,
matching native ORT's release. Its WASM execution provider runs one thread in
YACE's existing worker, requiring no application backend or isolation headers.
YACE's bindings, model inputs, decoding, candidate recovery, source checks,
chart building and export are Rust. Runtime licensing and package integrity
are retained in `web/runtime/`.

Both clients now share `models/postprocessing.rs`, `generation_build.rs`, piano
view selection, SwiftF0 frame decoding and initial vocal review attachment.
Browser resampling uses repeated rational filter phases; it no longer evaluates
trigonometric functions for every tap of every output sample. Float channel
mixing follows the native decoder instead of silently averaging stereo input.

## Actual browser evidence

A real three-second excerpt of the existing acceptance recording was processed
through the browser app, using the actual downloaded checkpoints:

- All six stems retained exactly 132,300 stereo frames at 44.1 kHz. Against the
  native model, maximum sample delta was 0.000002444 and normalized RMS delta
  was below 0.000087. This is numerical consistency, not musical approval.
- Drum/guitar/bass inference produced 23/22/15 detections. Every compared note
  identity and start/end time matched native exactly. Maximum score deltas were
  0.0000011 / 0.000957 / 0.0000068 respectively.
- Piano/Pro Keys inference completed; this excerpt produced no keyboard notes.
  This is not a positive musical keyboard acceptance fixture.
- Cancellation during GPU preparation kept the chart and recording available;
  restarting from the verified model cache completed successfully.
- The generated draft opened with twelve difficulty tracks and three linked
  instrument families, source-bound separated audio and timed review flags.
- Actual app playback reached the three-second end on the recording and all six
  stem channels. A generated snare moved from tick 250 to 251; Undo restored 250.
- The actual song ZIP was downloaded. Native inspection read all twelve tracks
  successfully with no error. Source/evidence and private results stay out of Git.
- Browser Basic Pitch/SwiftF0/DSP vocal inference produced one pitched note at
  ticks53–570, MIDI52, with a typed missing lyric. Its exported vocal JSON was
  byte-identical to the native no-recognition/neural-pitch reference. Native
  validation accepted the combined export. This does not establish sung-pitch
  accuracy or lyric recognition.
- Fresh import attached the missing-lyric review window and both checks. After
  save/reload, the vocal note, pending review and audio sources were recovered.
  Existing saved projects remained readable when large audio files moved to
  checksum-verified shared IndexedDB blocks; mutable JSON keeps its existing
  replacement encoding. Worker output aliases avoid copying the same audio
  again for each generated project path.
- The actual native picker reused the desktop pitch checkpoint and rejected an
  invalid local model. No file was sent to an inference service.

Regression coverage includes shared native decoding tests, native library tests,
static-tool audio tests and strict WASM/native Clippy. Explicit ignored tests in
`web/tools/tests.rs` compare actual browser downloads with real native outputs;
they require named private input paths and never fabricate inference results.

## Open gates

- Positive keyboard inference and native comparison, full-song vocal review,
  browser recognition/lyric alignment and the remaining optional detector modes.
- Finish recoverable browser generation jobs and generating a selected part into
  an existing chart. The desktop's request/status protocol and source/timing/edit
  guards are now shared; the browser job-stage/restore UI still needs completion.
- Complete 243.880-second generator workflow, project recovery/backup and playable
  export in the actual browser, plus native interchange validation.
- Full regression checks, final source rebase, push/merge and public static deploy
  with actual live verification. Current public manual-parity release is unchanged.
