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

Both **Standard** and **High accuracy (experimental)** construct pitches and
note boundaries from acoustic evidence before attaching words. Transcription
and online lyric lookup are off by default; disabling them retains pitch
assistance. High additionally proposes stable, acoustically supported detector
alternatives, preserves acoustic gaps, and detects local attacks. Changing a
transcript cannot change either mode's acoustic pitches or boundaries. A word with ambiguous ownership
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

The profile was selected using four development recordings. Its initial recovery
variant failed the false-voicing check; an energy-filtered variant passed an
earlier private automated gate. That result did not generalize to the rebuilt
12-recording regression corpus: on its eight validation artists, the error
reduction relative to faster High was about 11.46% for contours and 12.06% for
exported-note frames, below the required 20%. Previously evaluated artists are
explicitly marked as exposed. The option remains experimental. Private audio,
references, exposure history and detailed reports stay outside Git. Tests do not
establish complete transcription, listening, GUI or game acceptance.

`vocals generate-project PROJECT --analysis-mode high-accuracy
--independent-pitch` selects the same desktop workflow. Install the matching
five Rust helpers and checksum-pinned local model pack explicitly. Missing tools
or models stop this requested profile; they never select Standard or faster
High. Native packaging adds `--vocal-tools RELEASE_TARGET --vocal-models
PINNED_LOCAL_MODEL_CACHE --vocal-runtime ONNXRUNTIME_PUBLISHER_DIRECTORY`
alongside `--include-models`. No new weights are
downloaded implicitly by the multi-model pack.

The private macOS arm64 pack includes ONNX Runtime 1.24.4 and its license notices.
The publisher directory contains `capi/libonnxruntime.1.24.4.dylib`, `LICENSE`
and `ThirdPartyNotices.txt`; all three source checksums are pinned in
`vocal_runtime.rs`. Packaging signs the copied library and records its packaged
checksum in the resource seal. Helpers receive that absolute path per child
process, so launching from the Dock requires no developer environment setting.
Missing or replaced packaged resources fail explicitly. Runtime identity also
separates model caches; known compatible results survive app updates, while old
results with unknown runtime provenance remain preserved in their prior folder.
Research replay tools can point `--cache` at the corresponding
`onnx-runtime-SOURCE_SHA256` directory. The bundled runtime supports macOS arm64;
no other platform runtime is silently substituted.

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

### Acoustic calibration experiment

A private Rust comparison evaluated logistic calibration, boosted trees and
ranking several acoustic pitch candidates. Only the four development artists
trained these models; cutoff selection used leave-one-artist-out predictions.
Words, keys, scales, song identity and absolute MIDI pitch were excluded from
the feature vectors. Several confidence-only variants failed development
checks and were not evaluated as promoted backends.

