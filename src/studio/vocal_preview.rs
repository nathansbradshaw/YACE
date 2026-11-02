//! Source-clock vocal preview. Draw only authored notes; gaps remain gaps.
use super::state::Studio;
use crate::{
    timing::TempoMap,
    vocals::{Syllable, VocalChart, VocalTone},
};
use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct VisibleNote {
    x: f64,
    width: f64,
    y: f64,
    active: bool,
    talkie: bool,
    pitch: Option<u8>,
    lyric: String,
}

const LEFT: f64 = 90.0;
const RIGHT: f64 = 970.0;
const TOP: f64 = 80.0;
const BOTTOM: f64 = 470.0;

fn visible_notes(
    chart: &VocalChart,
    timing: &TempoMap,
    position: f64,
    ahead: f64,
) -> Vec<VisibleNote> {
    let start = position - 1.0;
    let end = position + ahead;
    let x = |seconds: f64| LEFT + (seconds - start) / (end - start) * (RIGHT - LEFT);
    // Valid lead notes are ordered and monophonic. Skip earlier material
    // before allocating visible geometry, including on very long songs.
    let first = chart
        .notes
        .partition_point(|note| timing.seconds(note.end) <= start);
    chart.notes[first..]
        .iter()
        .take_while(|note| timing.seconds(note.start) < end)
        .filter_map(|note| {
            let onset = timing.seconds(note.start);
            let offset = timing.seconds(note.end);
            if offset <= start || onset >= end {
                return None;
            }
            let (y, talkie) = match note.tone {
                VocalTone::Pitched { midi } => {
                    (BOTTOM - (midi as f64 - 36.0) / 48.0 * (BOTTOM - TOP), false)
                }
                VocalTone::Talkie { .. } => (515.0, true),
            };
            Some(VisibleNote {
                x: x(onset.max(start)),
                width: x(offset.min(end)) - x(onset.max(start)),
                y,
                active: onset <= position && position < offset,
                talkie,
                pitch: match note.tone {
                    VocalTone::Pitched { midi } => Some(midi),
                    VocalTone::Talkie { .. } => None,
                },
                lyric: match &note.lyric {
                    Syllable::Text { text, join_next } => {
                        format!("{text}{}", if *join_next { "-" } else { "" })
                    }
                    Syllable::Continue => "+".into(),
                    Syllable::ContinueWord => "…".into(),
                    Syllable::Missing => "Missing lyric".into(),
                },
            })
        })
        .collect()
}

fn pitch_range(chart: &VocalChart) -> Option<(u8, u8)> {
    let mut pitches = chart.notes.iter().filter_map(|n| match n.tone {
        VocalTone::Pitched { midi } => Some(i16::from(midi)),
        VocalTone::Talkie { .. } => None,
    });
    let first = pitches.next()?;
    let (low, high) = pitches.fold((first, first), |(low, high), p| (low.min(p), high.max(p)));
    let low = (low - 2).max(0);
    let high = (high + 2).min(127);
    let span = (high - low).max(12);
    let low = ((low + high) / 2 - span / 2).clamp(0, 127 - span);
    Some((low as u8, (low + span) as u8))
}
fn compact_pitch(pitch: Option<u8>, range: Option<(u8, u8)>) -> f64 {
    match (pitch, range) {
        (Some(pitch), Some((low, high))) if high > low => {
            8. + f64::from(high.saturating_sub(pitch)) / f64::from(high - low) * 28.
        }
        _ => 44.,
    }
}
fn pitch_name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(midi % 12)],
        i16::from(midi) / 12 - 1
    )
}

