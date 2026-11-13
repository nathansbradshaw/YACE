# Acoustic note/rest training

Offline Rust experiment. **Not a production quality mode.** Lyrics do not enter
features, labels, inference, or MIDI construction. Private recordings, charts,
feature matrices, models, and reports must stay outside public Git.

The initial four-game-chart training run and seven previously unused solo
singers **failed the quality gate**: coverage improved, but wrong pitches and
gap false notes increased. The private run retains both annotations, all frozen
predictions, invalid-reference reasons, fitted models, and source hashes. This
tool adds a reproducible research path; it does not enable that model in YACE.

Train timing decisions from trusted game charts while retaining the original
RMVPE observations. The three binary heads predict pitched-note presence,
attack, and release. None predicts a frequency or pitch class. MIDI pitches
come from the median original F0 within each decoded note, rounded only for
MIDI. This preserves measurements, not a guarantee that segmentation or MIDI
is correct. Missing F0 remains a gap even when the classifier predicts singing.

## Run

Reuse a compatible Cargo target cache. No downloads or external ML runtime are
needed; the source reports must already contain pinned RMVPE observations.
Audio decoding uses the existing native FFmpeg path.

```sh
cargo test --locked --manifest-path tools/vocal-note-rest-trainer/Cargo.toml
cargo build --release --locked --manifest-path tools/vocal-note-rest-trainer/Cargo.toml
# Use the resulting yace-vocal-note-rest-trainer executable:
yace-vocal-note-rest-trainer extract /private/train.json /private/train-features
RAYON_NUM_THREADS=2 yace-vocal-note-rest-trainer train \
  /private/train.json /private/train-features /private/new-training-run
yace-vocal-note-rest-trainer extract /private/fresh.json /private/fresh-features
yace-vocal-note-rest-trainer predict /private/fresh.json /private/fresh-features \
  /private/new-training-run /private/new-predictions
# Only after every fresh prediction is saved:
yace-vocal-note-rest-trainer score /private/fresh.json /private/fresh-features \
  /private/new-predictions /private/new-report.json
```

Each manifest contains `recordings` with `id`, `artist`, `generation`, `stem`,
and `reference` paths. `reference` may be null during extraction and prediction.
Exactly four distinct artists are required for training; IDs must be unique and
contain only ASCII letters, digits, underscores, or hyphens. Fresh artist names
and recording hashes must not appear in the fitted model. Artist names alone
do not prove singer independence: audit aliases, earlier experiments, and source
provenance separately. Scoring accepts an otherwise identical manifest pointing
to a second independent annotation.

The stem must exactly match the generation's file hash and original 16 kHz PCM
hash. Duration, RMVPE model/implementation identity, and native 100 Hz clock are
validated. Feature blobs and source reports are hashed. Inference never opens
reference files. All prediction hashes must match the completed batch receipt
before scoring opens any references. Receipts detect edits; they do not certify
that a source was previously unseen.

## Fixed experiment

- Shared Rust RMVPE frontend: 128 log-mel bins, 80 ms differences, left/right
  band context at 80/320 ms, and relative F0/confidence/energy context. 336
  features; no absolute pitch, key, words, target timing, or target pitch input.
- Training-fold-only normalization; 64-unit tanh layer and six logits for three
  binary heads. Deterministic initialization seed 20261113, 64 full-batch Adam
  epochs, learning rate .003, betas .9/.999, decay .0001. Every second frame is
  sampled; each source has equal total weight. Rest weight 2, presence weight
  1; onset/offset positive weight 5, negative weight 1. Boundaries use ±15 ms.
- Four leave-one-artist-out development folds; final model trains on all four.
  Thirty-six predeclared decoding recipes: the original twelve combine presence
  cost 0/1/2, segmentation penalty .08/.16, and existing-note lock off/on. Six
  additional .16-penalty recipes use attack reward .32, allowing a confident
  same-pitch attack to justify a boundary. Stronger attacks remain experimental;
  over-segmentation still counts in evaluation. These eighteen recipes each have
  an additional `full_run: true` variant using pruned dynamic programming instead
  of the legacy 300-frame window. The selection objective and constraints
  are recorded in `frozen.json` before fresh scoring.

An existing frozen recipe without `attack_reward` or `full_run` keeps its
original .16 reward, legacy window, and serialized representation. Its predictions
are not silently reinterpreted. New frozen recipes explicitly pin either option.
No production analysis mode changes.
- Two-state voice/rest dynamic program and bounded piecewise-constant contour
  segmentation; learned attacks/releases affect transitions. No 100 ms minimum
  note filter. Valid F0 needs confidence ≥.03, RMS ≥.0015, and MIDI range
  [35.5,84.5). No octave folding or range clamping.
- Full-run segmentation does not force a boundary after three seconds. Its
  variable attack-cost pruning is compared with exhaustive optimization, including
  negative and zero boundary costs. Ties prefer fewer notes. It fails explicitly
  after 30 million segment-cost evaluations across the recording; no duration cap
  or fallback silently replaces the requested profile. Long sustains, repeated
  notes, and short legato changes with strong attacks have focused MIDI tests.
- Locked recipes retain every existing note's pitch, onset, offset, and lyric
  exactly and add only in its gaps. Consequently they also retain existing
  mistakes. They cannot independently solve wrong baseline notes.
- Verify real MIDI export/import pitch, count, and ticks. All proposed notes
  are explicitly uncertain research notes. Original raw RMVPE is checked for
  exact equality; `pitch_frames` remains the original baseline detector output,
  not the new classifier's voicing. Score actual MIDI separately.

## Acceptance

Report correct musical-pitch time, note-presence recall, wrong-pitch share,
false-note time in reference gaps, and event F1 separately. The report names
their order. Performed F0 and musical-note references remain separate. Invalid
or overlapping references are unavailable, not zero and not fitted to output.

The numerical diagnostic requires at least eight valid pairs, ≥20% relative
reduction in missing-or-wrong note time, recall loss ≤2 percentage points,
no worse wrong-pitch share or event F1, and gap false notes ≤2 points worse.
It does not certify per-language non-regression, retention, fresh-source
independence, full-mixture generalization, listening, browser/native or game
acceptance. `promotion_eligible` is always false. Do not deploy a recipe merely
because it wins the development grid or retains raw F0.
