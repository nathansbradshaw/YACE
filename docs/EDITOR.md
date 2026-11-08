# Edit and export

These controls also run in the [browser editor](BROWSER.md), using the same
Rust project, command validation and recovery history.

## Open a project

- **Import audio** generates a new draft from a recording.
- **Open Recent** on the welcome screen lists your ten most recently opened
  projects, newest first. Desktop entries reopen the saved editing folder;
  static-browser entries reopen the copy saved in this browser. The list survives
  restarting the app. Remove an entry with **×** or use **Clear list** to forget
  the history; both keep the projects and their saved edits. New browser imports
  appear after their first successful autosave.
- Desktop startup reopens the newest entry, whether it is a band project or a
  standalone Dance project. An explicitly requested project or charting folder
  takes priority. If the newest project is missing or history cannot be read,
  YACE reports the problem and keeps the recorded history instead of opening a
  different project. With an empty history, older Dance recovery remains available.
- If recent history cannot be read, explicitly opening a project still works.
  YACE shows a separate warning and keeps the unreadable history file instead
  of replacing it with a new list. Project saves and Undo remain available.
- On desktop, **Open charting folder** accepts a song, a saved project, or an
  entire collection. YACE searches up to three folders below the chosen folder.
  Pick a chart from the welcome list or the **Charts** selector beside the song
  title. Switch charts there without opening another window. Songs with the same
  title show their relative folder paths so you can distinguish them.
- **Open saved project** also finds projects inside a containing folder. Selecting
  a child of a saved project finds the project up to three parents above it.
- In the browser host, **Open song folder** keeps the existing separate-copy flow.

