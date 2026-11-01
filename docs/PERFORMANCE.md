# Performance audit and repair plans

Audit date: 2026-11-01. Product objective: make YACE feel at least as immediate
as OCTAVE, with measured advantages where Rust makes a difference. Choosing
Rust does not establish any performance advantage by itself.

Implemented follow-up: [generation, song parsing and part splitting](GENERATION_PERFORMANCE.md)
records the native validation/model-verification and tempo-window changes,
matched before/after measurements and remaining inference bottlenecks.

The dominant confirmed design problem is that interactive commands do work
proportional to the entire saved project. Playback also rebuilds a substantial
SVG subtree every frame. Audio preparation repeats full-song decoding, and most
ONNX paths do not request hardware acceleration. These are different problems
with different remedies. This document plans those remedies; it does not claim
they are implemented or that every possible bottleneck has been discovered.

## Sources and measurement boundaries

- Audited public source: `9f0aae9` on `nathansbradshaw/YACE` main. Work took place
  in an isolated worktree; ongoing desktop-workflow changes were inspected only
  as context and are not part of this baseline.
- Canonical packaged executable: `chart-engine/output/YACE.app/Contents/MacOS/chart-engine`.
  Its actual SHA-256 was
  `f8f844d3d37c6b43f33e611b315df1f5175e94be5e8b8330fa6d241e61a18f70`
  before and after each probe. Bundle metadata names source commit
  `571993953fe13ca1d18fcdfdfbe359cfa14566b6`, clean. The source difference from
  that commit to audited main is README text and a screenshot only.
- Bundle `build-info.json` records a different executable hash, `7099c84f…`.
  [The packager](../src/bin/package-macos.rs) records the input executable hash
  **before signing the staged app**, which can change executable bytes. Treat
  it as a pre-signing hash, not verification of installed bytes. Improve this
  provenance before relying on it for performance comparisons.
- A separate running process, PID 81940 during the audit, resolved through
  `lsof` to `Documents/GitHub/YACE/target/debug/chart-engine`, SHA-256
  `283928700004d7c00687f2e19bb1997192c4beb23bbb94a93430d94fe0f7d925`.
  Its checkout was clean at `9f0aae9`; the binary has no embedded build witness,
  so the checkout alone does not prove its complete build inputs.
- Hardware reported ARM64, 15 logical CPUs, 48 GiB RAM. The volume had about
  26 GiB available. Other builds were active. Tests were sequential with warm
  filesystem caches; these are local baselines, not controlled lab results.
- The OCTAVE source snapshot was `6026796a62405c8a6c52c2a5cb29fb9410d1138e`,
  version 1.0.17. Its source was read, not executed. Computer-use lookup could
  not find a running/launchable OCTAVE application. No live competitor timing,
  end-to-end GUI input latency, cold-window startup, current playback frame
  capture, acoustic latency or model-throughput comparison was completed.
- An idle three-second sample of the running debug process showed periodic
  `Studio::poll` → `Store::report_live` work on the main thread. Footprint was
  491.9 MiB with a reported lifetime peak of 1.7 GiB. It does not identify what
  caused that peak or establish an active-editing CPU profile. WebView child
  process memory is not included in that footprint.

The reusable [Rust probe](../tools/performance-probe/README.md) runs actual
editor commands on temporary metadata-only copies, checks source/executable
hash stability, and imports production tempo/waveform code. It never edits the
original projects. Raw results are in [performance-baseline](performance-baseline/).
No recordings, project contents, lyrics, or model weights are committed there.

## Reproduced costs

Two fixtures were used: a 7-note, 2-track, 6,186-byte project and a 13,601-note,
12-track, 10,288,932-byte project. The latter has only six audit entries; a long
editing history is **not required** to reproduce the expensive command path.

Final release run: 20 samples and two warmups per operation. Running-debug
executable: 10 samples and two warmups. All values below are milliseconds.

| Full-song operation | Release median / p95 | Debug median / p95 |
| --- | ---: | ---: |
| Select track | 48.91 / 51.79 | 479.90 / 546.80 |
| Seek | 47.35 / 48.37 | 481.83 / 496.70 |
| Pause | 49.49 / 50.83 | 475.26 / 502.23 |
| Identical note update | 75.59 / 78.08 | 990.55 / 1,010.70 |
| Full CLI inspection | 84.05 / 89.57 | 926.48 / 960.20 |

