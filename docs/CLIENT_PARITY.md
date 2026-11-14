# Desktop and browser parity gate

Desktop and static Dioxus Web/WASM must support the same charting outcomes
before additional product features ship. Use one source revision and the same
full-length song project for acceptance. Sharing Rust modules establishes shared
rules; it does not establish that both applications expose working controls.

The chart toolbar now opens a text-event editor for ordinary `.chart` global
events and local instrument difficulties. Add, change text or tick, move between
scopes, and remove events with individual Undo/Redo. Identical imported events
remain separate. Sections, solos, BRE and drum-mix controls retain their dedicated
editors; unrecognized syntax and foreign sections remain untouched. New or moved
events outside the timeline refuse atomically. Quotes, backslashes, control
characters and documents over 16,384 editable events remain unsupported.

Actual static-browser checks at `7b6e3fe` and `14d8699` edited one of two identical
global events, moved an imported local event to global scope, removed another
global event and added a cue to a different difficulty. Dedicated-control and
out-of-range edits refused without advancing revision. Exact exported chart
bytes retained the other duplicate, malformed/foreign syntax, comments, CRLF,
notes, technique flags, solos, Star Power, other difficulty and tempo changes.
Original audio and sources remained exact. Complete backup restore, four Undo
and Redo steps, and an owned-browser restart retained the edits and history;
native CLI restoration, text removal, Undo and export passed independent checks.
Before/after restart backups retained identical project and content-addressed
files. This qualifies the controlled `.chart` subset.

Imported MIDI text editing remains pending and is explicitly read-only. Arbitrary
local chart events without a MIDI representation refuse conversion rather than
silently broadening their scope. Native GUI, musical/game behavior, broader
format/browser/platform coverage and larger event workloads remain open.

Bulk note groups now accept up to 100,000 notes and 32 MiB clipboards, with
indexed collision, shared-flag and vocal-word handling. The shared inspector
offers relative tick/row moves, operation status and explicit atomic failures.
An operation refuses if the project or selection changes before dispatch.

Actual static-browser checks at `8c0182f` copied 12,001 notes, moved them by
120 ticks and one row, refused an out-of-grid move without advancing revision,
used Undo/Redo, deleted the group and pasted the original copy. The downloaded
song and complete backup retained all 8,001 HOPO/tap flags, unrelated parts,
unknown events and original audio. Seven content-addressed backup files and
three history checkpoints verified independently; reopening that backup in
the browser and native CLI retained working Undo/Redo and exact native export.
Eighteen group regressions also cover large independent vocal phrases,
word ownership, collisions and oversized no-commit failures.

On this controlled 12,001-note browser run, moving/deleting/pasting displayed
progress before completion (about 0.20–0.68 seconds from start to completion).
The move included a 621 ms main-thread task; this does not establish uninterrupted
playback or broad performance parity. The maximum-size workload, actual native
GUI, other browsers, full-length musical/game workflows
and comparative resource measurements remain separate acceptance gates.

The hosted `8c0182f` runtime also passed 12,001-note copy/move/delete/paste,
export and owned-browser restart checks with an invented, audio-free fixture.
Its 22,923,206-byte WASM matched the local SHA-256
`9132276906cdba93dfc95228f9e76fe8e28d0f1739084c1202c59f2e3f564bf2`.
Complete backups before and after restart retained identical revision-7 project
JSON and three Undo checkpoints. Post-restart Undo removed the pasted group;
Redo restored all 12,001 notes. Independent export checks retained the flags,
unknown events and unrelated parts. This qualifies that controlled hosted subset.

Vocal group moves and deletes preserve authored scoring-phrase padding. Moving
every note in a phrase shifts its original boundaries; partial moves retain or
extend them. Overlapping scoring phrases refuse the complete edit. Imported
Lead scoring markers now retain channel, velocity and note-off representation
through editable JSON, history and MIDI export. Older documents still open;
export refuses a phrase rewrite when missing metadata would reset non-default
source marker attributes. Reimporting identical source in the browser can reopen
its existing editing copy, so it is not a metadata migration.

Controlled actual-browser checks reproduced the previous loss, then moved the
first syllable by 120 ticks and deleted/restored it. Independent MIDI checks
retained its padded 520–1120 scoring range, an untouched player-2 range at
1200–2000, original channels/velocities/note-offs, all unrelated raw tracks and
the original recording. An overlapping move refused without advancing revision.
A 17-file complete backup reopened with working Undo/Redo; native CLI restoration
and export independently retained the moved and original events. Native GUI,
older-document migration, broader browser coverage and musical/game acceptance
remain open.

Hosted `3c1f42a` passed the same audio-free move/delete, overlap refusal and
event-level MIDI checks. Its 22,903,792-byte WASM matched the local SHA-256
`c14dadef1725208dc42db8b983f02ac6d3466b51eba56bc10f0b9bb0957bcbc6`.
Sixteen-file backups before and after an owned-browser restart retained identical
revision-6 project JSON and working Undo/Redo. Reopening the backup and exporting
after Undo/Redo preserved exact scoring attributes and unrelated raw tracks.

Timing repair now previews the current selection or whole active canvas part.
Choose a maximum shift to preserve more distant off-grid notes, or explicitly
snap every note in scope. The preview reports changes, untouched notes and a
bounded sample. Apply preserves durations, pitches, gem IDs and MIDI attributes;
collisions, incompatible shared techniques and stale previews refuse the edit.
Band and Dance use the same integer musical grid.

Controlled actual-browser checks at `8f6ac08` repaired selected and whole-part
guitar notes, kept an intentional off-grid note, then moved it under the explicit
all-notes policy. Read-only previews, invalid settings, incompatible-attack
refusal and atomic Undo/Redo passed. Export and a nine-file complete backup kept
both technique flags, other difficulties, unknown events, tempo changes and
original audio. Browser restore/history/export and native CLI restore/history/
export passed independent checks. Lead repair retained scoring padding, original
MIDI attributes and other raw tracks through a 17-file backup, restore and export.
Dance repair preserved a seven-row hold while moving its onset and kept an
off-grid mine until explicitly snapped. Its three-file backup, two Undo steps,
restored browser workflow, native-core history and exact SM export retained the
other difficulty, timing, custom metadata and original audio.

Production runtime `d1c56fd` also passed near/all policies, incompatible-attack
refusal, exact chart/MIDI/SM exports, complete backups and Undo/Redo after
separate owned-browser restarts for Band, Lead and Dance. Lead scoring padding
and attributes, Dance hold lengths and unrelated authored data remained exact.
Dance can still show a stale welcome/import footer after reopen or history
actions despite a valid active chart; its broader status feedback remains open.

Repair also offers a whole-voice target for editable HARM1, HARM2 and HARM3.
Other singers, shared scoring ranges and shared lyric ranges stay unchanged.
Moves that leave the original shared range or overlap another note in that
voice refuse the complete edit. Original order retains syllable ownership.
Opaque/read-only harmony imports keep their existing authoring restrictions.

Actual-browser checks at `c0aa787` and `2d0f2f4` exercised each independent voice,
kept an intentional off-grid note, then snapped it under the explicit all-notes
policy. Read-only and no-op previews, collision/range refusal, four atomic Undo
steps, Redo, complete-backup reopen and an owned-browser restart passed. An
18-file backup and independent raw-MIDI audits retained shared markers, physical
note attributes, unchanged Lead/Guitar/tempo/venue tracks and original local
audio. Native CLI restore/history/export passed the same data checks.

