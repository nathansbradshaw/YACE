//! A read-only proposal overlay; only Apply enters the editor command log.
use super::state::Studio;
use crate::{editor::Project, parts::Prepared, reanalysis::Change};
use anyhow::{Context, Result};
use dioxus::prelude::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct Review {
    proposed: Project,
    range: crate::editor::TimeRange,
    changes: Vec<Change>,
    warnings: Vec<String>,
}
fn load(store: crate::editor::Store, current: Arc<Project>, job: PathBuf) -> Result<Review> {
    let draft = Prepared::load(&store, &current, &job)?;
    let range = draft.range().unwrap_or(crate::editor::TimeRange {
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
        proposed,
        range,
        changes,
        warnings: generated.review.coverage_limits,
    })
}

#[component]
pub fn ReviewDraft(
    mut studio: Studio,
    job: PathBuf,
    stale: bool,
    onapply: EventHandler<()>,
    ondiscard: EventHandler<()>,
) -> Element {
    let mut selected_comparison = use_signal(|| None::<String>);
    let review = use_resource(move || {
        let job = job.clone();
        // Transport edits may change the revision; load always rechecks the content witnesses.
        let _revision = studio.doc.read().as_ref().map(|doc| doc.revision);
        let inputs = (studio.store)().zip((studio.doc)());
        async move {
            let (store, current) =
                inputs.ok_or_else(|| "Open the target chart first".to_owned())?;
            #[cfg(not(target_arch = "wasm32"))]
            {
                tokio::task::spawn_blocking(move || load(store, current, job))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| format!("{error:#}"))
            }
            #[cfg(target_arch = "wasm32")]
            {
                load(store, current, job).map_err(|error| format!("{error:#}"))
            }
        }
    });
    use_effect(move || {
        if !(studio.generation_open)() || stale {
            studio.analysis_preview.set(None);
        }
    });
    use_drop(move || studio.analysis_preview.set(None));
    let value = review.read();
    let Some(result) = value.as_ref() else {
        return rsx! { p { role: "status", "Preparing draft comparison…" } };
    };
    let review = match result {
        Ok(review) => review.clone(),
        Err(error) => {
            return rsx! { p { role: "alert", "{error}" } button { onclick: move |_| ondiscard.call(()), "Discard draft" } };
        }
    };
    let default_track = review
        .changes
        .iter()
        .find(|change| change.track == review.proposed.selected_track)
        .map(|change| change.track.clone())
        .unwrap_or_else(|| review.proposed.selected_track.clone());
    let review_track = selected_comparison()
        .filter(|track| review.changes.iter().any(|change| &change.track == track))
        .unwrap_or(default_track);
    let range = review.range;
    let mut proposed = review.proposed.clone();
    proposed.selected_track = review_track.clone();
    let showing_proposed = (studio.analysis_preview)().is_some();
    let shown = if showing_proposed {
        Some(Arc::new(review.proposed.clone()))
    } else {
        (studio.doc)()
    };
    let timing = (studio.timing)();
    let missing_source = shown.as_ref().is_some_and(|doc| {
        doc.content
            .analysis_sources
            .get(&doc.selected_track)
            .is_some_and(|intervals| {
                intervals.iter().any(|interval| {
                    doc.content
                        .audition_stems(&doc.selected_track)
                        .is_none_or(|manifest| {
                            !manifest.stems.iter().any(|stem| {
                                stem.id == interval.source.stem()
                                    && crate::storage::is_file(&stem.path)
                            })
                        })
                })
            })
    });
    let mut glyphs = Vec::new();
    if let (Some(doc), Some(timing)) = (&shown, &timing) {
        if review_track == "Vocals" {
            if let Some(vocals) = &doc.content.vocals {
                for note in &vocals.notes {
                    let seconds = timing.seconds(note.start);
                    if crate::reanalysis::contains(range, seconds) {
                        let lane = match note.tone {
                            crate::vocals::VocalTone::Pitched { midi } => usize::from(midi % 12),
                            _ => 0,
                        };
                        glyphs.push((
                            (seconds - range.start) / (range.end - range.start) * 600.,
                            lane as f64 * 6.,
                            ((timing.seconds(note.end) - seconds) / (range.end - range.start)
                                * 600.)
                                .max(3.),
                        ));
                    }
                }
            }
        } else if let Some(notes) = doc.content.tracks.get(&review_track) {
            for note in notes {
                let seconds = timing.seconds(note.note.tick);
                if crate::reanalysis::contains(range, seconds)
                    && crate::editor::playable(&review_track, note.note.lane)
                {
                    glyphs.push((
                        (seconds - range.start) / (range.end - range.start) * 600.,
                        f64::from(note.note.lane % 12) * 6.,
                        ((timing.seconds(note.note.tick.saturating_add(note.note.sustain))
                            - seconds)
                            / (range.end - range.start)
                            * 600.)
                            .max(3.),
                    ));
                }
            }
        }
    }
    rsx! { section { class: "reanalysis-review",
        h3 { "Review reanalysis" }
        label { "Compare difficulty", select { value: "{review_track}", onchange: move |event| {
            let track = event.value(); selected_comparison.set(Some(track.clone()));
            if let Some(current) = (studio.analysis_preview)() { let mut current=(*current).clone();current.selected_track=track;studio.show_analysis_preview(Some(current)); }
        }, for change in &review.changes { option { value: "{change.track}", "{change.track}" } } } }
        p { "Repair range: {range.start:.2}–{range.end:.2} seconds. Crossing notes and vocal phrases are included." }
        if stale { p { role: "status", "Settings changed. Analyze again before applying." } }
        div { class: "generation-actions",
            button { aria_pressed: !showing_proposed, onclick: move |_| studio.show_analysis_preview(None), "Current" }
            button { disabled: stale, aria_pressed: showing_proposed, onclick: move |_| studio.show_analysis_preview(Some(proposed.clone())), "Proposed" }
            button { onclick: move |_| { let playing = studio.doc.peek().as_ref().is_some_and(|doc| doc.transport.playing); studio.dispatch(crate::editor::Action::Playback { playing: !playing }); }, "Play / Pause" }
            button { onclick: move |_| studio.dispatch(crate::editor::Action::Seek { seconds: range.start }), "Go to repair" }
        }
        label { input { r#type: "checkbox", checked: (studio.follow_analysis_sources)(), onchange: move |event| studio.follow_analysis_sources.set(event.checked()) }, "Follow section sound sources (clear to hear original recording)" }
        if missing_source { p { role: "status", "A section’s selected stem is unavailable; that section plays the original recording." } }
        svg { view_box: "0 0 600 80", width: "100%", height: "80", role: "img", "aria-label": if showing_proposed { "Proposed notes" } else { "Current notes" },
            rect { width: "600", height: "80", fill: "#111820" }
            for (x,y,width) in glyphs { rect { x: "{x}", y: "{y}", width: "{width}", height: "5", fill: "#66b7ff" } }
        }
        ul { for change in review.changes { li { "{change.track}: {change.before} → {change.after} notes · +{change.added} / −{change.removed}" } } }
        details { summary { "Detection warnings" } for warning in review.warnings { p { "{warning}" } } p { "Sensitivity controls detection strictness; instrument identity and musical accuracy require listening review." } }
        div { class: "generation-actions",
            button { class: "action-primary", disabled: stale, onclick: move |_| { studio.analysis_preview.set(None); onapply.call(()); }, "Apply reanalysis" }
            button { onclick: move |_| { studio.analysis_preview.set(None); ondiscard.call(()); }, "Discard draft" }
        }
    } }
}