These are **subprocess wall times**, including startup and output capture, not
direct measurements of UI events. Native UI dispatch additionally reloads and
publishes state before painting. Full inspection computes review JSON and is
not a direct measurement of `Store::load`. The note-update case reuses an
existing content checkpoint after warmup; a novel edit can cost more.

Release `--help` took 5.63 ms median; the tiny fixture's release commands took
about 9–11 ms median. The original release run during heavier contention had
62.72 ms median selection and 95.81 ms median update. Keep both results; do not
claim their difference is an optimization. Debug-versus-release results show
a large build-mode contribution but are not a controlled compiler-only A/B.

Production `TempoMap::seconds`, for 4,096 distributed queries per sample:

| Tempo events | Release median / p95 (ms) |
| --- | ---: |
| 1 | 0.0035 / 0.0036 |
| 100 | 0.1246 / 0.1338 |
| 1,000 | 1.2812 / 1.3060 |
| 10,000 | 12.8486 / 12.8933 |

This establishes linear tempo-map scaling. It does not establish that the real
song has 10,000 tempo changes. A cached 60-second synthetic waveform took
0.84 ms median to build and 0.34 ms to produce a 1,400-pixel viewport. That
geometry generator is not a leading measured CPU cost on its own.

On the full-song source recording, 3,784,912 compressed bytes became
86,040,864 bytes of stereo f32 PCM at 100% speed. The decode subprocess plus
copy/validation cost 174 ms median. At 75% it took 333 ms and produced
114,717,056 bytes; at 50% it took 432 ms median / 650 ms maximum and produced
172,077,176 bytes. Five measured samples per rate; these exclude hashing,
waveforms, device creation and all additional stems.

At 44.1 kHz stereo f32, memory is 352,800 bytes per source second per buffer.
A five-minute original plus six stems is about 741 MB of PCM at normal speed,
before envelopes, subprocess output, model tensors and UI memory. Slower
practice audio increases output duration. This is a capacity calculation,
not a measured whole-process peak.

## Bottleneck inventory

Evidence labels: **measured** means reproduced locally above; **source** means
the cost exists in inspected code but its share of user-visible time remains
unmeasured; **hypothesis** requires a trace before changing the design.

