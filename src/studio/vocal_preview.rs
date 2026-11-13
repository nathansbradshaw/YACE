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
    active: bool,
    talkie: bool,
    pitch: Option<u8>,
    lyric: String,
}

const LEFT: f64 = 90.0;
const RIGHT: f64 = 970.0;

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
            Some(VisibleNote {
                x: x(onset.max(start)),
                width: x(offset.min(end)) - x(onset.max(start)),
                active: onset <= position && position < offset,
                talkie: matches!(note.tone, VocalTone::Talkie { .. }),
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
#[derive(Clone, Copy)]
struct FocusedPlot {
    width: f64,
    height: f64,
}
impl FocusedPlot {
    fn new(width: f64, height: f64) -> Self {
        Self {
            width: width.max(160.),
            height: height.max(96.),
        }
    }
    fn x(self, normalized: f64) -> f64 {
        64. + (normalized - LEFT) / (RIGHT - LEFT) * (self.width - 76.)
    }
    fn pitch_y(self, midi: u8) -> f64 {
        20. + ((84. - f64::from(midi)) / 48.).clamp(0., 1.) * (self.height - 68.)
    }
    fn y(self, note: &VisibleNote) -> f64 {
        note.pitch
            .map_or(self.height - 24., |midi| self.pitch_y(midi))
    }
}

#[component]
fn CompactVocalPreview(
    notes: Vec<VisibleNote>,
    range: Option<(u8, u8)>,
    lookahead: f64,
    position: f64,
    playing: bool,
    audio: String,
    revision: u64,
) -> Element {
    let cursor = 100. / (lookahead + 1.);
    let range_label = range.map_or_else(
        || "Unpitched".into(),
        |(low, high)| format!("{}–{}", pitch_name(low), pitch_name(high)),
    );
    let current = notes.iter().find(|n| n.active).map(|n| n.lyric.clone());
    rsx! {div{class:"compact-vocal-preview",role:"img",aria_label:"Vocal pitch and lyrics on the recording clock","data-preview-track":"Vocals","data-preview-position":"{position}","data-preview-playing":"{playing}","data-preview-audio":"{audio}","data-preview-revision":"{revision}",
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
    let mut viewport = use_signal(|| (1000., 620.));
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
    let playing = doc.transport.playing && (studio.audio_ready)();
    let audio = doc.audio_sha256.clone();
    let revision = doc.revision;
    if compact {
        return rsx! {CompactVocalPreview{notes,range:range(),lookahead,position,playing,audio,revision}};
    }
    let (width, height) = viewport();
    let plot = FocusedPlot::new(width, height);
    let cursor = 64. + (plot.width - 76.) / (lookahead + 1.);
    rsx! {
        div { class:"focused-vocal-surface","data-preview-track":"Vocals","data-preview-position":"{position}","data-preview-playing":"{playing}","data-preview-audio":"{audio}","data-preview-revision":"{revision}",
            onresize:move |event| if let Ok(size)=event.get_content_box_size() {
                let next=(size.width,size.height);
                if next.0>0. && next.1>0. && viewport()!=next { viewport.set(next); }
            },
            svg { class:"studio-highway vocal-pitch-preview", view_box:"0 0 {plot.width} {plot.height}", role:"img",
                "aria-label":"Vocal draft pitch and lyrics on the recording clock",
                for (midi,label) in [(36,"C2"),(48,"C3"),(60,"C4"),(72,"C5"),(84,"C6")] {
                    {let y=plot.pitch_y(midi);rsx! {
                        line { x1:"64",x2:"{plot.width-12.}",y1:"{y}",y2:"{y}",stroke:"#334353" }
                        text { x:"8",y:"{y+4.}",fill:"#90a7b9",font_size:"10","{label}" }
                    }}
                }
                text { x:"8",y:"{plot.height-20.}",fill:"#90a7b9",font_size:"10","Talkie" }
                line { x1:"{cursor}",x2:"{cursor}",y1:"16",y2:"{plot.height-8.}",stroke:"#eef8ff",stroke_width:"1" }
                for note in &notes {
                    g {
                        rect { x:"{plot.x(note.x)}",y:"{plot.y(note)-4.}",width:"{plot.x(note.x+note.width)-plot.x(note.x)}",height:"8",rx:"3",
                            fill:if note.active{"#fff4bb"}else if note.talkie{"#dbb5f7"}else{"#61d4ca"}
                        }
                        text { x:"{plot.x(note.x)}",y:"{plot.y(note)+16.}",fill:if note.active{"#fff4bb"}else{"#deebf5"},font_size:"12","{note.lyric}" }
                    }
                }
                text { x:"{cursor}",y:"12",fill:"#90a7b9",font_size:"10",text_anchor:"middle","Now" }
            }
        }
        div { class:"vocal-preview-caption","Lead vocal · {chart.notes.len()} notes · draft" }
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
            phrases: notes
                .last()
                .map(|n| {
                    vec![VocalPhrase {
                        midi: None,
                        start: 0,
                        end: n.end,
                        player: 1,
                    }]
                })
                .unwrap_or_default(),
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
            lyric: if pitch.is_some() {
                Syllable::Missing
            } else {
                Syllable::Text {
                    text: "talk".into(),
                    join_next: false,
                }
            },
            midi: None,
        }
    }
    #[test]
    fn compact_range_contains_authored_extremes_and_keeps_talkies_unpitched() {
        let low = chart(vec![note(0, Some(36))]);
        let high = chart(vec![note(0, Some(84))]);
        low.validate().unwrap();
        high.validate().unwrap();
        assert_eq!(pitch_range(&low), Some((30, 42)));
        assert_eq!(pitch_range(&high), Some((78, 90)));
        let both = chart(vec![
            note(0, Some(36)),
            note(480, Some(84)),
            note(960, None),
        ]);
        both.validate().unwrap();
        assert_eq!(pitch_range(&both), Some((34, 86)));
        assert!((8.0..22.0).contains(&compact_pitch(Some(84), pitch_range(&both))));
        assert!((22.0..36.0).contains(&compact_pitch(Some(36), pitch_range(&both))));
        assert_eq!(compact_pitch(None, pitch_range(&both)), 44.);
        assert!(pitch_range(&chart(vec![note(0, None)])).is_none());
        assert_eq!(pitch_name(0), "C-1");
        assert_eq!(pitch_name(60), "C4");
    }
    #[test]
    fn long_vocal_tracks_only_allocate_notes_intersecting_the_source_window() {
        let timing = TempoMap::new(480, 0., vec![Tempo { tick: 0, bpm: 120. }]).unwrap();
        let chart = chart((0..100_000).map(|i| note(i * 480, Some(60))).collect());
        chart.validate().unwrap();
        let notes = visible_notes(&chart, &timing, 20_000., 3.6);
        assert_eq!(notes.len(), 10);
        assert_eq!(notes.first().unwrap().x, LEFT);
        let last = notes.last().unwrap();
        assert!((last.x + last.width - RIGHT).abs() < 0.0001);
        assert_eq!(notes.iter().filter(|n| n.active).count(), 1);
    }
    #[test]
    fn focused_plot_fills_the_actual_viewport_without_scaling_away_its_text() {
        for (width, height) in [(900., 140.), (900., 300.), (300., 500.)] {
            let plot = FocusedPlot::new(width, height);
            assert_eq!((plot.width, plot.height), (width, height));
            assert_eq!(plot.x(LEFT), 64.);
            assert_eq!(plot.x(RIGHT), width - 12.);
            assert_eq!(plot.pitch_y(84), 20.);
            assert_eq!(plot.pitch_y(36), height - 48.);
        }
        let plot = FocusedPlot::new(0., 0.);
        assert_eq!((plot.width, plot.height), (160., 96.));
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
                midi: None,
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
