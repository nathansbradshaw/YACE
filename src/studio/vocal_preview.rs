//! Source-clock vocal preview. Draw only authored notes; gaps remain gaps.
use super::state::Studio;
use crate::{
    timing::TempoMap,
    vocals::{Syllable, VocalChart, VocalTone},
};
use dioxus::prelude::*;

#[derive(Debug)]
struct VisibleNote {
    x: f64,
    width: f64,
    y: f64,
    active: bool,
    talkie: bool,
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
    chart
        .notes
        .iter()
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

#[component]
pub fn VocalPreview(studio: Studio, lookahead: f64) -> Element {
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
                    start: 0,
                    end: 480,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Text {
                        text: "Sonríe".into(),
                        join_next: false,
                    },
                },
                VocalNote {
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
