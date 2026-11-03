# Compact project backups

Choose **Save project backup…** (desktop) or **Download project backup…**
(browser), then **Prepare backup**. Original audio, authored files, notes,
metadata, analysis documents and all undo/redo checkpoints are always retained.
Generated instrument stems and drum-part audio are included by default. Clear
**Include generated stems** when portability matters more than retaining solo
audio. This changes only the backup; the open project's stems remain intact.

Repeated assets occupy one ZIP entry. The dialog defaults to **High quality ·
24-bit listening copies**. Generated float WAV samples are rounded to 24-bit
PCM precision using the stem's peak to preserve headroom, then packed with
stereo decorrelation, adaptive prediction and ZIP compression. There is no
resampling, timing shift or clipping. This reduces precision; it is not bit-exact
preservation or a claim of human listening approval. Original recordings and any
asset also used as source/unknown authored data are always retained exactly.

Choose **Exact · original generated samples** to retain every float bit,
including negative zero and nonfinite representations, using reversible channel
prediction and byte planes before ZIP compression. The programmatic backup API
defaults to Exact for compatibility. Restored checksums are verified in either mode.
RIFF float and WAVE_FORMAT_EXTENSIBLE float headers are supported; other files
are compressed without changing their bytes. Compression savings depend on the
recording. This reduces portable backups, not the live project's WAV files.

The ZIP contains `yace-backup.json` (version 1) and content-addressed
`yace-assets/<sha256>` entries. The manifest maps every original relative path
to its asset, length, original/restored checksums and optional audio layout. The reader
validates paths, duplicate names, version, lengths, checksums and expansion
limits before publishing the imported project. An unsuccessful import does not
replace the active project. Compact backups need an updated YACE; ordinary
older project/song ZIPs remain supported. Game exports retain their existing
format and compatibility.

Imports read one compressed entry at a time. Browser workers seek 1 MiB Blob
slices and transfer completed entries; desktop reads from the original file.
The browser accepts archives up to 16 GiB, with at most 2 GiB of unique expanded
files and 1 GiB per file. The format permits 16,384 logical files and 32 GiB
of total logical data, including duplicates. Desktop permits 2 GiB per file.
Unique open files still occupy memory; this does not yet provide arbitrary-size,
disk-backed projects or remove audio-decoding and model working-set limits.

When generated stems are omitted, their editing-history references remain in
place and resolve as unavailable after relocation. Original recording playback
and chart edits remain available; regenerate or reattach stems to resume solo
audition. Unknown user assets are preserved rather than guessed to be caches.

Listening copies record original-to-restored hashes in `yace-portable-audio.json`.
Relocation updates current and undo/redo stem references and checkpoint IDs,
retaining original checkpoint bytes as before. Stem provenance explicitly
records the portable encoding. Saved kit receipts continue to describe their
original inference; a separate mapping binds derived listening files without
rewriting the receipt into a claim of a new model run. Source-bound analysis
reports are preserved and can become stale when their input stem hash changes.
