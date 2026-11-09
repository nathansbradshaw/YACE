# Chart in a browser

The browser app is a static Dioxus Web application compiled to WebAssembly.
Rust runs in the browser: chart state, validation, editing, undo, review,
collaboration files, audio decoding, playback control, ZIPs and recovery storage.
Deploy the contents of `web/dist` to an ordinary static HTTPS host. No application
server, account, WebSocket, FFmpeg process or generation service is needed.

## Build and run

Install the Rust target, matching official binding generator and Binaryen 132
(`wasm-opt`) once:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
wasm-opt --version # wasm-opt version 132
```

From the repository root:

```sh
cargo run --locked --manifest-path web/tools/Cargo.toml -- build
cargo run --locked --manifest-path web/tools/Cargo.toml -- serve 3030
```

Open `http://127.0.0.1:3030/`. The small Rust server serves static bytes for local
testing; any static host with `application/wasm` support can replace it. Keep the
generated `snippets` directory with the JS/WASM files. Relative asset URLs also
support hosting the app below a directory. HTTPS or loopback is required for
browser storage and clipboard APIs. A desktop shell is not required.

For production updates, use the immutable-release staging command documented
in [DEPLOYMENT.md](DEPLOYMENT.md). To preview a staged hosting directory on the
existing local port, use `yace-static-web serve 3032 /path/to/hosting/dist`.

Preview ports are limited to **3030 (primary)** and **3032 (secondary)**. Both
launchers reject other ports. Check listener ownership before starting; reuse,
coordinate a handoff, or wait when an allowed port is occupied. Never select a
third port. `yace-static-web serve 3030 --check` reports the selected source and
served directory without opening a listener. Stop task-owned previews after
verification and confirm their listeners have closed. Preserve active owners.

The Rust build tool optimizes the generated module with the pinned Binaryen
release and rejects an artifact above the static host's 25 MiB per-file limit.
No additional JavaScript loader or application service is required.

The hand-written JavaScript imports and starts the WASM module, displays startup
failures and starts the module worker. Dioxus and wasm-bindgen generate
the DOM/browser glue. Application logic stays in Rust, with `web-sys` for browser
APIs. The static crate imports the shared native Rust editor and chart modules.

To exercise folder saves with real browser filesystem handles, first check that
port 3032 is free, then run:

```sh
NO_HEADLESS=1 WASM_BINDGEN_TEST_ADDRESS=127.0.0.1:3032 \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
cargo test --locked --manifest-path web/Cargo.toml --target wasm32-unknown-unknown \
  --features browser-tests --test project_folder
```

Open the runner's local URL in a supported browser. The tests use isolated OPFS
folders and remove their fixtures. They cover source preservation, chunked
writes, undo history, external-edit conflicts, and retrying a failed write.
Stop the runner after verification.

## Open, edit, save and export

New blank projects and projects created from a recording ask **Where should this
project be saved?** before enabling **Create chart**. In Chrome and Edge, select
an empty project folder for direct autosave and desktop interoperability. The
initial document and source are written before the new project replaces the
editor. Canceling folder selection leaves the destination unchosen; a failed
folder save stays in setup with an error and never falls back to browser storage.
Alternatively, explicitly choose **Use browser storage**. Firefox cannot select
a writable desktop folder: setup explains this limitation and the complete
project ZIP workflow before requiring that explicit browser-storage choice.
Desktop continues to ask for a project folder in its native creation save dialog.

- **New blank chart** creates a manual chart without audio. Its local timeline
  supports playback, seeking and practice speed. **Attach recording** adds a
  recording covering the authored events without removing edits; Undo returns
  to the chart-only state. Audio tools follow this transition. Project ZIPs
  preserve the attachment history. Chart-only game exports contain no playable
  root recording; retained source evidence can remain in private recovery paths.
- **New chart from audio** opens the native file picker, then asks for title,
  artist and starting tempo. Recording tags and embedded covers populate editable
  album, year and genre fields. Choose a local cover or explicitly look up matching
  release information and cover art; lookup shares metadata, never audio.
  Local audio analysis proposes tempo changes across
  the recording; **Use detected tempo map** is selected when a pulse is measured.
  Edit the starting BPM or clear this option for constant timing. Review the
  detected markers with **Timing**, especially the first beat and half/double-time
  choices. It creates an empty Guitar chart for manual
  authoring; add other parts with **Add a part**. No generation runs.
