# Generate a chart

Open **Generate**, or choose **Import audio** on the starting screen.

To add one part to an open chart, use **Add part → Generate…** beside the
instrument. The job uses the current recording and tempo map. Other parts stay
in place. When it finishes, select **Add** or **Replace** to apply the draft;
Undo restores the previous notes. If that part changed during generation,
YACE keeps both versions and refuses to overwrite your edits.

### Reanalyze a part

Select **Reanalyze…** beside the track picker, then choose **Whole track** or
**Current loop**. Loop repairs expand to include crossing sustains and complete
vocal phrases. Changes to linked easier difficulties are included in the review;
their manual overrides remain in place. Notes outside the displayed repair range
stay intact.

Move **Sensitivity** from 0 (fewer detections) to 100 (more detections), then press
**Analyze**. **50 · Default** preserves the existing detector settings. Changing
the slider or another analysis option makes the previous proposal outdated;
Apply becomes available after a successful analysis with those settings.
Sensitivity changes detector admission thresholds, not the chart difficulty.
Experimental Synth uses duration and phrase continuity because it supplies no
per-note confidence scores. Higher sensitivity can add incorrect detections.

For a Guitar chart, **Guitar chart follows** can select Guitar, Other melody /
synth, or Piano. This selects a separated sound to arrange for five frets; it
does not prove that separation correctly distinguished the instruments.

Use **Current / Proposed** and **Play / Pause** to compare notes at the same
playhead position, loop and speed. The proposal is read-only. Review the note
changes, affected difficulties, final repair range and detection warnings, then
choose **Apply reanalysis** or **Discard draft**. Apply is one undoable edit.
Cancelled, failed and empty proposals keep your chart and earlier saved drafts.
**Review previous successful draft** returns to the preceding result after a retry.

Applied Guitar sections retain their followed sound, including mixed-source
tracks. Audition crossfades between the matching stems; clear **Follow section
sound sources** to hear the original recording. Missing stems fall back to the
original recording. Source choices travel with project saves, archives and undo
history. Game exports retain the existing audio-routing policy.

Analysis retains the full recording context even for a short loop. Matching
separation caches are reused, but a new sensitivity may require fresh detector
inference. Saved requests include `detection: { sensitivity, policy_version }`;
omitted settings mean sensitivity 50 with policy version 1. A completed part job
also saves `reanalysis-review.json` with its effective range and change summary.
Neither successful generation nor automated checks establish musical accuracy.

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