#[component]
fn CompactVocalPreview(
    notes: Vec<VisibleNote>,
    range: Option<(u8, u8)>,
    lookahead: f64,
    position: f64,
) -> Element {
    let cursor = 100. / (lookahead + 1.);
    let range_label = range.map_or_else(
        || "Unpitched".into(),
        |(low, high)| format!("{}–{}", pitch_name(low), pitch_name(high)),
    );
    let current = notes.iter().find(|n| n.active).map(|n| n.lyric.clone());
    rsx! {div{class:"compact-vocal-preview",role:"img",aria_label:"Vocal pitch and lyrics on the recording clock","data-preview-track":"Vocals","data-preview-position":"{position}",
        span{class:"compact-vocal-range",title:"Displayed authored pitch range","{range_label}"}
        span{class:"compact-vocal-talkie","Talkie"}
        if let Some(current)=current {output{class:"compact-vocal-current",aria_label:"Current lyric",title:"{current}","{current}"}}
        div{class:"compact-vocal-grid",
            for y in [8.,22.,36.] {div{class:"compact-vocal-guide",style:"top:{y}px"}}
            div{class:"compact-vocal-cursor",style:"left:{cursor}%",aria_hidden:"true"}
            for (index,note) in notes.iter().enumerate() {{
                let left=(note.x-LEFT)/(RIGHT-LEFT)*100.;let width=note.width/(RIGHT-LEFT)*100.;
                let top=compact_pitch(note.pitch,range);
                let title=note.pitch.map_or_else(||format!("Talkie · {}",note.lyric),|midi|format!("{} · {}",pitch_name(midi),note.lyric));
                rsx!{div{key:"{index}",class:if note.active{"compact-vocal-note active"}else if note.talkie{"compact-vocal-note talkie"}else{"compact-vocal-note"},style:"left:{left}%;width:{width}%;top:{top}px",title:"{title}"}
                    span{class:if note.active{"compact-vocal-lyric active"}else{"compact-vocal-lyric"},style:"left:{left}%;width:{width}%",title:"{note.lyric}","{note.lyric}"}
                }
            }}
        }
    }}
}

