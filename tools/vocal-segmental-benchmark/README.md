# Continuous vocal note segmentation comparison

Offline Rust research tool. Fits continuous RMVPE pitch before assigning MIDI
notes, then compares the actual exported notes with independent references.
It does not change production generation or edit a project.

## Run

Use the repository's existing Cargo target cache. No model downloads or Python
runtime are needed; the inputs are previously extracted, source-bound evidence.

```sh
cargo test --locked --manifest-path tools/vocal-segmental-benchmark/Cargo.toml
cargo run --locked --release --manifest-path tools/vocal-segmental-benchmark/Cargo.toml -- \
  /private/manifest.json /private/recipes.json /private/new-results
```

The output directory must not exist. Keep recordings, references, manifests and
results outside public Git. Every recording ID must be unique. Manifest paths
refer to existing local files, not audio to download:

```json
{
  "recordings": [{
    "id": "recording-01",
    "group": "development",
    "generation": "/private/generation.json",
    "jdc": "/private/source-bound-jdc.json",
    "references": ["/private/reference-A1.json", "/private/reference-A2.json"]
  }]
}
```

Recipes are an array. This is an evaluated research configuration, **not an
approved production setting**:

```json
[{
  "confidence": 0.8,
  "singing": 0.95,
  "penalty": 0.08,
  "articulation_db": 3.0,
  "baseline_mask": false
}]
```

`baseline_mask` additionally requires the original generation's accepted pitch
mask. Otherwise, both experimental decoders use the same RMVPE confidence, RMS
and JDC class-mass gates. `singing` uses one minus the non-singing class
probability, or the sum of the legacy `note_band_mass` values; it does not use
the separate JDC detection head. The RMS floor is 0.0015, matching the compared
development experiment. No new activity classifier is trained here.

`penalty` is measured in squared-semitone seconds, so changing frame density
does not change the intended objective. `articulation_db: null` disables the
separately testable envelope-valley heuristic.

## Method and checks

- Preserve continuous observations and accepted source-time frame cells.
- Within each uninterrupted voiced run, minimize duration-weighted squared
  residuals plus a boundary penalty using PELT pruning. The fitted center is
  continuous; MIDI rounding happens after segmentation.
- Merge adjacent segments that round to the same MIDI pitch. Optional envelope
  rearticulation can split a repeated pitch without adding a silence gap.
- Retain all accepted time cells, including brief notes. Do not bridge rests.
- Bound input size and cost evaluations; fail rather than hang on a pathological
  quadratic case. Constant-pitch pruning is covered by a work-count test.
- Save and hash every prediction before reading references. The receipt includes
  input JSON hashes, implementation and executable identity, and output hashes.
- Use the existing `vocal_benchmark::score_exported_notes` scorer, separately for
  each annotation. Verify MIDI export/import preserves notes and ticks, and
  require frame-rounding and segmental output to retain identical voiced
  coverage on each reference clock.

The library tests compare pruned optimization with an exhaustive dynamic-program
oracle, check frame-density invariance, short semitone changes, rests,
sub-semitone modulation, repeated-pitch articulation, invalid input, and the work
budget. The envelope heuristic and squared-error model are research hypotheses;
the tests do not establish musical accuracy.

## November 13 development result

Thirty-five settings were evaluated over eight already-exposed solo excerpts
and eight already-exposed mixed excerpts. The following macro averages use the
recipe above. The matched frame-rounding control reproduced the corresponding
saved development metrics. Both decoders use the same singing gate and source
clock; this isolates the effect of segmentation.

| Measure | Matched frame rounding | Segmental candidate | Existing High baseline |
| --- | ---: | ---: | ---: |
| Solo exported-note pitch-frame accuracy | 82.44% | 64.11% | 61.99% |
| Solo voiced recall | 87.87% | 87.87% | 73.84% |
| Solo wrong pitch among detected voiced frames | 6.21% | 27.17% | 16.57% |
| Solo false voicing | 3.25% | 3.25% | 5.55% |
| Solo event-note F1, annotation A1 | 17.21% | 52.87% | 37.67% |
| Solo event-note F1, annotation A2 | 10.45% | 44.14% | 33.81% |
| Mixed exported-note pitch-frame accuracy | 46.58% | 49.50% | 29.59% |
| Mixed voiced recall | 56.82% | 56.82% | 31.99% |
| Mixed wrong pitch among detected voiced frames | 17.52% | 12.55% | 6.36% |
| Mixed false voicing | 8.60% | 8.60% | 7.18% |
| Mixed event-note F1 | 10.61% | 16.84% | 7.26% |

**Not promoted.** Better note-event matching does not cancel the solo pitch-frame
regression, and mixed wrong-pitch proportion remains worse than the established
High baseline. Fixed MIDI notes also cannot reproduce all performed vibrato and
slides, so event targets and continuous-F0 targets must stay separate. These
development results demonstrate a tradeoff, not a passed held-out gate, listening
acceptance, or improved production detector. Later experiments must freeze a
candidate before using untouched singers; do not reuse these recordings as fresh
validation.

The initial JDC 0.9 / RMVPE 0.03 comparison and original-High-mask ablations also
failed to establish a universal improvement. Private prediction receipts retain
their results. Do not select whichever annotation, recording group, metric, or
penalty looks best after scoring and call it a win.