| ID / priority | Area and evidence | Concrete cost | Repair and verification |
| --- | --- | --- | --- |
| B01 / P0 | Build/launch; measured | A debug executable is currently running, and large-project debug commands cost roughly half a second to a second. | Use the canonical release app for acceptance. Add build profile, source revision and installed-byte identity to diagnostics. Reproduce identical workflows on release and dev builds. This does not replace the remaining fixes. |
| B02 / P0 | Interactive persistence; measured + source | `Studio::dispatch` synchronously calls `Store::apply`; selection, seek and pause load/validate the project, rebuild its chart hash, append audit, serialize all project JSON and `sync_all`, then `reload` loads/validates/hashes again. | Keep a live authoritative Rust session; move durable work to a serialized worker. Separate session/transport state from content revisions with an explicit compatibility design. Preserve CLI/AI revision checks, recovery and durable-save acknowledgements. Trace every phase. |
| B03 / P0 | Repeated immutable-source work; source | `source_chart` reads, SHA-hashes and parses the same immutable chart through `load`, `timing`, `validate_notes`, `chart_text` and `reload`. `chart_hash` reconstructs/sorts the whole chart even after selection or transport changes. | Cache verified source bytes/parse/timing and content hashes by their actual dependencies. Revalidate when source identity changes; preserve source-tamper detection and external edit handling. Count reads/parses/hash recomputations per event. |
| B04 / P1 | Checkpoints and growing audit; source | Content edits clone complete `Content`, serialize/hash full checkpoints and rewrite the full project plus all previous actions. `SetVocals` can embed a whole vocal chart in each audit entry. History increases future command cost. | Introduce versioned edit deltas, periodic full checkpoints and compact append records on the persistence worker. Preserve exact undo/redo, actor history and crash recovery. Benchmark 100/1,000/10,000 actions, real new checkpoints and full vocal edits. |
| B05 / P1 | Reactive invalidation; source | One `doc` signal changes after transport/selection as well as content. Track geometry, classification and the shared difficulty snapshot depend on it; changing selection can invalidate analysis for unchanged tracks. | Publish independent content/track, timing, selection, review and transport versions. Cache derived state by those inputs, not the global revision. Assert zero reclassification/difficulty recomputation for pure seek/playback/selection. |
| B06 / P1 | Highway drawing; source | Each playhead update formats the highway's SVG definitions, board, rails, grain, beats, gems and receptors, then replaces `dangerous_inner_html`. The WebView reparses and paints the subtree. Note visibility scans the whole classified track and sorts visible notes each frame. | First measure Rust geometry, IPC bytes, DOM replacement and paint separately. Retain static geometry; update dynamic transforms/attributes; index the visible time interval. Prototype a retained GPU/canvas surface if that still misses the budget. Compare implementations before committing to a renderer rewrite. |
| B07 / P1 | Frame delivery/IPC; source | Native RAF sends a tick across the bridge every display frame, Rust publishes a signal, and DOM updates return across the bridge. A 16 ms fallback timer also wakes continuously. High-refresh displays increase traffic; static WASM uses a 16 ms timer rather than display pacing. | Keep one authoritative audio clock; coalesce outstanding ticks and bound queue depth. Use display-paced presentation, isolate playhead consumers, pause timers/animation when inactive, retain a tested fallback. Measure 60/120 Hz, stalls, background/foreground, seek and loops. |
| B08 / P1 | Timeline scroll/zoom; source | Instrument `track_geometry` scans all notes, converts both endpoints and clones visible notes on every viewport invalidation; note handlers/attributes rebuild. `is_cymbal` scans the whole drum track repeatedly per visible note. | Use sorted onset/range indexes plus sustain-overlap lookup, a marker map keyed by tick/lane, bounded viewport geometry and shared note references. Coalesce pointer/zoom events once per display frame. Check long sustains crossing into the viewport and drum-marker correctness. |
| B09 / P1 | Vocal editor/preview; source | `VocalRoll` builds SVG nodes/labels/handlers for the entire vocal-note list and every phrase; CSS clipping does not cull construction. Vocal preview also filters the full list each update. Vocal changes submit a complete vocal document. | Cull phrases and syllables by time/pitch viewport, retain stable indices/IDs, and use a local edit patch/one atomic gesture commit. Verify pitch rows, lyric continuations, selection and phrase changes. Stress a 10,000-syllable fixture. |
| B10 / P1 | Tempo queries; measured | `seconds` and `tick` scan from the first tempo event; geometry, validation, snapping and review multiply that cost by notes/queries. | Build prefix-seconds segments with binary search and optionally a monotonic cursor. Verify zero/negative offsets, tempo boundaries, inverse rounding and all supported PPQ values against the current implementation. Target the 10,000-event benchmark separately from constant-tempo songs. |
| B11 / P1 | Review/difficulty work; source | Region fingerprints scan notes and serialize/hash scoped content during status lookups. Review lists and difficulty snapshots recompute against coarse project dependencies. Group retention can repeatedly search notes by ID. | Add note-ID and region interval indexes; cache fingerprints/status/rules by content, timing and policy. Virtualize long review lists. Verify that stale approvals still invalidate and linked difficulty ancestry survives edits/undo. |
| B12 / P1 | Audio open and rate changes; measured + source | `load_review_audio` hashes/decode-loads original plus supplied stems, rebuilds waveform pyramids and writes peak caches. A non-1× rate then decodes original and every stem again with `atempo`. Existing peak files are persisted but this path rebuilds them. | Share verified source PCM/envelopes per recording, load the original first, prepare requested stems lazily, and cache bounded rate variants by source hash/settings. Reuse waveforms across practice speeds. Trace time to first audible original separately from time to all stems. |
| B13 / P1 | PCM memory and cancellation; measured + source | FFmpeg `.output()` holds complete stdout while a second f32 vector is built. Full-song original/stem buffers coexist; previous audio remains referenced during replacements. An epoch discards stale preparation results but does not stop already-running `spawn_blocking` decoding. | Stream decode into bounded chunks or shared PCM/mapped cache; bound memory by explicit budget and evict rate variants. Cancel subprocesses/cooperate between decode stages. Stress repeated speed/track switches and long songs while measuring peak aggregate RSS and wasted work. |
| B14 / P2 | Real-time audio; source | `ReviewSource::next` takes an `RwLock` for every stereo frame (44,100 reads/s at source rate) and computes mixing per frame. Rate replacements recreate the output device. The preferred output buffer is 512 frames, with an unmeasured fallback. | Snapshot mix state per audio block without blocking the callback, reuse the output device, and move preparation outside it. Measure callback deadlines/underruns and acoustic start/seek latency. Preserve atomic stereo mixing and the one-cursor stem clock. Do not shrink buffers blindly. |
| B15 / P2 | Idle telemetry/polling; source + idle sample | `poll` stats the project and writes live-state every ~288 ms even while paused; frame batches also write sidecars. This work runs in UI tasks. | Move diagnostics to a worker, emit only changed state at a bounded rate, use notifications plus a fallback for external edits. Gate detailed sampling explicitly. Verify external automation still sees correct state and idle wakeups/file writes drop. |
| B16 / P1 | Model execution; source | Basic Pitch, ADTOF, Kong, MuScriptor, Transkun, LarsNet and other ONNX sessions use fixed CPU thread counts without an explicitly requested accelerator. HQ keyboard separation explicitly identifies itself as CPU fp32. Kong disables graph optimization. Rust wrappers do not make tensor kernels faster. | Time model verification/load, frontend, inference and decoding separately. Benchmark CPU thread budgets, safe graph optimizations and supported accelerators, including actual provider/fallback reporting. Preserve shapes, output quality and source clock; require numerical and musical regression checks before adopting a backend. |
| B17 / P2 | Generation reuse/scheduling; source | Several analyzers decode the same stem at different rates, verify large model files multiple times and load fresh model sessions; instrument stages in `transcribe_with_isolation` are largely sequential. Mixed HQ Piano jobs can deliberately require two separate separator models. | Build a bounded job dependency graph, share decoded/resampled audio and verified model/session handles, cache raw activations by all relevant identities, and run independent CPU/GPU work only within a measured resource budget. Preserve intentional source routing; do not substitute one separator's outputs to save time. |
| B18 / P2 | Separation; source | Default Demucs uses Burn/WGPU, then full-song channel/output buffers and six sequential audio encodes. HQ separation uses overlapping CPU ONNX chunks. Existing Demucs output extraction already batches GPU readback into two sync points for all stems. | Trace model load, GPU/CPU frontend, chunk inference, readback, reconstruction, encoding and cache validation. Reuse buffers/plans, pipeline bounded encoding, compare chunk/batch sizes and backend support. Do not recommend the already-present bulk-readback optimization again. |
| B19 / P1 | Static web open/import; source | Rust/WASM decoding copies compressed input, accumulates full PCM, makes a WAV buffer and copies it to a Blob for media playback. Stems load sequentially; hashing, waveform building and ZIP work also use the browser thread. Decoder yields every 256 packets, not a measured time budget. | Move decode/hash/ZIP/waveform work to a Rust/WASM worker with transferable buffers; progressively open the chart and original audio. Consider direct compressed playback with a separately cached waveform where codec support permits. Verify memory, main-thread long tasks, codec fallback and sample/timing alignment. |
| B20 / P2 | Static web saves/export; source | Browser dispatch still runs synchronous shared-store validation/copying. Every dirty save enumerates file manifests, copies dirty bytes into IndexedDB arrays and writes project/checkpoint records. ZIP export assembles archive bytes in memory. Autosave is polled at 500 ms. | Use incremental content records, worker-side encoding, bounded archive streaming and durable generation IDs. Retain IndexedDB transaction/revision conflict checks, retry semantics and honest save status. Measure input stalls during autosave and large exports. |
| B21 / P2 | Startup, library/import/export; source + hypothesis | CLI startup itself was about 6 ms warm, but WebView/window startup is unmeasured. Project open already uses background work in several stages; immutable source parsing is repeated later. Recursive scanning parses each song; native export dispatched from UI can copy media/hash/render complete outputs synchronously. Artwork/base64 can amplify copies. | Add first-window/usable-editor/first-audio milestones, incremental cancellable library scans, thumbnail caches and background export with progress. Profile the actual path before blaming startup on model loading. Do not initialize model sessions just to open a chart. |
| B22 / P0 | Performance evidence; confirmed gap | Existing frame reports cover short continuous-playback windows and do not capture input latency, opening, dragging, speed preparation or total browser paint cost. A prior 120-frame browser result cannot establish overall speed or OCTAVE parity. | Add stage spans and input-to-present correlation, queue depth, IPC byte counts, memory, frame-tail/long-task capture and reproducible workload/identity metadata. Keep low-overhead counters separate from full tracing and account for observer overhead. |

