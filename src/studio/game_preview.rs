//! Game-style audition preview using the editor's recording clock.
//!
//! Instrument semantics were checked against OCTAVE's NoteGem, Highway and
//! Strikeline source. All SVG paths, materials and effects here are original
//! vector artwork; no YARG meshes, textures, shader code or geometry is copied.
//! Autoplay animation is an editing aid, never a score or controller judgment.
use super::state::Studio;
use crate::{
    editor::Project,
    gameplay_view::{self, DrumMode, Dynamics, GemShape, Policy, Technique},
    timing::TempoMap,
};
use anyhow::Result;
use dioxus::prelude::*;
use std::{cell::RefCell, fmt::Write, rc::Rc};

const STRIKE_Y: f64 = 535.0;
const FAR_Y: f64 = 66.0;
const CENTER_X: f64 = 500.0;
const HALF_WIDTH: f64 = 274.0;
const FLASH_SECONDS: f64 = 0.20;
const MAX_CONTIGUOUS_STEP: f64 = 0.30;
const DEFAULT_LOOKAHEAD: f64 = 3.6;

#[derive(Clone, Debug, PartialEq)]
struct PreviewNote {
    start: f64,
    end: f64,
    lane: u8,
    cymbal: bool,
    selected: bool,
    technique: Option<Technique>,
    dynamics: Dynamics,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StrikeFlash {
    lane: u8,
    at: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Animation {
    track: String,
    revision: u64,
    position: Option<f64>,
    playing: bool,
    flashes: Vec<StrikeFlash>,
}

/// Forward audio-clock crossings generate visual feedback once. Seeks,
/// pauses, track/revision changes and long UI stalls prime a new baseline.
fn advance_animation(
    previous: &Animation,
    track: &str,
    revision: u64,
    position: f64,
    playing: bool,
    notes: &[PreviewNote],
) -> Animation {
    let mut next = Animation {
        track: track.into(),
        revision,
        position: position.is_finite().then_some(position),
        playing,
        flashes: Vec::new(),
    };
    // A chart may begin on the first audio sample. There is no earlier frame
    // to cross that attack, so prime it once when starting at the beginning.
    if playing && !previous.playing && (0.0..=0.05).contains(&position) {
        next.flashes = notes
            .iter()
            .take_while(|note| note.start == 0.0)
            .map(|note| StrikeFlash {
                lane: note.lane,
                at: 0.0,
            })
            .collect();
    }
    let Some(before) = previous.position else {
        return next;
    };
    let delta = position - before;
    if !playing
        || !previous.playing
        || previous.track != track
        || previous.revision != revision
        || !position.is_finite()
        || !(0.0..=MAX_CONTIGUOUS_STEP).contains(&delta)
    {
        return next;
    }
    next.flashes = previous
        .flashes
        .iter()
        .copied()
        .filter(|flash| (0.0..FLASH_SECONDS).contains(&(position - flash.at)))
        .collect();
    let start = notes.partition_point(|note| note.start <= before);
    for note in notes[start..]
        .iter()
        .take_while(|note| note.start <= position)
    {
        next.flashes.push(StrikeFlash {
            lane: note.lane,
            at: note.start,
        });
    }
    next
}

fn scene_notes(doc: &Project, timing: &TempoMap, policy: &Policy) -> Result<Vec<PreviewNote>> {
    let selected = doc
        .selected_note
        .as_ref()
        .and_then(|id| doc.find_note(id))
        .map(|(_, item)| (item.note.tick, item.note.lane));
    // Technique depends on the preceding attack, which may be outside the
    // viewport. Always classify the complete raw track before any culling.
    let Some(track) = doc.content.tracks.get(&doc.selected_track) else {
        return Ok(Vec::new());
    };
    let raw: Vec<_> = track.iter().map(|item| item.note.clone()).collect();
    Ok(
        gameplay_view::classify(&doc.selected_track, &raw, timing, policy)?
            .into_iter()
            .map(|gem| PreviewNote {
                start: gem.seconds,
                end: gem.end_seconds,
                lane: gem.lane,
                cymbal: gem.shape == GemShape::Cymbal,
                selected: selected == Some((gem.tick, gem.lane)),
                technique: gem.technique,
                dynamics: gem.dynamics,
            })
            .collect(),
    )
}

fn preview_policy(studio: Studio, timing: &TempoMap, pro_drums: bool) -> (Policy, bool) {
    let mut unknown = false;
    let mut policy = gameplay_view::yarg_chart_policy(timing.resolution, &studio.metadata.read())
        .unwrap_or_else(|_| {
            unknown = true;
            Policy::unknown()
        });
    policy.drums = if pro_drums {
        DrumMode::Pro
    } else {
        DrumMode::Standard
    };
    (policy, unknown)
}

#[component]
pub fn Preview(studio: Studio) -> Element {
    let mut pro_drums = use_signal(|| true);
    let mut lookahead = use_signal(|| DEFAULT_LOOKAHEAD);
    let animation_history = use_hook(|| Rc::new(RefCell::new(Animation::default())));
    let classified = use_memo(
        move || -> std::result::Result<(Vec<PreviewNote>, bool), String> {
            let Some(doc) = studio.snapshot() else {
                return Ok((Vec::new(), false));
            };
            let Some(timing) = (studio.timing)() else {
                return Err("Chart timing unavailable".into());
            };
            let (policy, unknown) = preview_policy(studio, &timing, pro_drums());
            scene_notes(&doc, &timing, &policy)
                .map(|notes| (notes, unknown))
                .map_err(|error| format!("Cannot interpret this track: {error:#}"))
        },
    );
    let animation = use_memo(move || {
        let document = studio.doc.read();
        let Some(doc) = document.as_ref() else {
            return Animation::default();
        };
        let mut previous = animation_history.borrow_mut();
        let classified = classified.read();
        let notes = classified
            .as_ref()
            .map_or(&[][..], |(notes, _)| notes.as_slice());
        let next = advance_animation(
            &previous,
            &format!("{}:{}", doc.audio_sha256, doc.selected_track),
            doc.revision,
            (studio.playhead)(),
            doc.transport.playing && (studio.audio_ready)(),
            notes,
        );
        *previous = next.clone();
        next
    });
    let doc = studio.doc.read();
    let track = doc
        .as_ref()
        .map_or("ExpertDrums", |doc| doc.selected_track.as_str())
        .to_owned();
    let drums = track.ends_with("Drums");
    let position = (studio.playhead)();
    let unknown_policy = classified
        .read()
        .as_ref()
        .is_ok_and(|(_, unknown)| *unknown);
    let classification_error = classified.read().as_ref().err().cloned();
    let geometry = doc
        .as_ref()
        .and_then(|doc| {
            let timing = (studio.timing)()?;
            let classified = classified.read();
            let (notes, _) = classified.as_ref().ok()?;
            let beats = beat_lines(&timing, position, lookahead());
            Some(render_highway(&Scene {
                notes,
                beats: &beats,
                flashes: &animation().flashes,
                position,
                lookahead: lookahead(),
                drums,
                pro_piano: crate::keys::is_pro(&track),
                pro_drums: pro_drums(),
                playing: doc.transport.playing && (studio.audio_ready)(),
            }))
        })
        .unwrap_or_default();
    let telemetry_audio = doc.as_ref().map_or("", |doc| doc.audio_sha256.as_str());
    let telemetry_revision = doc.as_ref().map_or(0, |doc| doc.revision);
    let telemetry_playing =
        doc.as_ref().is_some_and(|doc| doc.transport.playing) && (studio.audio_ready)();
    rsx! {
        style { "{PREVIEW_STYLE}" }
        section { class: "audition-view", "data-preview-position":"{position}", "data-preview-playing":"{telemetry_playing}", "data-preview-revision":"{telemetry_revision}", "data-preview-audio":"{telemetry_audio}",
            header { class: "desk-heading audition-heading",
                h2 { "Audition" }
                span { class: "audition-track", "{super::track_picker::title(&track)}" }
                span { class: "audition-mode", "Autoplay" }
            }
            div { class: "audition-surface",
                if track == "Vocals" {
                    super::vocal_preview::VocalPreview { studio, lookahead: lookahead() }
                } else if doc.is_some() {
                    svg { class: "studio-highway", view_box: "0 0 1000 620", preserve_aspect_ratio: "xMidYMid meet", role: "img",
                        "aria-label": "Autoplay editing preview with instrument notes; no scoring or controller judgment",
                        dangerous_inner_html: "{geometry}"
                    }
                    if (studio.audio_stalled)() && telemetry_playing { div { class: "game-preview-policy-warning", "Audio position stalled · preview held" } }
                    if unknown_policy && !drums && !crate::keys::is_keys(&track) { div { class: "game-preview-policy-warning", "HOPO policy unknown · neutral note caps" } }
                    if let Some(error) = classification_error { div { class: "game-preview-policy-warning", "{error}" } }
                } else {
                    div { class: "studio-empty-preview",
                        div { class: "empty-state-title", "No song selected" }
                        div { "Open a song folder to preview its chart" }
                        button { onclick: move |_| studio.create_dialog(), "Open song folder" }
                        button { onclick: move |_| studio.open_dialog(), "Open saved project" }
                    }
                }
            }
            super::practice::PracticeControls { studio,
              super::stems::StemControls { studio }
            div { class: "audition-settings",
                if drums {
                    label { class: "form-check",
                        input { r#type: "checkbox", checked: pro_drums(), onkeydown: move |event| event.stop_propagation(), onchange: move |event| pro_drums.set(event.checked()) }
                        span { "Pro Drums" }
                    }
                }
                label { class: "game-preview-lookahead",
                    span { "Look ahead" }
                    select { value: "{lookahead}", onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if let Ok(value) = event.value().parse::<f64>() { lookahead.set(value.clamp(2.0, 6.0)); } },
                        for seconds in [2.0, 2.8, 3.6, 4.5, 6.0] { option { value: "{seconds}", selected: seconds == lookahead(), "{seconds:.1}s" } }
                    }
                }
            }
            }
        }
    }
}

const PREVIEW_STYLE: &str = include_str!("../../assets/game-preview.css");

/// Perspective of a straight fretboard. Far notes converge while event time
/// remains the same recording time used by the waveform and audio player.
fn project(fraction: f64, seconds_ahead: f64, lookahead: f64) -> (f64, f64, f64) {
    let scale = 1.0 / (1.0 + 4.6 * seconds_ahead / lookahead);
    let far_scale = 1.0 / 5.6;
    let depth = (scale - far_scale) / (1.0 - far_scale);
    (
        CENTER_X + (fraction * 2.0 - 1.0) * HALF_WIDTH * scale,
        FAR_Y + (STRIKE_Y - FAR_Y) * depth,
        scale,
    )
}

fn beat_lines(timing: &TempoMap, position: f64, lookahead: f64) -> Vec<f64> {
    if timing.resolution == 0 || !position.is_finite() || !lookahead.is_finite() || lookahead <= 0.0
    {
        return Vec::new();
    }
    let first = timing.tick(position.max(timing.offset)).unwrap_or(0) / timing.resolution as u64;
    (first..first.saturating_add(128))
        .filter_map(|beat| beat.checked_mul(timing.resolution as u64))
        .map(|tick| timing.seconds(tick))
        .filter(|seconds| *seconds >= position && *seconds <= position + lookahead)
        .collect()
}

struct Scene<'a> {
    notes: &'a [PreviewNote],
    beats: &'a [f64],
    flashes: &'a [StrikeFlash],
    position: f64,
    lookahead: f64,
    drums: bool,
    pro_piano: bool,
    pro_drums: bool,
    playing: bool,
}

fn lane_color(drums: bool, lane: u8) -> &'static str {
    if drums {
        match lane {
            0 | 32 => "#ffa63f",
            1 => "#f34f60",
            2 => "#ffdf54",
            3 => "#46b0ff",
            _ => "#52e475",
        }
    } else {
        match lane {
            0 => "#52e475",
            1 => "#f34f60",
            2 => "#ffdf54",
            3 => "#46b0ff",
            4 => "#ffa63f",
            _ => "#c889ff",
        }
    }
}

