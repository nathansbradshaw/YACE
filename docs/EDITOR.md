# Edit and export

These controls also run in the [browser editor](BROWSER.md), using the same
Rust project, command validation and recovery history.

## Open a project

- **Import audio** generates a new draft from a recording.
- **Open song folder** copies an existing chart into a new review project.
- **Open saved project** continues a YACE project.

Keep the whole review-project folder. It contains the source copy, saved edits
and undo history. YACE saves chart edits automatically after each successful
change; it does not edit the original song folder.

## Edit notes

Use **Add part** beside **Track** to add guitar, bass, drums, Keys, Pro Keys or
vocals. Choose **Blank** to place notes yourself, or **Generate…** to make a
draft from the current recording. Generated drafts keep the current tempo map
and require **Add** or **Replace** before changing the chart. Undo restores
the previous part.

Choose an instrument and difficulty from **Track** beside the editing tools.
Audition, selection details and review follow that track.

- **Select**: click a note, or drag across empty space to select a group.
- **Shift-click** adds or removes a note. Drag a selected note to move the group.
- **Copy / Paste**: use the buttons beside Undo, or the shortcuts below. Paste starts at the playhead.
- **Place**: click an empty lane to add a note.
- **Erase**: click a note to remove it.
- **Snap**: choose a grid division, or turn snapping off for precise placement.
- Select a note to edit its position, duration, lane and supported flags in the selection desk.
- Drag a sustain's end handle to change its length. Drum notes do not sustain.

Use **Difficulty links** in the selection desk to preview a reduction from Expert.
Confirm the replacement before linking an existing lower difficulty. Local edits
to a linked row become overrides; Undo restores the earlier state.

See [Piano](PIANO.md) and [Vocals](VOCALS.md) for their dedicated controls.

## Song sections

Use **Sections** above the time ruler to add a name at the snapped playhead.
Click a marker to jump there. In the section list, use **Loop** to repeat it
through the next marker or the end of the song.

Use **Edit** to rename a section or change its start tick. Delete removes the
marker; Undo restores it. Section edits keep notes and the recording in place.
Names are saved in chart and MIDI exports. Note proposals do not include section
edits; share the saved project when those changes need to travel together.

## Tempo and time signatures

Open **Timing** above the ruler. Select a marker to edit it, or choose a type
and **Add**. **At playhead** uses the current snap setting.

Tempo changes keep notes on their beats and change when they play. Playback
pauses so you can check the alignment. Time signatures change bar lines and
numbers. The first tempo and time signature stay at tick 0.

Each change is one Undo step and appears in chart and MIDI exports. Tempo
changes make earlier note reviews stale; earlier vocal checks stay in history.
Review the parts again. Share the saved project to include timing changes;
note proposals cannot carry them.

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
| Select all notes in the current track | Command/Ctrl+A |
| Copy / Cut / Paste notes | Command/Ctrl+C / X / V |
| Clear selection | Escape |
| Delete selected notes | Delete or Backspace |
| Undo | Command/Ctrl+Z |
| Redo | Command/Ctrl+Shift+Z |

Text fields keep their normal typing controls. Escape closes a blocking dialog.
Each group edit is one undo step. Notes keep their spacing, sustains and drum
markers. Paste into the same kind of part; guitar and bass can share notes.
For vocals, select a whole joined word and its continuations together.
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
