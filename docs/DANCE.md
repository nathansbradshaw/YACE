# Dance and 4-key authoring

Choose **Dance** or **4-key** in the normal New chart setup, or add either part
from **Add part** on an existing recording. The desktop and static browser use
one Rust simfile library, command protocol, generation algorithm and editor.
Existing Clone Hero/YARG projects retain their original schema and workflow.

New audio charts choose a **First part**, then use the normal song project.
Dance and 4-key sit beside the other instruments in the track picker. Switching
parts keeps the resizable Audition, Selection and note-editor panes in place.
The same toolbar owns Select/Place/Erase, undo/redo, clipboard, snap, zoom and
Follow. The same practice controls own playback, seeking, speed, volume and loops.
Parts attached to the same source recording use its existing audio player and
clock; imported standalone simfiles supply their own backend to those controls.
Dance timing remains independent of guitar timing, and its horizontal beat grid
does not collapse notes inside warps or disappear under zero/reverse scroll.

Open one song folder or a song ZIP with its recording. SSC is selected over an
SM companion in the same folder. Start from **New chart from audio** for a new song.
Choose Dance pad or 4-key keyboard, set the starting BPM and first beat in the
recording, then generate the selected Beginner–Challenge drafts. Generation adds
charts; it does not replace authored work. Meters are estimates requiring review.

Use Select to click notes or drag a selection box; Shift-click toggles notes.
Use Place (or double-click with Select) to add the selected note type. Use Erase
to remove a note. Drag notes to move them; drag a sustain's end grip to resize it.
Note properties in the Selection pane change beat, lane, length, type and foot.
The four lanes are left, down, up, right. Holds and rolls have editable lengths.
Mines, lifts and fake notes remain distinct types. The beat grid stays independent
of scrolling effects; the preview follows the recording's clock.

Copy/paste, mirror, quantize and passage moves keep sustains together and form
one undo step. The 1/2/3 keys select Select/Place/Erase for every instrument.
Arrow keys move the beat cursor and Page Up/Down move the grid.
Ctrl/Cmd-Z, Shift-Z, A, C, X and V provide history and selection
shortcuts. Inputs keep these keys local while editing text or numbers.

**Advanced timing** edits the song map or a complete selected-chart override:
BPM, stops, delays, warps, fake sections, speeds, scrolls, time signatures, labels,
tick counts and combo settings. A stop occurs after a note at its beat; a delay
occurs before that note. First-beat audio time is the negative of the StepMania
OFFSET field. Warps make audio time ambiguous across several beats; the timeline
keeps that ambiguity explicit instead of guessing a unique inverse.

Footwork review estimates left/right assignments, honors manual overrides, and
flags conflicting holds, multi-foot rows and abrupt transitions. These estimates
are advisory. Manual foot labels and review comments live in the project backup,
not in simfile note symbols. Chart edits invalidate matching review decisions.

## Files and recovery

**Save/download project backup** from the project menu preserves the version-2 StepMania document, stable chart and
note IDs, immutable simfile text, SHA-256-bound source assets, undo/redo, foot
labels, review notes and audit entries. Reopening verifies source hashes.

Desktop recovery lives under the OS temporary directory in
`yace-stepmania-recovery`; it survives application restart but is not a permanent
backup. Save project ZIPs to a durable location. Browser recovery uses IndexedDB
and a separate StepMania pointer, with revision checks against competing tabs.
A failed save is reported; export a project ZIP before reopening.
Combined-project backups include both families, including the current dance
working copy after a conflicting save. Browser saves check the band and dance
revisions in the same transaction so another tab cannot overwrite either family.

**Export → Dance simfiles (.ssc)** produces a playable song directory and a recovery subtree.
Pad and keyboard charts go into separate `YACE-pad` / `YACE-keyboard` directories.
Move these song directories under a pack in the game's `Songs` directory. Source
simfiles are retained under `yace-history/source`, rather than beside the new
simfile where the engine could select an old SSC instead of a new SM.

Legacy SM export rejects chart-specific timing, SSC-only timing, or chart metadata it
cannot represent. It never silently flattens these features. Unsupported game
styles and note extensions are retained read-only. Malformed holds, conflicting
notes, unsafe paths and incomplete asset bundles produce errors. Legacy negative
BPM files require normalization before import. Arbitrary Lua/mod execution,
background animation playback, scoring, doubles and Pump editing are excluded.

## Automation

The command line exposes the same authoring model:

```sh
chart-engine stepmania new --audio song.wav --bpm 120 --first-beat 0.2 \
  --title Song --artist Artist --profile pad --output /path/to/review
chart-engine stepmania generate /path/to/review --seed 1
chart-engine stepmania show /path/to/review
chart-engine stepmania apply /path/to/review --request edit.json
chart-engine stepmania footwork /path/to/review --chart generated-1
chart-engine stepmania export /path/to/review --format ssc --output /path/to/export
```

Requests contain `expected_revision`, `actor`, and a tagged `action`. Note rows
use StepMania's 48 rows per quarter beat, independently of guitar/MIDI resolution.
The shared Rust API is in `crates/yace-stepmania`; it has no filesystem, network,
UI or native model dependency. Browser generation executes in a worker; desktop
generation executes on a blocking worker with a cancellation flag. Results publish
only against the original revision. Cancellation keeps existing charts.

## Learned generation evaluation

No learned StepMania provider is enabled. The deterministic generator chooses
measured spectral attacks on the reviewed beat map, then arranges separate pad
and keyboard patterns. It includes density, recovery, chord and sustain limits;
these constraints do not establish musical quality or physical comfort.