fn piano_color(pitch: u8) -> &'static str {
    if matches!(pitch % 12, 1 | 3 | 6 | 8 | 10) {
        "#76b9e9"
    } else {
        "#e9eff5"
    }
}

/// Fixed full keyboard for authoring: every playable pitch is visible together.
/// YARG's gameplay range-shift camera is not simulated by this editing preview.
fn render_piano_highway(scene: &Scene<'_>) -> String {
    let mut out = String::from(DEFINITIONS);
    let (left, far_y, _) = project(0.0, scene.lookahead, scene.lookahead);
    let (right, _, _) = project(1.0, scene.lookahead, scene.lookahead);
    let _ = write!(
        out,
        "<path d='M{left:.2} {far_y:.2}H{right:.2}L{:.2} {STRIKE_Y}H{:.2}Z' fill='url(#game-board)' stroke='#657382'/>",
        CENTER_X + HALF_WIDTH,
        CENTER_X - HALF_WIDTH
    );
    for index in 0..=25 {
        let fraction = index as f64 / 25.0;
        let (x, y, _) = project(fraction, scene.lookahead, scene.lookahead);
        let (end, _, _) = project(fraction, 0.0, scene.lookahead);
        let _ = write!(
            out,
            "<path d='M{x:.2} {y:.2}L{end:.2} {STRIKE_Y}' stroke='#71889a' stroke-opacity='.35'/>"
        );
    }
    for seconds in scene.beats {
        let (x, y, _) = project(0.0, seconds - scene.position, scene.lookahead);
        let (end, _, _) = project(1.0, seconds - scene.position, scene.lookahead);
        let _ = write!(
            out,
            "<path data-kind='beat' d='M{x:.2} {y:.2}H{end:.2}' stroke='#b7c6d5' stroke-opacity='.4'/>"
        );
    }
    let mut notes: Vec<_> = scene
        .notes
        .iter()
        .filter(|note| {
            (48..=72).contains(&note.lane)
                && note.start.is_finite()
                && note.end.is_finite()
                && note.end >= scene.position
                && note.start <= scene.position + scene.lookahead
        })
        .collect();
    notes.sort_by(|a, b| b.start.total_cmp(&a.start));
    for note in notes {
        let fraction = (f64::from(note.lane - 48) + 0.5) / 25.0;
        let (x, y, scale) = project(
            fraction,
            (note.start - scene.position).max(0.0),
            scene.lookahead,
        );
        let color = piano_color(note.lane);
        let width = HALF_WIDTH / 25.0 * 0.75 * scale;
        if note.end > note.start && note.end > scene.position {
            let (tx, ty, tail_scale) = project(
                fraction,
                (note.end - scene.position).min(scene.lookahead),
                scene.lookahead,
            );
            let tail_width = HALF_WIDTH / 25.0 * 0.75 * tail_scale;
            let _ = write!(
                out,
                "<path data-kind='sustain' data-pitch='{}' d='M{:.2} {y:.2}L{:.2} {ty:.2}L{:.2} {ty:.2}L{:.2} {y:.2}Z' fill='{color}' opacity='.6'/>",
                note.lane,
                x - width,
                tx - tail_width,
                tx + tail_width,
                x + width
            );
        }
        if note.start >= scene.position {
            let stroke = if note.selected { "#ffd35a" } else { "#182432" };
            let _ = write!(
                out,
                "<rect data-kind='piano-note' data-pitch='{}' x='{:.2}' y='{:.2}' width='{:.2}' height='{:.2}' rx='1' fill='{color}' stroke='{stroke}'><title>{}</title></rect>",
                note.lane,
                x - width,
                y - 4.0 * scale,
                width * 2.0,
                (8.0 * scale).max(2.0),
                crate::keys::pitch_name(note.lane)
            );
        }
    }
    for pitch in 48..=72 {
        let black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
        let x = CENTER_X - HALF_WIDTH + f64::from(pitch - 48) * HALF_WIDTH * 2.0 / 25.0;
        let width = HALF_WIDTH * 2.0 / 25.0;
        let holding = scene.playing
            && scene.notes.iter().any(|note| {
                note.lane == pitch && note.start <= scene.position && note.end > scene.position
            });
        let flashing = scene.playing
            && scene.flashes.iter().any(|flash| {
                flash.lane == pitch && (0.0..FLASH_SECONDS).contains(&(scene.position - flash.at))
            });
        let fill = if holding || flashing {
            "#ffd35a"
        } else if black {
            "#253445"
        } else {
            "#e9eff5"
        };
        let text = if black && !holding && !flashing {
            "#e9eff5"
        } else {
            "#182432"
        };
        let _ = write!(
            out,
            "<g data-kind='piano-receptor' data-pitch='{pitch}'><rect x='{x:.2}' y='{STRIKE_Y}' width='{width:.2}' height='39' fill='{fill}' stroke='#0a111b'/><text x='{:.2}' y='{:.2}' text-anchor='middle' fill='{text}' font-size='8'>{}</text></g>",
            x + width / 2.0,
            STRIKE_Y + 28.0,
            crate::keys::pitch_name(pitch)
        );
    }
    out
}

