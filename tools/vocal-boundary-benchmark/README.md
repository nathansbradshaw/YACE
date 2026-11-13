# Learned vocal boundary comparison

Offline Rust research, not a production detector. Predict musical note presence
and attacks separately from the original RMVPE contour. Preserve raw evidence;
compare actual exported MIDI with both musicians' note events.

**Mixed-song status: unresolved.** Several follow-ups tested learned boundaries,
independent neural cues, lead/backing evidence and conservative corrections.
None met the paired false-note reduction and retention requirement. The audit
below makes losses of previously correct notes and pitch time explicit.
These experiments do not change the app's production detector.

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

## Paired retention audit

```sh
cargo run --locked --manifest-path tools/vocal-boundary-benchmark/Cargo.toml \
  --bin compare -- /private/manifest.json \
  /private/baseline-run 149 /private/candidate-run 4 /private/new-audit.json
```

Compare frozen predictions from two runs on exactly the same 16 recordings and
24 references. Each run needs `prediction-receipt.json` and its selected
`ID-NNN.json` notes. The baseline must pin the source generation reports and
references. The candidate must pin those same inputs or the baseline receipt
from which it was derived. Edited predictions, duplicate pins, changed source
reports, changed references and different recording hashes are rejected.
Receipts detect changes; they do not certify an experiment's independence.

All notes are exported on the saved source tempo map and checked through actual
MIDI export/import before scoring. The report records every formerly matched
reference-note index lost, every newly matched index gained, and seconds of
previously correct pitch time no longer covered by the same pitch. A net gain
cannot hide a different real note disappearing. Keeping note count and MIDI
pitches constant also cannot hide damage from moving correct boundaries.
Auxiliary event matching must agree with the owning scorer's match count.

The exposed development guard requires at least 20% lower macro false-note time,
no loss in presence recall, pitch-time accuracy, event recall or event F1,
no increase in wrong pitch, zero lost matched events, and zero previously
correct pitch time removed (10 ns arithmetic tolerance). Every recording must
also avoid regression. These are intentionally strict research requirements,
not a statistical guarantee. `promotion_eligible` always remains false: passing
an exposed-data audit would still require independent validation and acceptance.

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

## Mixed-song follow-up

The fixed comparator is the all-source model above, recipe 149. We tested
additional pitch estimators and acoustic voice evidence, experimental independent
boundary cues, offset-only trimming, conservative whole-note vetoes, bounded
boundary regression, a boundary-confidence guard, and joint note/rest duration
decoding. All 487 settings use the already exposed recordings; they are neither
487 independent experiments nor fresh validation. No candidate met the
retention requirement. The independent boundary observations are experimental,
without verified upstream numerical parity or cleared weight redistribution;
they remain private evaluation inputs and are not bundled or promoted here.

One later regression trial also trains its upstream proposal models with nested
source exclusion. Within each outer fold, each training proposal's frame model
excludes both that proposal's fold and the outer test fold. This prevents the
boundary learner from training only on proposals from a frame model that already
saw those labels. Models, source pins and outputs remain private.

Same eight mixed recordings, macro averages; these are selected, explicitly
identified development recipes, not a fresh leaderboard:

| Mixed-song recipe | Note F1 | Pitch-time accuracy | Presence recall | False-note time | Old matched events lost |
| --- | ---: | ---: | ---: | ---: | ---: |
| Previous all-source baseline, 149 | 23.77% | 62.56% | 73.68% | 12.02% | 0 |
| Conservative boundary regression, 0 | 24.93% | 62.74% | 73.70% | 11.90% | 9 |
| Nested boundary regression, 4 | 25.74% | 62.79% | 73.56% | 11.77% | 19 |
| Boundary-confidence guard, 6 | 23.90% | 62.53% | 73.62% | 11.97% | 0 |
| Other agent's frozen boundary model, duration 0 / no phase decoder | 21.20% | 63.28% | 77.44% | 24.40% | 394 |

“Lost” is relative to the previous all-source baseline and the unchanged strict
event-matching criterion. The nested regression retained every output note and
its MIDI pitch, but moved 19 previously matched notes outside timing tolerance;
73 other reference notes became matched. It removed 5.553 seconds of previously
correct pitch coverage despite a small net increase in pitch-time accuracy.
The guarded recipe lost no matched events, but removed 0.373 seconds of correct
pitch coverage. Both fail. The other agent's exported charts were frozen,
source/clock checked, MIDI-round-tripped and rescored here; they use a different
voicing gate, so that row is an end-to-end comparison, not a decoder ablation.

