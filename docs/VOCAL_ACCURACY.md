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
High uses this fixed DSP + RMVPE profile; Basic Pitch/SwiftF0 assistance remains
in Standard and cannot alter High's measured voicing mask. The optional lyric
recognizer does not select High pitches or note boundaries.

**Check pitches against the recording** compares existing note intervals with a
dense RMVPE contour. A correction needs at least four supported frames, 40%
interval coverage, 80% contour agreement, and raw model support within 75 cents
on at least half the interior frames with salience at least 0.1. Partial notes at
selection edges, talkies and unsupported intervals remain unchanged. The saved
proposal binds source/stem hashes, exact chart content and tempo map. Applying
the batch is atomic and undoable. Agreement is not a calibrated probability;
corrected notes retain their draft flag and previous pitch as an inspector
alternative. The model can still follow a backing singer. Dissonance alone is not evidence
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

At the user's explicit request, pitch error now includes both missing singing
and incorrectly selected pitches: `(missed + wrong) / reference voiced frames`.
Promotion requires at least 20% relative reduction in this error rate
on held-out songs, an improvement in the proportion of detected pitches that
are wrong, voiced recall loss no larger than two percentage points, and
no pitch-accuracy loss larger than two points in any represented language.
False voicing may increase by no more than two percentage points on reference
unvoiced frames. These checks apply separately to detector contours and the
actual exported notes, sampled on the reference clock; a good raw contour cannot
hide incorrect or missing MIDI notes. Uncertain draft notes count in full.
Rates are macro-averaged across held-out recordings, so long songs do not dominate.
The corpus must cover solo singing, harmony and rap. Language changes can be
annotated separately; they are not a prerequisite for pitch evaluation. An explicit
`--diagnostic-partial` run can report a subset but can **never** pass promotion.
Lyric improvements cannot offset a failed pitch gate. Listening, actual native
and browser interaction, and game acceptance remain separate requirements.

Unaccepted model profiles remain research-only. The explicit desktop multi-model
option described below is distinct from Standard and the faster High profile.
RMVPE comparison
is optional and High accuracy remains experimental pending acceptance.
Language-specific recognition results are separate from pitch acceptance.

## Evaluation status

The earlier detector-only High draft failed the incorrect-pitch gate: additional
recovered notes also introduced wrong pitches. A subsequent independent RMVPE
contour with temporal tracking passed the numerical pitch-reduction and recall
thresholds on a fresh eight-artist set after development-only tuning. Failed test
sets remain disclosed as exposed diagnostics. Full promotion remains blocked by
the pending acoustic category audit, listening and game acceptance; High remains
experimental. Authored-chart targets are not measurements against performed
continuous F0. Detailed corpus manifests, measurements, model/singer errors and
limitations remain in private reports; code/test success does not establish
musical reliability. Section corrections were also evaluated on generated note
intervals without feeding authored pitches, timings or words to the checker.

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

## Source-clock and note-model diagnostics

The Rust examples `vocal_song_probe`, `vocal_contour_audit` and
`vocal_consensus_replay` separate detector evidence from note assembly. They
never modify the input song. A prior YACE-generated MIDI is a comparison draft,
not a trusted reference. Authored game MIDI can supply target-note references;
it is not a measured continuous performance contour. Keep target MIDI/timestamps
out of inference and score both selected frames and actual assembled notes.

`vocal_generation::assemble_notes` makes the same acoustic assembly available
without rerunning a model. It does not establish file/model bindings; callers
must validate those before supplying frames. Missing lyric text remains allowed.

The initial two-model recovery experiment increased correct coverage but also
increased wrong-note frames on authored game targets. It is research-only and
is not called by Standard or High accuracy. Agreement between monophonic
models does not establish lead-singer identity or prove that a sound belongs
in the chart. Shortening/deletion still require review.

[ROSVOT Rust evaluation](../tools/rosvot-convert/README.md) evaluates a learned
note-event model, without running Python or requiring recognized word text.
`vocal_note_replay` freezes the profile/executable identity, produces all
predictions before loading annotations, and scores both contours and assembled
notes. `vocal_pitch_calibrate` is development-only parameter exploration;
its results cannot establish a held-out promotion gate. Reused evaluation songs
must be reported as previously exposed. `vocal_note_consensus` corrects only
existing voiced frames and validates source/model/implementation/PCM identities.
The first correction profile missed the required validation improvement and
remains experimental. None of these research profiles changes app defaults.

