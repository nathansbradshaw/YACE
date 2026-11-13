# Lead-pitch support study

Offline Rust research. **No detector or quality mode is enabled in YACE.**
Keep recordings, authored charts, fitted weights, features and reports private.

The broader study trained on 24 artists, calibrated on eight other artists,
and reserved eight fresh artists. Each preparation artist supplied clean vocals
and synchronized instrumental bleed at 10% of vocal RMS. Source duration came
from audio, with no label-derived padding. Labels used PART VOCALS, not harmony
lanes. Authored game notes are not measured performed F0 or certified singer
identities; artist grouping also needs an alias and prior-exposure audit.

The result is **still insufficient**. A global pitch-support threshold could
not remove any calibration note without touching protected pitch. Conditional
rules lost correct pitch on the exposed mixed-song regression set. Requiring
both learned heads to agree removed seven additional predictions there while
retaining all 677 previously matched notes and all previously correct pitch
time. Combining that primary rule with earlier conservative experts removed
59 of 2,785 predictions. Macro event F1 was only 24.07%; note-presence recall
fell from 73.68% to 72.83%, and gap false time improved only 1.49% relatively.
The quality gate failed. The eight fresh artists remain unscored.

These mixed-song results are development diagnostics, not independent validation.
No threshold, rule or composition is automatically promoted.

## Run

Reuse a compatible Cargo cache. FFmpeg decodes existing audio; gzip stores
lossless feature caches. No model download or Python runtime is needed.

```sh
cargo test --locked --manifest-path tools/vocal-lead-support-trainer/Cargo.toml
cargo build --release --locked --manifest-path tools/vocal-lead-support-trainer/Cargo.toml
# Use the resulting executables:
yace-vocal-lead-support-trainer extract /private/calibration.json /private/cal-features
RAYON_NUM_THREADS=2 yace-vocal-lead-support-trainer train \
  /private/train.json @live /private/calibration.json /private/cal-features /private/new-frame-run
yace-vocal-lead-support-trainer extract /private/test.json /private/test-features
yace-vocal-lead-support-trainer predict \
  /private/test.json /private/test-features /private/new-frame-run /private/new-frame-predictions
yace-vocal-note-agreement fit-consensus \
  /private/calibration.json /private/new-frame-run /private/new-agreement-run
yace-vocal-note-agreement predict \
  /private/boundary-study.json /private/new-frame-predictions /private/new-frame-run \
  /private/new-agreement-run /private/frozen-baseline 149 /private/new-note-predictions
```

`@live` computes training features in memory. A previously extracted training
cache can replace it. Fitting requires 24 training artists and eight disjoint
calibration artists. Repeated artists are allowed for augmentation conditions;
each recording receives equal total training weight. Keep condition counts
balanced. IDs must contain only ASCII letters, digits, underscores or hyphens.

The frame manifests contain `recordings`, each with `id`, `artist`, `generation`,
`stem` and nullable `reference` paths. Optional `baseline_generation` provides
another note assembly on the exact same recording, clock and original RMVPE
observations. It supplies candidate notes for prediction or agreement calibration.
It must never contain target-derived inference notes. Use comparable assembled
notes for calibration: tiny raw pitch fragments generalized poorly in this study.

For replay of external proposals, `fit-consensus` accepts a final proposal-root
argument. Each `ID.json` has `source_sha256` and `proposals`, whose entries contain
`penalty` and `notes` (`start`, `end`, `midi`). This study uses penalty .16 only.
The root's `receipt.json` must declare `reference_values_read: false` and pin each
`id`/`proposal_sha256` in `rows`. The existing boundary benchmark manifest/baseline
receipt format is used for agreement prediction. Groups containing `mixed` are
processed; other groups retain the baseline unchanged as a neutral control.

## Fixed model and retention policy

- 354 features: the existing 336 shared RMVPE log-mel, delta and context values,
  plus absolute candidate pitch, mean spectral level and eight coarse harmonic
  center/side comparisons. These mel features are not a resolved harmonic or
  singer-identity measurement. Words and target timing are not acoustic features.
- Four binary heads: authored lead presence, attack, release, and whether the
  rounded original raw pitch matches an active authored lead note. The last
  label distinguishes an octave error during singing from a correct pitch.
- Training-only normalization, 64 tanh units, deterministic seed 20261113,
  48 full-batch Adam epochs, rate .003, betas .9/.999 and decay .0001. Sample
  every 40 ms. Presence/support weights are 1 positive and 2 negative; boundary
  weights are 5 positive and 1 negative, using a ±15 ms boundary window.
- Frame calibration searches fixed mean/peak thresholds separately for presence
  and pitch support. No qualifying cut means no deletion, never an unsafe fallback.
- Agreement calibration searches 31 quantile cuts for both heads together,
  with zero protected examples removed, at least 20 rejected negatives, and at
  least five negatives in each of three calibration artists. Both heads share
  a network; their agreement does not establish statistical independence.
- Agreement recipe 0 uses both note means and protects same-pitch overlap of at
  least .5 with the source High chart. Recipe 1 uses both maxima; recipe 2
  protects any source overlap; recipe 3 removes that overlap guard. These are
  diagnostics. `fit`/`fit-proposals` expose the earlier rejected conditional-rule
  experiment; they are not the primary agreement policy.
- Neither tool predicts replacement frequencies, smooths F0, moves a retained
  note, folds octaves or creates new notes. Missing/unsupported raw observations
  remain conservative. Frame outputs preserve retained note metadata and scoring
  phrases; a deletion that invalidates word continuations leaves the chart intact.
- Validate file/PCM hashes, native 100 Hz clock, RMVPE identity, source/artist
  exclusion, frozen model and frame-observation receipts. Prediction never opens
  reference files. Hashes prove binding, not annotation quality or independence.

Score exported MIDI, strict reference-note identities, previously correct pitch
time, presence, wrong pitches and false gaps separately. Keeping raw F0 is not
a musical non-regression guarantee. Listening and game acceptance remain open.
