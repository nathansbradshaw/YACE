# Generate a chart

Open **Generate**, or choose **Import audio** on the starting screen.

To add one part to an open chart, use **Add part → Generate…** beside the
instrument. The job uses the current recording and tempo map. Other parts stay
in place. When it finishes, select **Add** or **Replace** to apply the draft;
Undo restores the previous notes. If that part changed during generation,
YACE keeps both versions and refuses to overwrite your edits.

Part jobs are saved under the song's `.yace/jobs/parts` folder. Keep `.yace` with
the song to retain generation evidence and recovery files. Older jobs in a
review project's `generation-jobs` folder remain readable.

1. **Choose audio.** Select a local recording. Expand **Song details** to check
   the title, artist and detected tempos. The detected tempo map is used by
   default. Clear **Use detected tempo map**, or edit the starting BPM, to use
   one constant tempo instead.
2. **Choose parts.** Select Guitar, Bass, Drums, Piano, Pro Piano or Vocals.
   For vocals, choose English, Spanish or automatic language detection.
3. **Choose where to save.** Pick a location for the new song folder, then
   select **Generate charts**.

The draft opens in the editor when generation finishes. Listen through each
part before exporting. Generated notes can be missing, extra or on the wrong
instrument. The original recording stays separate from your edits.

Audio analysis compares overlapping sections throughout the recording and adds
markers for sustained pulse changes. **Detected tempos** lists the proposed
times and BPMs. These markers are included in the generated chart and exports;
saved jobs retain the same map when resumed. Existing charts keep their timing
when generating another part.

The analyzer searches 60–200 BPM and favors changes lasting roughly 12 seconds
or longer. Brief changes, small fluctuations (about 2% or less), weak beats,
half/double-time choices and gradual drift may need manual correction. It does
not establish the first beat or bar. Review **Timing** against the recording.
Silence or an unreliable pulse falls back to an editable starting BPM.

To inspect a recording without generating notes:

```sh
chart-engine generation analyze-tempo recording.wav
```

The command returns both analysis details and a proposed `tempo_map` (or `null`
when no pulse was measured). Headless generation requests can supply that map
via `tempo_map`
(`resolution`, `offset`, and `tempos` containing `tick` and `bpm`), with `bpm`
matching its first marker. Requests without `tempo_map` retain constant timing.

## Progress and saved files

Desktop generation shows the current operation and elapsed time. Separation
reports completed audio chunks; note detection reports the recording time it has
processed. The progress bar describes the current step, rather than estimating
the whole job. Model downloads report transferred bytes. Expand **Recent worker
activity** for the latest worker messages. Closing the dialog leaves a progress
button in the corner; select it to reopen the job.

Model inference can still take several minutes, especially the first time a GPU
model loads. Matching separation caches and completed job stages are reused on
retry. YACE does not discard a completed separation when chart generation fails.

Each generated song has one destination folder:

```text
Song chart/
  notes.chart or notes.mid
  song.ini
  song.wav and optional guitar/bass/drums/vocals/keys audio
  album.png or album.jpg when supplied
  .yace/
    project/            editing state, source snapshot, retained stems and undo
    jobs/generation/    import, settings, intermediate results, progress and logs
    cache/separation/   reusable instrument separation
    analysis/, audio/, review/, working/, originals/, history/, export/
```

The visible files are the initial generated game draft. Open that song folder to
continue editing; its existing `.yace/project` is found automatically. Export
again after making edits. Stem caches are reusable acceleration data; the editing
project retains its own audio copies.

The worker publishes chart files after their audio and metadata are ready. Retry
checks already published files and refuses to replace files changed by a person.
Old generation folders containing `input`, `song`, `project` and `game-song`
directly still open and resume with their original layout.

## Options

Expand **Generation options** to control model downloads, lyric lookup, standard
drums, or drums-only analysis of the original mix. Full generation uses local
models; stem separation needs a compatible GPU. Model terms are listed under
**Model licenses** and in [THIRD_PARTY.md](../THIRD_PARTY.md).

**Find lyrics online** sends song metadata to LRCLIB. Audio is processed locally.
See [Privacy](PRIVACY.md).

## Cancel or resume

Closing the dialog lets generation continue. **Cancel** stops the worker and
keeps the saved job. Use **Resume saved job** to reopen a job folder, then
**Resume generation**. Completed jobs offer **Open generated chart**.

Keep the complete job folder if you want to resume later. It contains the
import, settings and intermediate results.

For command-line options:

```sh
chart-engine generation --help
```
