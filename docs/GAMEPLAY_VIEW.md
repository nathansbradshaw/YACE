# Chart preview

The preview displays the selected instrument and difficulty alongside recording
playback. Use it to inspect patterns; edit the notes in the timeline below.
Choose an instrument and difficulty in the preview header. **Playback options**
contains the stem, loop and view settings.

The five-fret display supports Guitar/Bass, four-pad Drums and five-lane Piano.
Pro Piano and Vocals use their own pitch-based representations. **Pro Drums**
changes the cymbal display. **Look ahead** changes how much upcoming material
is visible without changing the chart.

HOPO and forced-note display uses the supported chart flags and song settings.
The selection desk edits those flags for whole guitar/bass attacks. MIDI export
resolves the chart policy into explicit strum/HOPO markers and difficulty-scoped
Phase Shift tap ranges. Open frets use enhanced-open MIDI notes.
When the policy is unknown, note caps remain neutral. This is a visual editing
preview, with no scoring, controller judgment or microphone gameplay.

Compatibility references are YARG.Core commit
[`3beb94e526558134145bcd3e409428f759001a40`](https://github.com/YARC-Official/YARG.Core/tree/3beb94e526558134145bcd3e409428f759001a40):
`MoonNote.cs`, `SongEntry.IniBase.cs`, `NoteFlagPriority.cs` and the Guitar/Drums
loaders. The preview independently interprets the file-format rules; it does
not include YARG game code, meshes, textures or venue assets.
