# Song offset

Open **Timing**, enter **Song offset (ms)**, then choose **Apply offset**.
Positive values move chart events later against the recording. Negative values
move them earlier. The field replaces the current offset; it does not add another
delay to an imported song. Values use whole milliseconds so both chart and MIDI
exports can represent them exactly.

Notes, sustains, phrases, section ticks, tempo events and audio bytes keep their
positions in their original data. The editor's note grid and preview use the new
chart clock against the existing waveform. Playback pauses after a change. Undo
and Redo restore the offset together with the other timing state.

Negative offsets can place events before recording time zero. Those events are
retained in the project and export; playback starts at zero. A change that would
push notes beyond an attached recording is refused. A chart without a recording
grows its timeline when needed, subject to the 24-hour timeline limit.

Chart exports store the offset in `notes.chart`, with no additional INI delay.
MIDI exports store it once in `song.ini` as `delay`. An offset-only edit to an
imported MIDI keeps the original MIDI bytes, including unsupported tracks and
events. Existing source files and retained originals stay unchanged.

Earlier vocal review evidence is archived when the chart clock changes; Undo
restores it. Recheck the changed timing against the recording before approving it.

Browser verification exercised a +250 ms chart offset: an authored note moved
from 2.500 to 2.750 seconds, Undo restored it, and Redo/reload kept the shift.
The downloaded chart retained tick 2400 and stored `Offset = 0.25` without an
additional INI delay. An untouched game-MIDI import was also shifted by -250 ms;
its exported MIDI remained byte-for-byte identical, with `delay = -250` in INI.
Actual downloads and the private source recording remain outside Git.
