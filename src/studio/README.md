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
Use graphite faces, cream transport keys and thin section dividers consistently.
`surfaces.css` adds subtle contact wear to the project bar edge and transport keys.
Keep labels, panel faces, ordinary controls and editing areas free of texture.
See [third-party notices](../../THIRD_PARTY.md) for component ancestry.

The track picker drives editing, audition and review together. Selection details
follow the current note; playback controls stay with the audition view. Keep lane
heights in sync with the pointer coordinates in `note_canvas.rs`.
