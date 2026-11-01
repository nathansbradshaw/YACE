# Desktop and browser parity gate

Desktop and static Dioxus Web/WASM must support the same charting outcomes
before additional product features ship. Use one source revision and the same
full-length song project for acceptance. Sharing Rust modules establishes shared
rules; it does not establish that both applications expose working controls.

The sole approved exception is AI inference: browser generation, separation,
recognition and assistant backends stay disabled when they cannot run entirely
inside the browser. No optional application service is substituted. Reviewing
saved results and manual tools are still in scope.

## Acceptance ledger

| Workflow | Current evidence | Remaining gate |
| --- | --- | --- |
| Start from recording or existing chart | Browser recording, song folder/ZIP and project ZIP exercised | Desktop manual recording start; repeat on final revision |
| Edit every instrument/difficulty | Shared Rust parts, note, vocal and difficulty commands | Paired native/browser UI run, including new note types |
| Timing, sections and Star Power | Browser edits, exports and undo exercised | Paired final-revision export comparison |
| Playback, seek, loop, speed and waveform | Full browser song ended at 243.880 s; loop/speed exercised | Same-revision native comparison, stems and saved kit playback |
| Attach and replace aligned stems | Real full-length supplied manifest attached in browser with worker validation; shared retained-stem and game export routines | Final-revision portable roundtrip and keyboard/primary stem switching |
| Read-only part analysis | Shared panel and Rust worker exercised on the full song | Paired report comparison and saved kit playback |
| Clean audio manually | Supplied word/plan load and actual worker render/download passed; full 10,755,108-frame output compared to native DSP, maximum delta 0.00000054 | Final native-panel comparison and rendered-copy listening |
| Review and flags | Browser flags and decisions exercised | Same final-revision decisions, source bindings and roundtrip |
| Team proposals | Direct optional GitHub transport implemented; API protocol tests cover retries, checksums, wrong songs, permissions and idempotency; browser reached GitHub and rejected a source repo | Paired two-contributor UI conflict and merge. User excludes creating throwaway GitHub repositories or publishing test review commits; authenticated remote writes are not claimed as verified |
| Recovery, undo and backups | Browser IndexedDB restore and stale-tab rejection exercised | Final-revision project roundtrip in both clients, interrupted operation checks |
| Export playable song | Browser source audio preserved byte-for-byte; chart undo restored original hash | Final-revision native/browser semantic and source-byte comparison |
| Navigation and errors | Shared header, welcome screen and manual setup; browser chart-folder discovery/switching and saved-project chooser; OS pickers | Final-revision native/browser UI and narrow-window checks |
| Release | Public static origin and custom domain configured | Tested source matches native package and published static assets |

Do not mark a row passed from compilation alone. Record the revision, fixture,
actual application actions, downloaded/exported results, and any limitations.
Keep private recordings, transcripts and generated receipts outside Git.

## Platform boundaries

Native filesystem operations use the OS picker. Browser imports use the OS
picker and local browser storage; exports use downloads. A folder snapshot is
not an ongoing sync connection. Git subprocesses, SSH and arbitrary disk writes
are not browser APIs. Their replacement must preserve the proposal/review and
conflict outcomes and make the transport explicit.

The static entry point stays small. Application rules, audio processing and
browser API calls remain Rust. CPU-heavy work must allow editing and playback
to continue; an enabled button that freezes the editor does not close a gap.
