# Listen to stems

An isolated stem estimates one instrument from the recording. It may contain
other instruments, miss notes or change the sound. Keep the original nearby
when deciding what belongs in a chart.

## Choose a mix

Select a track, then use the playback strip:

- **Original** plays the full recording.
- **[Instrument] only** solos the matching stem.
- **With backing** keeps the selected instrument loud and other stems quieter.
- The mixer controls adjust individual stem gain, mute and solo.

All mixes share the same playback position and loop. Changing practice speed
preserves pitch and keeps chart positions tied to the original recording.

## Attach existing stems

Choose **Attach stems** and select a YACE stem manifest. It must match the
recording and the referenced audio files. Generated song folders can attach
their matching isolation manifest automatically when a project is created.

Each available instrument also has its own waveform on the timeline. A missing
stem is labeled unavailable; the full mix is not shown as an isolated stem.
Matching lengths alone do not establish good alignment. Listen before using a
stem for close timing edits or vocal-only cleaning.

Stem separation uses local models. Their terms are in
[THIRD_PARTY.md](../THIRD_PARTY.md).

## Keep stems and export game audio

New projects and manually attached stems keep their own verified copies inside
the song's `.yace` working data. Project state and undo snapshots
refer to those copies with relative paths, so moving a project or clearing the
separator cache does not remove its instrument audio.

Exporting a complete six-part primary separation writes `guitar.wav`, `bass.wav`,
`drums.wav`, `vocals.wav` and `keys.wav`. `song.wav` contains only the remaining
instruments. YARG can mute each part separately when its mute-on-miss setting is
enabled. The editor's solo/gain settings do not change those exports. A separate
high-quality keyboard analysis is retained for review but is not mixed into the
primary separation, which would duplicate instruments.

The original recording and prior source channels stay in `.yace/originals/audio/`, outside
the game's top-level audio scan. Reopening an export restores the original
recording for editing. `.yace/export/yace-audio.json` records the routing. Retained
export stems and manifests live in `.yace/audio`. A partial separation
keeps the original audio instead of replacing it with an incomplete song; missing
or changed attachments must be repaired before exporting their files.

Earlier exports using `.yace-audio` and top-level sidecars remain readable. New
exports consolidate YACE-specific files under `.yace`, while playable instrument
audio keeps the usual top-level names.

This is a song-folder export for compatible games. Rock Band console packages
and separation quality still need their own tooling and listening/playtesting.

The routing follows YARG's `YARG.Core/Audio/AudioHelpers.cs` and
`YARG.Core/Song/Entries/Ini/SongEntry.IniBase.cs` in its YARG.Core repository.
Mute-on-miss behavior is implemented in the game repository's
`Assets/Script/Gameplay/GameManager.Audio.cs` (`ChangeStemMuteState`). This source
check establishes supported filenames and settings, not a completed game playtest.
