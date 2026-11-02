# Desktop and browser parity gate

Desktop and static Dioxus Web/WASM must support the same charting outcomes
before additional product features ship. Use one source revision and the same
full-length song project for acceptance. Sharing Rust modules establishes shared
rules; it does not establish that both applications expose working controls.

The user has expanded the parity gate to include local browser generators and
vocals/instrument separation. The earlier blanket inference exception no longer
closes this work. No application service is substituted. Browser generation,
recognition and supplied-lyric alignment have actual full-song verification;
see [the acceptance record and the disabled HQ model exception](BROWSER_GENERATORS.md).

## Acceptance ledger

| Workflow | Verification evidence | Result |
| --- | --- | --- |
| Start from recording or existing chart | Both OS-picker/manual starts exercised; native setup now reaches Ready after save; identical chart/INI settings and unchanged encoded source | Verified |
| Blank charts and recording attachment | Actual browser blank creation, note placement, silent timeline playback, short-recording rejection, attachment/Undo/Redo, export, backup and restored Undo; source WAV unchanged and post-Undo export has no playable root recording | Verified; fixed browser open/source-state handling after the chart-only rebase |
| Instrument/difficulty editing | Shared command tests plus actual guitar timing/Tap, Keys, Pro Keys, vocal editing, and native edit/Undo; paired full-song MIDI/master/vocal export matches | Verified supported workflows |
| Timing, sections and Star Power | Browser edits/Undo and native tempo, section and phrase Add/Undo exercised through their actual dialogs | Verified |
| Playback, seek, loop, speed and waveform | Full 243.880 s browser playthrough; native whole-song playback; browser loop/speed; all five saved kit components at 75%; native kit backup opened at saved speed/selection | Verified; acoustic synchronization is unmeasured |
| Supplied stems and part analysis | Real full-length supplied drums, worker validation, native cached kit analysis/audition and portable roundtrip; attachment Undo/Redo replaced actual browser audio channels | Verified supplied/cached workflows |
| Local generation and separation | Actual six-stem inference, full-song drum/guitar/bass and vocal generation, supplied-lyric alignment, standard piano and Synth; generated drafts played, edited/undone, exported and restored; full-song local kit inference compared with native | Verified exercised workflows; detector/ASR differences and disabled HQ exception are documented in BROWSER_GENERATORS.md |
| Manual clean audio | Both actual panels rendered the same bound plan; native output matched shared DSP exactly; 10,755,108 browser stereo frames, zero lag, maximum decoder delta 0.00000054 | Verified functional render; musical acceptance remains with the listener |
| Review and flags | Shared rules, actual browser queue/decisions and portable source-bound review history | Verified |
| Team proposals | Actual browser proposal → native import/independent approval/merge → browser conflict resolution → fresh native approval → browser merge; new parts preserved | Verified file exchange and protocol tests. Authenticated GitHub writes remain untested at the user's request |
| Recovery, undo and backups | IndexedDB restore/stale-tab guard; same-name library switching with separate histories; full project/kit roundtrips; imported first checkpoint and native/browser Undo/Redo | Verified |
| Playable export | Actual client exports had byte-identical MIDI, editable master, vocal JSON, INI and original Ogg; partial-stem game receipt avoids duplicate mix playback | Verified export equivalence; game/controller acceptance is separate |
| Navigation and errors | Shared controls and OS pickers; actual narrow browser layout; lazy folder discovery; explicit downloads; native dialogs expose their accessibility role | Verified exercised paths |
| Release | Public static origin/custom domain and canonical native identity configured | Final publication verification is recorded in DEPLOYMENT.md |

Do not mark a row passed from compilation alone. Record the revision, fixture,
actual application actions, downloaded/exported results, and any limitations.
Keep private recordings, transcripts and generated receipts outside Git.

During generator integration, inspect each fetched desktop change before
rebasing and rerun the affected browser workflow. The integration through
`4c24191` includes shared solo, drum-detail, rhythm/co-op, six-fret, five-lane
Keys MIDI and legacy-delay fixes. The delay enhancement changes were carried
into `generation_build.rs` when resolving the extraction conflict. Actual
browser import/export/reimport retained the expected 0.620-second note position
with conflicting legacy fields, cleared exported INI delay and preserved the
original chart/INI witnesses. Native library migration is an OS filesystem maintenance tool;
browser imports must still recover its resulting project layout, exact saved
audio and editing history. A successful rebase alone does not verify that path.
Timing-anchor support passed actual browser import, tempo editing, Undo,
export and reimport. Changing 120 to 150 BPM compensated the following segment
to 100 BPM while preserving anchored attacks at 1.250 and 2.250 seconds; both
original microsecond anchors remained in the exported chart. Ordinary game
MIDI also passed actual browser import, edit, Undo and export: unchanged export
preserved the complete original MIDI bytes, and edited export retained injected
opaque program/controller/SysEx/text events. Explicit chart/MIDI source choice
is integrated; its browser recovery identity also includes alternate-source
provenance so a pre-choice editing copy cannot discard a newly supplied source.

See [the dated acceptance evidence](BROWSER_ACCEPTANCE.md) for revisions, private
artifacts, test coverage and limits. This gate concerns the supported YACE
clients; it does not claim feature completeness against OCTAVE/Moonscraper or
musical correctness. No throwaway GitHub repository or remote review test event
was created. GitHub publication behavior is covered by transport tests and an
actual anonymous read/rejection, rather than an authenticated live write.

## Platform boundaries

Native filesystem operations use the OS picker. Browser imports use the OS
picker and local browser storage; exports use downloads. A folder snapshot is
not an ongoing sync connection. Git subprocesses, SSH and arbitrary disk writes
are not browser APIs. Their replacement must preserve the proposal/review and
conflict outcomes and make the transport explicit.

The static entry point stays small. Application rules, audio processing and
browser API calls remain Rust. CPU-heavy work must allow editing and playback
to continue; an enabled button that freezes the editor does not close a gap.