The CSS was also inspected. There is no measured evidence that generic theme
styling is a leading cost. Avoid a cosmetic rewrite as a performance fix.
Network import/lyrics/assistant wait time must be timed separately from local
work; provider latency and downloads cannot be attributed to Rust execution.
Assistant requests, generation jobs and several analysis/open paths already use
blocking workers or subprocesses. The problem is not that every task runs on
the UI thread; it is that critical interactive paths still do.

Owning code:
[commands/store](../src/editor.rs), [native session](../src/studio/state.rs),
[preview](../src/studio/game_preview.rs), [note lanes](../src/studio/note_canvas.rs),
[gestures](../src/studio/note_gestures.rs), [selection](../src/studio/selection.rs),
[frame pacing](../src/studio/frame_pacing.rs), [difficulty memo](../src/studio/difficulty.rs),
[review](../src/studio/review.rs), [tempo](../src/timing.rs),
[waveforms](../src/waveform.rs), [stem loading](../src/stem_review.rs),
[model/decode paths](../src/models.rs), [generation pipeline](../src/pipeline.rs),
[HQ separator](../src/keyboard_separation.rs), [Kong](../src/piano_model.rs),
[Demucs readback](../vendor/demucs-core/src/lib.rs),
[WASM session](../web/src/studio/state.rs), [WASM media](../web/src/studio/media.rs),
[WASM decode](../web/src/audio_decode.rs), [IndexedDB](../web/src/persistence.rs).

