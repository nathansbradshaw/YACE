# Official keyboard reference benchmark

Rust-only, local read-only inventory and reference measurements for installed
SNG/YARGSONG charts. Original archives are never modified or uploaded.

```sh
cargo run --release --manifest-path tools/official-keyboard-benchmark/Cargo.toml \
  --bin official-keys-audit -- /path/to/setlist/installation > inventory.json
cargo run --release --manifest-path tools/official-keyboard-benchmark/Cargo.toml \
  --bin official-keys-audit -- extract /path/to/song.yargsong /new/output/folder
cargo run --release --manifest-path tools/official-keyboard-benchmark/Cargo.toml \
  --bin reference -- infer piano clip.wav models inference.json
cargo run --release --manifest-path tools/official-keyboard-benchmark/Cargo.toml \
  --bin reference -- infer synth clip.wav models inference.json
cargo run --release --manifest-path tools/official-keyboard-benchmark/Cargo.toml \
  --bin reference -- compare notes.mid inference.json 0 5 25 comparison.json
```

`compare` arguments specify the original recording time at the start of the
input clip and the absolute evaluation start/end. It uses the original MIDI
tempo map, and does not fit a timing shift. Use context outside the scored
window, e.g. score 5–25 seconds of a 0–30 second inference clip.

`compare-baseline` accepts the same arguments but uses the frozen
`pro-keys-register-v3` and five-lane arranger in `src/baseline_*.rs`. These
historical copies are measurement controls, never a production fallback.
For a controller-only comparison, pass identical inference to both commands.
For the full route, use the historical detector output with
`compare-baseline` and the new detector output with `compare`; preserve both
inputs. Compare exact register and pitch-class scores separately.

Reference Pro Keys pitches are 48–72 in `PART REAL_KEYS_X`; Expert five-lane
gem pitches are 96–100 in `PART KEYS`. The report includes strict pitch/lane
and octave-equivalent one-to-one 50ms onset matching, independent offset
matching, raw notes, current controller arrangements, and all decisions.
The test module includes a golden tempo-change MIDI, velocity-zero releases,
and the shared scorer's exhaustive matching/duplicate/offset regressions.

`infer` runs the exact native detector and synth selection function, without
separation. To test the complete piano route, use the production CLI:

```sh
chart-engine transcribe --audio full-mix.wav --model-dir models \
  --instruments piano,pro_piano --keyboard-sound piano --output events.json
reference compare notes.mid events.json 0 5 25 comparison.json
```

`piano-input SOURCE MANIFEST MODELS NEW_OUTPUT` reuses an already source-bound
separation manifest, runs the exact production piano detector/selection and
validates its complete transcription receipt. It also checks that input
preparation reproduces the recorded sample hashes and scalar gain. This avoids
confounding detector comparisons with a repeated separation run.

Compare both the supplied keyboard audio and the full production path. Freeze
rules before examining held-out chart scores. Retain failures. A Keys chart
can intentionally simplify, omit, or transpose real accompaniment, so an
unmatched detection is a chart disagreement rather than automatic proof of a
hallucination. This helper does not certify instrument identity or live input
latency. Large song assets and model weights remain local under ignored output.

The container reader adapts the YARG.Core IO format implementation; see
`NOTICE.md` and `LICENSE`. The referenced scoring module and YACE library are
MIT-licensed under the root project license.

`native-routing EXECUTABLE RUNNING_QA_PROJECT NEW_OUTPUT_DIRECTORY` checks a
running native editor through its existing revision-checked command API. Use
an isolated mixed guitar/keyboard QA project with independent keyboard stems.
It observes a pending keyboard load, cancels back to the existing audio,
checks actual player state and cursor progress, then auditions the keyboard
source and restores original transport/mix preferences. Requests and live
samples are retained; notes, approvals and source attachments must stay equal.
This checks native routing, not acoustic acceptance or hardware latency.
