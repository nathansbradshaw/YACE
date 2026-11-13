# Pitch-preserving vocal segmentation comparison

Offline Rust research tool. Replays source-bound RMVPE/JDC evidence and compares
the actual exported MIDI notes with independent references. It does not change
production generation or edit a project.

## Run

Reuse the existing Cargo target cache. Inputs are already-extracted evidence;
no models, audio downloads or Python runtime are needed.

```sh
cargo test --locked --manifest-path tools/vocal-segmental-benchmark/Cargo.toml
cargo run --locked --release --manifest-path tools/vocal-segmental-benchmark/Cargo.toml -- \
  /private/manifest.json /private/recipes.json /private/new-results
```

The output directory must not exist. Keep recordings, references, manifests and
results outside public Git. Recording IDs must be unique. Manifest paths point
to existing local files:

```json
{
  "recordings": [{
    "id": "recording-01",
    "group": "exposed-development",
    "generation": "/private/generation.json",
    "jdc": "/private/source-bound-jdc.json",
    "references": ["/private/reference-A1.json", "/private/reference-A2.json"]
  }]
}
```

Recipes are an array. This conservative configuration was evaluated, but is
**not an approved production default**:

```json
[{
  "confidence": 0.8,
  "singing": 0.95,
  "penalty": 0.08,
  "articulation_db": 3.0,
  "baseline_mask": false,
  "method": {
    "mode": "guarded",
    "maximum_residual": 0.55,
    "maximum_excursion_seconds": 0.04
  }
}]
```

## Decoder styles

| Method | Behavior |
| --- | --- |
| `{"mode":"pitch_exact"}` | Keep every independently rounded source-cell pitch. Group adjacent identical labels; optionally split repeated pitches at envelope valleys. No smoothing or pitch-label changes. |
| `{"mode":"guarded", ...}` | Use continuous PELT segmentation as a proposal. Change a source-cell label only within a short rounded-pitch run, and only when the proposed MIDI label stays within the configured residual of the original continuous observation. |
| `{"mode":"legacy"}` | Original unconstrained duration-weighted PELT least-squares fitting. Preserved for reproduction, including its observed over-smoothing failure. |

Omitting `method` retains `legacy` for old recipes. `maximum_residual` is in
semitones, from 0.5 to 1.0. `maximum_excursion_seconds` is from 0 to 0.2 seconds.
The conservative example permits only runs of at most 40 ms and residuals below
55 cents. A centered semitone change cannot be flattened by that setting,
including a 40 ms note. This is an explicit constraint, not a claim that all
brief ambiguous changes are mistakes. Wider bounds trade pitch fidelity for
more grouping.

All methods preserve the original observations, accepted time cells and rests.
`pitch_exact` additionally preserves every rounded MIDI label. Optional
`articulation_db` is a separately tested envelope-valley heuristic; `null`
disables it. A split of a repeated pitch does not add silence. The `penalty` is
in squared-semitone seconds and affects PELT proposals; `pitch_exact` does not
run the optimizer.

`baseline_mask` additionally requires the original generation's accepted pitch
mask. Otherwise all methods share the RMVPE confidence, RMS and JDC class-mass
gates. `singing` uses one minus the non-singing class probability, or the sum of
legacy `note_band_mass`; it does not use the separate JDC detection head. The
RMS floor is 0.0015. No activity classifier is trained here.

## Verification

- Bound input size and optimizer work; fail rather than hang on pathological
  input. Check pruned PELT against an exhaustive dynamic-program oracle.
- Audit every output against entire accepted cells and total accepted duration,
  not just sample midpoints. Reject deleted cells, filled rests and invalid
  notes. Save changed MIDI-cell count, changed duration and maximum residual as
  `pitch_fidelity` in each prediction.
- Exhaustively check 15,625 short pitch/rest patterns in `pitch_exact`. Test
  actual short semitone changes, ambiguous blips, sustained alternative pitches,
  doubled frame density, repeated-pitch articulation and invalid bounds.
- Write and hash every prediction before reading references. The receipt records
  input JSON hashes, implementation/executable identity and output hashes.
- Score each annotation independently with the existing
  `vocal_benchmark::score_exported_notes`; retain its original metrics. Check
  MIDI export/import preserves pitch, note count and ticks. Require the matched
  control and candidate to retain identical voiced coverage.

## November 13 protected-decoder replay

The follow-up evaluated 93 exact/guarded/penalty settings and 54 additional
center-protection variants on eight already-exposed solo excerpts and eight
already-exposed mixed excerpts. The additional variants did not establish a
winner and remain private research. These are development comparisons, not new
held-out validation. Both four-recording subgroups are already exposed too.

Below, exact uses no envelope splitting; guarded uses the example recipe;
legacy uses penalty 0.08 and articulation 3 dB. All three experimental styles
share confidence 0.8, class mass 0.95, RMS floor and source clock. The saved High
baseline has its own gates, so that column is an end-to-end comparison.

| Measure | Pitch exact / matched control | Guarded | Old unconstrained | Existing High |
| --- | ---: | ---: | ---: | ---: |
| Solo exported MIDI vs performed-F0 frame accuracy | 82.44% | 82.07% | 64.11% | 61.99% |
| Solo event F1, annotation A1 | 17.21% | 19.33% | 52.87% | 37.67% |
| Solo event F1, annotation A2 | 10.45% | 13.24% | 44.14% | 33.81% |
| Mixed exported MIDI vs reference-frame accuracy | 46.58% | 47.06% | 49.50% | 29.59% |
| Mixed event F1 | 10.61% | 11.57% | 16.84% | 7.26% |

Guarded improves event F1 for both solo annotations and mixed excerpts versus
the matched control, while limiting the solo frame-accuracy loss to 0.375
percentage points. It changes 192 of 8,674 accepted solo cells (1.916 seconds);
the largest residual is 0.549512 semitones. Exact preserves all labels but
envelope splitting alone did not improve both solo annotation sets. All styles
retain identical accepted voicing: solo recall 87.87% / false voicing 3.25%,
mixed recall 56.82% / false voicing 8.60% under the original reference-frame
definitions. The old unconstrained configuration reproduces its saved metrics.

**No overall win or production promotion.** Guarded still loses solo event F1
to High, and mixed false voicing remains worse (8.60% versus 7.18%). Increasing
smoothing strength did not remove the tradeoff. Earlier 35-setting experiments,
including gate and original-High-mask ablations, also failed promotion.

The other agent's exposed anchored-cell experiment (activity threshold 0.6,
voice-change cost 0, note-change cost 3.2, High-anchor weight 2) also beats this
conservative style on event F1: solo A1 35.50%, A2 27.57%, mixed 13.85%. It uses
a different activity model and gates, so this is an end-to-end snapshot, not an
isolated segmentation comparison. That experiment also reports failure of its
note-target acceptance gates. Neither result establishes a production winner.

Performed F0 and annotated musical notes are different reference objects. The
frame diagnostic above measures how exported fixed MIDI pitches follow the
performed contour; it is not annotated-note pitch accuracy. The event F1 uses
the annotated note events. Raw continuous observations remain unchanged in
every mode. Use separately labeled note-interval diagnostics as well when
assessing MIDI pitch/presence; never silently replace one reference basis with
the other or select whichever looks best. Fresh singers, mixed-song checks and
listening acceptance are still needed before choosing a production style.
