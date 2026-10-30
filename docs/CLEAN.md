# Make a clean-audio copy

Open **Clean audio** with a song loaded.

1. **Find words.** Select **Find explicit words**. Recognition is English-only,
   runs locally and downloads a 190 MB model on first use.
2. **Listen and adjust.** Review the words and mute ranges. **Listen source**
   loops the original recording around a range. Check missed words and timing.
3. **Export a copy.** Choose a save location and a new folder name, then select
   **Export clean audio**.

Finding words again replaces the current ranges. Recognition can miss sung
words or assign the wrong times; it cannot certify that a recording is clean.

## Change the word list

Expand **Words to mute and timing**. Edit one exact word or phrase per line,
adjust padding or fades, then choose **Rebuild mute ranges**. Rebuilding replaces
manual range edits. The preset is a starting point; review it for the recording.

**Use saved files** loads word timings or a saved clean plan. A plan without word
timings can be edited, but rematching a new word list needs recognition or
imported word timings.

## Choose an audio mode

- **Vocal-only** needs a full-length, aligned vocal stem and a note explaining
  how its timing was checked. Poor alignment can leave words or affect instruments.
- **Full mix** mutes the instruments and voice together in each selected range.

Matching duration alone does not prove stem alignment. Vocal-only mode does not
silently switch to full-mix muting.

## Save and listen

**Clean edits are not autosaved.** Use **Save plan** or export before quitting.
Closing the dialog keeps edits during the current app session.

Export creates `clean.wav`, `plan.json` and `receipt.json` in a new folder.
Original files stay untouched. Listen to the exported audio separately to judge
the actual edit; **Listen source** plays the original.
