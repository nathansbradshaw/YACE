# Browser workflow acceptance — 2026-10-30

This records functional acceptance, not musical approval or parity with OCTAVE
or Moonscraper. Source music and generated artifacts stay out of the repository.

## Static Dioxus Web/WASM acceptance

The static app in `web/` imported the complete song and a saved project through
the operating system's browser file picker. The same 243.880-second recording,
12 tracks and Expert note counts below were retained. The deployed assets are
HTML, generated Dioxus/wasm-bindgen bindings, WASM and license notices. No host
editor, HTTP file upload, WebSocket, generation endpoint or account is required.

The actual static browser app completed two uninterrupted runs from zero to
`currentTime == duration == 243.88`, with `ended == true`, 100% speed and the
original recording selected. Both the old LiveView process and the static HTTP
file server were stopped during playback. The browser continued playing,
editing, undoing, saving project ZIPs, reopening those ZIPs and exporting songs.

After the integrated theme/Star Power build's full-song run, a blue cymbal at
tick 232140 was moved to 232141. Its companion cymbal control moved with it.
The downloaded project JSON and game chart both contained the changed tick.
The project ZIP reopened at revision 14 with its checkpoint and selected note;
Undo restored the original chart exactly. The restored chart SHA-256 was
`7e2300633a8af340baed76b28d5db67cdcc27972f37740380fcf99f62b75a76d`.
The exported source recording hash remained
`4116e98add1b324105924afb9788379813c437c9792dbc3328d6ff2c03b5053a`.
Project and song ZIP integrity checks passed. Reloading the served site restored
the project from IndexedDB and displayed Saved locally after its transaction.

The shared collaboration dialog exported and reimported a review bundle using
normal browser downloads and native file selection. A Star Power phrase was
added, exported as `0 = S 2 1920`, and undone. A 125 BPM marker was edited to
126 BPM and undone. In the vocal lane a note was placed, assigned syllable
`Test` and MIDI pitch 60, saved to JSON and undone. Invalid multiword syllable
text was rejected by the shared validator. These test edits do not approve any
musical content.

Browser testing caught and fixed absolute-path handling on wasm32, incomplete
audio preparation, a restore/autosave revision race, missing dialog mounted
events, and seek navigation that left the paused timeline behind. The browser
uses typed immutable project snapshots and serializes pending JSON for durable
transactions and downloads instead of reparsing the full project on every
command. Missing external stems visibly retain original-recording audition.

Verification on the integrated main-based source: 551 core/all-target tests
passed (38 optional tests ignored), 132 LiveView/shared-UI tests passed
(12 ignored), two Rust codec tests passed, strict core and WASM Clippy passed,
native editor all-target compilation passed, and static release assets built.
The codec tests check stereo polarity, sample count and source duration through
the browser playback WAV conversion, plus malformed-input rejection.

An eight-second Opus sample imported and played to its end through the local
browser codec fallback. Malformed recording data and a malformed ZIP both
reported errors while leaving that valid project and its audio available.

Automatic chart generation is deliberately disabled in this static build:
the current native model runners are not portable to this WASM target. There is
no optional generation service. Hosted teams, broader browser coverage and
musical/controller acceptance remain outside these functional checks.

## Earlier LiveView harness recording and workflow

The actual browser editor generated Guitar, Bass and Drums for Bayside's
**Head On A Plate**, 243.880 seconds, with all four difficulties. The saved
project contains 12 tracks and 13,601 note records. Expert counts are 1,779
Guitar, 807 Bass and 1,429 Drums. The generation request fingerprint was
`417de879f8cd712a8f1b2c46c77c26a71d46d8f12c5822223e58fd2d15b832e4`.

The running browser completed uninterrupted playback to 4:03.9. Further checks
used the 12–24 second review region: selected a note, changed its timing by one
tick, changed its lane, and undid both changes back to the source. Shift-selected
two notes, copied/pasted through the browser clipboard at the playhead, and
undid the complete paste in one action. Isolated drums played at 75% pitch-preserving speed
with the loop enabled; the cursor remained inside the selected region.

Direct native file selection loaded the full recording through **Choose audio
file**. No intermediate workspace chooser appeared. Required title/artist fields
opened automatically when metadata was missing. **Export** and **Save project
ZIP** produced normal browser downloads without a custom location dialog.

The downloaded song ZIP passed archive integrity checks and contained
`notes.chart`, `song.ini`, `song.ogg`, source evidence and the export receipt.
Its recording hash matched the source exactly:
`4116e98add1b324105924afb9788379813c437c9792dbc3328d6ff2c03b5053a`.
The ZIP reopened through **Import song ZIP** as a new editing project with the
same three Expert note counts. After undo, exported/source chart hashes also matched. The receipt deliberately
retains `musical_acceptance: unverified`.

The project ZIP included `project.json`, source media/chart, review evidence and
all referenced undo checkpoints. Archive integrity and the 12-track/13,601-note
project contents were verified. **Open saved project** reopened that ZIP at
revision 15 with its undo history, review flags, loop and 75% speed preserved.
Project archiving takes the editor store lock
so a concurrent content save cannot split the backup across revisions.

The collaboration dialog exported `reviews.json` as a normal download, then
imported it through native file selection without errors. Downloads have an
explicit attachment response, including text/JSON files that a WebView might
otherwise display instead of downloading.

## Bottleneck measurement

The former rendering snapshot cloned the complete project, including all tracks,
review evidence and audit history. The browser now shares an immutable
`Arc<Project>`; selection and transport also avoid an unused content checkpoint
clone. Content edits keep their recovery behavior.

An isolated 300-iteration profile on this song measured the former deep copy at
2,670 microseconds median / 2,955 microseconds p95 in the final integrated build.
Sharing a snapshot measured
below the clock resolution at median / 0.042 microseconds p95. This measures
snapshot creation and release, not total editor speed or a competitor comparison.

A 120-frame playback sample recorded display/chart-position intervals of
8.3 ms median, 9.3 ms p95, 9.4 ms maximum and no gaps over 50 ms. It excludes
pauses, seeks, loops and revision changes. It does not establish acoustic latency.

## Verification and limits

The final integrated browser regression run passed 195 tests (12 ignored): shared
library, actual loopback HTTP host, note groups, editor, validation, collaboration,
game export, generation jobs, song sections and timeline.
Browser Clippy with warnings denied and native editor all-target compilation
also passed. The full browser feature set (separation and vocal alignment) compiled
with all targets enabled.

This delivery supports a local loopback browser, native file/folder selection,
normal downloads, client audio and the existing portable team review protocol.
It does not deliver hosted accounts, roles, live shared cursors or simultaneous
editing in one project. Browser uploads are snapshots; live Git/shared-folder
controls remain in the desktop app. Externally attached stems require separate
transfer when moving a project.

No generated section was approved just to make the workflow appear complete.
The recording still needs musical listening review, controller/game acceptance
and broader platform/browser coverage. Vocal phrase authoring and game
renderer parity remain separate gaps.