Production runtime `2d0f2f4` passed the three independent voice targets, near/all
policies and safe collision/range refusal. Complete-backup reopen and an owned
browser restart retained byte-identical revision-4 project data. Four Undo steps
and Redo exported the exact original and repaired voices, with shared phrase
owners, note attributes and unrelated tracks preserved.

A controlled no-audio 100,000-note Band fixture also passed preview, apply,
exact Undo export, Redo and a six-file backup with stable IDs and an unchanged
other difficulty. Long tasks up to 837 ms were observed across this workflow;
this establishes bounded-workload correctness, with responsiveness still open.

Native GUI, independent musical/game acceptance, broader platform/fault coverage
and maximum-size workloads for other repair targets remain open.

Snap now offers custom 1–1024 steps per beat and fine musical presets through
1/1024. Placement, dragging, sustain ends and visible grid lines use the same
rounded musical positions, including unusual PPQ and tempo changes. Pointer
positions retain their fractional tick until the musical grid is chosen.
Dance uses its 48-row resolution consistently for the roll, placement,
keyboard navigation and atomic selection quantization; finer subdivisions can
share a representable row. Dense grid rendering is bounded. Existing notes keep
their timing when the grid changes; Dance quantization is an explicit selection
edit.

Actual static-browser checks at `994ff6e` reproduced and corrected a 7-PPQ
triplet placement from tick 0 to tick 2. Custom five-step placement, dragging
tick 7 to 13, resizing its sustain to four ticks, fine placement, invalid-value
refusal and Undo/Redo passed. A ten-file backup retained five verified history
checkpoints; native CLI restore/history/export retained exact timing, IDs,
technique flags, unknown events, other parts and original audio. Actual Dance
checks placed and quantized onto row 10, restored row 13 with Undo, redid row 10,
advanced one row with a fine-grid keyboard step and reopened its complete backup.
The SM export retained the other difficulty, custom metadata and original audio.
Four grid, four timeline and 21 Dance regressions cover the underlying rules.
The hosted `994ff6e` runtime passed the same 7-PPQ chart-only placement, custom
and fine grids, drag/resize, invalid-input, export and restart workflows. Its
22,952,579-byte WASM matched the local SHA-256
`43eb2231794aa176ba87624ec3a46cbe231da9a39509d7c11d2646772533db02`.
Before/after-restart backups verified eight/nine files (the export report adds
one), five Undo checkpoints and byte-identical revision-12 project JSON:
`1988dfd9134e80770044924211a096ba488b83479861fdd8415a6a339d863a7f`.
Post-restart Undo/Redo retained the fine note and resized forced note. Hosted
chart-only Dance also passed custom placement/quantization, exact Undo/Redo,
fine keyboard navigation, SM export and complete-backup reopen; independent
audits retained the other difficulty and unknown metadata. A simfile declaring
an absent recording refused import while retaining the current project; an
explicitly chart-only simfile opened normally. Native GUI, broader platform
and fault coverage, and musical/game acceptance remain open.

Vocal overdrive now has shared native/static controls for independent Lead and
shared HARM1 power ranges. HARM1 supplies power to HARM2 and HARM3 in the target
parser; their original opaque power events remain untouched. Tick editing,
timeline markers, current-loop bounds and witnessed Undo/Redo use a source-bound
MIDI writer that preserves channel, onset velocity and note-off encoding.
Unsupported or ambiguous originals remain read-only. Explicit clears and export
omissions survive reopen; re-authoring an omitted singer does not restore its
archived power ranges. A retained percussion-only vocal owner can receive its
first power range without inventing singer notes or replacing its controls.

Actual static-browser checks at `a0701d5` moved Lead 480–960 to 720–1200, added
1920–2400, moved shared harmony 960–1440 to 1080–1560 and added 2880–3360.
A stale Apply after Undo refused while retaining its draft; Redo restored the
range. Removing all four ranges, Undo/Redo, export and actual song-ZIP reopen
retained the explicit clear. An independent MIDI parser checked exact edges and
attributes, unchanged singer notes/lyrics/scoring/percussion, opaque HARM2/3
controls, all other raw tracks and the original recording. The complete browser
backup restored 18 verified files and four Undo checkpoints in a pinned native
CLI; native exports and Undo/Redo matched the browser MIDI/WAV byte-for-byte.
Ten focused core/project regressions include chart-origin creation, marker-only
and inherited percussion-only owners, export/reopen, clear and omission/re-authoring.
Native GUI, broader hosted
qualification, game/controller, listening, selected-note bounds and broader
fault coverage remain open; shared source and CLI checks do not close them.

The hosted `0bd3c56` runtime also passed first-power creation in an inherited
percussion-only HARM1 owner, independent Lead creation/resize, Undo/Redo and
downloaded MIDI preservation checks using invented MIDI without audio. Its
20,689,261-byte WASM matched the local SHA-256
`ac4129d5c61a5d105d87040ec65717df4878a51091442e634caa5fbc36e20a41`.
Complete backups before and after terminating only the verification browser and
restarting its isolated profile each verified 16 file references and four Undo
checkpoints. The revision-7 project JSON was byte-identical:
`1564a41b54b8991e04ef258bff0d602aaad701716051f22dc5902f2431a2e51b`.
The restored controls displayed both Lead ranges and shared harmony power.
Removing shared harmony power afterward retained the Lead ranges and passed
another independent export audit. These controlled checks retain the separate
native GUI, musical/game, current-loop and broader fault acceptance gates.

Instrument trill and tremolo editing now has shared inspector controls and a
timeline lane. Chart sources keep independent difficulty ranges. Imported MIDI
exposes Expert and Expert+Hard scopes separately; Pro Keys trills belong to
their dedicated difficulty owner. Moving or resizing an imported marker keeps
its exact channel, onset velocity and note-off encoding. Unsupported or
ambiguous original markers stay read-only. MIDI conversion refuses independent
chart-tier ranges that the destination cannot represent.

Actual static-browser checks at `ff45171` used a controlled fixture to move a
shared tremolo, add an Expert trill, refuse a stale Apply after Undo, restore
with Redo, create Hard, and clear all Guitar ranges. Timeline labels, scope
filtering, reload, game export/reopen and the complete project download passed.
An independent MIDI parser checked the exact marker edges, all unrelated Guitar
events, every other raw track and the original WAV. Separate Pro Keys tiers
displayed independent trills. The actual browser backup restored 16 verified
file references and six Undo checkpoints in a pinned native executable; native
export, Undo and Redo matched the browser MIDI and WAV byte-for-byte. This
qualifies those controlled workflows. Native GUI, target-game behavior,
listening and broader fault coverage remain open.

On `d4c1f15`, the actual browser retained the project after an owned browser
process restart, created a Hard Pro Keys part with no notes and a trill, and
exported its new physical MIDI owner. Independent parsing verified that owner,
separate Expert/Easy Pro Keys ranges, the continued Guitar clear, every other
raw owner/event and the original recording. New-owner and representable
chart-to-MIDI scope cases also pass the codec regressions.

Harmony authoring has actual static-browser verification for the supported
editing subset below. Shared Rust controls expose HARM1–3 notes and lyrics,
recording loops, Undo/Redo, and the format's shared HARM1 scoring and HARM2 lyric
ranges. Phrase edits, splits and merges keep singer notes intact and refuse
uncovered notes or retained percussion. Harmony-only imports expose these
controls through Vocals. The core imports a supported ordinary MIDI subset and
exports independent singers while preserving unrelated source tracks and
controls. Clearing the voices retains source controls; explicitly omitted
harmonies stay omitted when reopening or authoring new notes later. Unsupported
imports remain opaque. Native GUI, broader hosted workflows and fault coverage,
listening and target-game acceptance remain open. Shared controls do not qualify
those workflows.

