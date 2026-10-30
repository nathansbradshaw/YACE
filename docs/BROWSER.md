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

The only hand-written JavaScript in the static entry point imports and starts
the WASM module and displays startup failures. Dioxus and wasm-bindgen generate
the DOM/browser glue. Application logic stays in Rust, with `web-sys` for browser
APIs. The static crate imports the shared native Rust editor and chart modules.

## Open, edit, save and export

- **New chart from audio** opens the native file picker, then asks for title,
  artist and starting tempo. It creates an empty Guitar chart for manual
  authoring; add other parts with **Add a part**. No generation runs.
- **Import song ZIP** opens a packaged `.chart` or MIDI song and its recording.
  **Open song folder** uses the operating system's folder picker.
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
  song ZIP using the same Rust export validation as the desktop app. A game
  export does not replace a project backup.

Files stay on the user's computer. Browser file inputs open the native picker;
there is no custom workspace file browser and no upload endpoint. ZIP imports
reject unsafe paths, symbolic links, duplicates and excessive expanded sizes.
Folder import reads a snapshot; it does not establish ongoing disk access.

IndexedDB stores the current project, source files and checkpoints in a single
transaction. The status says **Saved locally** only after that transaction
commits. Reopening the site restores the last saved project. Export project ZIPs
for independent backups: browser storage belongs to the current site/profile and
may be removed by clearing site data. A revision check rejects stale writes from
another tab instead of silently overwriting that tab's edits.

External stems from desktop projects require matching local files. Attach them
with the native picker; their content hashes must match. Missing stems are
reported, and the browser uses the original recording. HTML audio elements share
an AudioContext gain graph and resynchronize to the original recording's clock;
this does not establish sample-perfect stem synchronization or acoustic latency.

## Generation and collaboration

**Audio-to-chart AI generation is disabled in the static browser build.** The
current model runners require native ONNX/separation/alignment dependencies.
No service is substituted, and no weaker heuristic is presented as generation.
Existing generated projects, evidence and manual editing remain available.

**Collaborate** supports the shared proposal/comment/approval/conflict workflow
through downloaded and imported review files against matching source material.
Each contributor edits a separate project. Git/shared-folder automation stays in
the desktop app; choosing a local folder does not establish synchronization.
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
