# Source-bound Expert/Pro benchmark

MIT standalone Rust CLI using `chart-engine` with `default-features=false`.
It consumes standard extracted files, never links the optional LGPL archive
decoder, and downloads no recordings or charts. FFmpeg is required. Private
recordings, lyric text, generated drafts and reports stay outside Git.

Build from the repository root, reusing its target directory:

```sh
CARGO_TARGET_DIR="$PWD/target" cargo build --release --offline --manifest-path tools/chart-benchmark/Cargo.toml
target/release/chart-benchmark prepare CASE NEW_ORACLE_DIR
target/release/chart-benchmark run FROZEN_ENGINE CASE ORACLE NEW_RUN MODEL_DIR oracle
target/release/chart-benchmark compare CASE NEW_RUN/generated.mid NEW_REPORT.json
target/release/chart-benchmark batch FROZEN_ENGINE PREPARED_BATCH NEW_RESULTS MODEL_DIR oracle
target/release/chart-benchmark evaluate PREPARED_BATCH NEW_RESULTS NEW_EVALUATION
```

`CASE` contains `reference-source.json`, `source.mix.json`, `source.wav`,
`song.ini`, one receipt-listed `notes.mid` or `notes.chart`, and original role
files. The original archive/source hashes, reference-note hash, asset sizes,
original role hashes, sample clock and delay witnesses are checked. No title
match substitutes for recording identity. Receipts are trusted provenance
inputs, not signatures proving that the archive author was honest.

`prepare` verifies decoded canonical source length, then decodes direct authored
`bass`, `guitar`, `drums` and `vocals` assets at 44,100 Hz stereo float WAV. Only
shorter tails are zero-padded to the mix frame count. It never trims, shifts,
normalizes, applies the mix's global gain, or guesses split-stem/lead ownership.
Unavailable roles remain explicit in `preparation.json`. The typed manifest
is validated through absolute staging paths and published with relocatable
relative paths; output publication refuses existing paths atomically on
macOS/Linux. Original assets remain untouched.

The only reference-chart material supplied to generation is plain lyric TEXT.
No note times, pitches, tempo map or phrase times enter the inference runner.
Text joins are extracted using the existing benchmark text-only parser.
If no supported lyric text exists, preparation records that failure and does
not fabricate a lyric-source file. Missing alignment in model-free mode must
remain an abstention.

`run` conditions are `oracle`, `full-mix`, and `model-free-vocals`. Oracle uses
the verified authored stems. Full-mix invokes actual separation first and
feeds its source-bound manifest into the generator. Both use fixed 120 BPM for
conversion rather than authored tempo. These are staged supported CLI calls,
not a one-click import/provider retrieval test. Model/separation caches may
be warm; measured stage duration is neither cold-start time nor human editing
time. Failed stages retain logs and receipts. `batch` applies this to a frozen
prepared batch and retains every failure.

`compare` emits all five instruments: Expert Guitar, Bass, Drums, Pro Drums,
and Lead Vocals, including failures for absent/unsupported parts. No easier
difficulties are assessed. Reference delay is the verified metadata/INI delay
plus an embedded `.chart` offset; generated delay is 0, with its embedded chart
offset respected. No favorable offset is fitted. Independent generation
provenance must bind the generated artifact to the canonical source; `evaluate`
checks batch/run artifact/source identities before aggregating successful
comparisons. Reference charts guide useful drafts rather than exact copying.

One-to-one attack timing uses 25/50/100 ms precision/recall. New-instrument
sections describe density/chord sizes; Guitar additionally measures written
sustain occupancy (union of holds at least 100 ms). Drums separates kicks from
hands. Pro Drums separately matches pad+tom/cymbal hits, recognizes native
four-lane MIDI tom spans, chart cymbal flags and Expert disco flip, and rejects
five-lane conversion rather than guessing it. ExpertPlus kicks are reported
but excluded. No fret-color score is treated as acoustic pitch truth. These
metrics do not establish instrument purity, fun, listening acceptance or
editing speed; unavailable/failed cases must not disappear from conclusions.

Synthetic checks require FFmpeg and no models or copyrighted music:

`recovery-batch AUDIT_BINARY PREPARED_BATCH NEW_OUTPUT MODEL_DIR` runs a separate
six-case oracle ablation using the compiled `rhythm-audit` example. Freeze the
candidate source, audit executable and batch selection before invoking it.
It hashes the executable, benchmark driver, two instrumental models and frozen
selection before inference; an adjacent `recovery-followup-plan.json`, when
present, must bind that same selection. Existing output directories are refused.

