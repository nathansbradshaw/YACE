# Charts without a recording

Choose **New blank chart** on the welcome screen or in the project menu. Set the title, artist, tempo and timeline length. This creates a manual chart without transcribing or inventing audio. Add other supported parts through the editor's existing part controls.

You can also open a folder or song ZIP containing `notes.chart` or supported game MIDI without a recording. The timeline uses the declared song length or the chart's timed events. It grows when new notes extend beyond its current end. **Timeline (s)** changes its length explicitly; shortening cannot hide authored events. Timeline changes support Undo and Redo.

Play, pause, seek, looping and practice speed use a clock without an audio output device. There is no waveform or recorded sound until you attach audio. Cleaning and source-attack analysis require a recording.

Use **Attach recording** to choose local audio. The recording must cover the chart's timed events. Attachment keeps note edits, is undoable, and remains part of portable project recovery. A rejected short or damaged recording does not advance the project revision.

Chart-only song exports contain chart data and metadata without a fabricated audio file. After attachment, song exports include the selected recording. Keep a project ZIP to retain the complete editing history.

Explicit recording context is new project data: use an updated YACE build to reopen these projects. Existing recorded projects and their checkpoint encoding remain supported. The optional legacy live-view client supports editing, but chart-only playback currently requires the desktop app or static browser client.
