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
The native-compatible Kaiser filter now also matches phase tables, boundary
reflection and fractional output frame counts. Real piano input comparisons
against the native decoder differ by at most 0.000000120 per sample.

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
- Recoverable generation now freezes the recording, timing, requested parts and
  engine identity, checkpoints before/after stages and lists retained jobs after
  reload. In the actual browser a vocal-only job reused the chart's verified
  stems, completed, applied through the shared source/timing/part guards, and
  Undo restored the previous part while other tracks retained their counts.
  Reload recovered the complete job and its draft. Cancelling a full-song job
  during separation retained the source chart and exposed Resume remaining stages.
- A downloaded separated-parts archive now contains a relative-path
  `stem-manifest.json`. The explicit actual-download comparison again passed
  against native output; the six source clocks and hashes remained valid. The
  native transcription CLI also loaded that downloaded manifest and produced
  the expected 23/22/15 drum/guitar/bass detections.
- The real 243.880-second recording completed all four browser job stages after
  cancellation/reload/resume. All six stems contain 10,755,108 stereo frames.
  Inference retained 1,435 drum, 2,086 guitar and 971 bass detections, plus 298
  vocal notes with 466 issues. The generated project opened with twelve
  difficulty tracks, three linked families and source-bound vocal review.
  The resulting Expert tracks contained 1,429 drum, 1,777 guitar and 807 bass
  notes after arrangement, with all 298 vocal notes retained. Actual playback
  reached the end at 100% on the original and all six stem elements. A generated
  snare moved from tick 55,422 to 55,423; Undo restored it during playback.
  Reload recovered the project at the song end with its twelve tracks and stems.
- The full-song run exposed an incomplete worker input transfer: unused encoded
  stems were still required to validate the complete manifest. Keeping those
  encoded inputs fixed detection, and the saved separation resumed successfully.
  Only the required parts are decoded. Existing audio copied into the built
  project returns as verified aliases rather than repeated byte arrays.
- Full-song export exposed a worker resource failure. Export now validates WAV
  stems one at a time and writes the ZIP into browser-backed chunks. The actual
  downloaded 31-file archive passed all CRC checks; native inspection read its
  combined MIDI. Its archived original Ogg SHA-256 exactly matched the recording.
  WAV compression level 3 reduced the complete archive to 957,505,126 bytes;
  all CRC checks passed. Its actual file-picker import retained the three Expert
  counts, all 298 vocal notes, the vocal review windows and all seven 243.880 s
  audio clocks. Identical game/recovery audio shares allocations during import.
- The resulting full-song project backup was downloaded (490,070,498 bytes),
  passed archive integrity checks and reopened through Open saved project.
  A guitar edit and its undo history survived: Redo restored the edited attack
  at 0.210 seconds; Undo restored the original 0.209-second attack.
- A real five-second Linkin Park piano introduction exposed a resampling
  mismatch: the browser omitted one of native's ten candidates. Matching the
  native filter restored all ten, with identical pitches, maximum timing delta
  0.000000036 seconds and score delta 0.0000018 on the same browser-separated
  stems. Applying the corrected draft retained an unrelated Bass part created
  while inference ran, and produced seven arranged Keys attacks. A prior
  changed-target attempt was refused; Undoing that target edit allowed retry.
  This is numerical/workflow agreement, not a musical quality judgment.

Regression coverage includes shared native decoding tests, native library tests,
static-tool audio tests and strict WASM/native Clippy. Explicit ignored tests in
`web/tools/tests.rs` compare actual browser downloads with real native outputs;
they require named private input paths and never fabricate inference results.

## Open gates

- Positive keyboard inference and native comparison, full-song vocal review,
  browser recognition/lyric alignment and the remaining optional detector modes.
- Complete full-song cancellation/resume and edited-target conflict checks for
  recoverable browser generation jobs; the excerpt apply/Undo/reload path passed.
- The full-song generation/playback/editing/reload/song ZIP/project backup gates
  have passed the exercised engineering checks; game/controller and musical
  listening acceptance remain separate.
- Full regression checks, final source rebase, push/merge and public static deploy
  with actual live verification. Current public manual-parity release is unchanged.
