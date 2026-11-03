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
| Independent preview and piano-roll tracks | Native and browser Focused/Selected/All choices, stable order, collapse and reopen; actual hosted reload preserves separate choices without changing chart revision | Verified exercised paths; browser preferences stay in that profile/project |
| Timing, sections and Star Power | Browser edits/Undo and native tempo, section and phrase Add/Undo exercised through their actual dialogs | Verified |
| Playback, seek, loop, speed and waveform | Full 243.880 s browser playthrough; native whole-song playback; browser loop/speed; all five saved kit components at 75%; native kit backup opened at saved speed/selection | Verified; acoustic synchronization is unmeasured |
| Supplied stems and part analysis | Real full-length supplied drums, worker validation, native cached kit analysis/audition and portable roundtrip; attachment Undo/Redo replaced actual browser audio channels | Verified supplied/cached workflows |
| Local generation and separation | Actual six-stem inference, full-song drum/guitar/bass and vocal generation, supplied-lyric alignment, standard piano and Synth; generated drafts played, edited/undone, exported and restored; full-song local kit inference compared with native | Verified exercised workflows; detector/ASR differences and disabled HQ exception are documented in BROWSER_GENERATORS.md |
| Manual clean audio | Both actual panels rendered the same bound plan; native output matched shared DSP exactly; 10,755,108 browser stereo frames, zero lag, maximum decoder delta 0.00000054 | Verified functional render; musical acceptance remains with the listener |
| Review and flags | Shared rules, actual browser queue/decisions and portable source-bound review history | Verified |
| Team proposals | Actual browser proposal → native import/independent approval/merge → browser conflict resolution → fresh native approval → browser merge; new parts preserved | Verified file exchange and protocol tests. Authenticated GitHub writes remain untested at the user's request |
| Recovery, undo and backups | IndexedDB restore/stale-tab guard; independent browser-tab projects and standalone Dance recovery; same-name library switching with separate histories; full project/kit roundtrips; imported first checkpoint and native/browser Undo/Redo | Verified exercised paths; quota, eviction and hard browser termination remain unqualified |
| Playable export | Actual client exports had byte-identical MIDI, editable master, vocal JSON, INI and original Ogg; partial-stem game receipt avoids duplicate mix playback | Verified export equivalence; game/controller acceptance is separate |
| Navigation and errors | Shared controls and OS pickers; actual narrow browser layout; lazy folder discovery; explicit downloads; native dialogs expose their accessibility role | Verified exercised paths |
| Browser desktop workspace | Static and hosted thresholds, per-tab dismissal/reload, keyboard overflow and uninterrupted playback; actual Firefox zoom at 140%; native layout retained | Verified exercised paths; assistive technology and broader platforms remain open |
| Release | Public static origin/custom domain and canonical native identity configured | Final publication verification is recorded in DEPLOYMENT.md |

Do not mark a row passed from compilation alone. Record the revision, fixture,
actual application actions, downloaded/exported results, and any limitations.
Keep private recordings, transcripts and generated receipts outside Git.

## Current integration gate (2026-11-02)

Native practice playback now processes a bounded window on a Rust worker,
mixing aligned original-rate stems before pitch-preserving stretching. Playing
rate changes retain the decoded audio, waveform and output device; paused rate
changes prime at the held position. The device callback only consumes pooled
blocks. Position follows acknowledged original-source frames, including rate
transitions and continuous loops. The shared accepted range is 25–200 percent;
saved custom values such as 83 percent retain the existing project/action format.
Per-section reanalysis routes are mixed in original source time before native
stretching; the proposed-review guard continues to save only canonical transport.

Actual native checks exercised original recording, isolated stems and a restored
Kick audition; rate changes retained the source identity and play/pause state.
A complete 243.880-second recording played from beginning to end at 200 percent.
A 130–132-second loop wrapped continuously at 150 percent. Earlier observed runs
had zero post-readiness underflow frames. A later full-song run under concurrent
build and media load observed 663 underflow frames (about 15 ms at 44.1 kHz);
end-of-song priming resets that counter, so its final zero is not a zero-stall
claim. Controlled stress and listening acceptance remain open. Discontinuous
seek priming is counted separately. A sampled producer receipt observed a 200-to-50-percent change
within 183 ms of the saved command. This is not an audible response or hardware
latency measurement. Buffer ownership is fixed independently of song length;
FFT-plan memory and DAC latency remain unknown.

