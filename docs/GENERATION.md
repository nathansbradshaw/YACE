# Generate a chart

Open **Generate**, or choose **Import audio** on the starting screen.

To add one part to an open chart, use **Add part → Generate…** beside the
instrument. The job uses the current recording and tempo map. Other parts stay
in place. When it finishes, select **Add** or **Replace** to apply the draft;
Undo restores the previous notes. If that part changed during generation,
YACE keeps both versions and refuses to overwrite your edits.

Part jobs are saved in the review project's `generation-jobs` folder. Keep that
folder with the project to retain generation evidence and recovery files.

1. **Choose audio.** Select a local recording. Expand **Song details** to check
   the title, artist and estimated BPM. Correct these before generating.
2. **Choose parts.** Select Guitar, Bass, Drums, Piano, Pro Piano or Vocals.
   For vocals, choose English, Spanish or automatic language detection.
3. **Choose where to save.** Pick a location for the new song folder, then
   select **Generate charts**.

The draft opens in the editor when generation finishes. Listen through each
part before exporting. Generated notes can be missing, extra or on the wrong
instrument. The original recording stays separate from your edits.

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
