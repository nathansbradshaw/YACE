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
| Keys MIDI open notes and techniques | Actual hosted four-difficulty export/reopen with Pro Keys and vocals, independent MIDI inspection, edit/Undo/Redo and byte-identical restored export | Browser interchange verified; dedicated Keys authoring and native/game acceptance remain open |
| Drum accents and ghost notes | Actual native/static single and batch authoring, kick exclusion, cymbal retention, move, clipboard, delete, Undo/Redo, restart/reload and mixed-part MIDI export/reimport | Verified controlled fixture; in-game dynamics judgment remains unqualified |
| Double kicks and imported MIDI drum editing | Actual static placement, movement, clipboard, deletion, rejected duplicate, saved Undo/Redo, MIDI-only roundtrip and retained note-off intervals | Browser paths verified; current native controls and target-game acceptance remain open |
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

## Current integration gate (2026-11-03)

The shared drum inspector now authors Normal, Accent and Ghost dynamics for
selected pad hits and mixed selections. Kick hits stay unchanged. Piano-roll
labels and styling expose the chosen dynamics, while cymbal state remains
independent. Atomic edits retain imported MIDI attributes and unrelated flags.

Actual native and static checks created a ghost cymbal, moved it, copied and
pasted it, deleted the copy and restored it with Undo. Batch changes skipped
the kick and preserved cymbals through Normal/Accent/Ghost and Undo/Redo.
Both clients exported the same four-hit, three-part fixture: their MIDI,
editable chart and original recording were byte-identical. An ordinary MIDI
package without YACE history or a master chart reimported with both ghost
cymbals and the snare accent intact. The browser changed that imported ghost
to Normal, exercised Undo/Redo, restored Ghost and recovered it after reload.
Native restart retained the original four hits and selected snare accent.
Seven articulation regressions cover these operations and mixed-feature MIDI
roundtrip. These controlled checks do not establish musical quality or game
controller/velocity judgment. Prepared audio is checked separately; matching
original recordings do not imply byte-identical client-prepared PCM.

Double kicks now have a separate editable drum row. Moving an imported MIDI
drum hit preserves its note-off interval and attributes; those existing intervals
previously caused inspector edits to fail as unsupported sustains. New placements
use zero sustain, and edits cannot introduce a different positive sustain.
Successful placement and deletion
also update local selection, so Copy and the inspector follow the current hit.
Rejected edits retain the previous selection.

Actual static checks placed and moved a double kick with another hit selected,
copied and pasted it, deleted/restored it and recovered its Undo/Redo history
after reload. A seven-hit MIDI export without YACE sidecars reimported with three
double kicks, the ordinary kick, accent and cymbals intact. An imported double
kick then moved while retaining its one-tick note-off interval. The final static
build completed a 243.880-second recording with a temporary double-kick edit;
Undo restored the prior note count and matching export receipt. An earlier native
preview also verified imported one-tick edits, double-kick placement, real
clipboard paste, deletion and duplicate refusal without losing selection.
Restoring the original notes produced byte-identical MIDI, which reopened with
all seven hits and completed its ten-second recording. The latest integrated
native build still needs a full-song workflow and restart. Eleven articulation regressions
cover the core rules, including unrelated marker preservation.

Native vocal previews now carry the recording and revision metadata expected by
the display clock. A preview without that identity can still deliver clock ticks;
its timing samples are excluded from recording-specific measurements. Previously,
the first vocal batch in an all-parts view could stop the refresh task and leave
the preview on its fallback timer. The integrated native workflow must verify
that the display link remains active through vocal and combined previews.

MIDI represents double kicks on Expert only. Other difficulties require `.chart`
export. Imported MIDI songs continue to refuse `.chart` conversion when it could
discard original events; the inspector explains that boundary. Actual static
checks exercised both refusals and successful lower-difficulty `.chart` export
from a chart-based song. No unsupported conversion is silently substituted.

Hosted MIDI export was exercised with four Keys difficulties using distinct
note times alongside Pro Keys and vocals. Independent inspection of the actual
download verified each difficulty's open notes, HOPO markers and tap ranges,
plus shared Star Power, the Pro Keys sustain and the pitched lyric and phrase.
Reopening only the MIDI, INI and audio retained all six tracks. A Pro Keys edit,
Undo/Redo and mixed MIDI export preserved the four Keys parts; final Undo
restored byte-identical MIDI. Reload retained the saved Redo operation and its
matching export receipt. The controlled song also completed playback with all
instrument views visible. This qualifies browser interchange. Keys open-note
and technique authoring controls, fresh native acceptance and target-game
judgment remain open.

Export choices distinguish rhythm guitar, co-op guitar and each six-fret
instrument. Unknown source identifiers remain visible. These parts retain
their interchange support; dedicated rhythm/co-op and six-fret editing controls
remain open.

Actual browser checks used 34 chart choices across four difficulties, including
rhythm/co-op, five six-fret families, Keys, Pro Keys and vocals. Deselecting only
Expert six-fret bass removed exactly that difficulty's notes and techniques;
selecting all restored byte-identical MIDI. An ordinary MIDI-only reopen,
Pro Keys edit and Undo/Redo retained the other instrument parts and markers.
Final Undo again restored byte-identical MIDI. The current static build also
completed the original 243.880-second recording in a separate band project.
Fresh native controls, hosted release and target-game acceptance remain open;
retaining six-fret Keys does not establish target-game support for that part.

