# Import music

In desktop YACE, choose **Import audio** (or **Generate → Choose audio file**).
MP3, WAV, FLAC, Ogg, Opus, M4A and other FFmpeg-readable recordings are supported.
YACE reads embedded tags and cover art locally and measures the audio's pulse.
An untagged `Artist - Title` filename supplies editable title/artist suggestions.

Expand **Song details** to edit the title and artist, then select **Look up song
info & cover art**. This sends title, artist, album and duration to MusicBrainz,
the Apple music catalogue and, when lyrics are enabled, LRCLIB. It downloads
available covers from the Cover Art Archive. Audio is never uploaded.
Existing album/year/genre, lyrics and chosen artwork take priority. Clear a
field before looking up a replacement. **Choose cover art** also works offline.
Lookup failures and catalogue sources appear under **Imported information and
sources**; generation and manual editing remain available without a match.

Matching checks title, artist, recording version and duration. Album editions
can differ. Review the suggested details and cover before saving.

Choose a folder name and save location, then **Create blank chart** to start
manual authoring without models, or choose parts and **Generate charts**.
A blank import creates standard `notes.chart`, `song.ini`, `song.wav` and
available `album.png`/`album.jpg` files. Editing state lives in `.yace/project`,
import information in `.yace/analysis`, and the original recording in
`.yace/originals/audio`. The entire song folder can be moved; its source file and
temporary cover cache are no longer needed.

Detected tempo changes are retained in the project. Editing BPM chooses a
constant tempo. Audio alone can suggest the wrong half/double-time pulse or
first beat; listen and correct these in Timing. No stable pulse requires a
manual BPM for the command-line project importer. See [generation](GENERATION.md)
for the analyzer's range and limits.

## Command line

Analyze a local recording without network access:

```sh
cargo run --locked --release --no-default-features --bin yace-import-audio -- '/path/song.mp3'
```

Look up missing details and make a new project:

```sh
cargo run --locked --release --no-default-features --bin yace-import-audio -- \
  '/path/song.mp3' --title 'Song' --artist 'Artist' --lookup --output '/path/New song'
chart-engine editor open '/path/New song/.yace/project'
```

Omit `--lookup` to stay offline. Use `--bpm 120` to explicitly replace the
detected map with a constant tempo. Existing output folders are never replaced.

## Verification

`cargo test --locked --no-default-features --test song_import` encodes real MP3
fixtures with tags and attached artwork, measures known 120→90 BPM pulses,
creates a project, removes source/cache files, moves and reopens the song, then
exports its metadata and artwork. Catalogue selection fixtures reject remixes,
wrong artists and mismatched durations. The online smoke check is opt-in:

```sh
cargo test --locked --no-default-features --test song_import \
  live_catalogue_returns_album_year_and_cover -- --ignored --nocapture
```

Fixtures verify file handling and measured pulse accuracy, not general musical
accuracy or the availability of every online recording.
