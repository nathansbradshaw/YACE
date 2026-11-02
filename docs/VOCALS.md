# Vocals and lyrics

Select **Vocals** in the editor to view the pitch lane and vocal selection desk.
Vocals can be empty while you build a part; missing lyrics do not need filler.

## Generate a draft

Choose **Vocals** in Auto-Chart and select the vocal language. English, Spanish
and automatic detection are available for recognition. Listen carefully to
sung words, backing vocals and rapid syllables: recognition can miss them.

Pitch estimates and recognized lyrics are separate. A note can keep its pitch
while its word remains unknown. Review those gaps rather than assuming the
draft contains every sung word.

Choose **Standard** or **High accuracy (experimental)** for vocal analysis.
The experimental mode requires the separately downloaded, checksum-verified
RMVPE model (about 362 MB). It compares an independent contour with the existing
acoustic evidence and tracks stable note centres without using a musical key or
lyrics. It preserves the acoustic voicing mask, fixes note boundaries before attaching words and
retains supported uncertain pitches as amber draft notes. The note inspector
shows their evidence and alternative detector pitches. Failed optional lyric
recognition leaves the pitch draft available with a warning.

See [vocal accuracy and research](VOCAL_ACCURACY.md) for model comparisons,
English/Spanish research commands and the benchmark required before promotion.

Supplied-lyrics acoustic alignment is English-only. Unsupported or unclear words
can remain untimed. Model downloads run locally and have separate licenses;
see [THIRD_PARTY.md](../THIRD_PARTY.md).

## Edit a part

Use the vocal selection desk to work with phrases, notes and syllables. Select a
phrase or note, listen around it, then adjust pitch, timing and text. Use Undo
to recover an earlier edit. Export writes a separate song folder.

Use **Check pitches against the recording** to analyze a selected phrase or all
existing vocals. The computer proposes supported pitch changes and shows the
before/after notes; **Apply supported corrections** applies the batch with one
Undo. Unsupported notes remain unresolved. This check preserves lyrics, note
timing, phrases and talkies. Changing the recording, stem, tempo, notes or lyrics
invalidates pending proposals. Analysis can be cancelled without changing notes.
Both desktop and browser use the same model, preprocessing and correction rules.

Check phrase boundaries, missing lyrics, talkies and note releases before
testing the exported `PART VOCALS` track in a game. YACE supports a strict
subset of vocal MIDI; unsupported input is rejected rather than silently lost.

The preview helps with editing. It does not provide microphone scoring or a
complete vocal gameplay simulation.