Hosted MIDI preservation checks also exercised instrument and Pro Keys solos,
drum activation/fill and roll markers, four-difficulty drum velocities and
Expert double kick alongside pitched vocals. Independent inspection of actual
downloads confirmed the expected boundaries and notes. Reopening only MIDI,
INI and audio, editing Pro Keys, Undo/Redo and reload retained those parts;
final Undo restored byte-identical MIDI. This qualifies browser interchange,
with native controls and target-game interpretation still open. The chart's
S64 interval has no separate fill-versus-BRE distinction.

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
Release `80203e0b`, merged through PR 92 at `3c21644f`, additionally keeps each
browser tab's own project and chart family through reload. Actual hosted
independent band and standalone Dance recovery passed.
Release `1407a963`, merged through PR 93 at `a6e182f5`, includes drum dynamics
authoring and the open-editor recovery download. Its actual native, static and
hosted full-song checks passed; the served WASM matches the tested artifact.
Later desktop additions still need their own browser acceptance.
GitHub Pages serves that engine on `yace.n8bradshaw.com`; its approved
certificate and HTTPS redirect were verified. The served WASM matches the
staged artifact. Actual hosted overview seeking, local vocal pitch analysis and
chart-only export/reimport retain the three-second duration, two vocal notes,
phrases and lyrics. Existing same-origin browser saves reopen after migration.
Earlier engine assets remain accessible for tabs opened before the host change.
Hosted full-song keyboard seeking was also exercised in band, Dance and 4-key
views: paused moves retained pause, playing moves resumed playback, and an
active 130–132-second loop clamped Home/End to its bounds without autoplay.
Physical pointer cancellation and acoustic synchronization remain open.
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
Actual hosted checks exercised Cancel and both source choices using divergent
chart and MIDI content. Chart edits, an exact downloaded backup and restored
Undo/Redo retained the selected source and both original files. Choosing MIDI
separately exposed its own parts; reload and an unrelated Pro Keys edit kept
that choice. Downloads retained unknown chart headers, events and sections,
custom INI fields, source `.lock` companions and a Unicode binary asset. The
Unicode ZIP filename and payload were independently checked. An unsupported
chart-to-MIDI conversion and an archive containing a literal backslash filename
were explicitly refused with the accepted editing project intact. Native UI,
other-platform filename behavior and target-game acceptance remain open.

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

Direct hosted blank-chart setup passed cancellation, recording-free creation,
a twelve-second virtual playthrough and chart-only export/reimport. The package
retained its metadata and duration without fabricating audio. Current native
recording-free setup remains distinct from the earlier native recording-based
manual checks; it still needs its own qualified UI acceptance.

Failed static-browser imports and project opens now report a terminal failure
instead of leaving archive progress visible. Actual browser checks rejected an
archive with an ambiguous filename, retained the open project's revision,
cleared busy controls and exposed the error details. Retrying with a valid
MIDI song or editable backup returned to Ready; the backup's Undo/Redo history
and the song's parts and playback remained available. This correction changes
browser feedback only; native file handling is unchanged.

Actual hosted conventional-channel checks exercised gain and mute Undo/Redo,
solo playback to the ten-second endpoint and downloaded-backup restoration of
the audition settings and history. A playable export retained all ten original
WAV files byte for byte, including split drums/vocals, mono inputs and shorter
tails; its chart, source metadata and timing changes survived reimport. The
reimported game files start with ordinary audition defaults, while the editable
backup retains the previous mix settings. These checks establish browser state
and file preservation; native controls, acoustic balance, phase alignment,
long-file memory and target-game acceptance remain separate gates.

Failed browser song exports also show a terminal failure instead of retaining
preparation text. The failure appears inside the export dialog beside its retry
controls. An accepted retry clears that export's earlier error while retaining
unrelated editing, playback and persistence errors. Actual
browser checks exercised a refused conversion followed by a preserving chart
export, plus a separate rejected edit whose error survived a successful MIDI
export. Both downloads retained the original chart or MIDI bytes.

Native generation now explicitly releases its worker lock when a run or recovery
operation returns. A decoder that briefly inherits an open file description must
not keep a finished job locked and prevent immediate resume. A native OS-lock
regression retains a cloned description, checks that an active worker excludes a
second owner, and verifies that the next worker can acquire after completion.

The static entry point stays small. Application rules, audio processing and
browser API calls remain Rust. CPU-heavy work must allow editing and playback
to continue; an enabled button that freezes the editor does not close a gap.
Before the engine is ready, the static page displays a loading message with no
animation or progress estimate. Dioxus mounting hides it; module-download and
initialization failures use an alert with reload guidance. Actual browser checks
held back the built module, returned a download failure, and then reloaded the
complete editor successfully. Immutable release staging supports this startup
import and older entrypoints while retaining engines needed by open tabs. This
improves feedback without establishing a faster download or startup time.
The resulting browser build also completed a real 243.880-second song at
200 percent, retained a final-note edit through Undo/Redo, and reimported its
downloaded song ZIP with the original chart and recording bytes intact.