Each case is prepared through the verified four-role stem workflow, then one
native inference produces baseline and revised charts. Both use fixed 120 BPM,
`guitar_source=guitar` and blind title/artist values. No reference notes, authored
tempo, lyrics or phrase timing are passed to the audit. Only Expert Guitar,
Bass, Drums and native Pro Drums are compared; this condition performs no vocal
generation, full-mix separation or lower-difficulty evaluation. The baseline
disables guitar/drum recovery on identical native outputs, rather than reproducing
an older executable.

`summary.json` contains sanitized pooled and per-case 25/50/100 ms precision,
recall and F1, classified Pro hits, Guitar written-hold occupancy, raw recovery
counts and added/removed played heads/attacks. Control markers are excluded from
played-head counts. Bass raw fields and mapped Expert notes must remain identical.
Each side reports its scored/failing count and the paired-case count. All six
cases remain present even when preparation, inference, provenance or comparison
fails; failures make the command return nonzero after retaining its summary.
Full errors, arguments, source identities, timings, model outputs and comparison
diagnostics stay in private per-case directories. The sanitized summary contains
no paths, lyrics, audio, individual notes, song titles or artist names.

```sh
CARGO_TARGET_DIR="$PWD/target" cargo test --offline --manifest-path tools/chart-benchmark/Cargo.toml
```

`paired-instrument-batch BASELINE_AUDIT CANDIDATE_AUDIT PREPARED_BATCH NEW_OUTPUT MODEL_DIR`
compares two frozen executables on six once-prepared direct-stem cases. Both
executables accept the `rhythm-audit`/`instrument-audit` arguments. Each policy's
`revised-events.json`, `revised/notes.chart` and source-bound `comparison.json`
are required. An old executable's internal recovery-off baseline is deliberately
ignored. This command does not change `recovery-batch`'s ablation contract.

All case preparation completes before either executable runs. `plan.json`
records successful and failed preparation, archive-index/source/manifest/model
identities, both audit hashes and the benchmark binary/source witnesses;
`freeze.json` records its SHA. An adjacent `paired-instrument-followup-plan.json`
(or the older recovery declaration, but never both) must bind
`selection_sha256` to the frozen selection. The complete plan is written before
the first child starts. Inputs, plans, models and executable hashes are checked
again around each run. Per-side artifact hashes bind subsequent pairing.

Inference receives only canonical audio, the once-prepared stem manifest,
models, fixed 120 BPM, the Guitar role and blind title/artist values. Preparation
can extract reference lyric text as part of the shared case validation, but no
reference chart, lyric text, authored tempo or target notes enter either audit.
Only Expert Guitar/Bass/Drums and native Pro Drums are scored. Missing chart
sections with nonempty raw events fail that instrument; an explicit empty raw
array with no played notes is a scored abstention, retaining reference counts.
Omitted raw arrays fail validation. No unavailable case is substituted.

Reports retain separate side scores/failures and 25/50/100 ms paired metrics,
Guitar holds, kick/hand attacks, classified Pro hit classes, recovery/evidence
counts, articulation proposals/rejections, raw Guitar release changes and mapped
head/hold/control-marker deltas. Bass raw `bass`/`bass_*` fields and mapped
Expert notes/timing must stay identical, apart from the two code IDs below.
Raw kick/hand time multisets and exact
confidence must remain unchanged; only generic Tom to High/Mid/LowTom and
Cymbal to Ride/Crash refinements are permitted. Mapped kick/hand attack-tick sets
must also remain unchanged, while hand lanes and Pro markers may change.
The paired comparison exempts exactly `/bass_decoder/implementation_sha256` and
`/bass_analysis/implementation_sha256` from musical equality: these broad code
IDs include whole files also changed by Guitar and Drum work. Both full IDs are
retained in `invariants.bass_comparison`, including whether they differ. All
other Bass data and hashes, warnings, model/source identities, candidates,
rejections, corrections and mapped notes remain exact. Stored transcriptions and
cache-validity rules are unchanged. The single-executable `recovery-batch`
contract still compares every Bass field, including those IDs.

Invariant-failed measurements remain visible but cannot enter eligible paired
aggregates. Failure yields a nonzero exit after retaining all six case receipts.

Synthetic paired tests compile two distinct Rust fake audits. They poison the
old internal baseline, assert preparation/plan publication before execution,
and exercise source substitution, one-sided failures, Bass/timing violations,
missing versus explicitly empty parts, declaration mismatch and output refusal.
They run no ML inference and use no reference recordings.
