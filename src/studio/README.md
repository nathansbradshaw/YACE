# Desktop UI

Start with `mod.rs`: it launches the window, connects the clock and selects the
welcome screen or loaded workbench.

| Area | Files |
| --- | --- |
| Starting screen and toolbar | `welcome.rs`, `toolbar.rs` |
| Panel sizes and keyboard commands | `layout.rs`, `shortcuts.rs` |
| Track navigation and note inspector | `explorer.rs`, `properties.rs` |
| Flag decisions and shared modal behavior | `review.rs`, `modal.rs` |
| Playback and note rendering | `game_preview.rs`, `timeline.rs`, `vocal_preview.rs` |
| Playback controls and stems | `practice.rs`, `stems.rs` |
| Vocals, linked difficulties and part analysis | `vocal_editor.rs`, `difficulty.rs`, `part_analysis.rs` |
| Generation, clean audio and collaboration | `generation.rs`, `clean.rs`, `collaboration.rs` |
| Optional assistant | `assistant.rs` |
| Shared state, audio and live inspection | `state.rs` |
| Timing and desktop diagnostics | `presentation_clock.rs`, `frame_pacing.rs`, `diagnostics.rs` |

Views send actions through `Studio::dispatch`. Keep project validation, file
formats and saved edits in the shared editor modules so the GUI and CLI agree.
Long-running jobs belong outside render callbacks.

Use `Modal` for blocking dialogs. It uses the WebView's native modal layer for
focus containment and Escape. Closing a dialog retains its job state; it does
not cancel work unless the user selects Cancel.

Styles live in `assets/`. `identity.css` defines the YACE theme; `workbench.css`
covers welcome, focus and modal behavior. Feature styles have matching names,
including `generation.css`, `clean.css` and `game-preview.css`.
Keep the licensed upstream files in `assets/octave/` unchanged; add overrides
outside that directory. See [stylesheet attribution](../../assets/octave/README.md).