Vocal percussion has actual static-browser verification for the supported
editing subset below. The shared Rust pane exposes separate lead and
harmony hits, displayed/hidden flags, tick placement and movement, removal,
selection, scoring-phrase resize/split/merge, Undo/Redo and recording-phrase loops.
Its bounded preview follows the source transport clock and distinguishes hidden
hits from displayed hits. Selected forms retain exact hit/phrase witnesses when
other edits change their indices, so stale forms cannot retarget another event.
A separate Rust document validates displayed/hidden hits and dedicated
percussion scoring phrases without changing pitched/talkie syllables. Verified
dedicated percussion scoring is excluded from the lead singer import view, so
separate singing phrases remain editable; their original percussion markers and
physical hit attributes survive singer edits, export and reopen. Unsupported
uncovered hits retain the conservative read-only source behavior. It refuses
phrases that overlap the applicable lead or harmony singing phrases. Typed project
commands create, move, hide/show and delete hits, and resize, split or merge
dedicated phrases using exact selected-value guards. Moving a phrase's final hit
to a new dedicated range is one atomic, undoable command. First singers in a
verified percussion-only owner adopt the retained percussion in that same edit;
new-note defaults avoid its scoring ranges. Commands persist authored hits
or an explicit clear, with content-addressed Undo/Redo and portable project
relocation. Legacy documents inherit only active source parts; reopening an
instrument-only export does not revive omitted hits.
Singer edits refuse conflicts with supported inherited percussion before saving;
explicit authored or cleared percussion state governs subsequent edits and Undo.
The shared export pipeline stages verified source MIDI without changing its
archive, rebuilds singers, then writes the selected percussion. A MIDI-bound
working sidecar preserves authored, cleared and omitted state on reopen.
Logical Vocals/Harmonies selection replaces duplicate raw percussion owners;
omitted original controls stay omitted when singers are authored later.
Native GUI, broader hosted workflows, fault coverage and musical/game
judgments remain open. A strict MIDI reader extracts dedicated source phrases
with original hit and scoring-marker attributes;
ambiguous, mixed singing/percussion and uncovered source hits refuse the editable
view. Shared scoring ranges used by unmanaged harmony percussion also remain
read-only. The core checks the saved source SHA-256 before selectively adding,
editing or clearing percussion and its dedicated markers in existing owners.
Unmodified tracks remain byte-identical; singing, lyrics, overdrive and author
controls are retained. Supported harmony MIDI views exclude verified dedicated
percussion ranges from singing phrases; singing-phrase rewrites preserve the
physical percussion markers and reject new scoring conflicts. Project export,
reopen (including percussion-only selection), explicit omission, later singer
authoring, repeated export and portable
history regressions cover this backend path. Conflicting candidates and invalid
owner endings refuse before returning a rewritten song. These checks do not
qualify an actual application workflow on their own.

A source-hash-bound harmony writer can add first singers to retained
percussion/control-only owners while preserving aliases, physical hits and
unrelated tracks. It refuses existing singer content, changed source bytes,
overlapping opaque percussion and malformed edited-owner endings. The shared
pipeline uses it for first singers in managed percussion owners; the exercised
browser subset and remaining application gates are recorded below.

The user has expanded the parity gate to include local browser generators and
vocals/instrument separation. The earlier blanket inference exception no longer
closes this work. No application service is substituted. Browser generation,
recognition and supplied-lyric alignment have actual full-song verification;
see [the acceptance record and the disabled HQ model exception](BROWSER_GENERATORS.md).

## Acceptance ledger

Controlled hosted storage failure on `00e7481` used an isolated browser profile
and a one-byte origin quota. The accepted revision 2 retained coda 1560; editing
to 1500 produced revision 3 with an explicit autosave failure and unsaved status.
The recovery action downloaded a valid 13-entry ZIP containing the draft,
original MIDI and three hash-verified Undo checkpoints. A fresh tab independently
reopened accepted revision 2 with coda 1560. After restoring quota, Undo/Redo
saved the retained draft successfully. The revision-7 complete backup verified
12 references and three Undo checkpoints; another backup after owned browser
restart contained identical project JSON. A native CLI opened the failed-save
recovery ZIP's extracted project and undid/redid 1500→1560→1500. This qualifies
that controlled commit-failure/recovery/retry case. It does not establish
eviction, hardware disk failure or every browser/platform failure mode.
The actual browser saved-project importer also reopened the failed-save ZIP;
Undo/Redo retained 1500→1560→1500. Its subsequent complete backup verified 14
references and three Undo checkpoints with identical content, source binding
and Undo/Redo stacks to the recovered draft.

Removing the isolated profile's authoritative IndexedDB project store and
reloading produced an explicit unavailable-project warning. Opening its verified
complete backup restored revision 5 and usable Undo/Redo; the resulting backup
retained identical content, source binding and history. This is controlled
storage removal, rather than evidence of a browser's real pressure-eviction
policy.

Browser backup restoration now clears an earlier unavailable-project recovery
warning after the new project opens successfully, provided that no newer error
has replaced it. Actual local checks repeated storage removal and backup import,
then waited for idle: the restored revision stayed saved with no stale alert.
A separate one-byte-quota restore still displayed its new autosave failure and
unsaved status; restoring quota allowed Undo/Redo and saving again.

Production runtime `2d0f2f4` also exercised the browser's actual Leave/Stay
confirmation for both Band and standalone Dance after a quota-refused save.
Stay cancelled navigation and retained the unsaved draft. After verified draft
backups, Leave navigated away; reopening retained the byte-identical last
accepted save. The downloaded drafts reopened through the normal project picker,
with exact Undo/Redo and exported notes, flags, holds, mines, timing and unrelated
parts. After saving succeeded, navigation proceeded without a confirmation.
This qualifies those controlled unload/recovery paths; real pressure eviction,
other browser/platform behavior and broader fault acceptance remain open.