- **Import song ZIP** opens a packaged chart and its recording. Generated MIDI
  songs require their matching editable-master binding, as on desktop.
  **Open charting folder** uses the operating system's folder picker, finds charts
  up to three folders deep and exposes a chart switcher. Separate editing copies
  preserve source files and distinguish same-named songs. Discovery keeps local
  file handles and reads small INI files; recording/stem bytes load only for the
  selected song and are released from the collection snapshot after opening.
  A selected song is bounded to 16,384 files and 2 GiB, independently of the
  collection's total recording size (up to 100,000 selected file handles).
  **Open project ZIP** opens a fresh copy of the selected project with its
  review and undo history.
- In a browser that supports writable folder access, **Open project folder**
  opens a desktop YACE project in place, including a song folder that contains
  `.yace/project`. **Save to project folder** puts a browser project into an
  empty folder you choose. Edits then autosave to that folder. Close the project
  in one editor before editing it in the other. Reopen the folder from the
  picker after a browser reload.
- Pick tracks, select/place/erase notes, edit timing and song sections, work with
  difficulty links, author vocals, and undo/redo using the shared Rust commands.
- Play the recording, seek, loop and change pitch-preserving practice speed.
  Rust codecs decode supported recordings to a local playback buffer and build
  waveforms. Formats such as Opus use the browser's local decoder as a fallback,
  with a bounded wait and a visible error if neither decoder accepts the file.
  The original encoded recording is preserved in exports.
- **Download complete project…** in the YACE menu prepares one editable ZIP
  with source files, exact available stems, checkpoints and review history. Open
  it with **Open project ZIP** in the browser or **Import project ZIP** on
  desktop. Backups store duplicate files once. Use **Prepare smaller project
  backup…** under Tools to choose 24-bit listening copies or omit generated
  stems. Existing ordinary project ZIPs still open.
  **Export playable song…** opens the same choices as desktop: target preset,
  automatic / `.chart` / MIDI format, instruments and difficulties, and recording
  and stems included or omitted. All parts are initially selected. Incompatible
  choices are explained before export. The preview estimates source size before
  compression; the ready dialog shows the actual ZIP size. Download the prepared
  ZIP using its explicit download button. A game
  export does not replace a project backup.
  Backup and game-export preparation run on Rust workers so playback can continue.

Files stay on the user's computer. Browser file inputs open the native picker;
there is no custom workspace file browser and no upload endpoint. ZIP imports
reject unsafe paths, symbolic links, duplicates and excessive expanded sizes.
ZIP selection allows up to 16 GiB encoded. A dedicated worker seeks local Blob
slices through a 1 MiB cache, inflates one entry at a time and transfers each
completed file to the editor. The complete encoded ZIP is never copied into
WASM. Expansion is bounded to 32 GiB of logical entries, 2 GiB of unique data
and 1 GiB per file in the browser. Duplicate data shares one immutable
allocation. This is incremental archive reading, not unlimited disk-backed
editing: the open project's unique files still occupy memory, and decoding and
analysis need additional working space. Smaller devices can reach their memory
limits earlier. Exports write bounded browser chunks instead of growing one
large WASM archive buffer. Desktop import also seeks the original ZIP and
shares duplicate allocations while restoring files; its per-file limit is 2 GiB.
Open charting folder reads a snapshot. Open project folder uses ongoing disk access.

IndexedDB stores the current project, source files and checkpoints in a single
transaction. The status says **Saved in this browser** only after that transaction
commits. Reopening the site restores the last saved project; **Saved in this
browser** reopens other saved projects without loading their audio first. A
chart switch commits the current working copy before releasing its memory.
Download project backups for independent copies: browser storage belongs to the
current site/profile on this device, does not sync between browsers and may be
removed by clearing site data. Backups include the complete project regardless
of playable-song export selections. A revision check rejects stale writes from
another tab instead of silently overwriting that tab's edits.

The open band project's working files stay owned by the editor until another
project opens successfully. Import and generation cleanup cannot release that
active folder. Before saving, the browser checks that the document, source chart,
recording and referenced undo/redo checkpoints are present, and captures the files
before awaiting IndexedDB. An incomplete working set is refused without replacing
the previous durable manifest. This protects against missing-file saves; it does
not establish the cause of every browser storage failure.

Project opens also recheck their request identity after their final save wait.
An older reopen can be delayed behind autosave while a newer generated project
opens and retires the old folder. Previously that callback could then publish
its captured document and media against the removed folder. A superseded open
now returns an error before replacing the current editor. Save acknowledgments
also verify the current folder identity, not just a matching revision number.
Pending opens hold a counted lease on their working folder. Cleanup is deferred
until that folder has neither an active editor nor an in-flight reader, including
when an older canceled open shares a folder with a newer open.