One frozen candidate added direct period correlation and SWIPE-style harmonic
template features from the original vocal stem and experimental lead residual.
The kernel experiment follows [SWIPE research](https://arxiv.org/abs/2507.11233)
and the MIT [libf0 reference](https://github.com/groupmm/libf0) at revision
`533f3bf3ce884a1ff594b53ccf08516c29c0a700`. Its centered FFT, mel sampling,
window interpolation and tuning search are explicit Rust variants, without a
claim of numerical equivalence to either publisher implementation. Template
strength is not singer identity or a correctness probability.

On eight previously exposed validation artists, this candidate reduced
missing-plus-wrong errors by 12.64% for contours and 13.47% for exported-note
frames relative to the existing multi-model option. Against faster High,
reassembled with the same current acoustic rules, the reductions were 22.65%
and 23.91%. Exported-note voiced recall rose from 26.34% to 45.70%, wrong-pitch
proportion fell from 17.54% to 10.15%, and false voicing rose from 6.32% to 7.92%.
Absolute exported-note pitch accuracy was 42.54%; these gains do not establish
complete or near-perfect transcription. All twelve generated MIDI charts
round-tripped with identical notes and phrases.

The frozen candidate passes the numerical comparison against faster High on
this regression material, but not the stricter 20% improvement over the prior
multi-model option. It is not integrated into the app or promoted. Its next
validation failed as described below; resource verification and musical
acceptance also remain separate requirements.
The recordings, trained coefficients, feature recipes, frozen executables,
exposure history and detailed reports remain private and outside Git.

### Fresh validation and original-source preservation

The same frozen candidate was subsequently tested on eight artists excluded
from the recorded exposure history before inference. Five recordings supplied
original professional vocal channels; three required pinned native HTDemucs6s
isolation from their full mixes. Every compared method received the same vocal
input and original recording clock. Predictions and MIDI round-trips completed
before authored target notes were read. No validation targets trained or
selected the candidate.

Against faster High, missing-plus-wrong frame error fell by 32.35% for contours
and 34.62% for exported notes. Exported-note pitch accuracy rose from 32.08% to
55.59%, and voiced recall rose from 34.55% to 62.00%. However, the wrong-pitch
proportion rose from 11.61% to 13.93%, and false voicing rose from 7.92% to
10.74%. These failures prevent promotion despite the coverage improvement.

An additional comparison with current Standard, including Basic Pitch and
SwiftF0 assistance and no transcription, passed the numerical checks: exported
note error fell by 29.60%, wrong-pitch proportion fell from 16.23% to 13.93%,
and false voicing rose from 9.26% to 10.74%. This secondary comparison does not
override the failed primary High comparison. Against the existing multi-model
option, the reduction in exported-note error was only 15.22%, and wrong-pitch
proportion worsened from 8.98% to 13.93%.

Error decomposition identified an unsafe decision in this experimental
decoder: retaining original voicing still allowed its pitch to be replaced by
the derived lead's accepted pitch. That replacement did not require stronger
support and caused substantial damage on one recording. An experimental
revision preserves each accepted original pitch, requires original-source
neural corroboration for recovered gaps, and blocks recovery in exact
zero-energy mono parent frames. Quiet nonzero parent energy is not rejected by
this silence guard. Its threshold and corroboration requirement were selected
using only leave-one-development-artist-out predictions.

The revised frozen executable passed controlled checks for silent-parent
artifacts, quiet recovery, conflicting child octaves and child-only votes.
Existing original pitches were retained on twenty regression recordings.
These checks verify source-preservation behavior, not pitch correctness.
Results on the newly exposed eight artists are regression evidence for the
revision; they cannot establish its held-out quality gate. Another eight
previously untested artists were selected and its settings frozen before
reading their target notes.

That second eight-artist evaluation has now completed. All eight required native
HTDemucs isolation from their full mixes. Against faster High, missing-plus-wrong
error fell by 27.22% for contours and 29.51% for exported notes. Exported-note
pitch accuracy rose from 29.59% to 50.37%, voiced recall from 31.99% to 53.58%,
and wrong-pitch proportion fell from 6.36% to 5.67%. False voicing rose from
7.18% to 10.26%, exceeding the two-point limit. The primary gate failed. The
existing five-model option already had 9.68% false voicing on these recordings;
merely tightening added-gap thresholds while preserving all its pitches cannot
remove that baseline error. The revision remains private and experimental.
Its exact-zero guard measures mono energy: counterphase channels or missing
energy mappings must be checked before treating it as evidence of stereo silence.

On the four development artists, a separate short-window experiment tried
512/1024-sample YIN/McLeod probes, at least three fundamental cycles and RMVPE
agreement. No tested rule met the full development gate. An audit of missing
High frames found McLeod absent on about 96% of them, while RMVPE at confidence
0.1 matched about 60% of those missing targets. This identifies a classical
periodicity bottleneck; removing its veto without another activity check also
admits leakage and does not establish a safe replacement.

### Speech activity evidence

`tools/vocal-speech-probe` adds a pinned, local Rust Silero VAD experiment
independent of words and pitch selection. See its README for model revision,
checksum, MIT terms, preprocessing and runtime requirements. Raw scores retain
source/PCM/model/implementation identities and the original 32 ms block clock.
The framing and validation are shared with the browser target; only the native
research CLI runs the graph. No quality mode or generation filter is enabled.

Four development recordings gave bit-identical scores to an independent local
protocol implementation. A development-selected veto protecting faster High's
acoustic voicing was then checked on the already-exposed second eight-artist
set. Exported-note pitch accuracy was 45.49%, voiced recall 48.27%, wrong-pitch
proportion 5.44%, and false voicing 9.45% against High's 7.18%. The false-voicing
limit still failed; this is regression evidence, not a new held-out test.
Speech activity can reject genuine singing and must not be treated as evidence
that an unsupported vocal pitch or a missing word is silence.

### Additional acoustic estimator research

[Recent voting research](https://arxiv.org/abs/2602.01727) examines temporal
and frequency alignment and correlated estimator errors. It motivates testing
different acoustic methods; it does not establish that extra votes or inferred
frequency offsets improve YACE, especially when different singers overlap.

A separate Rust evaluation used unmodified Apache-2.0
[REAPER](https://github.com/google/REAPER) source at revision
`1d6e9b95e6b08b500fccbc9a043989dbda747276`. The pinned source archive SHA-256 is
`152af842bedc98e9d12e2f59352ba1e2487ce2689bf2fbf65c68c031ab173d02`.
The native estimator uses glottal-epoch tracking without trained model weights.
The Rust wrapper preserves input amplitude, quantizes 16 kHz mono to signed
16-bit PCM, retains the publisher's default high-pass/Hilbert settings, and
uses a fixed 50–1100 Hz search and 10 ms output clock. There is no lyric input,
target fitting, key prior or octave correction.

On the four development artists, REAPER on derived lead audio reached 67.00%
exported-note pitch accuracy and 75.78% voiced recall, but its 11.35% wrong-pitch
proportion and 14.93% false voicing prevented replacement of the existing
methods. Adding eight REAPER features to the acoustic ranker changed its best
eligible development note accuracy by only 0.016 percentage point. The
original frozen candidate and new validation protocol were retained; REAPER
remains experimental. These are chart-relative development results, not
measured continuous-F0 accuracy or musical acceptance.

Keep every corpus, recording, reference, salience cache and detailed report in a
private directory outside Git. Use distinct development/validation artists;
previously evaluated artists are regression material, never fresh held-out
evidence. Language tags with unreliable provenance remain unknown. Authored
game charts supply chart-relative target notes, not measured performance F0.

### Joint singing detection and pitch research

`tools/vocal-jdc-probe` reconstructs the publisher's JDC checkpoint in Rust
without executing Keras, using pinned weights, normalization and native
runtime. Its shared frontend and evidence validation compile for native and
browser, and `analyze` produces an explicit lyric-free experimental MIDI draft
with source-bound JDC/RMVPE evidence. It changes no production quality mode.
The tool README records hashes, preprocessing, license attribution and limits.

The port matched its frozen Rust prototype exactly on four development songs:
the converted graph, all722 pitch probabilities, singing scores, PCM and
timestamps. The shared decoder reproduced selected pitches, note boundaries
and phrases on those four plus eight previously exposed regression songs.
This establishes port parity, **not publisher numerical parity** or musical
acceptance.

A separate frozen run then predicted eight new artists before reading their
authored game-chart targets. Both methods used the same native HTDemucs6s
vocal stems on the original recording clock. The primary baseline was the
actual installed app at `c96a5a23b4b0ee6a5f89521e7ddfc7613644b07a`, with
High accuracy and no recognition or shell runtime override.

| Mean across eight fresh artists | High accuracy | JDC/RMVPE research |
| --- | ---: | ---: |
| Contour correct pitch / reference voiced frames |44.03%|50.47%|
| Contour voiced recall |48.69%|54.92%|
| Contour wrong pitch / detected voiced frames |11.04%|9.95%|
| Contour false voicing / reference unvoiced frames |12.28%|8.32%|
| Exported-note correct pitch / reference voiced frames |38.54%|47.29%|
| Exported-note voiced recall |41.48%|49.56%|
| Exported-note wrong pitch / detected voiced frames |8.58%|6.22%|
| Exported-note false voicing / reference unvoiced frames |9.50%|6.96%|

The relative reduction in missing-or-wrong pitches was11.51% for contours
and14.23% for exported notes, **below the20% gate**. Recall and wrong/false
voicing improved, but the overall gate failed. These artists are now exposed
regression material; subsequent tuning cannot reuse them as fresh held-out
evidence. Language tags without reliable provenance remain unknown. These
are chart-relative targets, not measured performed-frequency truth or a
100% correctness claim. Keep all recordings and detailed reports outside Git.

### Performed-pitch reference handling and rescan regression

Published performed-pitch datasets can contain overlapping human note
segmentations at transitions. The evaluator preserves those boundaries and
accepts onset-sorted overlaps for `performed_contour` references. Its raw
frame truth remains the independently annotated F0 contour. Authored game
charts retain the existing nonoverlap requirement; invalid clocks, unsorted
notes and source mismatches remain errors. This does not change the scorer's
pitch tolerance, error definitions or quality gate.

A further four-artist development experiment recovered original acoustic
notes when supported by JDC singing activity and independent RMVPE, then
rescanned supported gaps between equal pitches. The frozen settings passed
development controls. A new validation selected the last two unexposed
positive-vocal-difficulty game artists and the first six distinct singers in
numeric track order from [vocadito v3](https://zenodo.org/records/5578807).
Average-pitch metadata was ignored. All eight predictions preceded annotation
reads. The publisher's MD5 matched; archive SHA256 was
`e0d6b99d3f9c594afe5ae5c4d7bdacebe569e53b809e90b89d1c771c4f9990e3`.
The dataset declares CC-BY4.0 and musician-reviewed F0 and two note annotations.

Both methods received identical original-clock vocals: native HTDemucs6s
for the game mixes, and native-format solo derivatives verified bit-identical
to the original solo recordings' stereo decode. The installed c96a5a2 High
accuracy app remained the baseline, with recognition disabled. No offset or
pitch fitting used target values. The two domains are reported separately:

| Rescan regression | Separated game mixes (2) | Clean solo singing (6) |
| --- | ---: | ---: |
| Contour missing-or-wrong error reduction |22.70%|**-6.04%**|
| Exported-note missing-or-wrong error reduction |24.60%|**-11.13%**|
| Contour voiced recall, baseline → candidate |47.67% →62.78%|84.62% →76.24%|
| Exported-note voiced recall, baseline → candidate |39.78% →56.58%|79.07% →69.21%|

Across all eight, missing-or-wrong errors fell only5.89% for contours and
3.25% for notes, while mean recall fell2.51 and3.19 percentage points.
The full gate failed, including the stricter no-recall-loss requirement.
Two game recordings alone cannot establish the required eight-artist gate.
Lower wrong-pitch proportions did not compensate for deleted valid voice.
No rescan backend is promoted. These results motivate preserving existing
acoustic voicing while screening additions, rather than using disagreement
as a reason to remove it. Further tuning must treat these singers and game
artists as exposed regression data, not fresh validation.

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

## Recursive audio/MIDI review (experimental)

In **Check pitches against the recording**, enable **Deep rescan gaps and disagreements**, then analyze a phrase, loop, time range or all vocals. This is a slower opt-in review tool; it does not promote a new generation backend or claim the pitch quality gate has passed.

The shared Rust planner compares the actual current vocal note intervals with isolated-vocal pitch evidence at 100 Hz. It checks uncovered pitches, pitch disagreements, MIDI without detected pitch, and vocal-stem sound without reliable pitch. Six-second cores retain one second of surrounding original audio, then unresolved cores split into overlapping halves. Stable forty-millisecond residuals are identified before partitioning so chunk boundaries cannot hide a short sound. Audio stays on the original recording clock; note decoding and boundaries use the full contour rather than independently cutting notes at chunk edges.

The loop stops on resolution, unchanged evidence below one second, a 250 ms subdivision floor, depth six, 1,024 inference calls, or a bounded decoded-audio budget. The report shows scan counts, initial and remaining disagreements, retained pitch alternatives and an explicit processing-limit message. Unresolved sound remains a finding. A smaller selection can be reviewed again. Coverage is a scheduling signal, not a correctness score.

Variable-size inference sessions disable ONNX memory-pattern retention and the CPU arena and recycle every eight scans. Browser sessions are explicitly released between the baseline and recursive comparison. Long recordings must be measured for peak memory as well as runtime; short synthetic checks alone cannot establish this bound.

Missing-note proposals still need stable pitch and acoustic support. Deep review additionally tests the actual source waveform at the proposed period and both octave alternatives, requiring normalized periodicity at least 0.85, a half-period margin at least 0.03, a double-period advantage no more than 0.1, and matching estimates in two distinct model contexts. Context identity uses the actual surrounding audio supplied to the network, including clipping at recording edges. Short cores that receive identical input do not add confirmation. Repeating RMVPE is **not** a second independent detector. Periodic instrumental leakage or a backing singer can satisfy these conditions; singer identity and musical correctness remain review gates. This experimental support rule has not passed held-out musical acceptance or the backend promotion gate.

Applied draft evidence survives an adaptive implementation update when the original audio, stem, pitch model, timing and exact note still match. Such notes remain visibly uncertain and request a fresh check; obsolete proposals still cannot be applied.

Existing notes, lyrics, talkies and authored intervals are retained when a detector becomes unvoiced. Lack of model pitch is not proof of silence. Exact silence checks use squared **stereo** channel energy in both the original recording and vocal stem, so opposite channel phases cannot masquerade as silence. Suspected extra or long notes are flagged for review. Proposed additions use unknown lyrics, remain uncertain, export as ordinary playable vocal MIDI, and apply through the existing selected-repair action with one Undo batch.

Native and browser clients use the same planner, physical support, decoder and evidence profile. Native uses its installed ONNX Runtime; browser requires its explicitly checked ONNX WASM runtime and pinned RMVPE resource. Unavailable resources fail the request rather than silently selecting a faster quality mode. Cancellation leaves the project unchanged. Source, chart, timing and implementation witnesses invalidate stale reports.

On four already-exposed development recordings using the pinned ONNX Runtime 1.24.4 research build for comparison, applying all initial review proposals raised exported-note pitch accuracy from 28.49% to 35.92% and voiced recall from 30.27% to 38.05%. Wrong pitches among detected voiced frames fell from 6.39% to 6.11%; false voicing rose from 3.76% to 4.71%. The reduction in missing-or-wrong pitch frames was approximately 10.40%, below the 20% gate. These figures predate the short-input context-identity correction; they do not establish the corrected recipe's accuracy. They are development results, not held-out promotion or native/browser listening/game acceptance. Private source recordings, references and detailed reports remain outside Git.

A complete 171-second project check with the existing native release runtime performed 618 scans in 131 seconds at 1.29 GB maximum resident memory. Its scan budget was reached and remaining sections stayed flagged. The separate research runtime used substantially more memory and remains experimental; its resource figures cannot be substituted for the installed runtime. Recycling sessions preserved the complete repair report on a separate full-song regression, apart from implementation/provenance hashes.