fn render_highway(scene: &Scene<'_>) -> String {
    if !scene.position.is_finite() || !scene.lookahead.is_finite() || scene.lookahead <= 0.0 {
        return String::new();
    }
    if scene.pro_piano {
        return render_piano_highway(scene);
    }
    let lanes = if scene.drums { 4 } else { 5 };
    let mut out = String::from(DEFINITIONS);
    // The board continues beneath the receptors, avoiding floating fret buttons.
    let (left_far, far_y, _) = project(0.0, scene.lookahead, scene.lookahead);
    let (right_far, _, _) = project(1.0, scene.lookahead, scene.lookahead);
    let near_offset = -scene.lookahead * 0.017;
    let (left_near, near_y, _) = project(0.0, near_offset, scene.lookahead);
    let (right_near, _, _) = project(1.0, near_offset, scene.lookahead);
    let board = format!(
        "M{left_far:.2} {far_y:.2}L{right_far:.2} {far_y:.2}L{right_near:.2} {near_y:.2}L{left_near:.2} {near_y:.2}Z"
    );
    let _ = write!(
        out,
        "<defs><clipPath id='game-board-clip'><path d='{board}'/></clipPath></defs><ellipse cx='500' cy='560' rx='330' ry='36' fill='#000' opacity='.6'/><path d='{board}' fill='url(#game-board)' stroke='#657382' stroke-width='3'/><g clip-path='url(#game-board-clip)'><rect x='170' y='60' width='660' height='545' fill='url(#game-grain)' opacity='.48'/>"
    );
    // Animated cross-grain follows the same recording clock, with original art.
    let phase = (-scene.position * 2.4).rem_euclid(1.0);
    for index in 0..43 {
        let distance = ((index as f64 + phase) / 42.0) * scene.lookahead;
        let (left, y, _) = project(0.0, distance, scene.lookahead);
        let (right, _, _) = project(1.0, distance, scene.lookahead);
        let _ = write!(
            out,
            "<path d='M{left:.2} {y:.2}H{right:.2}' stroke='#bdc8d5' stroke-opacity='.045' stroke-width='1'/>"
        );
    }
    for lane in 1..lanes {
        let (a, b, _) = project(lane as f64 / lanes as f64, scene.lookahead, scene.lookahead);
        let (c, d, _) = project(lane as f64 / lanes as f64, near_offset, scene.lookahead);
        let _ = write!(
            out,
            "<path d='M{a:.2} {b:.2}L{c:.2} {d:.2}' stroke='#000' stroke-width='3' opacity='.6'/><path d='M{a:.2} {b:.2}L{c:.2} {d:.2}' stroke='#6d8292' stroke-width='1' opacity='.32'/>"
        );
    }
    for seconds in scene.beats {
        let (left, y, scale) = project(0.0, seconds - scene.position, scene.lookahead);
        let (right, _, _) = project(1.0, seconds - scene.position, scene.lookahead);
        let _ = write!(
            out,
            "<path data-kind='beat' d='M{left:.2} {y:.2}H{right:.2}' stroke='#d9e0eb' stroke-width='{:.2}' opacity='.42'/>",
            0.5 + scale * 1.9
        );
    }
    out.push_str("</g>");
    // Raised metal side rails sit outside the playing surface.
    for side in [0.0, 1.0] {
        let (a, b, _) = project(side, scene.lookahead, scene.lookahead);
        let (c, d, _) = project(side, near_offset, scene.lookahead);
        let _ = write!(
            out,
            "<path d='M{a:.2} {b:.2}L{c:.2} {d:.2}' stroke='#04070b' stroke-width='10'/><path d='M{a:.2} {b:.2}L{c:.2} {d:.2}' stroke='url(#game-metal)' stroke-width='5'/><path d='M{a:.2} {b:.2}L{c:.2} {d:.2}' stroke='#68b6ed' stroke-opacity='.24' stroke-width='1'/>"
        );
    }
    let mut visible: Vec<_> = scene
        .notes
        .iter()
        .filter(|note| {
            note.start.is_finite()
                && note.end.is_finite()
                && note.end >= scene.position
                && note.start <= scene.position + scene.lookahead
        })
        .collect();
    visible.sort_by(|a, b| b.start.total_cmp(&a.start).then(a.lane.cmp(&b.lane)));
    for note in visible {
        render_note(&mut out, scene, note, lanes);
    }
    render_receptors(&mut out, scene, lanes);
    out
}

