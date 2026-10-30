# Piano and Pro Piano

In an open chart, select **Add part** beside **Track**. Choose a difficulty,
then **Blank** or **Generate…** beside **Piano (Keys)** or **Pro Piano (Pro Keys)**.
Both options keep your other parts in place.

For a new song, use **Generate → Choose parts** and select either piano option,
or both.

- **Piano** arranges keyboard notes into five lanes.
- **Pro Piano** uses the game's 25-key keyboard range, MIDI pitches 48–72.

Choose **Keyboard sound → Piano** for acoustic or electric piano. Synth is an
experimental option and can include notes from other melodic instruments.
The higher-quality piano separator is slower and uses an additional model with
separate terms; check [THIRD_PARTY.md](../THIRD_PARTY.md) before downloading it.

## Review and edit

1. Select Piano or Pro Piano in the editor.
2. Listen to the original or a matching isolated stem.
3. Check attacks, missing notes, extra notes and releases.
4. Inspect octave and range adjustments before exporting Pro Piano.

An octave change can make a part fit the controller while still needing a
musical decision. A successful export does not tell you whether the part feels
right to play.

Pro Piano exports through MIDI `PART REAL_KEYS_E/M/H/X` tracks. Keep the YACE
project as the editable master. Controller testing and the target game's
support determine which exported parts you can use.
