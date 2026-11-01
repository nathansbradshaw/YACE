# Generation, song parsing and part splitting

Measured and repaired 2026-11-01. These are local release-build observations,
not an OCTAVE comparison or a general accuracy/speed claim.

## Changes

- Native stem validation now retains up to 64 process-local, metadata-only
  proofs of decoded frame counts. Every reuse freshly hashes the audio and
  checks canonical path, file identity and decoder/probe identity. A timestamp
  is never sufficient. A miss still checks native stem format, finite decoded
  samples and exact frame counts; failures are never retained. Platforms
  without the implemented stable identity use ordinary validation. Playback
  still loads actual PCM normally; no PCM or project validation is cached.
- Manifest construction/validation no longer allocates and discards the source
  plus all six decoded stems together. Cold validation releases each file's
  PCM before processing the next. This reduces the validation working set;
  process-tree peak memory was not measured.
- Cached separation wrappers reuse the model digest already computed for
  identity instead of immediately hashing the same weights again. Default and
  HQ weights still require their pinned digest; existing cache/inference checks
  remain. No model math, separation parameters or cache-key algorithm changed.
- Tempo analysis computes the identical Hann coefficients once, rather than
  repeating their cosine calculation for every FFT frame. Proposal semantics
  and thresholds are unchanged.

## Reproduction and boundaries

The [Rust example](../examples/generation-performance.rs) uses production
paths, installed local models and temporary outputs:

```sh
cargo run --offline --locked --release --example generation-performance -- \
  /path/to/short-source.wav /path/to/models /path/to/full-song/song.ogg
```

FFmpeg, ffprobe and the default separator's GPU backend must be available.
No downloads are requested. A sibling `notes.chart` is parsed when present.
Each complete job starts a fresh worker process, matching desktop generation.
Queued-job creation is outside its timer. The warm job receives a copy of the
cold job's matching on-disk separation cache **before** timing; process-local
proofs do not survive between workers. This also respects the new per-song
`.yace/cache/separation` layout. Standalone validation/cache/parse repeats run
five times in one process. Cold means an empty application separation cache,
not cold filesystem, driver or GPU shader caches.

Matched before/after binaries used the same harness and song-folder base
`1fa4566ed2597409286934baba3677b0608860c2`, Rust 1.97.1, FFmpeg 8.1.2 and default
release features on this ARM64 Mac. The after binary includes the performance
changes described above. Subsequent rebases preserve the adjacent metadata
lookup changes; those changes were not credited as performance improvements.
The identical harness has Git blob `d990e7cb41e03b09a74c6bfcda076e9302fabbc5`.
The generation/splitting fixture is a 12-second local CC0 excerpt. Tempo and
text parsing use a separate 243.88-second recording and 13,601-note chart.
Source hashes are recorded in the JSON; recordings and models are not included.
Original recordings remained unchanged.

## Results

| Operation | Before | After | Interpretation |
| --- | ---: | ---: | --- |
| Complete generation, empty separation cache | 15.655 s | 14.287 s | 9% less wall time |
| Complete generation, matching disk cache | 4.719 s | 3.454 s | 27% less wall time |
| Standalone six-part split, empty cache | 11.647 s | 11.430 s | No demonstrated inference gain |
| Standalone six-part cache hit, median | 706.8 ms | 311.6 ms | 56% less wall time |
| Stem validation, median | 352.1 ms | 53.8 ms | 85% less wall time |
| Full-song tempo analysis, median | 222.1 ms | 201.7 ms | 9% less wall time, includes decoding |
| Full text chart parse plus all note sections, median | 1.30 ms | 0.95 ms | Parser unchanged; timing variation |

End-to-end and cold-split rows contain one observation per condition, not a
statistical distribution. Other rows use five observations. Host contention is
uncontrolled. A prior after run overlapped compilation and measured 18.14 s
cold / 6.17 s warm; it is retained as `after-contended.json`, not treated as a
regression or discarded. Earlier `exploratory-legacy-*` results used base
`3b33796` and a different job/cache harness and must not be compared directly
with the matched pair. All [raw results](performance-baseline/generation-2026-11-01/)
are retained so these boundaries are reviewable.

Both complete jobs produced exactly **589 notes**, with identical chart SHA-256
before/after and cold/warm:
`f5dea4505f1040bad6a834a874f360354cd80d8064b4e159875683107065ad3a`.
Primary transcription and tempo-proposal hashes are also identical in the
matched results. This is output equivalence on these fixtures, not musical
accuracy acceptance.

## Verification and next bottlenecks

Local checks passed on `bb9b6b7` plus these changes: the full default all-target
suite (654 passed, 46 intentionally ignored), default/no-default all-target
Clippy with warnings denied, formatting, diff whitespace, shared static WASM
compilation and an optimized build of the benchmark example. Optional hosted
CI was skipped under the user's CI-budget instruction; main had no required
branch checks or rulesets at verification.
Regressions exercise same-length timestamp-preserving tampering, identical-byte
file replacement, uncached failures, invalid frame counts, nonfinite samples,
invalid native sample rate and unpinned default/HQ model files. Existing
changed-guitar diagnostics and cache isolation tests remain intact.

1. **Cold splitting dominates this fixture.** Instrument separator model
   loading, GPU upload/compile, chunk inference, reconstruction and output
   writes separately. Compare repeated sessions against fresh worker startup
   before choosing a persistent worker/session cache. Bound its retained memory
   and preserve cancellation, model identity and per-song isolation. Do not
   reduce overlap, skip stems or alter model precision without quality checks.
2. **HQ splitting still needs a separate benchmark.** Its CPU ONNX path was
   not accelerated in this pass. Measure supported hardware providers on the
   same inputs with reconstruction, note-quality and fallback checks before
   selecting one. No HQ neural-throughput gain is claimed here.
3. **Repeated inference setup remains.** The measured primary drum/guitar/bass
   phases were about 117/172/226 ms before and 113/162/224 ms after, with no
   intentional model change. Profile decode/resample/session initialization
   separately before sharing audio buffers or sessions across analyzers.
4. **Long-song coverage remains.** Repeat complete generation and peak
   process-tree memory measurements on 3-, 10- and 60-minute inputs, including
   vocals, keyboard/HQ and cancellation. The 12-second gains cannot establish
   full-song scaling. Text-chart parsing itself is not the leading cost in the
   measured 13,601-note case; project loading/persistence is tracked in the
   [broader performance audit](PERFORMANCE.md).

Source verification is separate from installation. Canonical native packaging
and desktop/web UI acceptance are coordinated with the parity/package owner.