fn render_note(out: &mut String, scene: &Scene<'_>, note: &PreviewNote, lanes: usize) {
    let bar = note.lane == 7 || (scene.drums && matches!(note.lane, 0 | 32));
    let lane = if scene.drums {
        note.lane.saturating_sub(1)
    } else {
        note.lane
    } as usize;
    if !bar && lane >= lanes {
        return;
    }
    let fraction = if bar {
        0.5
    } else {
        (lane as f64 + 0.5) / lanes as f64
    };
    let start = (note.start - scene.position).max(0.0);
    let (x, y, scale) = project(fraction, start, scene.lookahead);
    let color = lane_color(scene.drums, note.lane);
    if !scene.drums && note.end > note.start && note.end > scene.position {
        let (tx, ty, tail_scale) = project(
            fraction,
            (note.end - scene.position).min(scene.lookahead),
            scene.lookahead,
        );
        let width = if bar {
            HALF_WIDTH * scale * 0.65
        } else {
            8.0 * scale
        };
        let tail_width = if bar {
            HALF_WIDTH * tail_scale * 0.65
        } else {
            8.0 * tail_scale
        };
        let active = note.start < scene.position && scene.playing;
        let _ = write!(
            out,
            "<g data-kind='sustain'><path d='M{:.2} {y:.2}L{:.2} {ty:.2}L{:.2} {ty:.2}L{:.2} {y:.2}Z' fill='{color}' opacity='{}'/><path d='M{x:.2} {y:.2}L{tx:.2} {ty:.2}' stroke='#fff' stroke-opacity='.55' stroke-width='{:.2}'/></g>",
            x - width,
            tx - tail_width,
            tx + tail_width,
            x + width,
            if active { 0.85 } else { 0.58 },
            (scale * 2.5).max(0.5)
        );
    }
    if note.start < scene.position {
        return;
    }
    if bar {
        let (a, _, _) = project(0.025, start, scene.lookahead);
        let (b, _, _) = project(0.975, start, scene.lookahead);
        let height = 10.0 * scale + 2.0;
        let kind = if scene.drums { "kick" } else { "open" };
        let _ = write!(
            out,
            "<g data-kind='{kind}'><rect x='{a:.2}' y='{:.2}' width='{:.2}' height='{height:.2}' rx='{:.2}' fill='#07090d' stroke='#b2bdc9' stroke-width='{:.2}'/><rect x='{:.2}' y='{:.2}' width='{:.2}' height='{:.2}' rx='{:.2}' fill='{color}'/><path d='M{:.2} {:.2}H{:.2}' stroke='#fff' opacity='.65' stroke-width='{:.2}'/></g>",
            y - height / 2.0,
            b - a,
            scale * 3.0,
            (scale * 1.8).max(0.6),
            a + scale * 3.0,
            y - height / 2.0 + scale * 2.0,
            b - a - scale * 6.0,
            height - scale * 4.0,
            scale * 2.0,
            a + scale * 5.0,
            y - height / 2.0 + scale * 2.3,
            b - scale * 5.0,
            (scale * 1.5).max(0.5)
        );
        if note.lane == 32 {
            let _ = write!(
                out,
                "<text x='{x:.2}' y='{:.2}' text-anchor='middle' fill='#10131a' font-size='{:.2}' font-weight='800'>2×</text>",
                y + 3.5 * scale,
                10.0 * scale
            );
        }
        return;
    }
    let technique_scale = if !scene.drums && note.technique == Some(Technique::Hopo) {
        0.82
    } else {
        1.0
    };
    let rx = (HALF_WIDTH / lanes as f64 * 0.65) * scale * technique_scale;
    let ry = 14.0 * scale * technique_scale;
    let selected = if note.selected { "#fff" } else { "#9daab8" };
    let stroke = (1.6 * scale).max(0.7);
    if note.dynamics == Dynamics::Ghost {
        out.push_str("<g data-dynamics='ghost' opacity='.43'>");
    }
    if scene.drums && scene.pro_drums && note.cymbal {
        let _ = write!(
            out,
            "<g data-kind='cymbal' data-lane='{}'><path d='M{x:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z' fill='#020509' stroke='{selected}' stroke-width='{stroke:.2}'/><path d='M{x:.2} {:.2}L{:.2} {:.2}L{:.2} {:.2}Z' fill='{color}'/><path d='M{x:.2} {:.2}L{:.2} {:.2}' stroke='#fff' stroke-width='{:.2}' opacity='.72'/><ellipse cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='{:.2}' fill='#fff' opacity='.55'/></g>",
            note.lane,
            y - ry * 1.8,
            x + rx,
            y + ry * 0.8,
            x - rx,
            y + ry * 0.8,
            y - ry * 1.45,
            x + rx * 0.82,
            y + ry * 0.52,
            x - rx * 0.82,
            y + ry * 0.52,
            y - ry * 1.45,
            x - rx * 0.78,
            y + ry * 0.45,
            stroke,
            y - ry * 0.15,
            rx * 0.18,
            ry * 0.23
        );
    } else {
        let kind = if scene.drums { "tom" } else { "gem" };
        let _ = write!(
            out,
            "<g data-kind='{kind}' data-lane='{}'><ellipse cx='{x:.2}' cy='{:.2}' rx='{rx:.2}' ry='{ry:.2}' fill='#000' opacity='.55'/><ellipse cx='{x:.2}' cy='{y:.2}' rx='{rx:.2}' ry='{ry:.2}' fill='url(#game-gem-base)' stroke='{selected}' stroke-width='{stroke:.2}'/><ellipse cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='{:.2}' fill='{color}'/><ellipse cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='{:.2}' fill='url(#game-gem-sheen)'/><path d='M{:.2} {:.2}Q{x:.2} {:.2} {:.2} {:.2}' fill='none' stroke='#fff' stroke-opacity='.75' stroke-width='{:.2}'/></g>",
            note.lane,
            y + ry * 0.55,
            y - ry * 0.27,
            rx * 0.82,
            ry * 0.69,
            y - ry * 0.27,
            rx * 0.82,
            ry * 0.69,
            x - rx * 0.64,
            y - ry * 0.3,
            y - ry * 1.05,
            x + rx * 0.64,
            y - ry * 0.3,
            stroke
        );
    }
    match note.technique {
        Some(Technique::Hopo) => {
            let _ = write!(
                out,
                "<ellipse data-technique='hopo' cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='{:.2}' fill='#f7fdff' stroke='#d8f1ff' stroke-width='{:.2}'/>",
                y - ry * 0.3,
                rx * 0.40,
                ry * 0.42,
                (scale * 0.8).max(0.3)
            );
        }
        Some(Technique::Tap) => {
            let _ = write!(
                out,
                "<rect data-technique='tap' x='{:.2}' y='{:.2}' width='{:.2}' height='{:.2}' rx='{:.2}' fill='#fff' stroke='#d6ddff' stroke-width='{:.2}'/>",
                x - rx * 0.52,
                y - ry * 0.62,
                rx * 1.04,
                ry * 0.58,
                scale * 2.0,
                (scale * 0.8).max(0.3)
            );
        }
        _ => {}
    }
    if note.dynamics == Dynamics::Accent {
        let _ = write!(
            out,
            "<path data-dynamics='accent' d='M{:.2} {:.2}L{x:.2} {:.2}L{:.2} {:.2}' fill='none' stroke='#fff' stroke-width='{:.2}' stroke-linecap='round' stroke-linejoin='round'/>",
            x - rx * 0.60,
            y - ry * 1.5,
            y - ry * 1.95,
            x + rx * 0.60,
            y - ry * 1.5,
            (scale * 2.2).max(0.8)
        );
    }
    if note.dynamics == Dynamics::Ghost {
        out.push_str("</g>");
    }
}