The static browser repeated paused/playing 25–200-percent changes with seven
full-song media channels, looped at 200 percent, and reached their exact
243.880-second end while preparing a complete song export. The downloaded
package passed every ZIP CRC check. Preparing that large package exceeded the
UI-control observation timeout, so this does not establish smooth packaging
responsiveness. Both actual clients now restart a completed recording with Play
alone; a shared command regression covers rate, source and chart preservation.
Shared controls now provide a whole-percent slider, exact percentage input and
100-percent reset. Transient previews preserve canonical history until commit;
invalid drafts restore the saved rate. Actual native, static and optional hosted
clients exercised 25, 50, 83, 100, 150 and 200 percent while paused and playing,
including cached kit audition. The final static build completed separate kit and
original-recording full-song runs at 200 percent with eight media channels, then
saved and recovered 83 percent with selected note timing and duration intact.

The optional Rust-hosted LiveView client now uses original-rate local media and
the shared Rust drift policy. Local Blob media avoids an observed seventh-stem
range-loading stall at loop boundaries; seek epochs reject delayed EOF receipts.
Actual seven-channel loops and full-song playback passed after those fixes.
Independent listening for clicks, vocal artifacts and transient quality remains
open; media clock agreement does not establish acoustic synchronization.

The ledger above is acceptance evidence for the exercised workflows, not a
claim that later desktop changes are automatically verified on the web. The
practice-speed implementation is `2e0cbb89`, merged into main by PR 76
at `cd03655e`. All four normal CI jobs passed on that exact feature head.
The `b5432874` release combined that playback work, generation/draft review,
the browser workspace notice from PR 82 and native project-navigation recovery
from PR 86. Release `59f32a7b`, merged through PR 89 at `b1edfa87`, also includes
whole-value Dance timing edits and the synchronized vocal comparison work.
Release `cc342c3f` also includes linked chart workspace recovery and the vocal
time-range mouse-control correction. All four normal merged-main checks passed;
actual production reloads restored both linked 4-key and Dance selections.
Later desktop additions still need their own browser acceptance.
GitHub Pages serves that engine on `yace.n8bradshaw.com`; its approved
certificate and HTTPS redirect were verified. The served WASM matches the
staged artifact. Actual hosted overview seeking, local vocal pitch analysis and
chart-only export/reimport retain the three-second duration, two vocal notes,
phrases and lyrics. Existing same-origin browser saves reopen after migration.
Earlier engine assets remain accessible for tabs opened before the host change.
The earlier `6753fe7a` release recovered the existing full-song browser project and
its matching export receipt, restarted from EOF at 200 percent, and completed
both loaded 243.880-second channels at their exact end. The source revision's
native and local static apps separately completed the same full-length song;
native-to-browser restoration retained a saved 83-percent custom speed.
Independent track visibility passed an additional full-song browser check: preview
All with roll Focused, separate Selected preview choices with roll All, and
collapse/reload persistence kept the existing chart revision and selected note's
240-tick sustain unchanged. A hosted mixed-part project repeated All/All and
collapse/reload at the same revision. View hotkeys left the note tool unchanged.

Native `b5432874` checks found and fixed two navigation failures: an unrelated
import retained the previous chart-folder switcher, and a corrupt single-project
folder replaced that context while leaving the old document loaded and the
status at Opening. Successful opens now match their actual source/editing copy;
failed or cancelled automatic opens restore the prior catalogue, selection and
status. Actual native compact-backup import retained the selected Bass note at
9153 ticks with its 240-tick sustain, original recording and eleven Dance/4-key
charts. The corrupt-folder check retained the valid project and Ready status
while showing its parse error. Eight library regressions, saved-history
integration, strict native/hosted lints and all four normal CI jobs passed.

