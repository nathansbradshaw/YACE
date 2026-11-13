# Learned vocal boundary comparison

Offline Rust research, not a production detector. Predict musical note presence
and attacks separately from the original RMVPE contour. Preserve raw evidence;
compare actual exported MIDI with both musicians' note events.

## Run

Reuse the repository's existing Cargo target cache. No models or audio are
downloaded. All data and fitted models stay outside public Git.

```sh
cargo test --locked --manifest-path tools/vocal-boundary-benchmark/Cargo.toml
cargo run --locked --release --manifest-path tools/vocal-boundary-benchmark/Cargo.toml -- \
  /private/manifest.json /private/new-results solo-only
```

Use `all-sources` instead of `solo-only` for the second ablation. An optional
fourth argument reuses a previous run's hash-verified `models.json`; expected
training-source lists, exact feature schema and feature counts must match before
inference. Schema v2 uses neighboring pitch evidence as the origin for a missing
pitch cell; an earlier fixed placeholder leaked register into context features.
Models were refitted after this correction.

This tool reproduces the fixed exposed study: 16 recordings, the first eight
with `solo-` IDs and two note annotations, then eight mixed recordings. Manifest
format matches `vocal-segmental-benchmark`: each recording has unique `id`,
`generation`, `jdc`, and `references` paths. `group` may be retained. Inputs are
existing source-bound High generation reports, native JDC probabilities and
reviewed reference JSON. RMVPE observations must use their native 100 Hz clock.
The output directory must not exist.

## Method

- Extract 85 acoustic/context features: continuous-pitch shape relative to the
  current pitch, confidence, RMS and JDC singing class mass over 20–320 ms.
  Absolute pitch, artist identity, key, lyrics and reference timing are not
  prediction features.
- Train deterministic histogram logistic boosting: 64 trees, depth three,
  16 quantile bins, learning rate 0.1, L2 5, minimum 25 rows per leaf. Sample
  every second training frame and normalize each recording's total weight.
- Use four folds by recording index modulo four. In `solo-only`, each solo model
  trains on six other solos; mixed recordings use all eight solos. In
  `all-sources`, each model trains on 12 other recordings. Assert the evaluated
  source hash is absent from its model's training set. Training labels come
  only from annotation A1 on other sources. A2 is scored independently.
- Fit presence, onset and offset heads. The current decoder uses presence and
  onset; offset predictions are retained for later diagnostics.
- Decode notes with a bounded dynamic program using continuous pitch residuals
  and independent onset support. Estimate each note's pitch from its median
  contour. Preserve repeated-pitch attacks supported by the onset head.
- Evaluate gap handling separately: only a bounded hole with agreeing source
  pitches on both sides can be connected. These note-presence decisions can
  still fill real gaps incorrectly; false-note time remains part of evaluation.
- Save all predictions, models, source/reference hashes and receipts before the
  scoring pass. References are necessarily read for other folds' training and
  hash inventory; the held source never trains its own model. Verify real MIDI
  export/import preserves note pitch, count and ticks.

The grid contains 162 development settings: penalties 0.04/0.08/0.16, onset
weights 0/0.16/0.32, presence cutoffs 0.5/0.7/0.8/0.9/0.95/0.98 and gap bounds
0/40/80 ms. These cutoffs are research settings, not calibrated probabilities or
approved defaults. Source exclusion does not undo earlier development exposure
or parameter selection. The fixed study has distinct singers/artists; a new
corpus needs folds grouped by performer and source family.

## November 13 result

Same eight exposed solo and eight exposed mixed recordings as the previous
comparison. Values are macro averages by recording, separately per annotation.
The recipe below is the same for both learned styles: penalty 0.16,
onset weight 0.32, presence 0.7, maximum hole 80 ms (recipe index 149 in this
expanded grid; index 80 in the initial 81-setting grid).

| Event F1 | Solo A1 | Solo A2 | Mixed |
| --- | ---: | ---: | ---: |
| Existing High | 37.67% | 33.81% | 7.26% |
| Prior guarded smoother | 19.33% | 13.24% | 11.57% |
| Prior unconstrained PELT | 52.87% | 44.14% | 16.84% |
| Other agent's saved anchored hybrid | 35.50% | 27.57% | 13.85% |
| Learned, solo-trained with source exclusion | 59.48% | **53.01%** | 21.88% |
| Learned, solo+mixed with source exclusion | **60.64%** | 49.30% | **23.77%** |

The High and other-agent columns use their own gates/models; they are
end-to-end comparisons. All development hyperparameter selection remains
exposed. The High-anchor pitch/support ensemble did not improve the combined
result and is not part of this implementation.

A separate diagnostic of the frozen solo-trained predictions gives annotated
note-pitch time accuracy **83.43% / 82.42%**, versus High's 66.19% / 63.92%.
That diagnostic canonicalizes only nominal <=1 ns touching-boundary arithmetic
on reference copies, explicitly counts it, and leaves original reference files
and performed-F0 scoring unchanged. It is not the MIDI/performed-F0 frame score.
The public tool reports the original event/F0 metrics alongside the shared
note-interval scorer. Normalizations are counted per reference; genuine
overlapping annotations fail. The reference bases remain separately named.
Each row pairs `reference_frame_diagnostic` with `reference_kind`: performed
contour data and authored-chart data keep their own meanings. Mixed chart frames are not
presented as physical performed-F0 truth.

**Still not production-ready.** Solo-trained false-note time is 7.41% / 12.21%
on the solo annotations versus High's 3.30% / 4.43%, and 18.50% on mixed songs
versus High's 7.16%. Mixed-source training improves mixed event F1 but still
leaves 12.02% false-note time. These failures prevent promotion despite the
larger note-detection gain. No untouched voices, listening or gameplay acceptance
are claimed.

For context, the two human annotations score 65.25% / 66.30% event F1 against
each other under this same strict timing-and-pitch criterion. This measures
annotation disagreement, not an algorithm ceiling or proof of human-level
performance. An oracle with human boundaries and estimated median pitches scores
88.48% / 90.91%; that is failure decomposition, never detector performance.

Pitch tracking and note-event segmentation are distinct tasks. Relevant primary
work includes [Tony](https://webspace.eecs.qmul.ac.uk/s.e.dixon/pub/2015/MauchEtAl-TENOR2015-Tony.pdf),
[phoneme-informed note segmentation](https://webspace.eecs.qmul.ac.uk/s.e.dixon/pub/2021/LiDemirelProutskovaDixon-NLP4MusA2021.pdf),
and [vocadito's analysis of F0 versus notes](https://arxiv.org/abs/2110.05580).
This implementation is a small supervised boundary experiment, not a reproduction
of those systems.