fn render_receptors(out: &mut String, scene: &Scene<'_>, lanes: usize) {
    let left = CENTER_X - HALF_WIDTH;
    let right = CENTER_X + HALF_WIDTH;
    let _ = write!(
        out,
        "<path d='M{left:.2} {:.2}H{right:.2}' stroke='#000' stroke-width='12'/><path d='M{left:.2} {:.2}H{right:.2}' stroke='url(#game-metal)' stroke-width='5'/><path d='M{left:.2} {:.2}H{right:.2}' stroke='#ced6e2' stroke-width='1.5' opacity='.8'/>",
        STRIKE_Y + 6.0,
        STRIKE_Y + 4.0,
        STRIKE_Y + 2.0
    );
    for index in 0..lanes {
        let lane = if scene.drums {
            index as u8 + 1
        } else {
            index as u8
        };
        let (x, _, _) = project((index as f64 + 0.5) / lanes as f64, 0.0, scene.lookahead);
        let color = lane_color(scene.drums, lane);
        let flash = if scene.playing {
            scene
                .flashes
                .iter()
                .filter(|flash| flash.lane == lane)
                .map(|flash| (1.0 - (scene.position - flash.at) / FLASH_SECONDS).clamp(0.0, 1.0))
                .fold(0.0, f64::max)
        } else {
            0.0
        };
        let holding = !scene.drums
            && scene.playing
            && scene.notes.iter().any(|note| {
                note.lane == lane
                    && note.end > note.start
                    && note.start < scene.position
                    && note.end > scene.position
            });
        let active = flash.max(if holding { 0.70 } else { 0.0 });
        let rx = HALF_WIDTH / lanes as f64 * 0.81;
        let _ = write!(
            out,
            "<g data-kind='receptor' data-lane='{lane}'><ellipse cx='{x:.2}' cy='{:.2}' rx='{rx:.2}' ry='18' fill='#030509' stroke='#b3bdc8' stroke-width='2.2'/><ellipse cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='13' fill='{color}'/><ellipse cx='{x:.2}' cy='{:.2}' rx='{:.2}' ry='9' fill='{}'/><path d='M{:.2} {:.2}Q{x:.2} {:.2} {:.2} {:.2}' fill='none' stroke='#fff' stroke-opacity='.32' stroke-width='1.5'/></g>",
            STRIKE_Y + 4.0,
            STRIKE_Y + 1.0,
            rx * 0.85,
            STRIKE_Y,
            rx * 0.63,
            if active > 0.0 { color } else { "#333d48" },
            x - rx * 0.65,
            STRIKE_Y + 1.0,
            STRIKE_Y - 12.0,
            x + rx * 0.65,
            STRIKE_Y + 1.0
        );
        if active > 0.0 {
            render_flash(out, x, color, active, holding);
        }
    }
    let open_hold = !scene.drums
        && scene.playing
        && scene
            .notes
            .iter()
            .any(|note| note.lane == 7 && note.start < scene.position && note.end > scene.position);
    if scene.drums || open_hold || scene.flashes.iter().any(|flash| flash.lane == 7) {
        let lanes: &[u8] = if scene.drums { &[0, 32] } else { &[7] };
        let flash = if scene.playing {
            scene
                .flashes
                .iter()
                .filter(|flash| lanes.contains(&flash.lane))
                .map(|flash| (1.0 - (scene.position - flash.at) / FLASH_SECONDS).clamp(0.0, 1.0))
                .fold(0.0, f64::max)
        } else {
            0.0
        };
        let flash = flash.max(if open_hold { 0.70 } else { 0.0 });
        let color = lane_color(scene.drums, if scene.drums { 0 } else { 7 });
        let _ = write!(
            out,
            "<rect data-kind='bar-receptor' x='{left:.2}' y='{:.2}' width='{:.2}' height='5' rx='2' fill='{color}' opacity='{}'/>",
            STRIKE_Y + 27.0,
            right - left,
            0.30 + flash * 0.7
        );
        if flash > 0.0 {
            let _ = write!(
                out,
                "<rect data-kind='bar-flash' x='{left:.2}' y='{:.2}' width='{:.2}' height='18' rx='5' fill='{color}' opacity='{:.3}' filter='url(#game-glow)'/>",
                STRIKE_Y + 15.0,
                right - left,
                flash * 0.75
            );
        }
    }
}