The exact static `b5432874` artifact imported that compact backup in Firefox,
played the complete 243.880-second recording at 200 percent, switched Bass to
Dance during playback, and recovered the saved band project after reload.
Native integrated Dance also completed the song with its saved note edit.
Production HTTPS served the matching WASM bytes and independently completed
the same song, retaining the Bass edit and matching export-content receipt.
These muted checks establish continuity and persistence; listening acceptance
remains open. The compact fixture intentionally omitted generated stems, so it
does not replace the separate multi-stem acceptance. On that older artifact,
root reload returns to the
saved band track; reselecting Dance recovers its hidden-preview layout and saved
hold. Individual Dance note SVGs lack accessible note roles; that accessibility
gap remains open.

Dance beat and hold-length fields commit a complete entered value on change,
matching the band inspector, instead of saving every parseable keystroke.
Actual native and static Firefox checks typed a multi-digit fractional beat and
hold length: one Undo restored the complete previous value, and Redo restored
the intended value. Native typing left the saved revision and note unchanged
until blur, which added exactly one history action. Browser reload recovered
the hold at beat 0.875 with length 1.125 beats. These checks used the same mixed
full-song fixture and retained its eleven Dance/4-key charts.

Mixed projects also retain the selected linked Dance or 4-key chart through
native restart and browser reload. Returning to band editing clears that
choice. Actual checks exercised both linked profiles and the band return;
another native project retained its own Drums selection. Native project and
linked-chart files remained byte-identical through selection and restart,
preserving their existing Undo/Redo history. The preference is local to the
project workspace; it does not add a chart editing action. Browser view choices
remain in that browser profile.

Six focused regressions cover old preferences, separate view choices, atomic
history-preserving saves, project isolation, removed chart IDs and damaged
preference files. Removed or unknown linked charts fall back to band editing;
damaged preferences are retained rather than overwritten. Standalone selection
among multiple charts and assistive-technology acceptance remain unqualified.

Browser workspace recovery also keeps a separate project identity for each tab.
Previously, saving a different project in another tab changed what the first
tab reopened. The documents remained saved, but reload returned to the wrong
song. Rust now stores the current durable root and chart family in session
storage; a fresh tab uses the last opened durable workspace. Recovery chooses
one family before opening it, eliminating competing band and standalone Dance
restore operations. Background saves do not choose the tab's workspace.

Actual static checks opened a full-length mixed project and a separate saved
instrument fixture in two tabs. Both retained their own song, selected chart
and saved revision through reload. Opening a standalone Dance recording in the
second tab, reloading it and opening a fresh tab all restored Dance while the
first tab continued its independent band recording. Five focused regressions
cover per-tab priority, independent defaults, legacy virtual paths and rejected
targets. An invalid or unavailable bound target reports recovery failure rather
than silently substituting another song. Workspace preference failures have a
separate warning and do not change chart-save acknowledgments or stale-write
protection. Actual storage eviction/failure-injection acceptance remains open.

The browser keeps a desktop workspace at least 1024×600 CSS pixels. A smaller
window shows an inline notice with per-tab dismissal, without moving focus or
changing playback. Actual static and hosted checks exercised 1024×600,
1023×600, 1024×599, 1280×720 and 400×360; the narrow notice remained readable
and keyboard navigation scrolled to the Export control. Dismissal survived
same-tab reload and a fresh tab displayed its own notice. Real Firefox zoom at
140 percent also triggered it; zoom and the original tabs were restored after
verification. Resizing and dismissing the notice preserved playback and a
background full-song analysis. The native application retained its layout and
canonical identity. Native and exact static `6753fe7a` builds separately
completed the 243.880-second recording at 200 percent; static revision 118 kept
the selected note's 240-tick duration and matching export status. A native
140-receipt tail window observed zero underflows, which does not negate the
earlier stress underrun or establish acoustic acceptance.

For this integration, CI passed static WASM, dependency audit and performance;
the Mac job passed core tests, desktop build, 320 browser tests and HTTP
integration before reporting a test-fixture initialization lint. The final
test-only correction passed the same strict core all-target lint, the affected
vocal comparison regression, formatting and archive-converter tests locally.
Production artifacts remained unchanged. The supported skip-CI marker avoided
repeating those builds for the test-only correction; the normal merge did not
bypass any required branch check.

