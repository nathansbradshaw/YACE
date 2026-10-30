# Edit and export

## Open a project

- **Import audio** generates a new draft from a recording.
- **Open song folder** copies an existing chart into a new review project.
- **Open saved project** continues a YACE project.

Keep the whole review-project folder. It contains the source copy, saved edits
and undo history. YACE saves chart edits automatically after each successful
change; it does not edit the original song folder.

## Edit notes

Choose an instrument and difficulty from **Track** beside the editing tools.
Audition, selection details and review follow that track.

- **Select**: click a note, or drag it to change its time or lane.
- **Place**: click an empty lane to add a note.
- **Erase**: click a note to remove it.
- **Snap**: choose a grid division, or turn snapping off for precise placement.
- Select a note to edit its position, duration, lane and supported flags in the selection desk.
- Drag a sustain's end handle to change its length. Drum notes do not sustain.

Use **Difficulty links** in the selection desk to preview a reduction from Expert.
Confirm the replacement before linking an existing lower difficulty. Local edits
to a linked row become overrides; Undo restores the earlier state.

See [Piano](PIANO.md) and [Vocals](VOCALS.md) for their dedicated controls.

## Review a section

Select a flag in the **Review queue** or on the timeline. Listen around it,
edit the notes, then approve it or record another decision. Hiding a flag does
not approve the notes. Editing relevant notes makes an earlier decision stale.
Unflagged parts still need listening review.

## Keyboard and layout

| Action | Shortcut |
| --- | --- |
| Play or pause | Space |
| Select / Place / Erase | 1 / 2 / 3 |
| Delete selected note | Delete or Backspace |
| Undo | Command/Ctrl+Z |
| Redo | Command/Ctrl+Shift+Z |

Text fields keep their normal typing controls. Escape closes a blocking dialog.
Drag a panel divider to resize it, or focus it with Tab and use arrow keys.
Shift makes larger steps; Home and End select the size limits.

## Export

Use **Export** and choose a new song-folder path. Export is separate from
project autosave. Keep your review project for further editing and recovery.

Open the exported folder in Clone Hero, YARG or another editor and check the
parts you intend to use. See [Review a draft](CHARTING_STANDARD.md).

The command-line editor uses the same saved actions:

```sh
chart-engine editor --help
```