`vocal_fcpe_probe STEM PINNED_ONNX NEW_JSON` evaluates a pinned community FCPE
export. Its frontend follows official FCPE revision
`6a149c1afb1c7e7821b71869dfb31ad50c95b516`; the export is from
`deiteris/weights` revision `a6317b2f21619f35087d870b37c7a3eeb6bb3b4d`, SHA-256
`0467e0a627c4dd7e774c2d3a05fb5ca1830af7491451ca0158e62cb2621e0908`.
[Upstream code/weight license](../examples/support/FCPE-MIT.txt) is MIT;
the export publisher also declares MIT. This is a research candidate, not an
endorsed official FCPE ONNX export. The probe changes graph output metadata to
read raw salience, retains all weights/nodes, uses the embedded cents table and
implements local-argmax decoding in Rust. It clamps only probability roundoff
within 1e-6; zero denominator means no pitch, while nonfinite or larger invalid
outputs fail. Confidence is raw salience, not a correctness probability.
Preprocessing and chunk conventions are recorded with each private result.

## Explicit desktop multi-model pitch option

The vocal editor offers **Multi-model pitch (slower)**. It compares RMVPE,
CREPE full, FCPE, SwiftF0 and ROSVOT/RWBD without lyric recognition or lookup.
It uses the frozen `vocal_posterior::FROZEN_ENERGY_PROFILE`: joint pitch-band
costs and corroborated note centres correct existing voice; supported gaps
require ROSVOT/Swift agreement plus two of three spectral votes, stable pitch
context for 180 ms, and local RMS above both 0.0015 and half the median existing
voiced local RMS. Existing quiet voice is retained. No key, scale, octave fold,
authored pitch or reference timestamp enters the decoder. A short dropout can
use surrounding supported singing for context instead of being rejected as an
isolated short sound. Silence and unsupported evidence remain gaps.

The profile was selected using the four development recordings. Its initial
recovery variant failed the false-voicing check; the energy-filtered variant
passed the private automated contour and exported-note gates. The second run
uses previously exposed validation artists, with parameters selected only on
development material. Private audio, references, exposure history and detailed
reports stay outside Git. This does not establish complete transcription,
listening, GUI interaction or game acceptance.

`vocals generate-project PROJECT --analysis-mode high-accuracy
--independent-pitch` selects the same desktop workflow. Install the matching
five Rust helpers and checksum-pinned local model pack explicitly. Missing tools
or models stop this requested profile; they never select Standard or faster
High. Native packaging adds `--vocal-tools RELEASE_TARGET --vocal-models
PINNED_LOCAL_MODEL_CACHE` alongside `--include-models`. No new weights are
downloaded implicitly by the multi-model pack.

Source/model/implementation-bound salience and note-event caches are shared
across repeat runs of the recording. Cancellation terminates the owned worker
process group and leaves the current chart unchanged. Each assembled note
retains alternatives and per-model support counts; these are not probabilities.
Regeneration and import use ordinary editor history and Undo. Unknown lyrics
remain typed missing syllables and do not suppress pitch.

The browser compiles the same Rust acoustic decoder and supports source-bound
analysis JSON import, including per-note model evidence and alternatives. The
five-model computation currently requires the desktop runtime and local pack;
the browser states this explicitly. Browser recognition/faster High do not
silently stand in for the multi-model option.

Evaluator-only `vocal-research reference --harmonies` includes explicitly
authored HARM2/HARM3 lanes using the same phrase/talkie parser as lead targets.
It does not infer singers from stem names or replace missing harmony annotations.
The PANNs activity experiment is separate, not part of this pitch recipe; see
`tools/vocal-activity-probe/NOTICE.md` for pinned weights and preprocessing.

### Short-contour regression and development tools

