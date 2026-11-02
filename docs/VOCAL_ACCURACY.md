# Vocal accuracy and local research

Pitch accuracy takes precedence over lyric completeness. Missing or failed
recognition must not erase supported pitches or turn singing into talkies.
All generated parts remain drafts.

## Generation and review

The shared native/browser generator uses source-bound YIN/McLeod frames,
optional Basic Pitch/SwiftF0 corroboration, local energy and periodicity.
Recovery no longer requires a recognized word. Optional lyric recognition
failure leaves a pitch draft and a visible warning; invalid source bindings,
audio failures and cancellation still stop generation.

**Standard** retains the established word-aware chart assembly. **High accuracy
(experimental)** additionally proposes stable, acoustically supported detector
alternatives, preserves acoustic gaps, detects local attacks, and fixes note
boundaries before attaching words. Changing a transcript cannot change that
mode's acoustic note pitches or boundaries. A word with ambiguous ownership
can remain missing; all recognized/supplied text remains in the report.

Choose the analysis mode in Auto-Chart or the native vocal editor's Options.
CLI generation accepts `--analysis-mode standard|high-accuracy`. Omitted fields
in saved jobs/policies select Standard. Resuming a cached proposal with a
different requested mode is refused. High accuracy now requires optional RMVPE
evidence; unavailable or unsupported resources fail explicitly, without falling
back to Standard. The faster Standard model selection remains unchanged.

The experimental comparison uses the pinned RVC RMVPE ONNX model listed below.
Its continuous F0 and salience are retained independently of the selected notes.
A shared Rust change-point decoder compares 49 chromatic centres (MIDI 36–84),
salience-weighted squared residual capped at four, and note-change cost 3.2.
Model context can bridge at most eight DSP hops (about 80 ms), after which the
original voicing mask is restored exactly. It cannot fill those gaps or discard
accepted voiced frames. No key, consonance, lyric or octave-folding prior is used.
This is an independent note-centre decoder, not a complete pYIN implementation.

**Check pitches against the recording** compares existing note intervals with a
dense RMVPE contour. A correction needs at least four supported frames, 40%
interval coverage, 80% contour agreement, and raw model support within 75 cents
on at least half the interior frames with salience at least 0.1. Partial notes at
selection edges, talkies and unsupported intervals remain unchanged. The saved
proposal binds source/stem hashes, exact chart content and tempo map. Applying
the batch is atomic and undoable. Agreement is not a calibrated probability;
the model can still follow a backing singer. Dissonance alone is not evidence
that a pitch is wrong.

Generation reports add optional `note_evidence`: note tick bounds, selected MIDI
pitch, raw alternative detector pitches, support and uncertainty reasons. Source,
stem, algorithm and optional model identities remain in the enclosing report.
Review maps retain this evidence. Amber notes and the note inspector expose
uncertainty; choosing an alternative changes the edit form until Apply note is
clicked. Existing Undo, approval and hide actions still apply. Evidence for
changed timing/pitch or a different source is not presented as current.
Flagged pitches remain in normal MIDI export; review flags are editor sidecars.
Silence and passages with no acoustic pitch evidence remain gaps.

## Research tools

These commands run locally and keep heavier models outside Standard defaults.
Use a private directory for audio, model weights, charts, lyrics and reports.
Model pins are enforced before inference; downloads occur only with the explicit
research recognition download flag. Source/stem durations alone do not prove
alignment: input pairs must already share the recording's zero point.

```sh
cargo build --locked --no-default-features --features vocal-alignment \
  --bin vocal-research --bin vocal-benchmark

# Paired DSP analysis, no lyric or reference timing inputs.
vocal-research pitch --audio /private/source.wav --vocal-stem /private/vocals.wav \
  --output /private/new-pitch-comparison

# Exact source-clock preparation; decoding precedes sample-index trimming.
vocal-research excerpt --audio /private/vocals.opus --start-frame 1323000 \
  --end-frame 1852200 --output /private/excerpt.wav
vocal-research pad-stem --audio /private/source.wav --stem /private/vocals.opus \
  --output /private/vocals-full.wav

# Compare with small, large-v3, or large-v3-turbo, independently of pitch.
vocal-research recognize --audio /private/source.wav \
  --vocal-stem /private/vocals-full.wav --language spanish \
  --profile large-v3-turbo --model-dir /private/models \
  --phrase-segments --output /private/new-recognition.json

# Experimental lead/backing split; input must already be isolated vocals.
vocal-research separate --vocal-stem /private/vocals.wav \
  --model /private/models/UVR_MDXNET_KARA_2.onnx --output /private/new-split

# Explicit Spanish speech-trained XLSR model; no English-alphabet substitution.
vocal-research spanish-alignment --audio /private/source.wav \
  --vocal-stem /private/vocals-full.wav --lyrics /private/lyrics.txt \
  --model-dir /private/models/spanish-xlsr --output /private/new-alignment.json
```