#[component]
pub fn VocalPreview(studio: Studio, lookahead: f64, #[props(default)] compact: bool) -> Element {
    let range = use_memo(move || {
        studio
            .doc
            .read()
            .as_ref()
            .and_then(|doc| doc.content.vocals.as_ref().and_then(pitch_range))
    });
    let doc = studio.doc.read();
    let Some(doc) = doc.as_ref() else {
        return rsx! {};
    };
    let Some(chart) = &doc.content.vocals else {
        return rsx! { div { class: "studio-empty-preview",
            div { class: "empty-state-title", "No vocal chart yet" }
            div { if cfg!(target_arch="wasm32") { "Place notes in the vocal lane or import vocals in the selection desk." } else { "Generate or import vocals in the selection desk." } }
        }};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let position = (studio.playhead)();
    let notes = visible_notes(chart, &timing, position, lookahead);
    if compact {
        return rsx! {CompactVocalPreview{notes,range:range(),lookahead,position}};
    }
    let cursor = LEFT + (RIGHT - LEFT) / (lookahead + 1.0);
    rsx! {
        svg { class: "studio-highway vocal-pitch-preview", view_box: "0 0 1000 620", preserve_aspect_ratio: "xMidYMid meet", role: "img",
            "aria-label": "Vocal draft pitch and lyrics on the recording clock",
            for (midi, label) in [(36, "C2"), (48, "C3"), (60, "C4"), (72, "C5"), (84, "C6")] {
                {let y = BOTTOM - (midi as f64 - 36.0) / 48.0 * (BOTTOM - TOP);
                rsx! {
                    line { x1: "{LEFT}", x2: "{RIGHT}", y1: "{y}", y2: "{y}", stroke: "#334353" }
                    text { x: "30", y: "{y + 5.0}", fill: "#90a7b9", font_size: "16", "{label}" }
                }}
            }
            text { x: "20", y: "520", fill: "#90a7b9", font_size: "14", "Talkie" }
            line { x1: "{cursor}", x2: "{cursor}", y1: "55", y2: "550", stroke: "#eef8ff", stroke_width: "2" }
            for note in &notes {
                g {
                    rect { x: "{note.x}", y: "{note.y - 6.0}", width: "{note.width}", height: "12", rx: "4",
                        fill: if note.active { "#fff4bb" } else if note.talkie { "#dbb5f7" } else { "#61d4ca" }
                    }
                    text { x: "{note.x}", y: "{note.y + 27.0}", fill: if note.active { "#fff4bb" } else { "#deebf5" }, font_size: "18", "{note.lyric}" }
                }
            }
            text { x: "{cursor}", y: "585", fill: "#90a7b9", font_size: "16", text_anchor: "middle", "Now" }
        }
        div { class: "game-preview-badge", "VOCAL DRAFT" }
        div { class: "game-preview-track", "Lead · {chart.notes.len()} notes" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        timing::Tempo,
        vocals::{VocalNote, VocalPhrase},
    };

    fn chart(notes: Vec<VocalNote>) -> VocalChart {
        VocalChart {
            version: 1,
            resolution: 480,
            source: "Synthetic preview fixture".into(),
            phrases: Vec::new(),
            notes,
        }
    }
    fn note(start: u64, pitch: Option<u8>) -> VocalNote {
        VocalNote {
            start,
            end: start + 240,
            tone: pitch.map_or(VocalTone::Talkie { lenient: false }, |midi| {
                VocalTone::Pitched { midi }
            }),
            lyric: Syllable::Missing,
            midi: None,
        }
    }
    #[test]
    fn compact_range_contains_authored_extremes_and_keeps_talkies_unpitched() {
        let low = chart(vec![note(0, Some(0))]);
        let high = chart(vec![note(0, Some(127))]);
        assert_eq!(pitch_range(&low), Some((0, 12)));
        assert_eq!(pitch_range(&high), Some((115, 127)));
        let both = chart(vec![
            note(0, Some(0)),
            note(480, Some(127)),
            note(960, None),
        ]);
        assert_eq!(pitch_range(&both), Some((0, 127)));
        assert_eq!(compact_pitch(Some(127), pitch_range(&both)), 8.);
        assert_eq!(compact_pitch(Some(0), pitch_range(&both)), 36.);
        assert_eq!(compact_pitch(None, pitch_range(&both)), 44.);
        assert!(pitch_range(&chart(vec![note(0, None)])).is_none());
        assert_eq!(pitch_name(0), "C-1");
        assert_eq!(pitch_name(60), "C4");
    }
    #[test]
    fn long_vocal_tracks_only_allocate_notes_intersecting_the_source_window() {
        let timing = TempoMap::new(480, 0., vec![Tempo { tick: 0, bpm: 120. }]).unwrap();
        let chart = chart((0..100_000).map(|i| note(i * 480, Some(60))).collect());
        let notes = visible_notes(&chart, &timing, 20_000., 3.6);
        assert_eq!(notes.len(), 10);
        assert_eq!(notes.first().unwrap().x, LEFT);
        let last = notes.last().unwrap();
        assert!((last.x + last.width - RIGHT).abs() < 0.0001);
        assert_eq!(notes.iter().filter(|n| n.active).count(), 1);
    }

    #[test]
    fn follows_recording_offset_and_tempo_changes_and_clips_long_notes() {
        let timing = TempoMap::new(
            480,
            2.0,
            vec![
                Tempo {
                    tick: 0,
                    bpm: 120.0,
                },
                Tempo {
                    tick: 480,
                    bpm: 60.0,
                },
            ],
        )
        .unwrap();
        let chart = VocalChart {
            version: 1,
            resolution: 480,
            source: "fixture".into(),
            phrases: vec![VocalPhrase {
                start: 0,
                end: 1440,
                player: 105,
            }],
            notes: vec![
                VocalNote {
                    midi: None,
                    start: 0,
                    end: 480,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Text {
                        text: "Sonríe".into(),
                        join_next: false,
                    },
                },
                VocalNote {
                    midi: None,
                    start: 480,
                    end: 1440,
                    tone: VocalTone::Talkie { lenient: false },
                    lyric: Syllable::Text {
                        text: "sí".into(),
                        join_next: false,
                    },
                },
            ],
        };
        let visible = visible_notes(&chart, &timing, 2.5, 1.0);
        assert_eq!(visible.len(), 2);
        assert!(!visible[0].active);
        assert!(visible[1].active && visible[1].talkie);
        assert_eq!(visible[0].lyric, "Sonríe");
        assert_eq!(visible[1].x, (LEFT + RIGHT) / 2.0);
        assert_eq!(visible[1].x + visible[1].width, RIGHT);
        assert_eq!(visible_notes(&chart, &timing, 4.0, 1.0)[0].x, LEFT);
        assert!(visible_notes(&chart, &timing, 6.0, 1.0).is_empty());
    }
}