High accuracy now coalesces touching same-pitch attack fragments before applying
the minimum note duration. A late attack can therefore no longer discard the
accepted tail of a sustained pitch. Actual gaps and pitch changes remain
separate; repeated notes that both meet the duration minimum retain their
boundary. Explicitly requested notes below 80 ms retain their missing lyrics,
pitch and a visible short-contour uncertainty reason. This does not lower the
default duration or manufacture pitch in silence.

`vocal_accuracy_batch` can run the production multi-model pipeline, reassemble
existing accepted frames, or seed verified raw RMVPE caches. Its source/model/
implementation/PCM checks precede reuse. Reassembly does not copy stale per-note
model counts to changed boundaries; a complete cached-model replay recomputes
those counts. Generated MIDI is read back and compared with the saved chart.

`vocal_accuracy_grid` is restricted to development recordings. It runs the
posterior helper with explicit research-only recovery/note duration overrides,
finishes predictions before loading references, then scores raw and exported
notes separately. It also supports a research consensus extension: retain the
180 ms two-vote recovery and separately require three spectral votes for shorter
contours. Both passes calculate energy support from the original frames. All
overrides enter the saved recipe and algorithm identity, so the production
profile check cannot silently accept them. None of these experiments promotes
a backend or proves the added notes correct.

Further explicit research overrides compare transition cost, an energy reference
from nearby original accepted voice, and the winning pitch band's advantage
over its runner-up. The local-energy experiment requires at least ten nearby
accepted frames and otherwise retains the original song-wide floor. The
runner-up margin is a log-cost difference, not a probability. An acoustic-only
consensus trial requires SwiftF0 plus all three spectral estimators without
requiring a note-model event. These remain separate experimental recipes.

`vocal_posterior_replay --predict-short-consensus` freezes a development-selected
candidate: retain the original 180 ms two-vote recovery and add 100 ms contours
only with three spectral votes, ROSVOT/Swift agreement and runner-up margin 0.5.
Both passes use the original energy reference. It cannot be combined with
research overrides. `vocal_accuracy_batch --mode short-consensus --predictor
EXPLICIT_HELPER` checks the returned profile, executable identity and source
bindings and verifies MIDI round-trip. This candidate is not the production
profile. On eight validation artists, it reduced missing-plus-wrong frame error
by about 1.85% and exported-note frame error by about 2.20% relative to the prior
multi-model recipe. Voiced recall and wrong-pitch proportion improved, but the
required 20% reduction failed. These artists were previously exposed regression
material, not fresh held-out evidence. This candidate remains unpromoted.

Note assembly now follows each detector's actual frame clock in High accuracy.
Using the DSP hop for 10 ms, 16 ms or PESTO frames previously introduced artificial
gaps; short fragments could all disappear at the note-duration filter. Adjacent
cells now meet at their midpoint; absent frames and breaths still leave gaps.
Both modes fix acoustic boundaries before optional word attachment. Neither
these assembly fixes nor successful tests establish musical acceptance.

Keep every corpus, recording, reference, salience cache and detailed report in a
private directory outside Git. Use distinct development/validation artists;
previously evaluated artists are regression material, never fresh held-out
evidence. Language tags with unreliable provenance remain unknown. Authored
game charts supply chart-relative target notes, not measured performance F0.

### Residual section rescan experiment

`cargo run --release --no-default-features --example vocal_chunk_trial -- AUDIO
VOCAL_STEM GENERATION_JSON RMVPE_MODEL OUTPUT_JSON [--chunk-seconds 6]
[--reference PRIVATE_REFERENCE_JSON]` compares the exported chart with the raw
vocal contour, finds sustained coverage/pitch disagreements, and reruns only
marked chunks with one second of context on each side. Residual duration is
checked before chunk division, preserving short sounds across boundaries. All
frames retain the original recording clock; source, PCM, model, generation and
implementation identities are recorded. Existing audio and notes are unchanged.

This is experimental diagnostic tooling, not automatic repair or an independent
vote. The same model can consistently follow backing singers or instrumental
leakage. Optional reviewed references are loaded only after predictions and
score wrong, missing and falsely voiced raw frames separately. No generated
chart becomes ground truth. A smaller window is not promoted without the full
note/contour gate and voiced-recall checks. Keep outputs outside public Git.
