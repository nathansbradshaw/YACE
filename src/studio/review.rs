//! Flag review and decisions use the same commands as the headless editor.
use super::state::{Studio, clock};
use crate::{
    editor::{Action, DecisionStatus},
    review_regions::Reason,
};
use dioxus::prelude::*;
#[component]
pub fn ReviewPanel(mut studio: Studio) -> Element {
    let mut reason = use_signal(String::new);
    let mut pending_only = use_signal(|| true);
    let mut comment = use_signal(String::new);
    let mut draft_context = use_signal(|| None::<(String, String)>);
    use_effect(move || {
        let context = studio.snapshot().and_then(|doc| {
            doc.selected_region
                .clone()
                .map(|region| (region, doc.selected_track.clone()))
        });
        if draft_context() != context {
            comment.set(String::new());
            draft_context.set(context);
        }
    });
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let track = doc.selected_track.clone();
    let filter = reason();
    let regions: Vec<_> =
        doc.review
            .regions
            .iter()
            .filter(|region| region.tracks.contains(&track))
            .filter(|region| {
                filter.is_empty()
                    || region.issues.iter().any(|issue| {
                        issue.track == track && format!("{:?}", issue.reason) == filter
                    })
            })
            .filter(|region| {
                !pending_only()
                    || doc
                        .decision(&region.id, &track, &timing)
                        .is_none_or(|decision| decision.status == DecisionStatus::Pending)
            })
            .cloned()
            .collect();
    let selected = doc
        .selected_region
        .as_ref()
        .and_then(|id| doc.review.regions.iter().find(|region| &region.id == id))
        .cloned();
    rsx! {
        super::sheet::ToolSheet {
            id: "review-dialog", title: "Review flags", description: "Listen, inspect the evidence and decide.",
            class: "review-sheet", close_label: "Close review", onclose: move |_| studio.review_open.set(false),
            actions: rsx! { button { onclick: move |_| studio.review_open.set(false), "Close" } },
                            div { class: "field-row", style: "margin: 16px 0; align-items: end;",
                    div { class: "field-stack field-half", label { class: "field-label", "Track" }
                        select { aria_label: "Review track", onkeydown: move |event| event.stop_propagation(), class: "field-control", value: "{track}", onchange: move |event| studio.dispatch(Action::SelectTrack { track: event.value() }),
                            for track in super::track_picker::tracks(&doc) { option { value: "{track}", selected: track == doc.selected_track, "{super::track_picker::title(&track)}" } }
                        }
                    }
                    div { class: "field-stack field-half", label { class: "field-label", "Reason" }
                        select { aria_label: "Review reason filter", onkeydown: move |event| event.stop_propagation(), class: "field-control", value: "{reason}", onchange: move |event| reason.set(event.value()),
                            option { value: "", selected: reason().is_empty(), "Any reason" }
                            for entry in Reason::ALL {
                                option { value: "{entry:?}", selected: format!("{entry:?}") == reason(), "{entry.label()}" }
                            }
                        }
                    }
                    label { class: "form-check", style: "margin-bottom: 18px;", input { onkeydown: move |event| event.stop_propagation(), r#type: "checkbox", checked: pending_only(), onchange: move |event| pending_only.set(event.checked()) } span { "Pending only" } }
                }
                div { class: "review-results-heading", title: "Flags guide review; unflagged sections remain unverified", "{regions.len()} sections" }
                div { style: "display: grid; grid-template-columns: minmax(220px, .85fr) minmax(300px, 1.3fr); gap: 20px;",
                    div { style: "max-height: 48vh; overflow-y: auto;",
                        for region in regions {
                            {
                                let id = region.id.clone();
                                let active = doc.selected_region.as_ref() == Some(&id);
                                let status = doc.decision(&id, &track, &timing).map_or(DecisionStatus::Pending, |d| d.status);
                                let count = region.issues.iter().filter(|issue| issue.track == track).count();
                                rsx! { button { class: if active { "review-result active" } else { "review-result" }, style: "width: 100%; color: inherit; background-color: transparent; text-align: left; margin-bottom: 6px;", onclick: move |_| studio.dispatch(Action::SelectRegion { region_id: id.clone() }),
                                    div { class: "review-result-info", div { class: "review-result-time", "{clock(region.start)}–{clock(region.end)}" } div { class: "review-result-status", "{count} flags · {status:?} · {region.priority:?}" } }
                                } }
                            }
                        }
                    }
                    div {
                        if let Some(region) = selected {
                            div { class: "form-heading", "{clock(region.start)}–{clock(region.end)} · {track}" }
                            {
                                let id = region.id.clone();
                                rsx! { button { class: "field-button metadata-action", onclick: move |_| { studio.dispatch(Action::SelectRegion { region_id: id.clone() }); studio.review_open.set(false); }, "Jump to notes and loop section" } }
                            }
                            for issue in region.issues.iter().filter(|issue| issue.track == track) {
                                details { class: "review-evidence",
                                    summary { span { "{clock(issue.start)}" } strong { "{issue.reason.label()}" } }
                                    div { class: "review-evidence-body", "{issue.reason.description()}"
                                        for (key, value) in &issue.evidence { div { class: "review-evidence-values", "{key}: {value:.4}" } }
                                    }
                                }
                            }
                            if let Some(decision) = doc.decision(&region.id, &track, &timing) {
                                p { style: "font-size: 12px; color: var(--accent-secondary); margin: 10px 0;", "{decision.status:?} · {decision.actor.kind:?}: {decision.actor.name}" }
                                p { style: "font-size: 12px; color: var(--text-secondary);", "{decision.comment}" }
                            }
                            details { class: "review-evidence review-notes", summary { "Add review notes" }
                                div { class: "field-stack", style: "margin-top: 10px;", label { class: "field-label", "Reviewer" } input { aria_label: "Reviewer", onkeydown: move |event| event.stop_propagation(), class: "field-control", value: "{studio.reviewer}", oninput: move |event| studio.reviewer.set(event.value()) } }
                                div { class: "field-stack", label { class: "field-label", "Evidence or reason" } textarea { aria_label: "Review evidence or reason", onkeydown: move |event| event.stop_propagation(), class: "field-control", rows: "3", value: "{comment}", placeholder: "What did you check or change?", oninput: move |event| comment.set(event.value()) } }
                            }
                            div { style: "display: flex; gap: 6px; flex-wrap: wrap;",
                                for (status, label) in [(DecisionStatus::Reviewed, "Approve"), (DecisionStatus::Resolved, "Resolved"), (DecisionStatus::Dismissed, "Dismiss"), (DecisionStatus::Pending, "Reopen")] {
                                    {
                                        let id = region.id.clone();
                                        let track = track.clone();
                                        rsx! { button { class: if status == DecisionStatus::Reviewed { "review-approve" } else { "review-hide" }, onclick: move |_| studio.dispatch(Action::Decide { region_id: id.clone(), track: track.clone(), status, comment: comment() }), "{label}" } }
                                    }
                                }
                                { let id=region.id.clone();let hidden=doc.is_flag_hidden(&region.id,&track);let track=track.clone();
                                  rsx! { button { class: "review-hide", title: "Visibility is separate from approval", onclick: move |_| studio.dispatch(Action::SetFlagVisibility { region_id: id.clone(), track: track.clone(), hidden: !hidden }), if hidden { "Unhide" } else { "Hide" } } }
                                }
                            }
                        } else { div { class: "empty-state", div { class: "empty-state-description", "Choose a section to inspect its evidence and record a decision." } } }
                    }
                }
                if let Some(error) = (studio.error)() { div { class: "auto-chart-error", style: "margin-top: 12px;", "{error}" } }

        }
    }
}