Activity segmentation uses 20 ms RMS cells, a source-relative energy gate,
200 ms context, silence boundaries and a 28-second maximum. Each segment retains
its exact source-frame interval, file hash and detected language. Timings are
rebased rather than spread evenly. This is experimental music activity
segmentation, not an established singing/voice-identity classifier.

The Spanish CTC command uses native Rust PyTorch-state-dict reading; it does not
execute Python. Model revision `96d7e9b4e4a78af515a3c6d3cee7c0826045d276`
pins config, vocabulary and weights. Unsupported characters/weak acoustic
evidence remain untimed. It is not yet enabled as the app's production aligner.

## Research candidates and evidence boundaries

| Candidate | Reason to evaluate | Current boundary |
| --- | --- | --- |
| [CREPE](https://arxiv.org/abs/1802.06182), full/tiny | Independent monophonic F0 evidence | Rust ONNX probe; raw salience is not lead-singer identity. |
| [RMVPE](https://arxiv.org/abs/2306.15412) | Designed to estimate vocal pitch despite accompaniment | Pinned RVC ONNX research adapter; 30-second chunks with one-second context are experimental for a recurrent network. |
| [FCPE](https://arxiv.org/abs/2509.15140) | Context-based monophonic pitch estimation with a lightweight architecture | Research comparison only; published datasets/hardware do not establish YACE performance or harmony separation. |
| [UVR karaoke](https://github.com/Anjok07/ultimatevocalremovergui) | Second-stage lead/backing proposals | Pinned network predicts the remainder; compensated remainder is backing, and lead is a residual. This does not isolate each harmony singer. |
| [Mel-Band RoFormer karaoke](https://huggingface.co/becruily/mel-band-roformer-karaoke) | Alternative separation architecture | Publisher currently supplies no model card; native implementation, checkpoint provenance and weight terms need assessment before integration. |
| [Whisper large-v3/turbo](https://github.com/openai/whisper) | Larger multilingual recognition candidates | Native research profiles; Small remains the app default. Speech benchmarks do not establish sung-English/Spanish accuracy. |
| [Spanish XLSR CTC](https://huggingface.co/jonatasgrosman/wav2vec2-large-xlsr-53-spanish) | Spanish-specific acoustic alignment and vocabulary | Apache-2.0 publisher declaration; speech-trained, singing acceptance pending. |

[Source-separation/Whisper research](https://arxiv.org/abs/2506.15514) motivates
comparing original mix, separated vocals and segmented recognition separately;
cleaner stems alone do not guarantee better long-form lyric transcription.
[Vocal-ensemble research](https://arxiv.org/abs/2009.04172) treats simultaneous F0
estimation and voice assignment as separate problems. Neither a monophonic F0
model nor a lead/backing residual establishes separate harmony parts.

The karaoke adapter checks the original `UVR_MDXNET_KARA_2.onnx` SHA-256
`bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4`.
It uses 44.1kHz stereo, periodic-Hann FFT5120/hop1024, 2048 bins, zeroed bins
0–2, `[L real,L imaginary,R real,R imaginary]`, 256-frame chunks and trimmed
context. Backing is multiplied by 1.065; lead is source minus backing. No
individual stem normalization or time shift is added. Weight training provenance
is not established by a mirror's MIT label; weights are not distributed here.

## Private benchmark and promotion gate

`tools/reference-benchmark` inventories installed YARG archives without changing
them. It can add a lyric-text language estimate and select six fresh English
artists with direct vocal channels using `select-vocal-pitch INVENTORY
EXPOSURE_HISTORY NEW_MANIFEST` (`select-vocal-english` restricts this to English).
Language detection is metadata screening, not
human annotation. `prepare-vocal-selection MANIFEST NEW_DIRECTORY` preserves
archive/asset hashes and creates synchronized full mixes. The optional archive
tool retains its separate LGPL-3.0 license.

`vocal-research pair-batch --selection MANIFEST --prepared CASE_DIRECTORY
--output NEW_DIRECTORY` analyzes every source before reading authored target
notes. Optional `--baseline-binary FROZEN_RUNNER` compares an earlier generator.
Child analysis uses a frozen executable copy. No professional MIDI timestamps,
lyrics or pitches enter audio generation. `reference` is an evaluator-only
command: it converts PART VOCALS plus the explicit INI delay to source time.
Authored targets are marked `authored_chart`, distinct from `performed_contour`.
Their sustained target pitches are not ground-truth vibrato or continuous F0.

`vocal-benchmark --corpus /private/corpus.json --output /private/evaluation.json`
requires exactly twelve distinct artists: four development and eight held-out
recordings. Pitch reliability does not require language balance. Each entry has
`id`, `artist`, `language` (for example `en`, `es`, or `und`),
`split` (`development`/`held_out`), `categories`, audio path
and SHA-256, and optional reference/baseline/candidate report paths. Paths resolve
relative to the corpus file. Missing annotations/runs remain pending. References
require provenance, reviewed status, source hash/clock, ordered F0/target frames,
ordered notes and optional timed words; generated charts cannot self-certify.

Metrics retain absolute pitch correctness within 50 cents, octave errors,
voiced recall, false voicing, backing capture and role switches where annotated,
one-to-one note matches (50 ms onset; max(50 ms,20% duration) offset), boundary
errors, word edit counts and matched-word timing errors. Missing singer/word
annotations are not zero-error evidence. Runtime/RSS have separate run receipts.

Promotion requires at least 20% relative reduction in incorrect-pitch frame rate
on held-out songs, voiced recall loss no larger than two percentage points, and
no pitch-accuracy loss larger than two points in any represented language.
Rates are macro-averaged across held-out recordings, so long songs do not dominate.
The corpus must cover solo singing, harmony and rap. Language changes can be
annotated separately; they are not a prerequisite for pitch evaluation. An explicit
`--diagnostic-partial` run can report a subset but can **never** pass promotion.
Lyric improvements cannot offset a failed pitch gate. Listening, actual native
and browser interaction, and game acceptance remain separate requirements.

CREPE, FCPE and other unaccepted backends remain research-only. RMVPE comparison
is optional and High accuracy remains experimental pending acceptance.
Language-specific recognition results are separate from pitch acceptance.

## Evaluation status

Local professional-reference comparisons failed the incorrect-pitch gate:
additional recovered notes also introduced additional wrong pitches. No backend
was promoted. Authored-chart targets are not measurements against performed
continuous F0. Detailed corpus manifests, measurements and limitations remain
in private reports. A full pitch-category audit, listening and game acceptance
are still pending; code/test success does not establish musical reliability.

On one development recording, raw CREPE tiny/full and RMVPE F0 probes also showed
different recall, wrong-pitch and octave-error tradeoffs. They are not chart-note
generation results or a held-out backend selection. Native English/Spanish
Small, large-v3 and turbo recognition, Spanish CTC emissions, and the karaoke
split ran locally; unreviewed transcripts and stems do not establish WER,
alignment accuracy, singer identity or improved lead pitches. Recordings,
reference targets, full transcripts, model weights and detailed reports stay
outside public Git history.

Further methods worth comparing include [pYIN's multiple-candidate temporal
tracking](https://ieeexplore.ieee.org/abstract/document/6853678),
[FCPE](https://github.com/CNChTu/FCPE) and
[PESTO](https://github.com/SonyCSLParis/pesto). Published results and confidence
values do not establish YACE accuracy. FCPE's current paper and older bundled
weights differ; PESTO also introduces a distinct frontend/runtime and license.
Neither is silently substituted for the pinned comparison. Development sweeps
are available through the Rust `vocal-pitch-lab`; its sweep/weighted commands use
development entries only. Fresh held-out artists must be reserved after tuning;
failed held-out sets become exposed diagnostics and cannot be reused as fresh
evidence. Keep all corpus/run files outside public history.
