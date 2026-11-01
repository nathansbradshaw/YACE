# YACE performance probe

Measures the real editor CLI on disposable **metadata-only** project copies and
the production Rust tempo/waveform algorithms. No recording, source project,
running editor, model, or provider is modified. Temporary copies are removed on
exit. A source change or executable change during the run fails the measurement.

```sh
cargo run --locked --release --manifest-path tools/performance-probe/Cargo.toml -- \
  /absolute/path/to/chart-engine /absolute/path/to/project > performance.json
```

Supply several project paths to compare sizes. Use an instrumental selected
track with at least one note. Projects must have `source/notes.chart`; the tool
copies only it and `project.json`. Actions requiring external media, original
undo checkpoints, or external generated-part files are outside this probe.

`YACE_PERF_SAMPLES=20` sets measured iterations (3–1000); each group also runs
two warmups. The identical-note update exercises content validation and saves,
but reuses the existing content checkpoint after warmup. It is a lower-cost
case than edits creating new checkpoint contents. Each action family starts
with the original project; its audit grows by one entry per iteration.

Optionally set `YACE_PERF_AUDIO=/absolute/path/to/recording.ogg` to measure
FFmpeg decoding plus PCM copying/validation at 100%, 75%, and 50% speed, with
at most five measured iterations per rate. This mirrors the current decode
command, not the complete stem/audio preparation pipeline. FFmpeg must be on PATH.

JSON contains executable/fixture hashes, workload sizes, median/p95/max times
in milliseconds, and explicit scope limitations. CLI wall time includes process
startup and output capture; it is **not input-to-paint latency**. `editor show`
also generates review inspection JSON and cannot substitute for `Store::load`
timing. Algorithm measurements always use this tool's release/debug profile,
independently of the supplied editor executable's profile. Use release mode.
Filesystem caches are warm, process contention is uncontrolled, and p95 with
five or ten samples is a coarse descriptive statistic. Do not subtract median
startup time and describe the remainder as an exact in-process measurement.

Verification:

```sh
cargo fmt --manifest-path tools/performance-probe/Cargo.toml -- --check
cargo test --locked --manifest-path tools/performance-probe/Cargo.toml
cargo clippy --locked --manifest-path tools/performance-probe/Cargo.toml --all-targets -- -D warnings
```

See [the audit and implementation plans](../../docs/PERFORMANCE.md) for the
remaining GUI, audio-output, inference and OCTAVE comparison measurements.