fn render_flash(out: &mut String, x: f64, color: &str, strength: f64, holding: bool) {
    let height = if holding {
        38.0
    } else {
        24.0 + strength * 80.0
    };
    let _ = write!(
        out,
        "<g data-kind='autoplay-flash' opacity='{strength:.3}'><ellipse cx='{x:.2}' cy='{:.2}' rx='47' ry='15' fill='{color}' filter='url(#game-glow)'/><path d='M{:.2} {:.2}Q{:.2} {:.2} {x:.2} {:.2}Q{:.2} {:.2} {:.2} {:.2}Z' fill='url(#game-flash)'/><ellipse cx='{x:.2}' cy='{:.2}' rx='24' ry='7' fill='#fff' opacity='.9'/></g>",
        STRIKE_Y - 3.0,
        x - 20.0,
        STRIKE_Y - 3.0,
        x - 25.0,
        STRIKE_Y - height * 0.6,
        STRIKE_Y - height,
        x + 22.0,
        STRIKE_Y - height * 0.4,
        x + 20.0,
        STRIKE_Y - 3.0,
        STRIKE_Y - 4.0
    );
}

const DEFINITIONS: &str = "<defs><linearGradient id='game-board' x1='0' y1='0' x2='0' y2='1'><stop stop-color='#090d13'/><stop offset='.55' stop-color='#20242b'/><stop offset='1' stop-color='#252831'/></linearGradient><linearGradient id='game-metal' x1='0' y1='0' x2='1' y2='0'><stop stop-color='#202b39'/><stop offset='.3' stop-color='#b5c5d5'/><stop offset='.55' stop-color='#43566b'/><stop offset='.8' stop-color='#d9e3ee'/><stop offset='1' stop-color='#283442'/></linearGradient><linearGradient id='game-gem-base' x1='0' y1='0' x2='0' y2='1'><stop stop-color='#c8d4df'/><stop offset='.32' stop-color='#4b5868'/><stop offset='.63' stop-color='#111721'/><stop offset='1' stop-color='#04070b'/></linearGradient><linearGradient id='game-gem-sheen' x1='0' y1='0' x2='.15' y2='1'><stop stop-color='#fff' stop-opacity='.85'/><stop offset='.36' stop-color='#fff' stop-opacity='.13'/><stop offset='1' stop-color='#000' stop-opacity='.45'/></linearGradient><linearGradient id='game-flash' x1='0' y1='1' x2='0' y2='0'><stop stop-color='#fff' stop-opacity='.95'/><stop offset='.32' stop-color='#fff3ac' stop-opacity='.8'/><stop offset='1' stop-color='#ffb14a' stop-opacity='0'/></linearGradient><filter id='game-glow' x='-100%' y='-200%' width='300%' height='500%'><feGaussianBlur stdDeviation='7'/></filter><pattern id='game-grain' width='21' height='9' patternUnits='userSpaceOnUse'><path d='M0 2H21M3 6H16' stroke='#95a1b2' stroke-opacity='.075'/><path d='M1 0V9M14 0V9' stroke='#02050a' stroke-opacity='.4'/></pattern></defs>";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pro_piano_preview_renders_full_pitch_range_and_keeps_held_keys() {
        let notes = [note(0.0, 2.0, 48, false), note(1.5, 1.8, 72, false)];
        let scene = Scene {
            notes: &notes,
            beats: &[],
            flashes: &[],
            position: 1.0,
            lookahead: 3.6,
            drums: false,
            pro_piano: true,
            pro_drums: false,
            playing: true,
        };
        let svg = render_highway(&scene);
        assert_eq!(svg.matches("data-kind='piano-receptor'").count(), 25);
        assert!(svg.contains("data-kind='piano-note' data-pitch='72'"));
        assert!(svg.contains("data-kind='sustain' data-pitch='48'"));
        assert!(svg.contains("fill='#ffd35a'"));
        assert!(!svg.contains("data-kind='open'"));
        assert!(!svg.contains("data-technique='hopo'"));
        assert!(
            !render_highway(&Scene {
                playing: false,
                ..scene
            })
            .contains("fill='#ffd35a'")
        );
    }

    fn note(start: f64, end: f64, lane: u8, cymbal: bool) -> PreviewNote {
        PreviewNote {
            start,
            end,
            lane,
            cymbal,
            selected: false,
            technique: Some(Technique::Strum),
            dynamics: Dynamics::Neutral,
        }
    }

    #[test]
    fn preview_crossings_ignore_seeks_pauses_track_changes_and_repeat_frames() {
        let notes = [note(1.02, 1.02, 2, false), note(1.5, 1.5, 4, false)];
        let initial =
            advance_animation(&Animation::default(), "ExpertSingle", 1, 1.0, true, &notes);
        let crossed = advance_animation(&initial, "ExpertSingle", 1, 1.04, true, &notes);
        assert_eq!(crossed.flashes, [StrikeFlash { lane: 2, at: 1.02 }]);
        let repeated = advance_animation(&crossed, "ExpertSingle", 1, 1.04, true, &notes);
        assert_eq!(repeated.flashes.len(), 1);
        assert!(
            advance_animation(&crossed, "ExpertSingle", 1, 1.55, true, &notes)
                .flashes
                .is_empty()
        );
        assert!(
            advance_animation(&crossed, "ExpertSingle", 1, 0.9, true, &notes)
                .flashes
                .is_empty()
        );
        assert!(
            advance_animation(&crossed, "ExpertSingle", 1, 1.05, false, &notes)
                .flashes
                .is_empty()
        );
        assert!(
            advance_animation(&crossed, "ExpertDrums", 1, 1.05, true, &notes)
                .flashes
                .is_empty()
        );
        assert!(
            advance_animation(&crossed, "ExpertSingle", 2, 1.05, true, &notes)
                .flashes
                .is_empty()
        );
    }

    #[test]
    fn zero_time_attack_and_open_sustain_keep_autoplay_feedback() {
        let notes = [note(0.0, 2.0, 7, false)];
        let first = advance_animation(
            &Animation::default(),
            "ExpertDoubleBass",
            1,
            0.01,
            true,
            &notes,
        );
        assert_eq!(first.flashes, [StrikeFlash { lane: 7, at: 0.0 }]);
        let held = render_highway(&Scene {
            notes: &notes,
            beats: &[],
            flashes: &[],
            position: 1.0,
            lookahead: 3.6,
            drums: false,
            pro_piano: false,
            pro_drums: false,
            playing: true,
        });
        assert!(held.contains("data-kind='bar-flash'"));
        let paused = render_highway(&Scene {
            notes: &notes,
            beats: &[],
            flashes: &[],
            position: 1.0,
            lookahead: 3.6,
            drums: false,
            pro_piano: false,
            pro_drums: false,
            playing: false,
        });
        assert!(!paused.contains("data-kind='bar-flash'"));
    }

    #[test]
    fn drum_semantics_distinguish_kick_cymbal_and_tom_with_four_receptors() {
        let notes = [
            note(0.3, 0.3, 0, false),
            note(0.5, 0.5, 2, true),
            note(0.8, 0.8, 3, false),
            note(1.0, 1.0, 32, false),
        ];
        let scene = Scene {
            notes: &notes,
            beats: &[],
            flashes: &[],
            position: 0.0,
            lookahead: 3.6,
            drums: true,
            pro_piano: false,
            pro_drums: true,
            playing: false,
        };
        let pro = render_highway(&scene);
        assert_eq!(pro.matches("data-kind='receptor'").count(), 4);
        assert_eq!(pro.matches("data-kind='kick'").count(), 2);
        assert!(pro.contains("data-kind='cymbal' data-lane='2'"));
        assert!(pro.contains("data-kind='tom' data-lane='3'"));
        let standard = render_highway(&Scene {
            pro_drums: false,
            ..scene
        });
        assert!(!standard.contains("data-kind='cymbal'"));
        assert!(standard.contains("data-kind='tom' data-lane='2'"));
    }

    #[test]
    fn held_sustains_remain_visible_after_their_head_passes_the_strike_line() {
        let notes = [note(0.2, 2.0, 0, false), note(1.2, 1.2, 7, false)];
        let scene = Scene {
            notes: &notes,
            beats: &[],
            flashes: &[],
            position: 1.0,
            lookahead: 3.6,
            drums: false,
            pro_piano: false,
            pro_drums: false,
            playing: true,
        };
        let svg = render_highway(&scene);
        assert_eq!(svg.matches("data-kind='receptor'").count(), 5);
        assert!(svg.contains("data-kind='sustain'"));
        assert!(svg.contains("data-kind='open'"));
        assert!(svg.contains("data-kind='autoplay-flash'"));
        assert!(
            !render_highway(&Scene {
                playing: false,
                ..scene
            })
            .contains("data-kind='autoplay-flash'")
        );
    }

    #[test]
    fn projection_is_monotonic_and_places_all_lane_centers_within_the_board() {
        let mut prior_y = STRIKE_Y + 1.0;
        for step in 0..=100 {
            let ahead = step as f64 / 100.0 * 3.6;
            let (left, y, _) = project(0.0, ahead, 3.6);
            let (right, _, _) = project(1.0, ahead, 3.6);
            assert!(y < prior_y);
            prior_y = y;
            for lane in 0..5 {
                let (x, _, _) = project((lane as f64 + 0.5) / 5.0, ahead, 3.6);
                assert!(x > left && x < right);
            }
        }
        assert!((project(0.5, 0.0, 3.6).1 - STRIKE_Y).abs() < 1e-9);
        assert!((project(0.5, 3.6, 3.6).1 - FAR_Y).abs() < 1e-9);
    }
}