Desktop songs use the usual charting-folder layout: `notes.chart` or `notes.mid`,
`song.ini`, audio, artwork and optional video are together at the top level. This
is the song-folder convention used by [OCTAVE](https://octavestudio.tools/reference/file-formats)
and compatible game editors.

YACE keeps its extra files inside that song's `.yace` folder:

```text
Artist - Song/
  notes.chart             chart, or notes.mid for MIDI exports
  song.ini                song information
  song.ogg                full mix or backing; instrument stems may sit beside it
  album.png               optional artwork
  .yace/
    project/              editing state, source snapshot and undo checkpoints
    jobs/                 generation inputs, intermediate results and worker logs
    audio/                retained stem assets and source-bound manifests
    analysis/             transcription, generation reports and imported information
    review/               review markers, decisions and part origins
    working/              editable masters for game MIDI and vocal documents
    originals/            archived recording and prior game audio channels
    history/              retained earlier analysis and export evidence
    export/               chart and audio routing receipts
    cache/                reusable separation data
```

Switching charts, opening a containing folder, or opening a song directly reuses
the same `.yace/project` editing state. Keep `.yace` when moving or backing up a
song. Earlier collection workspaces in `.yace/projects` and standalone review
projects still open with their saved edits and undo history. YACE does not move
or delete those existing folders.

Editing autosaves YACE's project state. **Export** writes a separate standard song
folder containing the current edits and its own `.yace` evidence. Original chart
and audio files remain available. Copying the whole song with `.yace` preserves
both playable files and the full YACE working project.

Keep the whole review-project folder. It contains the source copy, saved edits
and undo history. YACE saves chart edits automatically after each successful
change; it does not edit the original song folder.

Project ZIP backups retain unknown companion files, including source filenames
ending in `.lock`. Only YACE's known session locks at the project root are omitted.

If the browser reports a missing project file while your chart is still open,
keep the tab open and choose **Download recovery copy…** in the error banner or
YACE project menu. This preserves the committed editor state and every available
workspace file without requiring a successful autosave or normal backup. It
excludes unapplied analysis previews and leaves the original workspace intact.
Download the prepared ZIP before closing the tab. A `yace-recovery/report.json`
inside the ZIP identifies missing source files, checkpoints and stems. A complete
copy opens through **Open saved project…** with its undo history; a partial copy
preserves your edits but needs the missing files from an earlier backup before
it can reopen. Recovery cannot recreate missing audio from a waveform display.

ZIP export refuses filenames it cannot represent faithfully, including literal
backslashes or invalid UTF-8, rather than silently renaming them. The project and
source stay available after a refusal. ZIP imports also refuse backslash entry
names instead of treating them as a different directory structure.

The status bar names the saved editing revision separately from the last song
export. A later content edit marks the song copy as older; seeking or changing
practice speed does not. Undo can restore the editing content that was exported.
Selected-part exports and copies without audio remain labeled as such.

On desktop, a song export is recorded after its separate folder is published.
In the browser, preparing a ZIP and requesting its download are distinct states;
the editor cannot confirm that the browser finished saving a file to disk. A
project backup keeps undo history and this export record. Export status describes
the editing snapshot, not a continuing check that an external copy still exists.

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

For guitar and bass, **Type** offers **Auto**, **Forced** and **Tap**. Forced flips
the automatic strum/HOPO type. Each setting applies to the whole chord, including
when only one gem is selected. A group edit is one Undo step.

The note lanes mark forced attacks with **F** and taps with **T**. Copy, paste and
movement keep those flags. The preview follows the song's HOPO settings; MIDI
exports resolve those settings and keep tap markers separate by difficulty.

Imported five-pad drum charts keep their source layout: lane 4 is Orange and
lane 5 is Green. The preview has five pad receptors. Pad hits support Normal,
Accent and Ghost dynamics, and copying between four-pad and five-pad layouts
is refused. Removing the last Green hit keeps the song's five-pad profile.
Use `.chart` for playable five-pad export; MIDI conversion, generated drum
replacement and automatic difficulty reduction are not supported for this
layout. Four-pad charts retain their existing Green lane and cymbal controls.

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

## Star Power

Choose **Star Power** in the selection desk. Use **Selected notes** to cover a
selection, or **At playhead** to start a four-beat phrase. Adjust the start and
end ticks, then choose **Add**.

Phrases appear in a thin lane above that track's notes. Click one to edit it,
drag its middle to move it, or drag an edge to resize. Phrases cannot overlap.
Each edit is one Undo step. Deleting a phrase keeps its notes.

Star Power belongs to the selected instrument and difficulty. Chart exports
keep those separate; MIDI shares Star Power within an instrument and combines
overlapping difficulty ranges. Note proposals cannot carry phrase changes;
share the saved project to include them.

## Tempo, time signatures and song offset

Open **Timing** above the ruler. Select a marker to edit it, or choose a type
and **Add**. **At playhead** uses the current snap setting.

**Song offset (ms)** shifts chart events against the recording: positive moves
them later, negative moves them earlier. Choose **Apply offset** to replace the
current value. Notes keep their beat positions. Undo restores it, and export
stores it once. See [Song offset](SONG_OFFSET.md) for bounds and format details.

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

Use **Export playable song…** to choose instruments and difficulties, a target
preset, chart format and whether to include recordings and stems. All available
parts and audio are selected by default. Automatic format preserves imported MIDI
and uses MIDI for vocals and Pro Keys. A `.chart` choice refuses incompatible
parts or imported MIDI rather than losing their events. Clone Hero requires
you to deselect vocals, Pro Keys and other original MIDI parts explicitly.
Original MIDI parts without editable views appear as `MIDI: …` entries.

Review the package contents and approximate source size, then choose a new
song-folder path. Source size is measured before compression; generated files
and audio conversion can change the final size. An export without audio needs
matching audio before game playback. Export choices affect the separate copy;
the project and its undo history remain complete.

**YACE menu → Save project backup…** saves the complete editable project ZIP,
including available source assets and recovery history. Backups always retain
the full project regardless of playable-song export selections.

Open the exported folder in Clone Hero, YARG or another editor and check the
parts you intend to use. See [Review a draft](CHARTING_STANDARD.md).

The command-line editor uses the same saved actions:

```sh
chart-engine editor --help
```
