# Browser workflow acceptance — 2026-10-30

This records functional acceptance, not musical approval or parity with OCTAVE
or Moonscraper. Source music and generated artifacts stay out of the repository.

## Recording and workflow

The actual browser editor generated Guitar, Bass and Drums for Bayside's
**Head On A Plate**, 243.880 seconds, with all four difficulties. The saved
project contains 12 tracks and 13,601 note records. Expert counts are 1,779
Guitar, 807 Bass and 1,429 Drums. The generation request fingerprint was
`417de879f8cd712a8f1b2c46c77c26a71d46d8f12c5822223e58fd2d15b832e4`.

The running browser completed uninterrupted playback to 4:03.9. Further checks
used the 12–24 second review region: selected a note, changed its timing by one
tick, changed its lane, and undid both changes back to the source. Note copying
used the browser clipboard. Isolated drums played at 75% pitch-preserving speed
with the loop enabled; the cursor remained inside the selected region.

Direct native file selection loaded the full recording through **Choose audio
file**. No intermediate workspace chooser appeared. Required title/artist fields
opened automatically when metadata was missing. **Export** and **Save project
ZIP** produced normal browser downloads without a custom location dialog.

The downloaded song ZIP passed archive integrity checks and contained
`notes.chart`, `song.ini`, `song.ogg`, source evidence and the export receipt.
Its recording hash matched the source exactly:
`4116e98add1b324105924afb9788379813c437c9792dbc3328d6ff2c03b5053a`.
After undo, exported/source chart hashes also matched. The receipt deliberately
retains `musical_acceptance: unverified`.

The project ZIP included `project.json`, source media/chart, review evidence and
all referenced undo checkpoints. Archive integrity and the 12-track/13,601-note
project contents were verified. Project archiving takes the editor store lock
so a concurrent content save cannot split the backup across revisions.

## Bottleneck measurement

The former rendering snapshot cloned the complete project, including all tracks,
review evidence and audit history. The browser now shares an immutable
`Arc<Project>`; selection and transport also avoid an unused content checkpoint
clone. Content edits keep their recovery behavior.

An isolated 300-iteration profile on this song measured the former deep copy at
2,372 microseconds median / 2,532 microseconds p95. Sharing a snapshot measured
below the clock resolution at median / 0.042 microseconds p95. This measures
snapshot creation and release, not total editor speed or a competitor comparison.

A 120-frame playback sample recorded display/chart-position intervals of
8.3 ms median, 9.3 ms p95, 9.4 ms maximum and no gaps over 50 ms. It excludes
pauses, seeks, loops and revision changes. It does not establish acoustic latency.

## Verification and limits

The browser regression run passed 168 tests (12 ignored): shared library,
actual loopback HTTP host, note groups, editor, collaboration and game export.
Browser Clippy with warnings denied and native editor all-target compilation
also passed. Additional validation and generation/timeline regressions passed
earlier in the same workflow.

This delivery supports a local loopback browser, native file/folder selection,
normal downloads, client audio and the existing portable team review protocol.
It does not deliver hosted accounts, roles, live shared cursors or simultaneous
editing in one project. Browser uploads are snapshots; live Git/shared-folder
controls remain in the desktop app. Externally attached stems require separate
transfer when moving a project.

No generated section was approved just to make the workflow appear complete.
The recording still needs musical listening review, controller/game acceptance
and broader platform/browser coverage. Manual tempo/phrase authoring and game
renderer parity remain separate gaps.