The opt-in `lifetime-fixture` build seeds a four-second synthetic completed vocal
job and provides a controlled scheduler regression in
`web/tests/support/project_lifetime.rs`. Open its retained generated draft, then
run **Reproduce overlapping project opens**. It pauses a real reopen at the save
wait, completes a competing real open, then resumes the old callback. Before the
fix, it reproduces missing document/source files; afterward it checks rejection
of the old callback plus the newer recording, vocal notes and undo checkpoints.
**Remove disposable lifecycle fixtures** removes only those named synthetic
snapshots and their workspace pointers, retaining shared content-addressed audio.
This feature is excluded from ordinary application and production builds.

Portable project ZIPs include retained stems and saved kit results. Older
projects with external assets require matching local files. Attach files or a
new source-bound stem manifest with the native picker; checks cover hashes,
format, exact decoded frame counts and source identity. Missing stems are
reported, and the browser uses the original recording. HTML audio elements share
an AudioContext gain graph and resynchronize to the original recording's clock;
this does not establish sample-perfect stem synchronization or acoustic latency.

**Part analysis** runs the shared read-only analysis on a local Rust worker.
In the Drums tab, **Kit isolation → Split kit…** opens the operating system's
folder picker for the five optional local LarsNet ONNX files. Their lengths and
checksums must match the desktop pins. Inference runs locally in a worker with
one model session at a time and shares the desktop Rust FFT and reconstruction.
Five full-clock float WAVs and their source-bound receipt are saved with the
project; notes and review decisions remain unchanged. The weights are not bundled
or downloaded automatically. See [DRUM_PARTS.md](DRUM_PARTS.md) for their license.
Existing validated results remain available after reload or project restore.
**Clean audio** accepts
local English word recognition, word timings, saved plans and manual ranges. It auditions the source and renders
a separate float WAV, plan and receipt locally. Download and listen to the
rendered copy before accepting it; clean-panel edits require an explicit saved
plan or export.

## Generation and collaboration

**Generation runs locally in the browser.** The Rust six-stem
separator runs on WebGPU in a dedicated worker. The same pinned instrument
models run through ONNX Runtime WASM, with shared Rust decoding, recovery,
arrangement and export. Downloads are checksum verified and cached locally;
an already-downloaded pinned model can also be selected using the OS picker.
No recording is sent to an inference service. The small upstream ONNX runtime
loader is vendored; YACE's processing and orchestration remain Rust.
Instrument parts, standard Piano/Pro Keys, Synth, vocal pitch, word recognition
and supplied-English-lyric acoustic alignment are available. The optional HQ
piano checkpoint is disabled after failing browser runtime verification;
standard piano remains available. No model service is substituted.
See [actual full-song checks and remaining limits](BROWSER_GENERATORS.md).

**Collaborate** supports the shared proposal/comment/approval/conflict workflow
through downloaded and imported review files against matching source material.
Each contributor edits a separate project. **Share with GitHub** optionally pulls
and publishes the same immutable event files directly through GitHub's API.
Public repositories can be read anonymously; publishing requires an optional
GitHub token with Contents read/write permission for a dedicated review repo.
The token stays in tab memory, outside IndexedDB, project JSON and ZIPs; clear it
with **Clear GitHub authorization**. No token is needed for ordinary charting.
Sync rejects source repositories/unrelated files and validates incoming source
bindings, event checksums and the review graph. Publishing preserves commits,
adds missing events and never forces a branch update. A competing push triggers
a new validated pull. Incoming proposals still require explicit review and
merge. Native Git/shared-folder tools remain available on desktop; choosing a
local browser folder does not establish synchronization.
Review/analysis files also use an explicit prepared-file download button, so
asynchronous work cannot silently lose a download to browser activation rules.
Static hosting does not supply accounts, roles, live shared cursors or a team
server.

## Verification

```sh
cargo test --locked --no-default-features --all-targets
cargo clippy --locked --manifest-path web/Cargo.toml \
  --target wasm32-unknown-unknown -- -D warnings
cargo test --locked --manifest-path web/tools/Cargo.toml
```

See [the full-song acceptance record](BROWSER_ACCEPTANCE.md) for actual browser
results and their limits. This delivery does not establish musical accuracy,
complete OCTAVE/Moonscraper parity, all-browser compatibility or faster total
frame times than another editor.

The older `yace-browser` LiveView executable remains an optional local diagnostic
harness. Its host-side uploads, model runners and WebSocket UI are separate from
the static app and are not required for browser charting.
