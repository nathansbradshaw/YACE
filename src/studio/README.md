# Desktop UI

Start with `mod.rs`: it launches the window, connects the clock and selects the
welcome screen or loaded workbench.

| Area | Files |
| --- | --- |
| Starting screen and project actions | `welcome.rs`, `project_bar.rs` |
| Pane sizes and keyboard commands | `workspace.rs`, `shortcuts.rs` |
| Part creation, track selection and note editing | `parts.rs`, `track_picker.rs`, `chart_desk.rs`, `selection_desk.rs` |
| Flag decisions and tool windows | `review.rs`, `sheet.rs`, `modal.rs` |
| Playback and note rendering | `game_preview.rs`, `note_canvas.rs`, `vocal_preview.rs` |
| Group selection, gestures and clipboard | `selection.rs`, `note_gestures.rs`; shared edits in `../note_groups.rs` |
| Song sections and navigation | `sections.rs`; shared events in `../sections.rs` |
| Playback controls and stems | `practice.rs`, `stems.rs` |
| Vocals, linked difficulties and part analysis | `vocal_editor.rs`, `difficulty.rs`, `part_analysis.rs` |
| Generation, clean audio and collaboration | `generation.rs`, `clean.rs`, `collaboration.rs` |
| Optional assistant | `assistant.rs` |
| Shared state, audio and live inspection | `state.rs` |
| Timing and desktop diagnostics | `presentation_clock.rs`, `frame_pacing.rs`, `diagnostics.rs` |

Views send actions through `Studio::dispatch`. Keep project validation, file
formats and saved edits in the shared editor modules so the GUI and CLI agree.
Long-running jobs belong outside render callbacks.

Use `ToolSheet` for tool windows and `Modal` for other blocking dialogs. They use the WebView's native modal layer for
focus containment and Escape. Closing a dialog retains its job state; it does
not cancel work unless the user selects Cancel.

Styles live in `assets/`. Start with `theme.css` for shared colors and controls,
`workspace.css` for the workbench, `forms.css` for form controls, `timeline.css` for note lanes and `dialogs.css`
for modal windows. `workbench.css` covers the welcome screen; `details.css`
covers vocals and review details. Other feature styles have matching names.
Use black and steel-gray control panels, bright blue selection/primary actions,
green playback/approval, amber warnings and red destructive states. `brand.rs` supplies the original
branching Y mark. Use system sans for controls and tabular monospace readouts.
Keep the start screen to direct project actions, without promotional text or
decorative cards. Keep lane heights and musical colors
unchanged. Scope dark canvas tokens to `.chart-desk-scroll` so review and vocal
details remain readable in either surface. Use semantic status tokens instead of
hardcoded warning/success fills. All visual assets work offline; no photographic
textures or downloaded fonts are loaded.
See [third-party notices](../../THIRD_PARTY.md) for component ancestry.

For a visual check on a disposable project, run
`cargo run --no-default-features --features browser --example studio-preview -- /path/to/project 3050`.
This uses the shared Rust UI and leaves other preview servers running. It writes
normal editor state to the supplied project, so use a test copy.

The track picker drives editing, audition and review together. Selection details
follow the current note; playback controls stay with the audition view. Keep lane
heights in sync with the pointer coordinates in `note_canvas.rs`.
