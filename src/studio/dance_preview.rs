//! Original vector dance noteskin. Motion and feedback follow source audio time.
use super::{Chart, NoteKind, Timing, TimingKind};
use crate::stepmania::timing::Boundary;
use dioxus::prelude::*;
use std::{cell::RefCell, fmt::Write, rc::Rc};

const RECEPTOR: f64 = 82.;
#[cfg(test)]
const HEIGHT: f64 = 560.;
const SPACING: f64 = 42.;
const FLASH: f64 = 0.24;
const ARROW: &str = "M0-30 30 0 19 11 12 4V29H-12V4L-19 11-30 0Z";

fn lane_x(lane: u8) -> f64 {
    80. + f64::from(lane) * 80.
}
fn rotation(lane: u8) -> i32 {
    [-90, 180, 0, 90][lane as usize]
}
fn color(row: i64) -> &'static str {
    for (step, color) in [
        (48, "#ff627d"),
        (24, "#59bcff"),
        (16, "#be8aff"),
        (12, "#f6d66a"),
        (8, "#ef8fdd"),
        (6, "#ffab65"),
        (4, "#69e0dc"),
        (3, "#92df88"),
    ] {
        if row % step == 0 {
            return color;
        }
    }
    "#c6d7ea"
}
#[derive(Clone, PartialEq)]
struct Note {
    row: i64,
    lane: u8,
    kind: NoteKind,
    seconds: f64,
    end_seconds: f64,
    scroll: f64,
    end_scroll: f64,
    judgable: bool,
}
#[derive(Clone, PartialEq)]
struct Speed {
    beat: f64,
    seconds: f64,
    target: f64,
    previous: f64,
    length: f64,
    in_seconds: bool,
}
#[derive(PartialEq)]
struct Field {
    notes: Vec<Note>,
    // A neutral-speed copy lets the existing timing adapter compute scroll
    // integrals once per note, rather than rebuilding them every frame.
    scroll: Timing,
    timing: Timing,
    speeds: Vec<Speed>,
    bpm: f64,
}
impl Field {
    fn new(chart: &Chart, timing: &Timing) -> Self {
        let mut scroll = timing.clone();
        scroll.events.retain(|e| e.kind == TimingKind::Scrolls);
        let mut notes: Vec<_> = chart
            .notes
            .iter()
            .filter(|n| n.lane < 4)
            .map(|n| {
                let beat = n.row as f64 / 48.;
                let end = n.end_row.unwrap_or(n.row) as f64 / 48.;
                Note {
                    row: n.row,
                    lane: n.lane,
                    kind: n.kind,
                    seconds: timing.seconds(beat, Boundary::Note),
                    end_seconds: timing.seconds(end, Boundary::Note),
                    scroll: scroll.display(beat, 0., false),
                    end_scroll: scroll.display(end, 0., false),
                    judgable: timing.judgable(beat)
                        && !matches!(n.kind, NoteKind::Fake | NoteKind::Mine),
                }
            })
            .collect();
        notes.sort_by(|a, b| {
            a.seconds
                .total_cmp(&b.seconds)
                .then(a.row.cmp(&b.row))
                .then(a.lane.cmp(&b.lane))
        });
        let mut events: Vec<_> = timing
            .events
            .iter()
            .filter(|e| e.kind == TimingKind::Speeds)
            .collect();
        events.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        let speeds = events
            .iter()
            .enumerate()
            .map(|(i, e)| Speed {
                beat: e.beat,
                seconds: timing.seconds(e.beat, Boundary::Note),
                target: e.values[0],
                previous: if i == 0 { 1. } else { events[i - 1].values[0] },
                length: e.values[1],
                in_seconds: e.values[2] != 0.,
            })
            .collect();
        Self {
            notes,
            scroll,
            timing: timing.clone(),
            speeds,
            bpm: timing.bpm(0.),
        }
    }
    fn scale(&self, beat: f64, seconds: f64) -> f64 {
        let Some(s) = self.speeds.iter().rev().find(|s| s.beat <= beat) else {
            return 1.;
        };
        let elapsed = if s.in_seconds {
            seconds - s.seconds
        } else {
            beat - s.beat
        };
        let fraction = if s.length == 0. {
            1.
        } else {
            (elapsed / s.length).clamp(0., 1.)
        };
        s.previous + (s.target - s.previous) * fraction
    }
    fn projection(&self, frame: &Frame) -> Projection {
        Projection {
            seconds: frame.seconds,
            cmod: frame.cmod,
            rate: self.bpm / 60.,
            offset: self.scroll.display(0., frame.beat, false),
            scale: self.scale(frame.beat, frame.seconds),
        }
    }
}
struct Projection {
    seconds: f64,
    cmod: bool,
    rate: f64,
    offset: f64,
    scale: f64,
}
impl Projection {
    fn distance(&self, n: &Note, tail: bool) -> f64 {
        if self.cmod {
            ((if tail { n.end_seconds } else { n.seconds }) - self.seconds) * self.rate
        } else {
            ((if tail { n.end_scroll } else { n.scroll }) + self.offset) * self.scale
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
    lane: u8,
    at: f64,
    color: &'static str,
}
#[derive(Clone, Default, PartialEq)]
struct Feedback {
    identity: String,
    position: Option<f64>,
    playing: bool,
    hits: Vec<Hit>,
}
impl Feedback {
    fn advance(&self, identity: &str, seconds: f64, playing: bool, notes: &[Note]) -> Self {
        let mut next = Self {
            identity: identity.into(),
            position: seconds.is_finite().then_some(seconds),
            playing,
            hits: Vec::new(),
        };
        if !playing || !seconds.is_finite() {
            return next;
        }
        // Starting at sample zero is the only attack without an earlier frame.
        if !self.playing && (0.0..=0.05).contains(&seconds) {
            next.hits.extend(
                notes
                    .iter()
                    .take_while(|n| n.seconds == 0.)
                    .filter(|n| n.judgable)
                    .map(|n| Hit {
                        lane: n.lane,
                        at: 0.,
                        color: color(n.row),
                    }),
            );
        }
        let Some(before) = self.position else {
            return next;
        };
        if !self.playing || self.identity != identity || !(0.0..=0.3).contains(&(seconds - before))
        {
            return next;
        }
        next.hits.extend(
            self.hits
                .iter()
                .copied()
                .filter(|h| (0.0..FLASH).contains(&(seconds - h.at))),
        );
        let first = notes.partition_point(|n| n.seconds <= before);
        next.hits.extend(
            notes[first..]
                .iter()
                .take_while(|n| n.seconds <= seconds)
                .filter(|n| n.judgable)
                .map(|n| Hit {
                    lane: n.lane,
                    at: n.seconds,
                    color: color(n.row),
                }),
        );
        // Restart each receptor's burst on its newest attack. Dense passages
        // should not accumulate unbounded overlapping particle layers.
        let mut lanes = [false; 4];
        next.hits = next
            .hits
            .into_iter()
            .rev()
            .filter(|h| {
                let fresh = !lanes[h.lane as usize];
                lanes[h.lane as usize] = true;
                fresh
            })
            .collect();
        next
    }
}
struct Frame {
    seconds: f64,
    beat: f64,
    playing: bool,
    cmod: bool,
    height: f64,
}

#[component]
pub fn Preview(
    chart: Memo<Option<Chart>>,
    timing: Memo<Timing>,
    beat: f64,
    seconds: f64,
    playing: bool,
    cmod: bool,
    identity: String,
    zoom: f64,
) -> Element {
    let mut field_size = use_signal(|| (400_f64, 320_f64));
    let width = (360. * zoom).min(field_size().0.max(1.));
    let height = (field_size().1 * 400. / width).clamp(120., 1200.);
    let field = use_memo(move || chart.read().as_ref().map(|c| Field::new(c, &timing.read())));
    let history = use_hook(|| Rc::new(RefCell::new(Feedback::default())));
    let feedback = use_memo(use_reactive(
        (&identity, &seconds, &playing),
        move |(identity, seconds, playing)| {
            let field = field.read();
            let next = history.borrow().advance(
                &identity,
                seconds,
                playing,
                field.as_ref().map_or(&[], |f| f.notes.as_slice()),
            );
            *history.borrow_mut() = next.clone();
            next
        },
    ));
    let frame = Frame {
        seconds,
        beat,
        playing,
        cmod,
        height,
    };
    let geometry = field
        .read()
        .as_ref()
        .map_or(String::new(), |f| render(f, &frame, &feedback.read()));
    rsx! {div{class:"dance-field-viewport",onresize:move|event|if let Ok(size)=event.get_content_box_size(){field_size.set((size.width,size.height));},
        svg{class:"studio-highway dance-note-field",style:"width:{width}px;height:100%;flex:none",view_box:"0 0 400 {height}",preserve_aspect_ratio:"xMidYMid meet",role:"img",
        "aria-label":"Dance arrow field with beat-colored notes, holds and playback feedback",
        "data-dance-position":"{seconds}","data-dance-playing":"{playing}","data-dance-hits":"{feedback.read().hits.len()}",
        dangerous_inner_html:"{geometry}"
    }}}
}

fn arrow(out: &mut String, lane: u8, x: f64, y: f64, tint: &str, scale: f64, hollow: bool) {
    let _ = write!(
        out,
        "<g transform='translate({x:.3} {y:.3}) rotate({}) scale({scale:.3})' color='{tint}'>",
        rotation(lane)
    );
    if hollow {
        let _ = write!(
            out,
            "<path d='{ARROW}' fill='#0b111a' stroke='currentColor' stroke-width='3'/><path d='{ARROW}' transform='scale(.76)' fill='none' stroke='currentColor' stroke-width='1' opacity='.5'/>"
        );
    } else {
        let _ = write!(
            out,
            "<path d='{ARROW}' transform='translate(0 3)' fill='#000' opacity='.7'/><path d='{ARROW}' fill='url(#dance-bevel)' stroke='#03080d' stroke-width='2'/><path d='{ARROW}' transform='scale(.85)' fill='currentColor' stroke='currentColor' stroke-width='1'/><path d='{ARROW}' transform='scale(.72)' fill='url(#dance-sheen)'/><path d='M-21-1 0-22 21-1M-6 4V21' fill='none' stroke='#fff' stroke-opacity='.6' stroke-width='2' stroke-linecap='round'/>"
        );
    }
    out.push_str("</g>");
}
fn render(field: &Field, frame: &Frame, feedback: &Feedback) -> String {
    let mut out = String::from(DEFINITIONS);
    let height = frame.height;
    let _ = write!(
        out,
        "<defs><clipPath id='dance-clip'><rect x='31' y='6' width='338' height='{}' rx='13'/></clipPath></defs>",
        height - 12.
    );
    // The field is straight like a dance notefield, with YACE's metal, slate
    // and blue accents. There are no borrowed game textures or Lua execution.
    let _ = write!(
        out,
        "<rect x='30' y='5' width='340' height='{}' rx='14' fill='url(#dance-field)' stroke='#3e5268'/><path d='M43 6H357M43 {}H357' stroke='#95b9d7' stroke-opacity='.28'/><g clip-path='url(#dance-clip)'>",
        height - 10.,
        height - 6.
    );
    for lane in 0u8..4 {
        let x = lane_x(lane);
        let _ = write!(
            out,
            "<rect x='{}' y='7' width='78' height='{}' fill='{}' fill-opacity='.32'/><path d='M{} 6V{}' stroke='#9bbad6' stroke-opacity='.09'/>",
            x - 39.,
            height - 14.,
            if lane % 2 == 0 { "#111f30" } else { "#172a3c" },
            x - 40.,
            height - 6.
        );
    }
    let projection = field.projection(frame);
    let y_note = |n: &Note, tail| RECEPTOR + projection.distance(n, tail) * SPACING;
    for i in -24..48 {
        let b = frame.beat.floor() + f64::from(i);
        let distance = if frame.cmod {
            (field.timing.seconds(b, Boundary::Note) - frame.seconds) * field.bpm / 60.
        } else {
            (field.scroll.display(b, 0., false) + projection.offset) * projection.scale
        };
        let y = RECEPTOR + distance * SPACING;
        if (RECEPTOR + 30. ..height).contains(&y) {
            let _ = write!(
                out,
                "<path d='M41 {y:.3}H359' stroke='#8fb8d7' stroke-opacity='{}' stroke-width='{}'/>",
                if (b as i64) % 4 == 0 { ".22" } else { ".07" },
                if (b as i64) % 4 == 0 { "1.4" } else { ".7" }
            );
        }
    }
    out.push_str("<rect x='32' y='8' width='336' height='114' fill='url(#dance-target-shade)'/><path d='M40 82H360' stroke='#88b9e0' stroke-opacity='.18'/>");
    // Sustains are drawn before the heads, and remain visible after their head
    // has crossed the receptors. A long held note cannot disappear at culling.
    let mut visible = Vec::new();
    let mut held = [false; 4];
    for n in &field.notes {
        if n.seconds < frame.seconds - 0.025
            && (!n.kind.sustained() || n.end_seconds < frame.seconds)
        {
            continue;
        }
        let mut head = y_note(n, false);
        let tail = y_note(n, true);
        let sustaining = n.kind.sustained()
            && n.judgable
            && n.seconds <= frame.seconds
            && frame.seconds < n.end_seconds;
        if sustaining {
            head = RECEPTOR;
            held[n.lane as usize] = frame.playing;
        }
        if !(-40. ..height + 40.).contains(&head)
            && (!n.kind.sustained() || head.min(tail) > height + 40. || head.max(tail) < -40.)
        {
            continue;
        }
        if visible.len() >= 2000 {
            break;
        }
        if n.kind.sustained() {
            let x = lane_x(n.lane);
            let top = head.min(tail).clamp(-40., height + 40.);
            let bottom = head.max(tail).clamp(-40., height + 40.);
            let tint = if n.kind == NoteKind::Roll {
                "#c88aff"
            } else {
                "#f1cf76"
            };
            let opacity = if sustaining && frame.playing {
                1.
            } else {
                0.72
            };
            let _ = write!(
                out,
                "<g data-kind='{}' opacity='{opacity}'><rect x='{}' y='{top:.3}' width='24' height='{:.3}' rx='7' fill='#090f18' stroke='{tint}' stroke-width='2'/><rect x='{}' y='{top:.3}' width='16' height='{:.3}' rx='4' fill='{}' opacity='.75'/><path d='M{x} {top:.3}V{bottom:.3}' stroke='{tint}' stroke-width='3' opacity='.9'/>",
                if n.kind == NoteKind::Roll {
                    "roll-body"
                } else {
                    "hold-body"
                },
                x - 12.,
                (bottom - top).max(1.),
                x - 8.,
                (bottom - top).max(1.),
                if n.kind == NoteKind::Roll {
                    "url(#dance-roll)"
                } else {
                    "url(#dance-hold)"
                }
            );
            if n.kind == NoteKind::Roll {
                let phase = (frame.seconds * 45.).rem_euclid(16.);
                let _ = write!(
                    out,
                    "<path d='M{x} {top:.3}V{bottom:.3}' stroke='#f2dcff' stroke-width='13' stroke-dasharray='2 14' stroke-dashoffset='{phase:.3}' opacity='.65'/>"
                );
            }
            let _ = write!(
                out,
                "<rect data-kind='sustain-tail' x='{}' y='{:.3}' width='26' height='9' rx='4' fill='{tint}' stroke='#fff' stroke-opacity='.6'/></g>",
                x - 13.,
                tail.clamp(-40., height + 40.) - 4.5
            );
        }
        visible.push((n, head));
    }
    for lane in 0u8..4 {
        let x = lane_x(lane);
        let hit = feedback.hits.iter().rev().find(|h| h.lane == lane);
        let phase = hit.map_or(1., |h| ((frame.seconds - h.at) / FLASH).clamp(0., 1.));
        let active = held[lane as usize];
        let tint = hit.map_or(if active { "#f1d78c" } else { "#93a6bb" }, |h| h.color);
        let pulse = if frame.playing {
            (1. - frame.beat.rem_euclid(1.)) * 0.08
        } else {
            0.
        };
        let intensity = if active { 0.65 } else { (1. - phase).powi(2) };
        let _ = write!(
            out,
            "<g data-kind='receptor' data-lane='{lane}'><ellipse class='dance-beat-aura' cx='{x}' cy='82' rx='35' ry='34' fill='#7baed7' opacity='{:.3}'/>",
            pulse
        );
        if intensity > 0.001 {
            let _ = write!(
                out,
                "<rect data-kind='{}' x='{}' y='78' width='70' height='210' fill='url(#dance-held-glow)' opacity='{intensity:.3}'/>",
                if active { "held-lane" } else { "receptor-glow" },
                x - 35.
            );
        }
        arrow(&mut out, lane, x, RECEPTOR, "#54677b", 1.05, true);
        let _ = write!(
            out,
            "<g transform='translate({x} 82)'><g class='dance-receptor-pulse' transform='scale({:.3})'>",
            if active {
                1.04
            } else {
                1. - 0.17 * (1. - phase)
            }
        );
        arrow(&mut out, lane, 0., 0., tint, 0.79, false);
        out.push_str("</g></g></g>");
    }
    for (n, y) in visible {
        let x = lane_x(n.lane);
        let tint = if !n.judgable && !matches!(n.kind, NoteKind::Mine) {
            "#8195ab"
        } else {
            color(n.row)
        };
        let opacity = if n.kind == NoteKind::Fake || !n.judgable && n.kind != NoteKind::Mine {
            0.38
        } else {
            1.
        };
        let _ = write!(
            out,
            "<g data-kind='dance-note' data-lane='{}' data-row='{}' opacity='{opacity}'>",
            n.lane, n.row
        );
        if n.kind == NoteKind::Mine {
            let angle = frame.beat * 40.;
            let _ = write!(
                out,
                "<g data-kind='mine' transform='translate({x} {y:.3}) rotate({angle:.3})'><path d='M0-26 7-17 18-18 17-7 26 0 17 7 18 18 7 17 0 26-7 17-18 18-17 7-26 0-17-7-18-18-7-17Z' fill='#3a202d' stroke='#ff7d8e' stroke-width='2'/><circle r='14' fill='#111823' stroke='#f3a17c' stroke-width='2'/><path d='M-5-5 5 5M-5 5 5-5' stroke='#ffb8bb' stroke-width='3'/></g>"
            );
        } else {
            arrow(&mut out, n.lane, x, y, tint, 0.9, n.kind == NoteKind::Lift);
            if n.kind == NoteKind::Lift {
                let _ = write!(
                    out,
                    "<circle data-kind='lift' cx='{x}' cy='{y:.3}' r='33' fill='none' stroke='{tint}' stroke-width='1.5' stroke-dasharray='3 5'/>"
                );
            }
            if n.kind == NoteKind::Fake {
                let _ = write!(
                    out,
                    "<path data-kind='fake' d='M{} {:.3}L{} {:.3}' stroke='#a8b5c6' stroke-width='2'/>",
                    x - 21.,
                    y + 22.,
                    x + 21.,
                    y - 22.
                );
            }
        }
        out.push_str("</g>");
    }
    for hit in &feedback.hits {
        let p = ((frame.seconds - hit.at) / FLASH).clamp(0., 1.);
        let x = lane_x(hit.lane);
        let alpha = (1. - p).powi(2);
        let _ = write!(
            out,
            "<g class='dance-hit' data-kind='autoplay-flash' data-lane='{}' opacity='{alpha:.3}' color='{}'><circle cx='{x}' cy='82' r='{:.3}' fill='none' stroke='currentColor' stroke-width='{:.3}'/><circle cx='{x}' cy='82' r='{:.3}' fill='currentColor' opacity='.2' filter='url(#dance-bloom)'/>",
            hit.lane,
            hit.color,
            25. + p * 28.,
            3. - p * 2.,
            21. + p * 15.
        );
        for i in 0..8 {
            let angle = std::f64::consts::TAU * f64::from(i) / 8.;
            let r = 30. + p * 37.;
            let _ = write!(
                out,
                "<path d='M{:.3} {:.3}l{:.3} {:.3}' stroke='currentColor' stroke-width='2' stroke-linecap='round'/>",
                x + angle.cos() * r,
                RECEPTOR + angle.sin() * r,
                angle.cos() * 7. * (1. - p),
                angle.sin() * 7. * (1. - p)
            );
        }
        out.push_str("</g>");
    }
    let _ = write!(
        out,
        "</g><rect x='30' y='5' width='340' height='{}' rx='14' fill='none' stroke='#58728c' stroke-opacity='.5'/>",
        height - 10.
    );
    out
}

const DEFINITIONS: &str = r#"<defs>
<linearGradient id='dance-field' x2='0' y2='1'><stop stop-color='#111d2a'/><stop offset='.45' stop-color='#0a101a'/><stop offset='1' stop-color='#101c2a'/></linearGradient>
<linearGradient id='dance-target-shade' x2='0' y2='1'><stop stop-color='#050b12' stop-opacity='.9'/><stop offset='1' stop-color='#050b12' stop-opacity='0'/></linearGradient>
<linearGradient id='dance-bevel' x2='.3' y2='1'><stop stop-color='#f0f8ff'/><stop offset='.3' stop-color='#95adc2'/><stop offset='.52' stop-color='#3b5269'/><stop offset='.7' stop-color='#182c41'/><stop offset='1' stop-color='#6c879f'/></linearGradient>
<linearGradient id='dance-sheen' x2='.2' y2='1'><stop stop-color='#fff' stop-opacity='.85'/><stop offset='.42' stop-color='#fff' stop-opacity='.08'/><stop offset='.46' stop-color='#000' stop-opacity='.12'/><stop offset='1' stop-color='#000' stop-opacity='.5'/></linearGradient>
<linearGradient id='dance-held-glow' x2='0' y2='1'><stop stop-color='#9fd8ff' stop-opacity='.5'/><stop offset='1' stop-color='#72b0ee' stop-opacity='0'/></linearGradient>
<pattern id='dance-hold' width='16' height='20' patternUnits='userSpaceOnUse'><rect width='16' height='20' fill='#b3984f'/><path d='m-4 0 12 12L20 0M-4 10 8 22 20 10' fill='none' stroke='#ffeaa6' stroke-opacity='.4' stroke-width='2'/></pattern>
<pattern id='dance-roll' width='16' height='16' patternUnits='userSpaceOnUse'><rect width='16' height='16' fill='#6c448d'/><path d='M-4 4 4-4M0 16 16 0M12 20 20 12' stroke='#e4b5ff' stroke-width='5'/></pattern>
<filter id='dance-bloom' x='-.6' y='-.6' width='2.2' height='2.2'><feGaussianBlur stdDeviation='3'/></filter>
</defs>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stepmania::{Document, Event, Format};
    fn chart() -> Chart {
        Document::parse(
            "#BPMS:0=120;#NOTES:dance-single::Hard:7::1000\n0200\n0000\n0300;".into(),
            Format::Sm,
        )
        .unwrap()
        .charts
        .remove(0)
    }
    fn frame(seconds: f64) -> Frame {
        Frame {
            seconds,
            beat: seconds * 2.,
            playing: true,
            cmod: false,
            height: HEIGHT,
        }
    }
    #[test]
    fn subdivision_colors_and_four_rotations_are_distinct() {
        assert_ne!(color(48), color(24));
        assert_ne!(color(24), color(12));
        assert_eq!(color(0), color(96));
        assert_eq!(
            [rotation(0), rotation(1), rotation(2), rotation(3)],
            [-90, 180, 0, 90]
        );
    }
    #[test]
    fn note_types_have_distinct_art_and_mines_and_fakes_never_trigger_hits() {
        let mut c = chart();
        c.notes[0].kind = NoteKind::Mine;
        c.notes[1].kind = NoteKind::Roll;
        for (lane, row, kind) in [
            (2, 96, NoteKind::Lift),
            (3, 0, NoteKind::Fake),
            (7, 0, NoteKind::Fake),
        ] {
            c.notes.push(crate::stepmania::Note {
                id: format!("fixture-{lane}"),
                row,
                lane,
                kind,
                end_row: None,
            });
        }
        let f = Field::new(&c, &Timing::default());
        assert_eq!(f.notes.len(), 4);
        assert!(
            Feedback::default()
                .advance("a", 0., true, &f.notes)
                .hits
                .is_empty()
        );
        let svg = render(&f, &frame(0.), &Feedback::default());
        for kind in ["mine", "roll-body", "lift", "fake"] {
            assert!(svg.contains(&format!("data-kind='{kind}'")));
        }
        assert!(
            !svg.contains("<text"),
            "The noteskin uses vector geometry, not platform font arrows"
        );
    }
    #[test]
    fn preview_crossings_reset_on_seek_pause_revision_and_loop_without_fake_hits() {
        let field = Field::new(&chart(), &Timing::default());
        let started = Feedback::default().advance("a", 0., true, &field.notes);
        assert_eq!(started.hits.len(), 1);
        let before = started.advance("a", 0.4, true, &field.notes);
        let hit = before.advance("a", 0.51, true, &field.notes);
        assert_eq!(hit.hits.len(), 1);
        assert_eq!(hit.hits[0].lane, 1);
        assert!(hit.advance("a", 0.51, false, &field.notes).hits.is_empty());
        assert!(hit.advance("a", 0.1, true, &field.notes).hits.is_empty());
        assert!(hit.advance("b", 0.55, true, &field.notes).hits.is_empty());
        assert!(hit.advance("a", 2., true, &field.notes).hits.is_empty());
        assert_eq!(hit.advance("a", 0.51, true, &field.notes).hits.len(), 1);
    }
    #[test]
    fn dense_passages_restart_each_receptor_instead_of_stacking_bursts() {
        let field = Field::new(&chart(), &Timing::default());
        let notes: Vec<_> = (0..100)
            .map(|i| {
                let mut n = field.notes[0].clone();
                n.seconds = f64::from(i) * 0.001;
                n.end_seconds = n.seconds;
                n.lane = (i % 4) as u8;
                n.row = i64::from(i);
                n
            })
            .collect();
        let start = Feedback::default().advance("a", 0., true, &notes);
        let end = start.advance("a", 0.11, true, &notes);
        assert_eq!(end.hits.len(), 4);
        assert!(end.hits.iter().all(|h| h.at > 0.09));
    }
    #[test]
    fn held_bodies_survive_head_culling_and_pause_does_not_emit_feedback() {
        let mut c = chart();
        c.notes[1].end_row = Some(1920);
        let field = Field::new(&c, &Timing::default());
        let svg = render(&field, &frame(8.), &Feedback::default());
        assert!(svg.contains("data-kind='hold-body'"));
        assert!(svg.contains("data-kind='held-lane'"));
        assert_eq!(svg.matches("data-kind='receptor'").count(), 4);
        assert!(!svg.contains("data-kind='autoplay-flash'"));
        let paused = render(
            &field,
            &Frame {
                playing: false,
                ..frame(8.)
            },
            &Feedback::default(),
        );
        assert!(paused.contains("data-kind='hold-body'"));
        assert!(!paused.contains("data-kind='held-lane'"));
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
    #[test]
    fn cached_display_matches_the_timing_adapter_through_stops_reverse_and_speed_transitions() {
        let mut timing = Timing::default();
        timing.events.extend([
            Event::value(TimingKind::Stops, 1., 1.),
            Event::value(TimingKind::Scrolls, 2., -1.),
            Event::value(TimingKind::Scrolls, 4., 0.),
            Event {
                kind: TimingKind::Speeds,
                beat: 1.,
                values: vec![2., 2., 1.],
                text: String::new(),
            },
            Event::value(TimingKind::Bpms, 5., 180.),
            Event {
                kind: TimingKind::Speeds,
                beat: 6.,
                values: vec![0.5, 2., 0.],
                text: String::new(),
            },
            Event::value(TimingKind::Warps, 8., 2.),
        ]);
        let mut c = chart();
        c.notes.push(crate::stepmania::Note {
            id: "warp".into(),
            row: 432,
            lane: 2,
            kind: NoteKind::Tap,
            end_row: None,
        });
        let field = Field::new(&c, &timing);
        assert!(!field.notes.iter().find(|n| n.row == 432).unwrap().judgable);
        for seconds in [0., 0.5, 0.8, 1.5, 2., 4., 6., 7., 8.] {
            let beat = match timing.position(seconds) {
                crate::stepmania::timing::AudioPosition::Beat(b)
                | crate::stepmania::timing::AudioPosition::Stop { beat: b }
                | crate::stepmania::timing::AudioPosition::Delay { beat: b } => b,
                crate::stepmania::timing::AudioPosition::Warp { to, .. } => to,
            };
            for cmod in [false, true] {
                let frame = Frame {
                    seconds,
                    beat,
                    cmod,
                    playing: true,
                    height: HEIGHT,
                };
                for n in &field.notes {
                    assert!(
                        (field.projection(&frame).distance(n, false)
                            - timing.display_at(n.row as f64 / 48., beat, seconds, cmod))
                        .abs()
                            < 1e-8
                    );
                    let source = c
                        .notes
                        .iter()
                        .find(|note| note.row == n.row && note.lane == n.lane)
                        .unwrap();
                    assert!(
                        (field.projection(&frame).distance(n, true)
                            - timing.display_at(
                                source.end_row.unwrap_or(source.row) as f64 / 48.,
                                beat,
                                seconds,
                                cmod
                            ))
                        .abs()
                            < 1e-8
                    );
                }
            }
        }
    }
}