## What OCTAVE does differently

The pinned source uses in-memory Zustand operations for selection and current
tick, canvas note/grid/playhead layers in `MidiEditor.tsx`, and a Three.js /
React Three Fiber canvas for its highway. YACE uses Rust Dioxus logic but still
renders through a WebView on desktop, and much of its highway is rebuilt SVG.
Neither implementation's language alone predicts the faster interaction.

OCTAVE also scans notes when building its visible-note list and throttles some
store playback updates to roughly 30 Hz. It is not a perfect architecture to
copy wholesale. Borrow the separation of live editing, persistence and retained
drawing; verify the result against both apps on matching workloads.

Primary source comparison:
[OCTAVE note rendering](https://github.com/opria123/octave/blob/6026796a62405c8a6c52c2a5cb29fb9410d1138e/src/renderer/src/components/MidiEditor.tsx),
[store](https://github.com/opria123/octave/blob/6026796a62405c8a6c52c2a5cb29fb9410d1138e/src/renderer/src/stores/songStore.ts),
[highway canvas](https://github.com/opria123/octave/blob/6026796a62405c8a6c52c2a5cb29fb9410d1138e/src/renderer/src/components/ChartPreview.tsx),
[playback](https://github.com/opria123/octave/blob/6026796a62405c8a6c52c2a5cb29fb9410d1138e/src/renderer/src/services/playbackController.ts).
These claims were checked against the local pinned source snapshot.

## Sequenced implementation plans

Effort estimates below are planning ranges for focused engineering work, not
delivery promises. Each change should be independently reviewable, verified,
merged into main, and exercised in desktop and static-web builds where shared.

1. **Identity and baseline (small, 1–2 days): B01/B22.** Preserve canonical
   `YACE.app`, path and bundle identifier. Add trustworthy release/debug/source
   identity and trace spans for input → mutation → validation → persistence →
   publication → paint. Record fresh matched OCTAVE baselines once launchable.
   Keep the Rust probe as a fast diagnostic, not the sole acceptance test.
2. **Immediate interactions (large, roughly 1–2 weeks): B02–B05.** Cache the
   immutable source/timing, stop computing chart hashes for session-only changes,
   introduce the live session and serial persistence worker, then split state
   subscriptions. Design transport/selection revision compatibility before
   changing it. First target: selecting/dragging/seeking never waits for full
   project JSON, full-content validation or fsync. Saving stays honest and
   recoverable; source changes and stale external commands still fail correctly.
3. **Bounded drawing (medium/large, several days to 2 weeks): B06–B11.** Add
   tempo and note/marker/region indexes; cull vocals; isolate static, selection
   and playhead layers; coalesce input and ticks. Try retained SVG updates before
   choosing a Rust-controlled canvas/GPU surface. Escalate to a renderer change
   only with a recorded paint/IPC bottleneck and a faster prototype.
4. **Fast audio readiness (medium/large, about 1 week plus audio validation):
   B12–B15.** Cache bounded source PCM/envelopes, reuse the output stream, start
   original audio before all stems are ready, cancel superseded preparation,
   then consider streaming time stretch. Remove per-frame callback locks only
   with correctness/underrun evidence. Keep one source clock and stem alignment.
5. **Generation throughput (large, model-dependent, 1–3 weeks): B16–B18.** Add
   per-stage real-time factors and actual backend reporting first. Compare CPU
   budgets and supported accelerator execution, reuse sessions/decoded audio,
   then add bounded pipeline concurrency. Preserve current source/model/cache
   identity, outputs and quality gates. Do not parallelize all models at once.
6. **Browser/large-project completion (medium/large, about 1–2 weeks):
   B04/B19–B21.** Worker decode/hash/ZIP, incremental durable records, long-history
   compaction/checkpoints, progressive import/library results and background
   exports. Check desktop/web parity and peak memory, not just WASM CPU timing.

Plans 2 and 4 should produce the largest immediate usability gains. Generation
speed work will not fix a slow note selection. Avoid another framework/language
rewrite until the measured architecture improvements have been evaluated.

## Proposed acceptance gates

Treat these as initial engineering targets to calibrate against hardware and
OCTAVE measurements, **not present capabilities**:

| Workflow | Target and required evidence |
| --- | --- |
| Select, seek, start/pause, single-note edit | Release input-to-visible-feedback p95 ≤50 ms on the 13.6k-note fixture; p99 and max reported. No full-project serialization/validation on the UI thread. Durable-save completion measured separately and clearly indicated. |
| Drag, pan, zoom | Feedback within one display refresh normally, p95 ≤33 ms on 60 Hz; no main-thread task >50 ms in a 60-second scripted interaction. Document behavior at 120 Hz and under generation load. |
| Playback | Frame/chart-presentation p95 ≤16.7 ms at 60 Hz, p99 ≤33 ms, zero unexplained gaps >50 ms over 60 seconds. Verify seeks, loops, pause/resume, background recovery and high-refresh pacing separately. Track acoustic latency independently. |
| Cached playback rate / stem switching | Immediate UI feedback ≤50 ms, cached audible readiness ≤100 ms; explicit progress/cancellation for uncached work. No accumulating decoder tasks or unbounded retained variants after repeated switching. |
| Open/startup | Provisional targets: warm usable empty window ≤500 ms, cold ≤1.5 s; warm existing chart visible ≤500 ms before optional stem preparation. Record first audio, cold-cache behavior, original-only versus six-stem timing, and host conditions before adopting final thresholds. |
| Dense tempos | 4,096 distributed conversions over 10k tempo events ≤1 ms in release, with boundary/roundtrip equivalence tests. Constant-tempo results must not regress materially. |
| Long projects/history | Exercise 1k/13.6k/50k/100k notes; 1/100/1k/10k tempos; 100/1k/10k edits; 3/10/60-minute audio; original plus zero/six stems. Report aggregate process-tree peak memory and memory retained after project switches. |
| Generation | Report seconds per source minute per stage and end-to-end cold/warm cache, backend, thread count and peak memory. Target ≥2× only for stages with a demonstrated avoidable bottleneck; measure musical/output parity before accepting throughput wins. |
| Versus OCTAVE | Same recording/chart, comparable feature load, zoom, track count, machine, display rate and cold/warm state. At least 30 repetitions of discrete actions and 60-second interaction/playback captures. Meet OCTAVE's p95 on all core editing workflows; aim for ≥20% better on explicitly selected speed-critical workflows. No blanket speed claim from CLI microbenchmarks. |

Recovery tests are mandatory for the persistence changes: crash after accepted
edit, disk full, failed fsync, stale external revision, conflicting browser tab,
undo/redo after restart, changed source and cancelled project switch. Optimizing
must preserve authored data, audit/recovery history, export fidelity and musical
acceptance boundaries. Performance results must show both median and tail
latency with exact workload/build identity and raw traces or probe JSON.

## First repair: verified source reuse and command serialization

Implemented against main `3597d20`, preserving that version's immediate pause,
library switching, retained stems, generation feedback and loading lifecycle.
The first implementation checkpoint is `db8d91f`; this is a partial repair of
B02/B03, not completion of the live-session/persistence-worker plan.

- Each live `Store` shares one bounded source snapshot with its clones. Every
  source access still reads and compares the actual current bytes. Matching
  bytes reuse parsed chart/tempo data and lazily cached sorted source notes.
  Modified bytes or a different expected hash are rejected; timestamps are
  never trusted as proof of unchanged content. No project validation result
  is cached, and there is no process-global accumulating source cache.
- Cursor, playback and selection actions retain their handler checks and the
  complete validation on project load, while avoiding a second full-content
  validation after an action that cannot change content. Content edits,
  undo/redo, recovery, stale revision checks and durable saves are retained.
- The editor command lock now remains held across loading, revision checking,
  preconditions, mutation and saving. Previously the guard was scoped to a
  short block and dropped before the transaction, allowing another command
  to save between a revision check and its commit. A regression test exercises
  competing commands and verifies that a fresh command succeeds afterward.

The new [in-process example](../examples/editor-performance.rs) complements the
subprocess probe by timing a long-lived store on a disposable metadata-only
copy. Run it in release mode:

```sh
cargo run --locked --release --example editor-performance -- /path/to/project
```

The identical example was compiled against `3597d20` and the implementation,
with the same default release features and Rust 1.97.1. On the original
13,601-note, 10,288,932-byte fixture, a repeated adjacent pair measured:

| In-process operation | Before median / p95 (ms) | After median / p95 (ms) |
| --- | ---: | ---: |
| Load and validate project | 14.58 / 15.32 | 13.41 / 13.66 |
| Resolve effective timing | 0.604 / 0.643 | 0.0184 / 0.0209 |
| Render/hash chart | 1.783 / 1.825 | 0.462 / 0.501 |
| Apply and durably save seek | 41.07 / 42.43 | 29.26 / 30.28 |

Twenty samples plus two warmups per operation. Seek median improved by about
29%; timing and hashing are narrower microbenchmarks, not whole-editor speedups.
The subprocess measurements showed smaller gains and variable inspection
timings, so they do not support a general application-speed claim. Source
projects remained byte-identical. Host contention remains uncontrolled.
Raw [in-process and subprocess results](performance-baseline/interactive-2026-11-01/)
include initial/repeated observations rather than only the fastest sample.

Validation: full default-feature local all-target suite (620 passed, 45 ignored),
focused competing-save, source-tampering and invalid-content regressions,
default-feature all-target Clippy with warnings denied, formatting/diff checks,
and compilation of the shared static WASM app. The source-tampering regression
also restores the file's previous modification timestamp after a same-length
edit. A sandbox-blocked loopback HTTP mock required rerunning the local suite
with loopback permission; no real provider requests were needed.

This does not remove project JSON serialization/fsync from interaction, skip
initial validation, change project/command JSON, or demonstrate input-to-paint
latency. Source measurements do not establish installation or device acceptance;
canonical app packaging and UI verification remain separate integration work.
Next: the serialized persistence worker/live session and narrower UI state
subscriptions, coordinated with the shared UI owner.
