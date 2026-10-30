# Chart in a browser

**Current status:** this is a local Dioxus LiveView workflow harness, not a static
website. Rust runs on the host and sends UI updates over a WebSocket. The delivery
target is a static Dioxus Web application compiled to WebAssembly, with chart
state, editing, undo, playback and file import/export running on the client.
Keep JavaScript limited to framework/browser bindings; do not put chart logic in
JavaScript. Native filesystem, subprocess/FFmpeg and inference dependencies need
to be separated from the browser-compatible editor core before that target works.
The LiveView host does not satisfy the static deployment requirement.

YACE's browser host runs the same Rust chart editor, generation jobs and saved
commands as the desktop app. Audio plays on the browser's computer. Open and
Import use the operating system's file picker through standard browser inputs.

```sh
cargo run --locked --release --no-default-features \
  --features browser,separation,vocal-alignment --bin yace-browser -- \
  serve --workspace output/browser-workspace
```

Open the loopback URL printed by the command and leave the host running. Download
models with the existing `models fetch` command first, or enable their download
in generation settings. The workspace holds uploads, songs, jobs, editing
projects and exports. Use `--port` to choose another local port.

Omit `separation,vocal-alignment` for a smaller browser build with manual editing
and the remaining model-free operations. Omitted model backends stay unavailable.
The separate `yace-browser` executable keeps browser builds from replacing the
native engine used to package the canonical YACE app.

## Import, edit and export

- **Import audio / Generate** opens the native file picker immediately. Choose
  a recording from your computer. Generated jobs are stored in the workspace.
- **Create from song folder** opens the native folder picker. **Import song
  ZIP** accepts a packaged song. **Open saved project** accepts a project ZIP.
- Choose a track, play the recording or isolated stems, seek, loop, change
  practice speed, and edit notes in the existing chart and selection desks.
  Undo, revision checks and saved review decisions use the same Rust store.
- **Export** downloads a song ZIP through the browser's normal download flow.
  The browser's download preferences determine whether a destination is prompted.
- **Save project ZIP** downloads the editing project, including stable note
  identities, review history and undo checkpoints. **Open saved project** resumes
  that ZIP. Externally attached stems must be supplied separately. A game export
  is not a project backup.

There is no custom workspace file browser. Files selected locally stream into
the host workspace for Rust processing; exports remain there as recovery copies.
Folder access uploads a snapshot, rather than granting ongoing filesystem access.
Cancelling the OS picker returns to the editor without starting an import.

Uploads stream over HTTP. They do not pass through the UI connection as arrays
of numbers. Exports stream existing compressed music into ZIP storage without
recompressing it. ZIP import validates paths and expanded size and rejects
symbolic links and files that leave the upload directory.

A single browser AudioContext starts all stems on the same clock. Gain changes
keep that cursor. The existing Rust/FFmpeg path prepares pitch-preserving practice
audio. Source-time waveform pyramids remain available at every playback speed.
After browser decoding, the host releases its full-song playback PCM, retaining
waveform detail without a second complete set of decoded stems.

Keyboard defaults are handled in the browser while Rust owns chart operations.
A lost connection stops playback and visibly blocks editing. Reopening loads
the last durable revision; pending commands are not silently replayed.

## Team charting

**Tools → Collaborate** retains proposals, comments, suggested edits, independent
approval and explicit conflict resolution. Review files use native file selection
and normal downloads. Git and live shared-folder controls remain in the desktop
app; uploading a folder in the browser does not establish synchronization.
Each contributor works in a separate project against
matching source material. Single-project locks and stale-revision rejection
remain enabled.

This is a local browser host. It binds only to `127.0.0.1`, requires a fresh
random session credential, sets an HttpOnly/SameSite cookie, checks Host/Origin,
and confines host file access to its workspace. It does not implement hosted accounts,
roles, shared cursors or a remote multi-user service. Exposing it through a tunnel
is outside the supported deployment.

## Measured progress and remaining gaps

User direction, 2026-10-30: combine useful OCTAVE and Moonscraper workflows,
remove measured bottlenecks with Rust, make charting easier, support reliable
teams and do all charting in the browser. A familiar appearance or Rust rewrite
alone does not satisfy that goal. Complete a real song workflow and fix its
friction before expanding feature scope.

Rendering snapshots now share an immutable `Arc<Project>` instead of cloning all
tracks, review evidence and audit history. Selection and transport commands also
skip an unused full-content undo clone. Content edits retain recovery checkpoints.

```sh
cargo run --locked --release --no-default-features \
  --example editor-snapshot-profile -- '/path/to/editing-project'
```

This isolates snapshot creation/release cost. The existing `frame-pacing.json`
measures browser display and chart-position intervals during playback. Neither
metric alone measures acoustic latency, musical accuracy or superiority to
another editor. See [the full-song acceptance record](BROWSER_ACCEPTANCE.md).

Vocal phrase authoring, full game-renderer parity, platform/browser
coverage and hosted team service remain separate gaps. This browser delivery is
not a claim of complete OCTAVE or Moonscraper feature parity.
