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
Undo. Corrected notes remain amber drafts, with the previous pitch available in
the inspector. Unsupported notes remain unresolved. This check preserves lyrics, note
timing, phrases and talkies. Changing the recording, stem, tempo, notes or lyrics
invalidates pending proposals. Analysis can be cancelled without changing notes.
Both desktop and browser use the same model, preprocessing and correction rules.
The section checker compares in both directions: it also searches for supported
singing outside charted notes. Choose a phrase, an existing loop, an explicit
time range in seconds, or all vocals. The blue pitch contour overlays current
gray notes; selecting a proposed repair previews its orange note boundaries.

Section repairs can extend a short note, split two stable pitches or a repeated
sound inside one note, and recover a missing note of at least about 80 ms.
Each proposal requires stable RMVPE support plus agreement from both independent
acoustic detectors, with local energy and clarity checks. This agreement is not
a correctness probability or proof of which singer is present. New segments
have unknown lyrics. Existing talkies, note attributes and words are preserved.
Supported singing outside authored scoring phrases receives a separate phrase;
authored phrase markers are not moved.

Check individual repairs, optionally include supported pitch corrections, and
choose **Apply selected repairs** for one Undo batch. **Loop and inspect** jumps
to a proposal for vocal-stem/MIDI audition. Weak short sounds, possible extra
notes and possible overlong notes are review findings. They never trigger
automatic shortening or deletion, which could sacrifice quiet or missed voices.
All repairs remain flagged drafts after saving and subsequent checks. There is
no claim of complete or near-perfect transcription from an empty findings list.

**Prepare current-note comparison** and **Prepare selected-repair comparison**
create a temporary stereo listening clip: the vocal stem is on the left and
tones from the notes are on the right, on the same sample clock. The clip follows
a loop of at most 30 seconds or the first 30 seconds of the analyzed interval.
Use headphones to compare channels; comparison playback pauses the main player.
Previews play at original speed and do not apply edits or enter project backups.

Checking another section retains review flags and previous-pitch alternatives
for earlier corrections while their source, timing and note bounds still match.
In the browser, **Generate vocals… → Use a downloaded model…** can import an
existing pinned `rmvpe.onnx` instead of downloading another copy. Model imports
are checksum verified before local caching.

Check phrase boundaries, missing lyrics, talkies and note releases before
testing the exported `PART VOCALS` track in a game. YACE supports a strict
subset of vocal MIDI; unsupported input is rejected rather than silently lost.

The preview helps with editing. It does not provide microphone scoring or a
complete vocal gameplay simulation.
