//! Conservative bidirectional chart/audio comparison. These are review proposals,
//! never an instruction to infer lyrics or remove an unsupported voice.
use crate::{
    editor::TimeRange,
    timing::TempoMap,
    vocal_generation::{Generation, NoteEvidence, PitchFrame, PitchSupport},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const ALGORITHM: &str = "section-audio-review-v1-raw0.1-dsp60-clarity0.5-flat0.35-stable80ms";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepairKind {
    Missing,
    Boundary,
    Split,
}
impl RepairKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Missing => "Missing note",
            Self::Boundary => "Note length",
            Self::Split => "Split note",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repair {
    pub kind: RepairKind,
    pub before: Vec<VocalNote>,
    pub after: Vec<VocalNote>,
    pub support_frames: usize,
    pub acoustic_frames: usize,
    pub detail: String,
}
impl Repair {
    pub fn bounds(&self) -> (u64, u64) {
        let mut notes = self.before.iter().chain(&self.after);
        let Some(first) = notes.next() else {
            return (0, 0);
        };
        notes.fold((first.start, first.end), |(a, b), n| {
            (a.min(n.start), b.max(n.end))
        })
    }
    pub fn evidence(&self) -> Vec<NoteEvidence> {
        self.after.iter().filter_map(|n| {
            let VocalTone::Pitched{midi} = n.tone else { return None; };
            Some(NoteEvidence {start:n.start,end:n.end,selected_midi:midi,
                alternatives:self.before.iter().filter_map(|n| match n.tone {VocalTone::Pitched{midi:p} if p!=midi=>Some(p),_=>None}).collect(),
                support:vec![PitchSupport::ContourPath], uncertain:true,models:vec![],
                reasons:vec![format!("Section review: {}. {} model frames, {} agreeing acoustic frames. Review the singer and note boundaries; support is not a correctness probability.",self.detail,self.support_frames,self.acoustic_frames)]})
        }).collect()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub start: f64,
    pub end: f64,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub seconds: f64,
    pub midi: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub repairs: Vec<Repair>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub contour: Vec<Point>,
    #[serde(default)]
    pub voiced_seconds: f64,
    #[serde(default)]
    pub uncharted_seconds: f64,
    #[serde(default)]
    pub applied_repairs: Vec<usize>,
    #[serde(default = "yes")]
    pub applied_pitch: bool,
}
fn yes() -> bool {
    true
}
impl Default for Section {
    fn default() -> Self {
        Self {
            algorithm: String::new(),
            repairs: vec![],
            findings: vec![],
            contour: vec![],
            voiced_seconds: 0.,
            uncharted_seconds: 0.,
            applied_repairs: vec![],
            applied_pitch: true,
        }
    }
}
#[derive(Clone)]
struct Run {
    start: f64,
    end: f64,
    midi: u8,
    frames: usize,
    acoustic: usize,
    acoustic_times: Vec<f64>,
    raw_pitches: Vec<f64>,
}
fn pitched(n: &VocalNote) -> Option<u8> {
    if let VocalTone::Pitched { midi } = n.tone {
        Some(midi)
    } else {
        None
    }
}
fn agrees(f: &PitchFrame, p: f64) -> bool {
    f.local_energy_active != Some(false)
        && f.local_rms.unwrap_or(f.rms) >= 0.0015
        // High caps display clarity at .59; retain the independent detector
        // frequencies and never demand an impossible post-refinement .75.
        && f.clarity >= 0.5
        && f.yin_midi
            .zip(f.mcleod_midi)
            .is_some_and(|(a, b)| (a - b).abs() <= 0.55 && (a - p).abs() <= 0.75 && (b - p).abs() <= 0.75)
}
fn supported(r: &Run) -> bool {
    if r.frames < 8 || r.end - r.start < 0.075 || r.acoustic * 5 < r.frames * 3 {
        return false;
    }
    // A decoded semitone plateau in a continuous slide is not a stable note.
    // Compare raw contour ends, retaining ordinary vibrato around a centre.
    let n = (r.raw_pitches.len() / 3).max(1);
    let early = r.raw_pitches[..n].iter().sum::<f64>() / n as f64;
    let late = r.raw_pitches[r.raw_pitches.len() - n..].iter().sum::<f64>() / n as f64;
    (early - late).abs() <= 0.35
}

/// No generated notes or words are treated as independent acoustic observations.
pub fn compare(
    chart: &VocalChart,
    timing: &TempoMap,
    g: &Generation,
    frames: &[PitchFrame],
    range: TimeRange,
) -> Result<Section> {
    let raw = g
        .rmvpe_pitch
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing independent contour"))?;
    chart.validate()?;
    ensure!(
        frames.len() <= 400_000 && frames.len() == raw.frames.len() && chart.notes.len() <= 200_000,
        "Section comparison exceeds budget"
    );
    let mut out = Section {
        algorithm: ALGORITHM.into(),
        applied_pitch: true,
        ..Default::default()
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut last_valid = false;
    let mut note_cursor = 0;
    for (i, f) in frames.iter().enumerate() {
        if f.time_seconds < range.start || f.time_seconds >= range.end {
            last_valid = false;
            continue;
        }
        let r = &raw.frames[i];
        let pitch = f.midi.map(|p| p.round()).filter(|p| {
            r.confidence >= 0.1
                && r.rms >= 0.0015
                && r.frequency_hz
                    .is_some_and(|hz| (69. + 12. * (hz / 440.).log2() - p).abs() <= 0.75)
        });
        let Some(p) = pitch else {
            last_valid = false;
            continue;
        };
        out.voiced_seconds += 0.01_f64.min(range.end - f.time_seconds);
        let tick = timing.tick(f.time_seconds);
        if let Some(tick) = tick {
            while note_cursor < chart.notes.len() && chart.notes[note_cursor].end <= tick {
                note_cursor += 1;
            }
            if chart.notes.get(note_cursor).is_none_or(|n| n.start > tick) {
                out.uncharted_seconds += 0.01_f64.min(range.end - f.time_seconds);
            }
        }
        let acoustic = g
            .pitch_frames
            .binary_search_by(|v| v.time_seconds.total_cmp(&f.time_seconds))
            .ok()
            .map(|j| agrees(&g.pitch_frames[j], p))
            .unwrap_or_else(|| {
                let j = g
                    .pitch_frames
                    .partition_point(|v| v.time_seconds < f.time_seconds);
                [j.checked_sub(1), Some(j)]
                    .into_iter()
                    .flatten()
                    .filter_map(|j| g.pitch_frames.get(j))
                    .any(|d| (d.time_seconds - f.time_seconds).abs() <= 0.012 && agrees(d, p))
            });
        if last_valid
            && let Some(last) = runs.last_mut()
            && last.midi == p as u8
            && f.time_seconds - last.end <= 0.005
        {
            last.end = (f.time_seconds + 0.01).min(range.end);
            last.frames += 1;
            last.acoustic += usize::from(acoustic);
            if acoustic {
                last.acoustic_times.push(f.time_seconds);
            }
            last.raw_pitches
                .push(69. + 12. * (r.frequency_hz.unwrap() / 440.).log2());
        } else {
            runs.push(Run {
                start: f.time_seconds,
                end: (f.time_seconds + 0.01).min(range.end),
                midi: p as u8,
                frames: 1,
                acoustic: usize::from(acoustic),
                acoustic_times: if acoustic {
                    vec![f.time_seconds]
                } else {
                    vec![]
                },
                raw_pitches: vec![69. + 12. * (r.frequency_hz.unwrap() / 440.).log2()],
            });
        }
        last_valid = true;
    }
    // Plot only the selected interval, bounded to 2000 points, with gaps retained.
    let a = raw.frames.partition_point(|f| f.time_seconds < range.start);
    let b = raw.frames.partition_point(|f| f.time_seconds < range.end);
    let stride = (b - a).div_ceil(2000).max(1);
    for chunk in raw.frames[a..b].chunks(stride) {
        let first = &chunk[0];
        let midi = chunk
            .iter()
            .all(|f| f.confidence >= 0.1 && f.frequency_hz.is_some())
            .then(|| 69. + 12. * (first.frequency_hz.unwrap() / 440.).log2());
        out.contour.push(Point {
            seconds: first.time_seconds,
            midi,
        });
    }
    let stable: Vec<_> = runs.iter().filter(|r| supported(r)).collect();
    let mut working = chart.clone();
    // Splits require two independently supported plateaus; smooth slides or
    // vibrato alone cannot supply two stable semitone centres.
    for (index, n) in chart.notes.iter().enumerate() {
        if pitched(n).is_none()
            || timing.seconds(n.start) < range.start
            || timing.seconds(n.end) > range.end
        {
            continue;
        }
        let start = timing.seconds(n.start);
        let end = timing.seconds(n.end);
        let first_run = stable.partition_point(|r| r.end < start - 0.025);
        let last_run = stable.partition_point(|r| r.start <= end + 0.025);
        let inside: Vec<_> = stable[first_run..last_run]
            .iter()
            .copied()
            .filter(|r| r.start >= start - 0.025 && r.end <= end + 0.025 && r.end - r.start >= 0.12)
            .collect();
        if inside.len() == 2
            && (inside[0].midi != inside[1].midi || inside[1].start - inside[0].end >= 0.02)
            && inside[1].start - inside[0].end <= 0.04
            && inside[0].start - start <= 0.04
            && end - inside[1].end <= 0.04
        {
            let middle = timing
                .tick((inside[0].end + inside[1].start) * 0.5)
                .unwrap_or(n.start);
            if n.start < middle && middle < n.end {
                let mut first = n.clone();
                first.end = middle;
                first.tone = VocalTone::Pitched {
                    midi: inside[0].midi,
                };
                let mut second = n.clone();
                second.start = middle;
                second.tone = VocalTone::Pitched {
                    midi: inside[1].midi,
                };
                second.lyric = Syllable::Missing;
                second.midi = None;
                let repair=Repair{kind:RepairKind::Split,before:vec![n.clone()],after:vec![first,second],support_frames:inside.iter().map(|r|r.frames).sum(),acoustic_frames:inside.iter().map(|r|r.acoustic).sum(),detail:if inside[0].midi!=inside[1].midi {"Two stable pitches inside one charted note; the new segment has an unknown lyric"}else{"Two supported same-pitch sounds separated by a brief unvoiced gap; inspect the repeated attack"}.into()};
                if let Ok(next) = apply(&working, std::slice::from_ref(&repair)) {
                    working = next;
                    out.repairs.push(repair);
                    continue;
                }
            }
        }
        // Only extend supported same-pitch audio. Never shorten or delete a
        // note merely because a detector failed to hear the singer.
        if let Some(r) = stable[first_run..last_run].iter().copied().find(|r| {
            Some(r.midi) == pitched(n) && r.start <= start + 0.025 && r.end >= end - 0.025
        }) {
            let phrase = chart
                .phrases
                .iter()
                .find(|p| p.start <= n.start && n.end <= p.end)
                .unwrap();
            let left = chart
                .notes
                .get(index.wrapping_sub(1))
                .map_or(phrase.start, |p| p.end.max(phrase.start));
            let right = chart
                .notes
                .get(index + 1)
                .map_or(phrase.end, |p| p.start.min(phrase.end));
            let mut after = n.clone();
            after.start = timing
                .tick(r.start.max(range.start))
                .unwrap_or(n.start)
                .max(left)
                .min(n.start);
            after.end = timing
                .tick(r.end.min(range.end))
                .unwrap_or(n.end)
                .min(right)
                .max(n.end);
            if timing.seconds(n.start) - timing.seconds(after.start) >= 0.04
                || timing.seconds(after.end) - timing.seconds(n.end) >= 0.04
            {
                let repair = Repair {
                    kind: RepairKind::Boundary,
                    before: vec![n.clone()],
                    after: vec![after],
                    support_frames: r.frames,
                    acoustic_frames: r.acoustic,
                    detail: "The same supported sung pitch extends beyond this note".into(),
                };
                if let Ok(next) = apply(&working, std::slice::from_ref(&repair)) {
                    working = next;
                    out.repairs.push(repair);
                }
            }
        }
        let a = frames.partition_point(|f| f.time_seconds < start);
        let b = frames.partition_point(|f| f.time_seconds < end);
        let heard = frames[a..b]
            .iter()
            .filter(|f| f.midi.is_some() && f.clarity >= 0.1)
            .count();
        if b > a && heard * 5 < b - a && out.findings.len() < 2000 {
            out.findings.push(Finding{start,end,detail:"Possible extra note or missed/quiet voice: little model support. No automatic deletion is proposed.".into()});
        } else if let Some(r) = inside.first()
            && out.findings.len() < 2000
            && (r.start - start >= 0.08 || end - r.end >= 0.08)
        {
            out.findings.push(Finding{start,end,detail:"Possible long note or breath: supported voice occupies only part of this note. Inspect onset/release before shortening.".into()});
        }
    }
    // Subtract ALL chart intervals, including talkies, from supported runs.
    // Clip at phrase borders; additions outside existing phrases get new
    // scoring phrases without moving authored phrase markers.
    for r in stable {
        let start = timing.tick(r.start).unwrap_or(0);
        let end = timing.tick(r.end).unwrap_or(start);
        // A continuous same-pitch tail belongs in boundary review, not a
        // fabricated repeated note when its authored note crosses selection.
        let left = chart.notes.partition_point(|n| n.end <= start);
        let right = chart.notes.partition_point(|n| n.start < end);
        if chart.notes[left..right]
            .iter()
            .any(|n| pitched(n) == Some(r.midi))
        {
            continue;
        }
        let mut cuts = vec![start, end];
        let a = working.notes.partition_point(|n| n.end <= start);
        let b = working.notes.partition_point(|n| n.start < end);
        for n in &working.notes[a..b] {
            cuts.extend([n.start.max(start), n.end.min(end)]);
        }
        let a = working.phrases.partition_point(|p| p.end <= start);
        let b = working.phrases.partition_point(|p| p.start < end);
        for p in &working.phrases[a..b] {
            for t in [p.start, p.end] {
                if start < t && t < end {
                    cuts.push(t);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for span in cuts.windows(2) {
            if timing.seconds(span[1]) - timing.seconds(span[0]) < 0.075
                || working
                    .notes
                    .iter()
                    .any(|n| n.start < span[1] && span[0] < n.end)
            {
                continue;
            }
            let a = timing.seconds(span[0]);
            let b = timing.seconds(span[1]);
            let support_frames = raw.frames.partition_point(|f| f.time_seconds < b)
                - raw.frames.partition_point(|f| f.time_seconds < a);
            let acoustic_frames = r.acoustic_times.partition_point(|t| *t < b)
                - r.acoustic_times.partition_point(|t| *t < a);
            if support_frames < 8 || acoustic_frames * 5 < support_frames * 3 {
                continue;
            }
            let repair=Repair{kind:RepairKind::Missing,before:vec![],after:vec![VocalNote{start:span[0],end:span[1],tone:VocalTone::Pitched{midi:r.midi},lyric:Syllable::Missing,midi:None}],support_frames,acoustic_frames,detail:"Stable model pitch agrees with both acoustic detectors in an uncharted interval; lyric unknown".into()};
            if let Ok(next) = apply(&working, std::slice::from_ref(&repair)) {
                working = next;
                out.repairs.push(repair);
            }
        }
    }
    // Surface ambiguous short attacks, pitch changes and gaps as review work.
    for r in &runs {
        if r.end - r.start >= 0.04 && !supported(r) && out.findings.len() < 2000 {
            out.findings.push(Finding{start:r.start,end:r.end,detail:"Short or disputed sound: insufficient stable agreement for a repair. Inspect the vocal stem and contour.".into()});
        }
    }
    ensure!(out.repairs.len() <= 200_000, "Too many section proposals");
    Ok(out)
}

/// The editor uses this same operation for validation and a single Undo batch.
pub fn apply(chart: &VocalChart, repairs: &[Repair]) -> Result<VocalChart> {
    let mut next = chart.clone();
    for r in repairs {
        ensure!(
            !r.after.is_empty() && r.after.len() <= 2 && r.before.len() <= 1,
            "Invalid section repair shape"
        );
        ensure!(
            (8..=400_000).contains(&r.support_frames)
                && r.acoustic_frames <= r.support_frames
                && r.acoustic_frames * 5 >= r.support_frames * 3,
            "Insufficient independent section support"
        );
        ensure!(r.detail.len() <= 2000, "Section explanation exceeds budget");
        match r.kind {
            RepairKind::Missing => ensure!(
                r.before.is_empty()
                    && r.after.len() == 1
                    && r.after[0].lyric == Syllable::Missing
                    && r.after[0].midi.is_none(),
                "Invalid missing-note repair"
            ),
            RepairKind::Boundary => ensure!(
                r.before.len() == 1
                    && r.after.len() == 1
                    && r.before[0].lyric == r.after[0].lyric
                    && r.before[0].tone == r.after[0].tone
                    && r.before[0].midi == r.after[0].midi
                    && r.after[0].start <= r.before[0].start
                    && r.after[0].end >= r.before[0].end,
                "Invalid extension repair"
            ),
            RepairKind::Split => ensure!(
                r.before.len() == 1
                    && r.after.len() == 2
                    && r.after[0].start == r.before[0].start
                    && r.after[1].end == r.before[0].end
                    && r.after[0].end == r.after[1].start
                    && r.after[0].lyric == r.before[0].lyric
                    && r.after[1].lyric == Syllable::Missing
                    && r.after[0].midi == r.before[0].midi
                    && r.after[1].midi.is_none(),
                "Invalid split repair"
            ),
        }
        for before in &r.before {
            ensure!(pitched(before).is_some(), "Do not modify a talkie");
            let i = next
                .notes
                .iter()
                .position(|n| n == before)
                .ok_or_else(|| anyhow::anyhow!("Section note changed or repairs overlap"))?;
            next.notes.remove(i);
        }
        for n in &r.after {
            ensure!(
                pitched(n).is_some(),
                "A section repair needs pitched evidence"
            );
            ensure!(
                !next
                    .notes
                    .iter()
                    .any(|old| old.start < n.end && n.start < old.end),
                "Section repairs overlap existing notes"
            );
            if !next
                .phrases
                .iter()
                .any(|p| p.start <= n.start && n.end <= p.end)
            {
                ensure!(
                    !next
                        .phrases
                        .iter()
                        .any(|p| p.start < n.end && n.start < p.end),
                    "Repair crosses a scoring phrase boundary"
                );
                next.phrases.push(VocalPhrase {
                    start: n.start,
                    end: n.end,
                    player: 1,
                });
            }
            next.notes.push(n.clone());
        }
        next.notes.sort_by_key(|n| n.start);
        next.phrases.sort_by_key(|p| p.start);
    }
    next.validate()?;
    Ok(next)
}
