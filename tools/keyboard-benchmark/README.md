# Native keyboard benchmark

All inference, scoring and audition synthesis are Rust. FFmpeg is required for
audio format handling. No Python runtime is used. The root application's model
files and the separately licensed benchmark soundfont must already be available.

From the chart-engine repository:

```sh
cargo build --release --manifest-path tools/keyboard-benchmark/Cargo.toml
tools/keyboard-benchmark/target/release/yace-keyboard-benchmark prepare \
  --output output/keyboard-benchmark --soundfont /path/to/TimGM6mb.sf2
tools/keyboard-benchmark/target/release/yace-keyboard-benchmark run \
  --corpus output/keyboard-benchmark/corpus.json \
  --model-dir models --detector kong
tools/keyboard-benchmark/target/release/yace-keyboard-benchmark report \
  --corpus output/keyboard-benchmark/corpus.json \
  --soundfont /path/to/TimGM6mb.sf2 \
  --name new-listening-report
cargo test --release --manifest-path tools/keyboard-benchmark/Cargo.toml
```

Run `basic`, `kong` and `transkun` separately. Cached runs are reused only when
input, decoder, shared audio implementation and all model hashes match. Preserve
old run folders before rerunning changed implementations. Reports refuse an
incomplete selected matrix or mixed decoder versions. `--primary` selects piano
routes, `--synth` selects the initial synth routes, and `--game-range` auditions
the Pro Keys arrangement with its decision receipt. These flags do not turn
real recordings into annotated references.

`prepare` creates synthetic controls from the supplied soundfont. `add-song` appends two
16-second passages from a 32-second excerpt with its source-bound stem manifest.
`add-controls` appends the long-held and repeated-key controls. Commands preserve
existing corpus/report folders. Corpus files record absolute input paths;
update those bindings explicitly if you move the recordings.

`audition --notes candidate.json --soundfont FONT --duration 16 --output NEWDIR`
renders an instrument-aware candidate's `notes` array with the same piano patch
and velocity 96. Optional `--programs 29,80` records an explicit source-program
selection; drums are omitted. This renderer assigns no musical correctness
score. MuScriptor/MT3 supplemental receipts remain separate from the completed
three-detector matrix, with raw program labels and token traces retained.

Use recordings and reference material you have permission to process.
Keep generated reports and audio outside the source repository.
