//! Non-destructive musical comparison; only Apply enters the editor command log.
use super::state::Studio;
use crate::{
    editor::{Action, Project, TimeRange},
    parts::Prepared,
    reanalysis::{
        Change,
        comparison::{Comparison, Difference, geometry, window},
    },
};
use anyhow::{Context, Result};
use dioxus::prelude::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct Review {
    proposed: Arc<Project>,
    range: TimeRange,
    changes: Vec<Change>,
    warnings: Vec<String>,
}
fn load(store: crate::editor::Store, current: Arc<Project>, job: PathBuf) -> Result<Review> {
    let draft = Prepared::load(&store, &current, &job)?;
    let range = draft.range().unwrap_or(TimeRange {
        start: 0.,
        end: current.duration,
    });
    let proposed = draft.preview(&current)?;
    let tracks = crate::reanalysis::affected(
        &current.content,
        &crate::parts::target(&job)?
            .context("Missing part target")?
            .track,
    );
    let changes = crate::reanalysis::changes(&current, &proposed, &tracks);
    let generated = crate::editor::Store::new(&crate::generation::project_path(&job)).load()?;
    Ok(Review {
        proposed: Arc::new(proposed),
        range,
        changes,
        warnings: generated.review.coverage_limits,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Changes,
    Current,
    Proposed,
}

fn show_version(studio: Studio, mut project: Project, track: &str) {
    if let Some(current) = studio.doc.peek().as_ref() {
        project.transport = current.transport.clone();
        project.revision = current.revision;
    }
    project.selected_track = track.into();
    studio.show_analysis_preview(Some(project));
}

fn lane_label(track: &str, lane: u8) -> String {
    if track == "Vocals" {
        if lane == 0 {
            return "Talkie".into();
        }
        return format!(
            "{}{}",
            [
                "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"
            ][usize::from(lane % 12)],
            i16::from(lane) / 12 - 1
        );
    }
    super::note_canvas::lane_name(track, lane).into()
}

#[component]
fn PassageChart(
    studio: Studio,
    comparison: Arc<Comparison>,
    track: String,
    view: TimeRange,
    mode: View,
    timing: crate::timing::TempoMap,
    position: f64,
) -> Element {
    let lanes = comparison.lanes(&track);
    let height = 34 + lanes.len().max(1) * 28;
    let width = 840.;
    let x = |seconds: f64| 100. + (seconds - view.start) / (view.end - view.start) * width;
    let start_tick = timing.tick(view.start).unwrap_or(0);
    let end_tick = timing.tick(view.end).unwrap_or(start_tick);
    let resolution = u64::from(timing.resolution);
    let label_step =
        (((end_tick - start_tick) as f64 / resolution as f64 * 48. / width).ceil() as u64).max(1);
    let beats: Vec<_> = (start_tick / resolution..=end_tick / resolution)
        .take(256)
        .map(|beat| (beat, timing.seconds(beat * resolution)))
        .filter(|(_, s)| *s >= view.start && *s < view.end)
        .collect();
    let title = match mode {
        View::Changes => "Changes · plus adds a note, minus removes a note",
        View::Current => "Current chart",
        View::Proposed => "Proposed chart",
    };
    rsx! { div { class: "ra-chart-scroll",
        svg { class: "ra-chart", view_box: "0 0 960 {height}", width: "100%", height: "{height}", role: "group", "aria-label": "{title}",
            rect { width: "960", height: "{height}", fill: "#111820" }
            for (row, lane) in lanes.iter().enumerate() {
                rect { x: "100", y: "{34 + row * 28}", width: "840", height: "28", fill: if row % 2 == 0 { "#19222c" } else { "#141c25" } }
                text { x: "90", y: "{52 + row * 28}", text_anchor: "end", fill: "{super::note_canvas::color(&track, *lane)}", font_size: "12", "{lane_label(&track, *lane)}" }
            }
            for (beat, seconds) in beats {
                line { x1: "{x(seconds)}", x2: "{x(seconds)}", y1: "28", y2: "{height}", stroke: "#384452", stroke_width: "1" }
                if beat % label_step == 0 { text { x: "{x(seconds) + 3.}", y: "21", fill: "#aebdcd", font_size: "10", "{super::state::clock(seconds)}" } }
                title { "Beat {beat + 1}" }
            }
            for event in comparison.events.iter().filter(|e| match mode { View::Changes => true, View::Current => e.difference != Difference::Added, View::Proposed => e.difference != Difference::Removed }) {
                if let Some((left, length)) = geometry(event, view, width) {
                    if let Some(row) = lanes.iter().position(|lane| *lane == event.lane) {
                        {let changed = mode == View::Changes && event.difference != Difference::Unchanged;
                        let fill = if changed { if event.difference == Difference::Added { "#6ee7ad" } else { "#ff98a8" } } else if track == "Vocals" { "#a8caff" } else { super::note_canvas::color(&track,event.lane) };
                        let y = 38 + row * 28 + if changed && event.difference == Difference::Added { 11 } else { 0 };
                        let note_height = if changed { 9 } else { 19 };
                        let seconds = event.start;
                        let status = if event.difference == Difference::Added { "Added" } else if event.difference == Difference::Removed { "Removed" } else { "Unchanged" };
                        let accessible = format!("{status} {} {} at {:.3} seconds. {}",lane_label(&track,event.lane),event.detail,event.start,if changed {"Select to seek."} else {""});
                        rsx! { g {
                            rect { class: "ra-note", x: "{100. + left}", y: "{y}", width: "{length}", height: "{note_height}", rx: if event.cymbal { "0" } else { "3" }, fill, fill_opacity: if mode == View::Changes && !changed { ".45" } else { "1" }, stroke: fill, stroke_dasharray: if changed && event.difference == Difference::Removed { "3 2" } else { "none" }, stroke_width: "1.5", role: "button", tabindex: "0", "aria-label": "{accessible}", onclick: move |_| studio.seek(seconds), onkeydown: move |event| { if event.key() == Key::Enter || event.key() == Key::Character(" ".into()) { event.prevent_default(); event.stop_propagation(); studio.seek(seconds); } }, title { "{accessible}" } }
                            if changed { text { class: "ra-note-symbol", x: "{101. + left}", y: "{y + 8}", fill: "#10231b", font_size: "10", font_weight: "700", if event.difference == Difference::Added { "+" } else { "−" } } }
                        }}}
                    }
                }
            }
            if position >= view.start && position <= view.end { line { x1: "{x(position)}", x2: "{x(position)}", y1: "28", y2: "{height}", stroke: "#fff", stroke_width: "2", pointer_events: "none" } }
            if comparison.events.is_empty() { text { x: "120", y: "53", fill: "#aebdcd", font_size: "13", "No notes in this difficulty" } }
        }
    } }
}

#[component]
pub fn ReviewDraft(
    mut studio: Studio,
    job: PathBuf,
    stale: bool,
    onapply: EventHandler<()>,
    ondiscard: EventHandler<()>,
) -> Element {
    let mut selected = use_signal(|| None::<String>);
    let mut mode = use_signal(|| View::Changes);
    let mut view_start = use_signal(|| None::<f64>);
    let mut span = use_signal(|| 8.0_f64);
    let mut loop_passage = use_signal(|| false);
    let mut owns_loop = use_signal(|| false);
    let mut auditioning = use_signal(|| false);
    // Transport updates must not reload and validate an on-disk draft.
    let content_key = use_memo(move || {
        studio.doc.read().as_ref().map(|doc| {
            (
                doc.audio_sha256.clone(),
                doc.source_chart_sha256.clone(),
                serde_json::to_string(&doc.content).unwrap_or_default(),
            )
        })
    });
    let review = use_resource(move || {
        let job = job.clone();
        let _key = content_key();
        let inputs = (studio.store)().zip(studio.doc.peek().clone());
        async move {
            let (store, current) =
                inputs.ok_or_else(|| "Open the target chart first".to_owned())?;
            #[cfg(not(target_arch = "wasm32"))]
            {
                tokio::task::spawn_blocking(move || load(store, current, job))
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| format!("{e:#}"))
            }
            #[cfg(target_arch = "wasm32")]
            {
                load(store, current, job).map_err(|e| format!("{e:#}"))
            }
        }
    });
    let comparison = use_memo(move || {
        let value = review.read();
        let review = value.as_ref()?.as_ref().ok()?;
        let track = selected()
            .filter(|track| review.changes.iter().any(|c| &c.track == track))
            .unwrap_or_else(|| {
                review
                    .changes
                    .first()
                    .map(|c| c.track.clone())
                    .unwrap_or_else(|| review.proposed.selected_track.clone())
            });
        let current = studio.doc.peek().clone()?;
        let timing = studio.timing.peek().clone()?;
        Some((
            track.clone(),
            Arc::new(Comparison::new(
                &current.content,
                &review.proposed.content,
                &track,
                &timing,
            )),
        ))
    });
    let original_loop = use_hook(move || {
        studio
            .doc
            .peek()
            .as_ref()
            .and_then(|doc| doc.transport.loop_range)
    });
    let active_window = use_memo(move || {
        let value = review.read();
        let review = value.as_ref()?.as_ref().ok()?;
        let (_, comparison) = comparison()?;
        let first = comparison
            .passages(review.range)
            .first()
            .copied()
            .unwrap_or(review.range.start);
        Some(window(
            review.range,
            view_start().unwrap_or((first - 1.).max(review.range.start)),
            span(),
        ))
    });
    use_effect(move || {
        if !(studio.generation_open)() || stale {
            studio.analysis_preview.set(None);
        }
    });
    use_effect(move || {
        if !loop_passage() {
            return;
        }
        let Some(view) = active_window() else {
            return;
        };
        if studio
            .doc
            .peek()
            .as_ref()
            .is_some_and(|doc| doc.transport.loop_range != Some(view))
        {
            owns_loop.set(true);
            studio.dispatch(Action::SetLoop { range: Some(view) });
        }
    });
    use_effect(move || {
        let position = (studio.playhead)();
        let Some(view) = active_window() else {
            return;
        };
        if auditioning() && !loop_passage() && position >= view.end {
            auditioning.set(false);
            studio.dispatch(Action::Pause { seconds: view.end });
        }
    });
    use_drop(move || {
        studio.show_analysis_preview(None);
        if owns_loop() {
            studio.dispatch(Action::SetLoop {
                range: original_loop,
            });
        }
    });
    let value = review.read();
    let Some(result) = value.as_ref() else {
        return rsx! { div { class:"ra-status",role:"status",strong {"Checking draft…"} p {"Preparing your comparison."} } };
    };
    let review = match result {
        Ok(review) => review,
        Err(error) => {
            let empty = error.contains("No notes were detected")
                || error.contains("No vocal notes were detected");
            let title = if empty {
                "No notes found"
            } else if error.contains("changed") {
                "Chart changed since analysis"
            } else {
                "Draft needs attention"
            };
            return rsx! { div {class:"ra-status",role:"alert",strong{"{title}"}p{if empty{"Try another sensitivity or sound source. Your chart is unchanged."}else{"Your edits and draft are kept. Start a new analysis to continue."}}details{summary{"Details"}pre{class:"ra-error-detail","{error}"}}}button{class:"action-secondary",onclick:move |_|ondiscard.call(()),"Back to setup"} };
        }
    };
    let Some((track, comparison)) = comparison() else {
        return rsx! {p {"Preparing chart timing…"}};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {p {"Preparing chart timing…"}};
    };
    let range = review.range;
    let passages = comparison.passages(range);
    let view = active_window().unwrap_or(range);
    let position = (studio.playhead)();
    let transport = studio.doc.read().as_ref().map(|doc| doc.transport.clone());
    let playing = transport.as_ref().is_some_and(|t| t.playing);
    let rate = transport.as_ref().map_or(1., |t| t.playback_rate);
    let previous = passages
        .iter()
        .copied()
        .rfind(|s| *s < view.start - 0.01);
    let next = passages.iter().copied().find(|s| *s >= view.end - 0.01);
    let added = comparison
        .events
        .iter()
        .filter(|e| e.difference == Difference::Added)
        .count();
    let removed = comparison
        .events
        .iter()
        .filter(|e| e.difference == Difference::Removed)
        .count();
    let proposed = review.proposed.clone();
    let proposed_track = track.clone();
    let current_track = track.clone();
    let changes_track = track.clone();
    let affected = review.changes.len();
    let title = super::track_picker::title(&track);
    let position_label = super::state::clock(position);
    let view_label = format!(
        "{}–{}",
        super::state::clock(view.start),
        super::state::clock(view.end)
    );
    let source_choice = review
        .proposed
        .content
        .analysis_sources
        .contains_key(&track);
    let shown = if mode() == View::Proposed {
        Some(review.proposed.clone())
    } else {
        (studio.doc)()
    };
    let missing_source = shown.as_ref().is_some_and(|doc| {
        doc.content
            .analysis_sources
            .get(&track)
            .is_some_and(|intervals| {
                intervals.iter().any(|interval| {
                    doc.content.audition_stems(&track).is_none_or(|manifest| {
                        !manifest.stems.iter().any(|stem| {
                            stem.id == interval.source.stem() && crate::storage::is_file(&stem.path)
                        })
                    })
                })
            })
    });
    rsx! { section { class:"ra-review",
        div {class:"ra-review-intro",div {strong{"Review {title}"}p {"Listen to a short passage, compare the notes, then apply when you’re ready."}}span {class:"ra-draft-badge","Unapplied draft"}}
        div {class:"ra-change-summary",span {class:"ra-added","+{added} added"}span {class:"ra-removed","−{removed} removed"}span {"{passages.len()} changed passages"}}
        if affected > 1 {label {class:"ra-field","Difficulty",select {aria_label:"Compare difficulty",value:"{track}",onchange:move |event|{let track=event.value();selected.set(Some(track.clone()));view_start.set(None);if let Some(current)=studio.doc.peek().clone(){show_version(studio,(*current).clone(),&track);}mode.set(View::Changes);},for change in &review.changes {option {value:"{change.track}","{super::track_picker::title(&change.track)}"}}}}}
        if stale {p {class:"ra-stale",role:"status","Settings changed. Analyze again before applying this draft."}}
        div {class:"ra-review-toolbar",
            div {class:"ra-segments",role:"group",aria_label:"Chart comparison",
                button {aria_pressed:mode()==View::Changes,onclick:move |_|{mode.set(View::Changes);if let Some(current)=studio.doc.peek().clone(){show_version(studio,(*current).clone(),&changes_track);}},"Changes"}
                button {aria_pressed:mode()==View::Current,onclick:move |_|{mode.set(View::Current);if let Some(current)=studio.doc.peek().clone(){show_version(studio,(*current).clone(),&current_track);}},"Current"}
                button {aria_pressed:mode()==View::Proposed,disabled:stale,onclick:move |_|{mode.set(View::Proposed);show_version(studio,(*proposed).clone(),&proposed_track);},"Proposed"}
            }
            label {class:"ra-inline-field","Show",select {aria_label:"Visible passage length",value:"{span()}",onchange:move |event|{if let Ok(value)=event.value().parse(){span.set(value);}},option{value:"4","4 seconds"}option{value:"8","8 seconds"}option{value:"16","16 seconds"}}}
        }
        div {class:"ra-passage-nav",
            button {class:"action-secondary",disabled:view.start<=range.start,onclick:move |_|{let start=(view.start-(view.end-view.start)).max(range.start);view_start.set(Some(start));studio.seek(start);},aria_label:"Previous passage","←"}
            strong {"{view_label}"}
            button {class:"action-secondary",disabled:view.end>=range.end,onclick:move |_|{let start=window(range,view.end,span()).start;view_start.set(Some(start));studio.seek(start);},aria_label:"Next passage","→"}
            button {class:"ra-link",disabled:previous.is_none(),onclick:move |_|{if let Some(start)=previous {let start=(start-1.).max(range.start);view_start.set(Some(start));studio.seek(start);}},"Previous change"}
            button {class:"ra-link",disabled:next.is_none(),onclick:move |_|{if let Some(start)=next {let start=(start-1.).max(range.start);view_start.set(Some(start));studio.seek(start);}},"Next change"}
        }
        div {class:"ra-legend",span {class:"ra-added","+ Added"}span {class:"ra-removed","− Removed"}span {"Dimmed: unchanged"}if track.ends_with("Drums"){span {"Square: cymbal · rounded: pad"}}}
        PassageChart {studio,comparison,track:track.clone(),view,mode:mode(),timing,position}
        label {class:"ra-scrub",span {"Seek within passage"}input {r#type:"range",min:"{view.start}",max:"{view.end}",step:"0.01",value:"{position.clamp(view.start,view.end)}",aria_label:"Review playhead",onkeydown:move |event|event.stop_propagation(),oninput:move |event|{if let Ok(seconds)=event.value().parse(){studio.seek(seconds);}}}}
        label {class:"ra-scrub",span {"Passage position in analysis"}input {r#type:"range",min:"{range.start}",max:"{(range.end - (view.end-view.start)).max(range.start)}",step:"0.1",value:"{view.start}",aria_label:"Review passage position",onkeydown:move |event|event.stop_propagation(),oninput:move |event|{if let Ok(start)=event.value().parse(){view_start.set(Some(start));studio.seek(start);}}}}
        div {class:"ra-listen-toolbar",
            button {class:"action-primary",onclick:move |_|{auditioning.set(!playing);if !playing{owns_loop.set(true);studio.dispatch(Action::SetLoop {range:if loop_passage(){Some(view)}else{None}});if !(view.start..view.end).contains(&position){studio.seek(view.start);}}studio.dispatch(Action::Playback {playing:!playing});},if playing{"Pause"}else{"Play passage"}}
            button {class:"action-secondary",onclick:move |_|studio.seek(view.start),"Restart"}
            label {class:"ra-check",input {r#type:"checkbox",checked:loop_passage(),onchange:move |event|{loop_passage.set(event.checked());view_start.set(Some(view.start));owns_loop.set(true);if !event.checked(){studio.dispatch(Action::SetLoop {range:None});}}},"Loop passage"}
            label {class:"ra-inline-field","Speed",select {aria_label:"Review playback speed",value:"{rate}",onchange:move |event|{if let Ok(rate)=event.value().parse(){studio.set_playback_rate(rate);}},option{value:"0.25","25%"}option{value:"0.5","50%"}option{value:"0.75","75%"}option{value:"1","100%"}option{value:"1.25","125%"}option{value:"1.5","150%"}option{value:"2","200%"}if ![0.25,0.5,0.75,1.,1.25,1.5,2.].contains(&rate){option{value:"{rate}","{rate * 100.:.0}%"}}}}
            span {class:"ra-hint","{position_label}"}
        }
        if source_choice {label {class:"ra-check",input {r#type:"checkbox",checked:!(studio.follow_analysis_sources)(),onchange:move |event|studio.follow_analysis_sources.set(!event.checked())},"Hear original mix"}}
        if missing_source {p {class:"ra-hint",role:"status","Selected stem unavailable · playing the original recording."}}
        p {class:"ra-hint","Audio plays the recording or selected stem. Compare chart timing and lanes as you listen."}
        div {class:"ra-review-actions",div {strong {"Apply the entire draft"}p {class:"ra-hint","Updates all {affected} affected difficulties. You can undo this edit."}}button {class:"action-secondary",onclick:move |_|{studio.show_analysis_preview(None);ondiscard.call(());},"Discard draft"}button {class:"action-primary",disabled:stale,onclick:move |_|{studio.analysis_preview.set(None);onapply.call(());},"Apply draft"}}
        details {class:"ra-advanced",summary {"Affected difficulties ({affected})"}ul {class:"ra-change-list",for change in &review.changes {li {"{super::track_picker::title(&change.track)}: {change.before} → {change.after} notes"}}}}
        details {class:"ra-advanced ra-evidence",summary {"Analysis evidence & limitations"}p {class:"ra-hint","Generated notes need listening review."}for warning in &review.warnings {p {class:"ra-hint","{warning}"}}}
    } }
}
