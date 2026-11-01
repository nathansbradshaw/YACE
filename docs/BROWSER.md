# Chart in a browser

The browser app is a static Dioxus Web application compiled to WebAssembly.
Rust runs in the browser: chart state, validation, editing, undo, review,
collaboration files, audio decoding, playback control, ZIPs and recovery storage.
Deploy the contents of `web/dist` to an ordinary static HTTPS host. No application
server, account, WebSocket, FFmpeg process or generation service is needed.

## Build and run

Install the Rust target and matching official binding generator once:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127 --locked
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

The hand-written JavaScript imports and starts the WASM module, displays startup
failures and starts the module worker. Dioxus and wasm-bindgen generate
the DOM/browser glue. Application logic stays in Rust, with `web-sys` for browser
APIs. The static crate imports the shared native Rust editor and chart modules.

## Open, edit, save and export

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
  A selected song is bounded to 4096 files and 1 GiB, independently of the
  collection's total recording size (up to 100,000 selected file handles).
  **Open saved project** opens a project ZIP with review and undo history.
- Pick tracks, select/place/erase notes, edit timing and song sections, work with
  difficulty links, author vocals, and undo/redo using the shared Rust commands.
- Play the recording, seek, loop and change pitch-preserving practice speed.
  Rust codecs decode supported recordings to a local playback buffer and build
  waveforms. Formats such as Opus use the browser's local decoder as a fallback,
  with a bounded wait and a visible error if neither decoder accepts the file.
  The original encoded recording is preserved in exports.
- **Save project ZIP** downloads a portable backup with source files, stable note
  identities, checkpoints and review history. **Export** downloads a playable
  song ZIP using the same Rust export validation as the desktop app. Download
  the prepared ZIP using its explicit download button. A game
  export does not replace a project backup.
  Backup and game-export preparation run on Rust workers so playback can continue.

Files stay on the user's computer. Browser file inputs open the native picker;
there is no custom workspace file browser and no upload endpoint. ZIP imports
reject unsafe paths, symbolic links, duplicates and excessive expanded sizes.
Folder import reads a snapshot; it does not establish ongoing disk access.

IndexedDB stores the current project, source files and checkpoints in a single
transaction. The status says **Saved locally** only after that transaction
commits. Reopening the site restores the last saved project; **Saved in this
browser** reopens other saved projects without loading their audio first. A
chart switch commits the current working copy before releasing its memory.
Export project ZIPs
for independent backups: browser storage belongs to the current site/profile and
may be removed by clearing site data. A revision check rejects stale writes from
another tab instead of silently overwriting that tab's edits.

Portable project ZIPs include retained stems and saved kit results. Older
projects with external assets require matching local files. Attach files or a
new source-bound stem manifest with the native picker; checks cover hashes,
format, exact decoded frame counts and source identity. Missing stems are
reported, and the browser uses the original recording. HTML audio elements share
an AudioContext gain graph and resynchronize to the original recording's clock;
this does not establish sample-perfect stem synchronization or acoustic latency.

**Part analysis** runs the shared read-only analysis on a local Rust worker.
Existing validated kit results can be reviewed and auditioned; generating new
separations is being verified separately below. **Clean audio** accepts
word timings, saved plans and manual ranges. It auditions the source and renders
a separate float WAV, plan and receipt locally. Download and listen to the
rendered copy before accepting it; clean-panel edits require an explicit saved
plan or export.

## Generation and collaboration

**Local browser generation is under active verification.** The Rust six-stem
separator runs on WebGPU in a dedicated worker. The same pinned instrument
models run through ONNX Runtime WASM, with shared Rust decoding, recovery,
arrangement and export. Downloads are checksum verified and cached locally;
an already-downloaded pinned model can also be selected using the OS picker.
No recording is sent to an inference service. The small upstream ONNX runtime
loader is vendored; YACE's processing and orchestration remain Rust.
See [generator acceptance and outstanding work](BROWSER_GENERATORS.md).

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