| Candidate | Pinned evidence | Release decision |
| --- | --- | --- |
| [DDC](https://github.com/chrisdonahue/ddc/tree/812aa57f31d77a5fb91bc2eb5d1e7d8c03416235) | MIT source; checked-in placement/selection checkpoints of 1,296,281 and 1,197,691 bytes. Upstream demo requires TensorFlow 0.12.1 and Essentia; preprocessing uses spectral features and recurrent state. | Research reference. No Rust/native or WASM numerical parity, modern portable artifact, resource measurement or human chart acceptance was established. |
| [DDCL](https://github.com/miguelomalley/DDCL/tree/5b1375c642bb708b3c66baf5d880fbf865b85097) | TensorFlow >2, Essentia and beat-aligned multi-resolution features. Git LFS records onset weights of 135,826,677 bytes (`f56dd9abba6b627a069055b53d64e9c2ef0cbee17aa12ed7b760ac8f398f5b39`) and selection weights of 31,405,209 bytes (`d2f1d2320880810983b718a4707cf678e5a6e8c6cce77ea484768ad48a3b27f4`). No repository license was detected at this revision. | Not enabled: weight/source permission and a portable implementation remain unresolved. The 167,231,886-byte checkpoint pair also needs browser download, memory and execution measurements. |

No Python, TensorFlow runner, source songs or candidate model weights were
executed, installed, downloaded or bundled for this evaluation. A future provider
must pin permitted weights and feature extraction, run through Rust orchestration
on both clients, pass numerical comparison, and improve blind musical/pad and
keyboard assessments. Models remain optional; a text assistant is not an audio
chart-generation model.

## Attribution and validation boundaries

Timing's before/note/after cache design adapts [SMEditor](https://github.com/tillvit/smeditor/tree/bff17f0839109acd5dbcbe6f2febca7002f3f20c),
MIT, copyright 2022 tillvit; its notice is retained in
`licenses/SMEditor-MIT.txt`. The footwork interface follows SMEditor's documented
advisory predictions and manual corrections; YACE's conservative solver and
pattern arranger are implemented in Rust. No upstream rendering assets or noteskins
are bundled. The [provided chart-design video](https://www.youtube.com/watch?v=dR10fr28ads)
identifies positioning, turning, flow, musical impact and variety as review topics;
only its description/chapter list was available, not a verified transcript.

Boundary tests use source-derived expectations from
[StepMania `825467b`](https://github.com/stepmania/stepmania/blob/825467bcd81c812b33ad684dc04dd151b2d5dec3/src/TimingData.cpp),
especially `FindEvent`, `GetElapsedTimeInternal` and `IsWarpAtRow`. They cover
coincident delays/stops, offset signs, BPM changes within overlapping warps and
stop exceptions inside warps. Source-derived assertions are distinct from an
independent running-engine oracle. Code checks and synthetic generation fixtures
also do not establish native gameplay, synchronization on a particular audio
system, musical quality, difficulty accuracy or dance-pad acceptance. Those
require explicit exported-song playtests, separately for pad and keyboard.

Synthetic native checks exercise recording playback, the preview clock, note
placement, persistence and undo. Browser checks exercise normal New chart and
Add part setup, switching between band and dance tracks, reload recovery,
five-difficulty generation and rejection of a stale edit from another tab.
These checks are engineering evidence, not game or musical acceptance.

## Integrated chart families

Dance is a part family inside YACE's ordinary authoring flow. It uses the same
project bar, New chart setup (including recording metadata, cover art and tempo
analysis), Add part menu, track selector, Generate, Review, Save and Export
controls. Opening dance songs and backups uses the normal folder/ZIP controls.
Adding Dance or 4-key to a band recording retains the band's authored data and
stores its dance document alongside the band project. Project backups retain
both; the selected export target chooses band game files or dance simfiles.
The file adapter is currently SM/SSC. No future YARG DDR MIDI encoding or gameplay
behavior is assumed or advertised; that adapter can be added when its contract
exists, without encoding dance arrows as guitar notes.

## SMEditor references

| Authoring behavior | Reference and integration |
| --- | --- |
| New song, chart list, note kinds, mouse placement and snap | [Basic guide](https://tillvit.github.io/smeditor/guide/basic-guide.html); normal YACE setup and track picker, with a dedicated arrow representation. |
| Passage authoring | Stable selections, copy/paste, shifts, lane mirroring and quantization use revision-checked Rust commands and one undo step per operation. |
| Synchronization | [Detect Sync](https://tillvit.github.io/smeditor/guide/auto-sync.html); YACE reuses its existing measured tempo proposals in New chart setup, exposes first-beat alignment, and requires listening review. |
| Timing lanes and split timing | [Timing tracks](https://tillvit.github.io/smeditor/guide/timing-events.html); editable typed song/chart timing, explicit stop/delay boundaries and preserved source tags. |
| Tempo-dependent / constant scrolling | [XMod/CMod](https://tillvit.github.io/smeditor/guide/xmod-cmod.html); preview clocks are separate from the always-readable editing grid. Constant scrolling uses actual audio time even during a stop. |
| Footwork feedback | [Parity checking](https://tillvit.github.io/smeditor/guide/parity.html); advisory left/right assignments, transition warnings and explicit overrides. |

This is selective adaptation into Rust, not a claim of complete SMEditor parity.
Its script execution, noteskin system, live recording, complete gameplay judging,
spectrogram/tempogram interface and all bulk transformation variants are
not implemented. The implementation and license pins above remain the source of
truth for reuse; screenshots or successful parsing do not establish game feel.