False-note time is false pitched seconds divided by annotated gap seconds;
pitch-time accuracy and presence recall use annotated pitched seconds. Mixed
references are reviewed authored musical targets, not measured performed F0 or
a source-separated singer-identity truth set. A diagnostic using reference
boundaries found roughly 72% of excess note time attached to partly real notes,
and about two thirds within 50 ms of a reference boundary. That is failure
decomposition, not an available-at-inference oracle or proof that trimming is safe.

## Lead-only neural context follow-up

The requested target is the lead vocal line. Generic singing presence is useful
evidence, but does not establish whether a voice belongs to that line. Added two
optional research inputs to test this separately from the original pitch contour:

- `game_features`: 31 features from the official GAME small ONNX model's
  boundaries, presence confidence and relative pitch agreement. The encoder
  includes its own waveform front end. A protobuf output-only modification
  exposes the estimator's existing confidence tensor. Original presence and
  pitch outputs must match the unmodified model exactly on each recording's
  first chunk. This checks instrumentation, not end-to-end PyTorch parity.
- `role_features`: 32 features from pinned MDX karaoke lead/backing proposals:
  local energy, amplitude share and correlation at the original pitch's period,
  twice that period and half that period. Context covers 50/200 ms on each side.
  Both proposed stems are evidence; neither is treated as a singer-identity label.
  The original RMVPE contour remains the pitch source for the note decoder.

Either field must appear for all 16 sources if used. Feature schemas are V2/85
without extras, V5/116 with GAME, V6/117 with role evidence, and V6/148 with both.
Source clocks, model pins, feature hashes and parent generation hashes are
checked. Reused models must match feature schema, width and training sources.

Same fixed recipe 149, same eight exposed mixed songs, macro averages:

| Variant | Note F1 | Pitch time | Recall | False/gap time | Previously matched notes lost |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original V2 comparator | 23.77% | 62.56% | 73.68% | 12.02% | 0 |
| GAME K2 context | 27.92% | 65.03% | 75.94% | 11.77% | 71 |
| GAME K8 context | 27.10% | 63.90% | 75.52% | 11.44% | 78 |
| GAME K8 + lead/backing context | 26.11% | 63.31% | 74.59% | 10.74% | 127 |
| Other agent's saved High-gap recovery + PENN, recipe 0 | 8.87% | 64.81% | 82.76% | 25.29% | 318 |

The other agent's row is rescored from frozen exported charts on these same
eight sources; its newly exposed singers are excluded. It uses a different
gate, so this is an end-to-end comparison. No new model satisfies the retention
guard. For example, GAME K2 gains 167 matched notes but loses 71 old matches and
15.873 seconds of previously correct pitch coverage. Role + GAME gains 190 but
loses 127 and 30.771 seconds. Whole-note vetoes and bounded glide corrections
also failed; preserving retained note pitches alone did not preserve all real
coverage. No production detector change, untouched-corpus result or listening
acceptance is claimed.

### Generate private evidence in Rust

Both producers require a compatible native ONNX Runtime through `ORT_DYLIB_PATH`.
No Python runtime, model export, model download or app installation is performed.
Models and audio are supplied explicitly and remain private. GAME code/pipeline
attribution is in `licenses/GAME-MIT.txt`; the released weights have separate
[CC BY-NC-SA 4.0 terms](https://github.com/openvpi/GAME/releases/tag/v1.0.0).

```sh
cargo run --locked --release --manifest-path tools/vocal-boundary-benchmark/Cargo.toml \
  --bin vocal-game-observations -- MODEL_DIR STEM.wav EXPECTED_STEM_SHA NEW_CUE.json 8 0
cargo run --locked --release --manifest-path tools/vocal-boundary-benchmark/Cargo.toml \
  --bin vocal-lead-evidence -- SOURCE_INVENTORY.json KARAOKE_MODEL.onnx NEW_ROLE_DIR
```

For the lead producer, each inventory recording has `id`, `generation`, and
either `vocal_stem` or `separation_cache`. A cache contains `separation.json`,
`lead.wav` and `backing.wav`; their parent/model/file hashes must match. The
producer returns a manifest with `role_features`, plus its inference receipt.
No annotated reference files are opened by either evidence producer.

GAME context uses 20-second owned cores with one second of context per side,
universal language 0 and K2/K8 sampling. Its native encoder supplies only complete
10 ms frames; the sub-10 ms tail is explicitly reported. Custom chunking and
FFmpeg decoding are not claimed equivalent to the publisher's full inference
pipeline. Multi-step boundary removal is stochastic; the study freezes one
realization per recording. For training, add `game_features` paths to the
manifest and pin their files in sibling `inference-receipt.json`:
`{"steps":8,"rows":[{"id":"mixed-1","file":"mixed-1.json","stem_sha256":"...","sha256":"..."}]}`.
Use the main benchmark's existing source-excluded training and paired comparison
commands. Do not infer independent validation from an exposed-data guard.

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