- Local RMVPE pitch comparison ran on the real three-second vocal excerpt.
  Deliberately moving MIDI 52 to 64 produced one supported 64-to-52 correction;
  Apply preserved both lyrics and note intervals, retained the previous pitch
  as an alternative, and left explicit draft uncertainty. Undo, Redo, committed
  IndexedDB recovery and the downloaded revision-8 project backup were checked.
- The full 243.880-second recording produced five Dance and five 4-key drafts
  in the actual browser. Independent native runs matched all note positions,
  lanes, kinds, lengths and estimated meters, ignoring newly allocated IDs.
  Dance note counts were 126/373/486/941/1023; 4-key counts were
  129/427/833/1464/1798. Actual placement/Undo/Redo and SSC downloads passed.
  Playback reached the encoded recording's exact 243.882902-second end while
  generation and exports were exercised. This is engineering evidence, not a
  dance-pad, keyboard-gameplay or musical-quality assessment.
- Browser generation fingerprints now include the RMVPE model preprocessing
  and contour-tracking implementation, preventing changed analysis code from
  silently resuming an older job.
- Combined band/Dance browser backups use the existing bounded archive worker
  and share immutable parent audio buffers while staging. Compression must not
  run on the UI thread or allocate one archive-sized WASM vector. The actual
  854 MiB combined project download passed every ZIP CRC check. Playback and
  grid navigation continued during compression; picker restore retained all
  11 Dance/4-key charts, the band tracks, and the saved Redo/Undo operation.

The shared Dance workspace now uses the ordinary preview, transport, inspector,
editing tools and history controls. A real mixed band/Dance project was opened
in two browser tabs: the first saved its note at beat 1, while the stale tab's
beat-2 edit was rejected. A guarded reload retained the rejected working copy;
the accepted tab still reopened at beat 1. The stale tab's independent backup
passed CRC checks and restored beat 2 with its original Undo/Redo history.
The test browser did not expose an explicit leave/stay dialog, so this records
observed retention rather than browser-specific confirmation presentation.

A second local vocal pitch comparison retained the earlier correction's amber
uncertainty, previous MIDI 64 alternative, corrected MIDI 52, lyrics and timing.
The HQ Piano/native-assistant AI exception remains explicit; those capabilities
are not implemented browser features. Browser process termination, storage
eviction and musical/game acceptance remain separate acceptance boundaries.

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

Chart-only exports archive vocal generation diagnostics with the omitted
recording's evidence. The authored vocal notes remain editable on reimport;
checks are not rebound to an absent recording. Actual browser export/reimport
of a dense full-song chart retained a Guitar Tap and a 240-tick Bass sustain.
Native regression coverage also verifies that an export including audio still
attaches its matching vocal checks.
Audio-free exports declare the known timeline length in `song.ini`. Actual
browser export/reimport retained both a 3.000-second excerpt and a 243.880-second
full song, instead of inferring a 60-second default or padding its final note.
The game metadata records length to the nearest millisecond.

Browser reload/close protection watches unsaved band and Dance/4-key projects,
including hidden linked parts, pending save metadata and in-flight writes. A
failed save keeps protection active even after automatic retries stop. The
listener is removed after persistence catches up and when the app unmounts.
The browser controls whether and how a leave-page confirmation is presented;
this does not protect against process termination, device failure or storage
eviction. Keep an independent downloaded project backup for recovery.

Native filesystem operations use the OS picker. Browser imports use the OS
picker and local browser storage; exports use downloads. A folder snapshot is
not an ongoing sync connection. Git subprocesses, SSH and arbitrary disk writes
are not browser APIs. Their replacement must preserve the proposal/review and
conflict outcomes and make the transport explicit.

The static entry point stays small. Application rules, audio processing and
browser API calls remain Rust. CPU-heavy work must allow editing and playback
to continue; an enabled button that freezes the editor does not close a gap.