Terminal big rock ending authoring at `00e7481` shares Rust coda/range controls,
Undo/Redo, timeline bands and recording-clock preview labels across clients.
Five-fret guitar, bass, rhythm/co-op and standard Keys use physical MIDI
120–124 controls; drums use their existing activation ranges plus global coda
context. Guitar/keys endings force MIDI in Automatic and block `.chart` in the
export dialog. Drum endings can use `.chart` coda plus S64, as documented in the
[primary format reference](https://github.com/TheNathannator/GuitarGame_ChartFormats/blob/main/docs/Chart-File-Formats/chart-format/Tracks/Drums.md#special-phrase-mechanics).
Middle-of-song coda pairs, incomplete guitar lane groups and ambiguous source
contexts remain read-only; their original bytes stay retained.

Actual static-browser controls moved coda 1920→1680, guitar 1920–2400→2160–2640,
added guitar 3840–4320 and moved drums 1920–2400→2880–3360. Stale coda/range
actions refused while keeping the draft; Undo/Redo restored the corresponding
states. Preview labels appeared inside the guitar/drum endings and disappeared
outside, without a conflicting drum-fill label. Independent downloaded-MIDI
inspection verified the coda, ranges, original guitar lane channels/velocities
and note-on-zero releases, all other notes/text/techniques/mix events/raw owners
and the unchanged WAV. A fresh import on the final build also retained the
moved drum ending's original channel, attack velocity and release velocity.
The same source-bound preservation path covers fill and roll moves; ambiguous
physical attributes refuse edits while retaining the original source.

The revision-15 complete backup verified 14 references and four Undo checkpoints.
A pinned native CLI opened it and exported identical MIDI/WAV before and after
native Undo/Redo. Restarting only the owned browser on the final guarded build
produced identical project JSON in another complete backup. Removing all three
endings, Undo/Redo, game export and actual ZIP reopen retained both explicit
clears and coda context.

The final preservation build repeated these edits from a fresh import. Its
revision-11 backup verified 14 references and four Undo checkpoints, and native
CLI export/Undo/Redo again matched the browser MIDI and recording exactly.
After a selected-track change and owned browser restart, revision 12 retained
identical authored content and Undo/Redo stacks, including the physical drum
attributes. Transport/selection revisions are separate from content history.

A new chart without audio authored a marker-only guitar ending. Actual `.chart`
selection disabled preparation with the format error; Automatic produced MIDI
with the five expected lanes and reopened as an editable empty guitar part.
After clearing guitar and adding a drum ending, actual `.chart` export/reopen
retained coda 1920 and S64 at 2880–3360. A separate middle-coda fixture exposed
read-only controls and exported byte-for-byte unchanged. Ten BRE cases, two
drum-attribute preservation cases and 28 existing MIDI integrations pass with
strict native/WASM checks. Native GUI, target-game/listening, current-loop
controls and wider fault acceptance remain open.

Hosted acceptance on the same `00e7481` release repeated coda, guitar and drum
edits, stale refusal and Undo/Redo with an invented fixture containing no audio.
The actual downloaded MIDI matched the locally audited export byte for byte,
including the moved original drum attributes. The complete backup verified 13
references and four Undo checkpoints; another backup after owned browser
restart contained identical project JSON. The live app loaded the verified
20,911,415-byte WASM and displayed the guitar ending at the matching clock time.

Disco authoring at `3ed7142` exposes independent drum difficulties through shared
Rust inspector controls, tick bounds, current-loop bounds, removal, Undo/Redo
and timeline bands. The Pro Drums preview interprets red/yellow swaps at each
note's original tick; Standard Drums retains its original interpretation.
Original text, stem configuration and explicit resets are retained. Open-ended
flips, changing stem configurations and ambiguous transitions remain read-only.
Mix-only difficulties without playable notes are included in export choices;
explicitly omitting one remains effective after export/reopen.

Actual static-browser controls moved Expert 480–960 to 720–1200, added
1920–2400, created a Hard part and moved its independent range to 1080–1560.
Stale Apply refused while retaining the draft. Extending the Expert range over
a yellow cymbal changed the actual Pro preview to a red tom; Standard showed
a yellow tom, and Undo restored the yellow cymbal. Downloaded MIDI retained the
other difficulty's non-flip mix text, original non-mix drum events, unrelated
raw tracks and WAV. Removing both Expert ranges, Undo/Redo and reopening the
game ZIP retained the clear and the independent Hard range.

The actual revision-8 complete backup verified 15 references and four Undo
checkpoints. A pinned native CLI opened it and produced byte-identical MIDI/WAV,
including after native Undo/Redo. The later revision-15 backup verified 18
references and six Undo checkpoints. Terminating only the verification browser
and reopening its profile produced an identical project JSON in a second
complete backup. Eighteen drum-mix regressions and 28 existing MIDI integration
cases pass. Native GUI, musical/game/listening, current-loop controls and wider
recovery acceptance remain open. A drum activation interval alone does not
establish BRE context.

The deployed `3ed7142` runtime repeated independent Expert 1920–2400 and Hard
480–960 disco creation, Pro preview and Undo/Redo using invented data without
audio. Independent downloaded-MIDI inspection retained every original drum
note/fill/roll attribute and other raw owner. Complete revision-5 backups before
and after restarting only the verification browser verified the same 14
references, two Undo checkpoints and byte-identical project JSON. The loaded
20,461,479-byte WASM matched the locally tested artifact.

Static-browser controls at `131f989` independently created three simultaneous
harmony singers beside retained lead and percussion. Each singer then held two
adjacent notes at ticks 0–240 and 240–480. Timing, pitch and lyric edits, shared
HARM1 scoring split/merge and Undo/Redo worked. A split through a held note and
a singing phrase overlapping dedicated percussion refused without saving or
losing the entered draft. Independent inspection of downloaded MIDI verified
the six pitches/lyrics, adjacent scoring boundaries, separate HARM2 lyric range,
original percussion attributes, unchanged other raw tracks and original WAV.

The percussion pane changed a hidden zero-length hit at tick 2208 to displayed;
the downloaded MIDI changed only its key from 97 to 96, retaining channel 2,
velocity 93 and its note-on/zero-velocity off representation. Undo/Redo restored
the corresponding states. A complete project backup verified all 34 referenced
files, reopened in the browser with its ten Undo checkpoints and working Redo,
and survived reload. The pinned native executable opened that portable backup,
exported matching MIDI/WAV and repeated Redo/Undo with byte-identical results.
This establishes the exercised browser and native file-interchange paths;
native GUI, acoustic preview, game acceptance and broader fault
coverage remain open.

Hosted `b260e1e` repeated three simultaneous singers beside retained lead and
dedicated percussion using invented MIDI without audio. A hidden-to-displayed
hit edit, Undo/Redo and reload passed; independent downloaded-MIDI inspection
verified all singers, marker owners, the unchanged lead and exact physical
percussion attributes. A complete revision-7 backup verified all 16 references
and four Undo checkpoints. After termination of only the verification browser,
the same profile reopened revision 7 with all edits. A second complete backup
verified every reference and contained byte-identical project JSON. These checks
qualify that controlled hosted editing and restart subset, with the independent
native, musical, game and broader fault gates retained.

Deleting the final pitched lead-vocal notes exposed an export preservation gap:
the export selector omitted `PART VOCALS`, including untouched percussion,
overdrive and author controls. The shared Rust fix retains that source part and
any scoring phrase required by its percussion. Conflicting phrase edits refuse
export rather than displace the retained percussion. Explicitly deselecting
Vocals remains durable across export, reopen and later lead-note authoring.
Actual static-browser checks deleted both lead notes, exported with Vocals
selected, reopened the downloaded song and exported again. Independent MIDI
inspection retained the exact percussion, scoring, overdrive and author events,
unrelated tracks and original recording; pitched notes and lyrics stayed gone.
A separate UI export explicitly omitted Vocals. After reopening it and authoring
a new C4 note at ticks 960–1200, two export/reopen cycles retained that new note
without restoring the omitted controls. Both repeated MIDI exports were
byte-identical. The deployed `95724ec0` runtime repeated keep, explicit omission
and new-note authoring with an invented chart-only fixture. Independent MIDI
inspection passed each downloaded result. Native GUI and game interpretation
remain separate acceptance gates.

An actual hosted MIDI export exposed another preservation gap: adding a new
Star Power phrase normalized an untouched phrase's channel and velocities.
The merge now matches complete unchanged physical marker intervals and retains
their original note-on and note-off events. New, resized or removed intervals
remain editable. Regressions cover adding/removing a phrase, Undo/Redo and
reopen, plus unchanged Star Power, instrument/Pro Keys solo and drum fill/roll
edges during each component rewrite. The actual static editor repeated the
hosted reproduction and retained channel 4, velocity 89 and release velocity 22
on the untouched phrase. Its export matched the native executable byte for
byte. The downloaded song reopened, a second phrase moved, Undo/Redo and reload
retained the acknowledged edit, and a final repeat export was byte-identical.
Native executable checks independently passed Add, Undo/Redo, reopen and repeat
export. The deployed `95724ec0` runtime repeated the same Add/export controls;
its MIDI was byte-identical to the qualified static and native outputs. Native
GUI and game interpretation remain separate acceptance gates.

| Workflow | Verification evidence | Result |
| --- | --- | --- |
| Start from recording or existing chart | Both OS-picker/manual starts exercised; native setup now reaches Ready after save; identical chart/INI settings and unchanged encoded source | Verified |
| Blank charts and recording attachment | Actual browser blank creation, note placement, silent timeline playback, short-recording rejection, attachment/Undo/Redo, export, backup and restored Undo; source WAV unchanged and post-Undo export has no playable root recording | Verified; fixed browser open/source-state handling after the chart-only rebase |
| Instrument/difficulty editing | Shared command tests plus actual guitar timing/Tap, Keys, Pro Keys, vocal editing, and native edit/Undo; paired full-song MIDI/master/vocal export matches | Verified supported workflows |
| Keys MIDI open notes and techniques | Actual hosted four-difficulty export/reopen with Pro Keys and vocals, independent MIDI inspection, edit/Undo/Redo and byte-identical restored export | Browser interchange verified; dedicated Keys authoring and native/game acceptance remain open |
| Drum accents and ghost notes | Actual native/static single and batch authoring, kick exclusion, cymbal retention, move, clipboard, delete, Undo/Redo, restart/reload and mixed-part MIDI export/reimport | Verified controlled fixture; in-game dynamics judgment remains unqualified |
| Double kicks and imported MIDI drum editing | Actual native/static placement, movement, clipboard, deletion, duplicate refusal, saved Undo/Redo, retained imported note-off intervals, full-song editing and export/reopen | Both clients verified on controlled fixtures; musical and target-game acceptance remain open |
| Five-pad drum authoring | Actual native/static-browser seven-hit fixture: Orange/Green lanes, five-receptor preview, dynamics/movement/clipboard/delete, restart/reload and browser backup restored in native with history; both restored `.chart` exports match source bytes | Both clients verified on controlled fixture; independently authored fixture and target-game acceptance remain open. Five-pad MIDI conversion, generation and reduction are unsupported |
| Stem mixer control names | Actual native six-channel and hosted eight-channel groups expose individually named Mute/Solo controls; keyboard mute, Undo/Redo, hosted solo playback and reload recovery exercised | Programmatic names and exercised controls verified; full assistive-technology acceptance remains open |
| Independent preview and piano-roll tracks | Native and browser Focused/Selected/All choices, stable order, collapse and reopen; actual hosted reload preserves separate choices without changing chart revision | Verified exercised paths; browser preferences stay in that profile/project |
| Timing, sections and Star Power | Browser edits/Undo and native tempo, section and phrase Add/Undo exercised through their actual dialogs | Verified |
| Playback, seek, loop, speed and waveform | Full 243.880 s browser playthrough; native whole-song playback; browser loop/speed; all five saved kit components at 75%; native kit backup opened at saved speed/selection | Verified; acoustic synchronization is unmeasured |
| Supplied stems and part analysis | Real full-length supplied drums, worker validation, native cached kit analysis/audition and portable roundtrip; attachment Undo/Redo replaced actual browser audio channels | Verified supplied/cached workflows |
| Local generation and separation | Actual six-stem inference, full-song drum/guitar/bass and vocal generation, supplied-lyric alignment, standard piano and Synth; generated drafts played, edited/undone, exported and restored; full-song local kit inference compared with native | Verified exercised workflows; detector/ASR differences and disabled HQ exception are documented in BROWSER_GENERATORS.md |
| Manual clean audio | Both actual panels rendered the same bound plan; native output matched shared DSP exactly; 10,755,108 browser stereo frames, zero lag, maximum decoder delta 0.00000054 | Verified functional render; musical acceptance remains with the listener |
| Review and flags | Shared rules, actual browser queue/decisions and portable source-bound review history | Verified |
| Team proposals | Actual browser proposal → native import/independent approval/merge → browser conflict resolution → fresh native approval → browser merge; new parts preserved | Verified file exchange and protocol tests. Authenticated GitHub writes remain untested at the user's request |
| Recovery, undo and backups | IndexedDB restore/stale-tab guard; independent browser-tab projects and standalone Dance recovery; same-name library switching with separate histories; full project/kit roundtrips; imported first checkpoint and native/browser Undo/Redo; controlled hosted process exit, quota-refused saves and explicit Band/Dance Leave/Stay recovery | Verified controlled paths; real pressure eviction and other storage/platform faults remain open |
| Playable export | Actual client exports had byte-identical MIDI, editable master, vocal JSON, INI and original Ogg; partial-stem game receipt avoids duplicate mix playback | Verified export equivalence; game/controller acceptance is separate |
| Navigation and errors | Shared controls and OS pickers; actual narrow browser layout; lazy folder discovery; explicit downloads; native dialogs expose their accessibility role | Verified exercised paths |
| Browser desktop workspace | Static and hosted thresholds, per-tab dismissal/reload, keyboard overflow and uninterrupted playback; actual Firefox zoom at 140%; native layout retained | Verified exercised paths; assistive technology and broader platforms remain open |
| Release | Public static origin/custom domain and canonical native identity configured | Final publication verification is recorded in DEPLOYMENT.md |

Do not mark a row passed from compilation alone. Record the revision, fixture,
actual application actions, downloaded/exported results, and any limitations.
Keep private recordings, transcripts and generated receipts outside Git.

Solo authoring has shared native/WASM inspector controls, timeline bands and a
recording-clock highway highlight on its feature branch. The inspector creates
ranges from selected notes, the current loop or explicit ticks, and provides
move/resize, removal, Undo/Redo and original-recording solo loops. Forms retain
typed drafts and exact selected ranges when an intervening edit makes them
stale; project/track guards prevent applying them after a switch. These controls
use shared Rust range validation and revision-checked commands. Exact previous-range
witnesses reject stale changes. Source parsing preserves malformed ranges
read-only; rendering changes only supported solo boundaries. MIDI instruments
share edited ranges across difficulties while Pro Keys keeps independent MIDI
owners. New MIDI difficulties inherit the current shared solos, including an
explicit clear; conflicting authored copies refuse edits. Regressions cover
history, export/reopen, explicit removal without resurrection, retained unrelated
events and note-proposal safety. Actual static controls moved/resized ranges,
created a range from selected notes, kept Pro Keys independent, refused stale
forms and exercised Undo/Redo, complete-backup reopening and browser restart.
The source-clock preview displayed SOLO inside a range and cleared it outside.
Native GUI, recording-loop/current-loop creation, independent
game/controller/musical and broader fault gates remain open.

The integrated `5d49dda` static runtime repeated a concrete late-difficulty case:
Guitar solo 120–720 moved to 240–960, a new Medium difficulty inherited both
current solos, removing them and adding Easy retained zero solos. Undo/Redo
restored the corresponding history. Independent downloaded-MIDI inspection
confirmed no resurrected solo markers, unchanged other guitar events, exact
other raw tracks and the original WAV. Twelve solo-related cases, 41 unfiltered
editor/export integrations and strict native/editor plus WASM Clippy passed.

The deployed `5d49dda` runtime repeated solo creation at ticks 1440–1920 and
inheritance by a new Medium difficulty using invented MIDI without audio.
Removing the range, Undo/Redo and adding Easy retained the cleared state.
Independent authored and cleared MIDI inspection retained every other raw track
and guitar event. The complete revision-7 backup verified all 15 references,
four Undo checkpoints and explicit empty solo arrays on Expert, Medium and Easy.
This qualifies the exercised hosted controls, export and backup subset;
remaining native, musical/game and broader recovery gates stay open.
Drum fill/activation and single/double-roll authoring has a shared Rust range
model and project commands on its feature branch. It uses the existing S64,
S65 and S66 conversion; S64 retains an activation interval without inventing a
separate fill-versus-BRE classification. Typed edits preserve unrelated rows,
reject stale witnesses before saving and participate in Undo/Redo. A bounded
range clipboard retains relative intervals and requires a matching chart
resolution. MIDI edits update the shared drum owner across difficulties;
asymmetric source activation lanes remain read-only while safe roll edits keep
their original events. Regressions cover explicit clears, export/reopen,
clipboard refusal and note-proposal safety. Shared inspector controls now expose
typed range bounds, current-loop bounds, removal and Undo/Redo, with separate
colored timeline bands and current-range preview labels. Native and browser
clipboard adapters use the same bounded format; asynchronous paste refuses a
changed project, track or revision. The inspector disables activation edits for
opaque source lanes. Actual static controls authored all three kinds, copied and
pasted ranges, moved and resized them, refused a stale draft, and exercised
Undo/Redo. Separate adjacent activation intervals survived downloaded MIDI
export. A recording loop crossed a tempo change and wrapped to its saved start;
this establishes clock behavior, without an acoustic synchronization claim.

New MIDI drum difficulties inherit the current shared ranges, including explicit
clears. Earlier projects with a later-added difficulty use the same authored
view. Conflicting shared copies refuse range edits instead of restoring source
markers. Actual browser backups confirm shared state on new Hard and Medium
parts and explicit empty arrays on all four drum difficulties after clearing
from one part and adding Easy. Downloaded cleared MIDI reopened and exported
again without restoring the ranges. Independent inspection retained original
drum notes, solo events, channels and velocities, every unrelated raw MIDI track
and original recording. A pinned native executable repeated authoring,
late-difficulty inheritance, clearing and Undo/Redo; authored and cleared MIDI
matched the browser byte for byte. Native GUI, clipboard OS
integration, game interpretation and musical acceptance remain open.
An actual asymmetric-activation fixture disabled the fill option while allowing
a single roll at ticks 960–1440. Its downloaded MIDI retained the opaque key-120
interval's channel 6, velocity 77 and release velocity 33 exactly, along with
the other tracks and recording. Chart-based projects retain independent
per-difficulty ranges: adding a double roll to Hard left Expert unchanged in
both the rendered controls and downloaded `.chart`.

The deployed `8fa1be97` runtime repeated drum authoring with an invented
chart-only MIDI fixture. Adding a double roll at ticks 1920–2400 and a new
Medium part retained the shared range view. Undo/Redo removed and restored
both edits. The complete downloaded backup passed all 14 manifest size/hash
references and retained three explicit range maps and both Undo checkpoints.
The playable MIDI retained the original activation and single-roll channels,
attack/release velocities and unrelated raw tracks. Reopening that downloaded
song and exporting again passed the same independent inspection and produced
byte-identical MIDI. This qualifies the exercised hosted controls and file
workflow; it does not establish target-game difficulty interpretation.

## Pro Guitar strings and frets (2026-11-08)

The shared editor supports separate 17- and 22-fret Pro Guitar parts at Easy,
Medium, Hard and Expert. Six independent string rows retain fret numbers and
MIDI articulation channels. Fret 0 means an open string. Forced HOPO uses a
separate attack marker so the sixth string remains a playable note. The
inspector exposes fret, articulation and forced HOPO; the roll and six-string
preview show numeric frets. The preview does not judge fingering, scoring or
controller playability, and does not infer natural HOPO from fret distance.

Ordinary MIDI uses `PART REAL_GUITAR` / `PART REAL_GUITAR_22`, difficulty bases
24/48/72/96, six string keys and velocity 100 plus fret. Forced HOPO uses the
following key. Export retains original note-off attributes and unmanaged
controls; unrelated original tracks remain byte-identical. Invalid fret
velocities refuse import without altering the original file. Pro Guitar uses
MIDI export; `.chart` and Clone Hero compatibility choices refuse these parts.
Automatic generation, difficulty reduction, tuning editing, chord-name and
arpeggio authoring are unavailable. Their original source data stays retained.
Pro Bass authoring is described below; the original Pro Guitar checks retained
it as an untouched source track.

Actual static-browser checks on `bed0731` imported all eight difficulty/profile
combinations, edited frets, articulation and forced HOPO, moved a note, copied
and pasted it, deleted it and exercised Undo/Redo. Reload retained the saved
notes and history. The downloaded playable MIDI passed an independent Rust
inspection of every difficulty, edited fret/channel/HOPO and note-off velocity;
the original PCM and unrelated Pro Bass raw track were unchanged. Fresh import
of only that MIDI, INI and recording retained the edits without YACE sidecars.
The complete project download passed archive checks.

The `30d767b` browser also created all eight parts from a blank chart, authored
six strings with frets 0, 3, 7, 12, 17 and 22 and several articulations, and
retained them through reload. A visual correction keeps two-digit fret labels
visible above short-sustain resize handles.

The actual native `30d767b` app exercised the same 17-/22-fret edits, Muted and
Pinch harmonic channels, forced HOPO removal, one-tick movement, clipboard,
deletion and Undo/Redo. Full process exit and automatic restart recovered the
acknowledged revision 20 project byte for byte with its seven-note part and
eight Undo checkpoints. Its subsequent MIDI export was byte-identical to the
browser export (`79896ea759f587e840a6a28249182641e21dcc3bca6050a13ebc7da5668fe566`)
and passed the same independent MIDI and original-audio inspection. Opening
the browser's complete project ZIP through the normal native picker recovered
revision 24 and its source/history; native Undo and Redo retained the edited
frets and articulation. Native blank creation added both profiles, pasted a
typed 17-fret note into the empty 17-fret part and edited it to fret 17. Pasting
that clipboard into the separate 22-fret part refused without changing it.
Direct native piano-roll placement and independent target-game/controller and
musical acceptance remain open.

PR119 merged as `b241cfe` and its Pages deployment completed successfully.
The live custom-domain module and served WASM matched the tested artifact
(`5883eacd594c1b5aca3f0de446b089ea6340b88086c84a3de2340080fce7b14f`).
The actual hosted editor reopened the complete backup, changed fret 21 to 22
and retained it after reload. Undo restored fret 21. A second tab's stale
tick/fret edit was refused with recovery guidance; its downloaded recovery ZIP
passed archive checks and retained tick 6601, fret 22, articulation and history.
The authoritative project reloaded with tick 6600 and fret 21. Its downloaded
MIDI matched the native and static-browser hash above. The original hosted
project was restored after verification; temporary tabs and previews were closed.

## Pro Bass strings and frets (2026-11-08)

The shared Rust editor adds separate 17- and 22-fret Pro Bass parts at Easy,
Medium, Hard and Expert. Bass has its own track names, instrument routing,
difficulty ranks and typed clipboard families. Its six MIDI string rows share
the fret, articulation and explicit HOPO controls described above. Ordinary
exports use `PART REAL_BASS` / `PART REAL_BASS_22`, difficulty bases 24/48/72/96
and velocity 100 plus fret. Original phrases, note-off attributes, unmanaged
controls and unrelated MIDI chunks stay retained. Older opaque imports refuse
blank replacement; importing the original MIDI as a new project makes it
editable. `.chart`, Clone Hero conversion, automatic generation and reduction
remain unavailable. Tuning, chord-name and arpeggio editing remain unavailable.

Actual native and static-browser `14e9941` checks imported all eight profiles,
edited frets and articulation, removed forced HOPO, moved and copied notes,
deleted and recovered a note, and edited and undid a Star Power phrase. An
initial native check exposed a missing preview track-name case; the corrected
build renders the Bass preview. Full native process exit and automatic restart
retained revision 18 byte for byte, including eight Undo checkpoints and one
Redo checkpoint. Browser reload retained the same editing content and history.

The actual native and downloaded browser MIDI exports were byte-identical
(`fdb88a418cffd9992af23576b426422061f24f9839cd1dfff6e65427efced473`). An
independent Rust inspection checked all eight parts, string/fret/channel values,
retained note-off velocities, HOPO changes, Star Power and solo timing, unknown
text/controller events, exact unrelated MIDI chunks and original recording.
Fresh browser import of only the exported MIDI, INI and audio retained both
edited profiles. The downloaded complete project ZIP passed archive checks.

The browser also created all eight Bass profiles from a blank chart and placed
all six strings in the 22-fret Expert part with frets 0, 3, 7, 12, 17 and 22.
Articulation and forced HOPO survived reload at revision 47 and independently
checked MIDI export. Pasting a 22-fret clipboard into the separate 17-fret part
refused without changing revision 40. The normal native project ZIP picker
restored the browser's complete revision 20 backup, including source and history;
native Undo at revision 21 and Redo at 22 restored matching editing content.
Native blank creation added separate 17- and 22-fret Expert parts at revision 2.
Direct native pointer placement remains unqualified because the desktop tool
could not target the piano roll. Independent target-game/controller and musical
acceptance remain open.

PR122 normally merged as `0fabf13`, including the concurrent browser project
lifetime fix. Pages run `42545458799` completed all three jobs successfully.
The actual custom-domain module loaded that immutable release and the served
WASM matched the rebuilt artifact
(`24fc94f6c5fe860a3c030ab3223f309be77538350abef0f424c31ea8313da268`).
The hosted editor opened the complete backup, changed fret 21 to 22, saved
revision 23 and retained it after reload. Undo restored fret 21 at accepted
revision 24. A second tab's tick 6601/fret 22 draft was refused with recovery
guidance. Its actual recovery ZIP passed archive checks and retained revision
24, ten Undo checkpoints, fret/channel/note-off attributes and no missing source
or history entries. After browser control became unavailable, the user confirmed
the final authoritative reload retained tick 6600/fret 21 and restored the
original hosted project. A fresh playable export from this hosted revision
remains an open check; the native/static MIDI comparison above is separate.

## Five-pad drum authoring (2026-11-08)

Source revision `80bc192` retains the song's drum profile before validating saved
selections. Explicit `five_lane_drums` metadata takes precedence; otherwise a
fifth-pad attack in a drum track identifies five-pad input. Five-pad lane 4 is
Orange and lane 5 is Green. Four-pad lane 4 remains Green, and ordinary four-pad
project serialization is unchanged. Deleting or moving the last fifth-pad hit
does not change the retained profile. Four-pad cymbal flags remain separate
from five-pad Yellow/Orange cymbal interpretation.

The actual static browser exposed all seven attacks in a ten-second controlled
fixture, including both kick rows. Its fifth-pad Ghost edit, one-tick move,
typed clipboard copy/paste, deletion and Undo/Redo survived reload. Moving and
deleting the last Green attack retained five-pad lane controls and all five
preview receptors. Pasting that clipboard into a four-pad project was refused
without changing its three notes or saved revision. A downloaded complete
project reopened through the normal ZIP picker with its five-pad profile and
Redo history; Redo removed the last Green attack and Undo restored it.

A `.chart` export with no remaining fifth-pad attacks still wrote
`five_lane_drums=true`; importing that exact browser download through the native
CLI retained the five-pad profile. Undo restored a seven-hit playable export
whose `notes.chart` was byte-identical to the original. Explicit MIDI export
refused safely, leaving the saved chart intact. Five-pad MIDI conversion,
generation replacement and automatic difficulty reduction remain unsupported
and are explicitly refused.

The actual native app exercised Green Ghost dynamics, a one-tick move, typed
Copy/Paste and deletion. Moving the last Green hit to Orange retained the profile
after full process exit and automatic restart. Deleting the last Green hit and
restarting recovered the exact acknowledged six-note save, all five pad rows and
its Undo/Redo history. Undo restored the original seven hits; the native playable
`.chart` export matched both the source and browser export bytes. The browser's
complete backup reopened through the native ZIP picker with its original audio,
five-pad profile and Redo history; native Redo removed Green and Undo restored it.
Opening the browser's no-Green playable copy also retained Orange/Green lane
semantics. Native explicit MIDI export refused safely without publishing a folder
or changing the project. These checks used the same `80bc192` native runtime;
later acceptance documentation did not change its implementation.

The deployed `536f22d` browser build also passed fifth-pad Ghost/tick edits,
typed clipboard copy/paste/delete and reload with the final Green hit moved to
Orange. Undo restored the original seven notes, and its downloaded YARG `.chart`
copy passed archive checks with the same original chart bytes and five-pad INI
profile. The original hosted project was restored after verification.
Independently authored Moonscraper input, acoustic and target-game acceptance
remain open; these controlled checks do not establish unsupported conversions
or generators.

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
native build completed a 243.880-second recording in both combined and focused
vocal views. A double-kick edit, clipboard paste, deletion and Undo/Redo worked
in the full-song chart. A compact backup passed archive checks; quitting,
restarting and reopening the project retained the accepted edit and Undo/Redo.
The edited playable export reopened with the added double kick. That earlier
build first restored an existing Dance session; the separate native startup
checks below qualify most-recent-workspace recovery. Eleven articulation
regressions cover the core rules, including unrelated marker preservation.

Native vocal previews now carry the recording and revision metadata expected by
the display clock. A preview without that identity can still deliver clock ticks;
its timing samples are excluded from recording-specific measurements. Previously,
the first vocal batch in an all-parts view could stop the refresh task and leave
the preview on its fallback timer. Actual combined and focused-vocal runs
reached the recording's natural end without the error and produced fresh,
recording-bound frame samples while playing. These muted transport checks do
not establish acoustic latency, sustained frame performance or musical quality.

The native mixed-part export displayed 34 distinct choices. Omitting only
Expert six-fret bass produced the same MIDI as the previously inspected browser
selection, preserving its lower difficulties and the other families. Native
export failures now appear once inside the dialog, with an enabled retry and a
terminal failure status. Closing and reopening retains the relevant error;
retrying clears only the previous export failure. Cancelling the native file
picker ends with a cancellation status. Actual native checks refused a source
folder destination, cancelled a retry and then exported all 34 parts to a valid
destination. Independent MIDI inspection retained every instrument family,
technique and phrase. Browser inline export feedback is verified separately below.

Rejected inspector edits now restore the accepted value and keyboard focus.
Shared Rust controls remount only the refused field; accepted edits retain their
element identity. Actual browser checks repeated lane and tick collisions,
then accepted a keyboard step and exercised Undo/Redo. A full recording reached
its natural end with a temporary drum edit, and undoing the edits exported the
original chart byte for byte. Native checks covered the same collisions and
refused dynamics/cymbal changes during a failed write, retaining the saved file;
restoring write access allowed a retry and Undo. Its restored MIDI export also
matched the original. These checks do not establish musical or game acceptance.

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
instrument. Unknown source identifiers remain visible. Rhythm and co-op guitar
now have manual authoring controls at all four difficulties. Six-fret authoring
has implementation coverage for all five families and four difficulties, with
white/black/open rows, a distinct clipboard family and fresh MIDI decoding.
Static browser checks exercised all 20 parts through individual Black3 timing
and Tap edits, typed clipboard paste/delete, Undo/Redo and reload. An independent
MIDI audit verified the exported edits, white/black/open attacks, sustains,
scoped techniques and Star Power, with seven unrelated raw track chunks
unchanged. Reimporting only MIDI, INI and audio exposed the same 20 edited
parts. A complete editable backup retained revision 95 and 42 Undo entries.
Blank-chart checks created all 20 parts; lane and sustain edits, independent
part/note Undo/Redo, reload and all 20 distinct `.chart` sections passed.
Older opaque MIDI imports refused all 20 conflicting blank parts without
changing the saved revision. Their automatic promotion remains unimplemented.
Native checks independently edited all 20 parts and created all 20 blank
parts through the actual controls. White/open lane and sustain edits,
clipboard-family refusal and independent part/note Undo/Redo passed. The
native blank `.chart` export matched the browser byte-for-byte. A full process
exit/restart retained the acknowledged project bytes and recovered Undo/Redo;
its edited MIDI and original audio matched the browser export exactly. The
browser's complete backup reopened natively with all 20 parts and 42 Undo
entries, including usable recovered note history. Hosted checks imported ordinary
MIDI and exercised all 20 timing controls, typed paste/delete and Undo/Redo,
save/reload and the paired preview. Its downloaded MIDI matched native and
local-browser exports exactly, including unchanged unrelated track chunks.
Independent released-Moonscraper and game/controller/musical acceptance remain
open.

Native and static browser checks created all eight rhythm/co-op parts from a
blank chart, entered typed five-fret notes and independently undid/redid part
creation and note entry. Duplicate parts were disabled. Reload retained the
browser edits. Both clients exported the same eight distinct `.chart` sections,
including Tap markers and 240-tick sustains.

An existing mixed chart and an ordinary MIDI-only reimport exposed all eight
parts in both clients. Actual note controls retained each difficulty's distinct
start, sustain, Tap/HOPO/open notes and Star Power. A native timing edit survived
a full process exit and restart with byte-identical saved project data; its
recovered Undo/Redo restored the original timing. The native mixed MIDI export
was byte-identical to the browser export. Independent inspection confirmed ten
unrelated MIDI chunks and the recording were unchanged. Undoing all browser
edits restored the original MIDI exactly.

These roles have no dedicated generator or automatic difficulty links. Older
projects whose rhythm/co-op MIDI was stored only as opaque source chunks retain
lossless export; creating a conflicting blank role is refused. Reimporting the
original MIDI as a new project exposes the editable tracks. Automatic promotion
of those older receipts, target-game/controller judgment and musical acceptance
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

Native startup uses one route for an explicit open,
the newest band or Dance project, or legacy Dance recovery when recent history
is valid and empty. Actual full-feature native debug builds reopened both
families after confirmed process exits. The latest `325ea8f` build recovered an
acknowledged note-position edit with byte-identical project data, selected note
and saved revision intact; Undo/Redo survived restart. Standalone Dance retained
its saved difficulty and Undo/Redo. A missing newest project displayed an error
without opening the older valid project; restoring its folder allowed retry.

Native malformed-history checks also refused automatic fallback. Explicitly
opening either a band or Dance project then succeeded with a visible warning
while retaining the unreadable history byte-for-byte. Previously, an explicit
open could replace that file with a new one-entry list. Both clients now use
the same strict read-before-write history update. Native repair/retry and the
static browser's ordinary saved-edit/reload/Undo paths were exercised; actual
browser storage-failure injection remains unqualified. Explicit command-line
startup priority has routing-test coverage. These controlled fixtures do not
establish acoustic, target-game or additional platform acceptance.

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

Stale-save recovery instructions now direct users to download a recovery copy,
open YACE in a new tab and use **Saved in this browser**, keeping the rejected
tab open until its copy is safe. Previously the warning suggested reopening the
saved project in the same tab; that action retried the rejected save and was
blocked. A controlled two-tab band edit retained the acknowledged note at tick
2 and the rejected draft at tick 1 in separate CRC-checked downloads. A fresh
tab reopened tick 2 and retained Undo/Redo. This guidance does not discard the
rejected draft or bypass revision checks. Browser termination, quota and
eviction acceptance remain open.

The rejected browser draft's recovery ZIP was also opened in the native app.
It retained its tick-1 note, Tap technique, 240-tick sustain and four Undo
checkpoints. Native Undo returned to tick 2; Redo restored tick 1 with identical
editing content. After an acknowledged save, full Quit and automatic startup
recovered the same project bytes and revision. This was a controlled chart-only
fixture; it does not qualify untested storage faults or acoustic behavior.

The hosted browser repeated the conflict with its accepted note at tick 3 and
rejected draft at tick 1. Reload retained the accepted edit. The actual recovery
download passed every ZIP CRC check and retained the rejected note and four
Undo checkpoints; a fresh tab's saved-project picker reopened the accepted copy,
with Undo returning to tick 2 and Redo restoring tick 3. The served module and
WASM matched the published artifact, and the original workspace was restored.

On 2026-11-09, an isolated Edge profile repeated this workflow against hosted
source `f201484e`. A new blank chart saved Green at tick 480. One tab saved tick
481; a stale tab's tick-479 edit was refused with the recovery instructions.
Reload retained tick 481. Undo returned to 480 and Redo restored 481, which
survived another reload. The downloaded recovery ZIP retained the refused
tick-479 draft and two Undo checkpoints; the independent complete backup
retained the accepted tick-481 revision. Both downloads passed every ZIP CRC
check. After forcibly terminating only that verification browser process,
restarting its profile automatically reopened the accepted revision and note.
This qualifies that controlled process-exit case; quota, eviction and other
storage-failure cases still need acceptance evidence.

The deployed `95724ec0` runtime repeated this controlled recovery case with
vocal edits. One tab acknowledged tick 961 at revision 6; the other tab's
tick-959 draft was refused and remained downloadable. Independent inspection
verified the rejected draft, recovery report and checkpoint hashes. Reload and
a forced exit of only the isolated verification browser retained the accepted
tick-961 edit. Complete backups downloaded before and after restarting that
profile passed all 20 manifest size/hash references and contained byte-identical
project JSON, including revision 6 and three Undo checkpoints. This qualifies
that controlled process-exit case, with the other fault gates still open.

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

### Static browser playback regression checks (2026-11-03)

Original/Complete mix previously started every attached media decoder, even when
all stems had zero output gain. A nine-channel synthetic fixture now runs only
the recording in that mode; solo audition runs the recording clock plus the
selected stem. Muted channels finish a 10 ms gain ramp before pausing and rejoin
at the current recording position when enabled. Ordinary editor actions no
longer reset an unchanged practice rate or its drift controller. Replaced
channels disconnect their audio nodes and release their decoder resources;
cancelled channel starts cannot fail a later playback request.

Runtime mixing now retains settings for the channels actually decoded, including
when a separate keyboard set is selected or optional stems are missing. An empty
set falls back to the recording. Strict project/action mix validation is
unchanged. Asset availability changes also refresh the static audio graph.

Actual static-browser checks exercised Guitar, Bass, Drums, Keys, Pro Keys and
Vocals while looping, solo/backing switches, mute/rejoin, seeking, pause, and
25/83/100/150/200-percent rate changes. Reloading the isolated fixture restored
83 percent on all nine paused media elements. Five focused Rust regressions
cover channel activity/fade cancellation and partial/empty runtime mixes. Preview
listening reported an improvement. These checks do not establish sample-accurate
acoustic alignment or artifact-free stretching on every browser/output device.
