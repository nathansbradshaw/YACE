//! Flag review and decisions use the same commands as the headless editor.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
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
                .map(|region| (region, doc.selected_track))
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
        super::modal::Modal { id: "review-dialog", label: "Review flagged sections", onclose: move |_| studio.review_open.set(false),
            div { class: "review-modal", style: "background-color: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: 8px; padding: 24px; width: 1000px; max-width: 95vw; max-height: 80vh; overflow-y: auto; box-shadow: 0 12px 40px rgba(0,0,0,0.6); display: flex; flex-direction: column;", onclick: move |event| event.stop_propagation(),
                div { class: "panel-header", style: "padding: 0 0 14px; background: transparent;",
                    span { class: "panel-header-title", "Review flagged sections" }
                    button { class: "icon-button", title: "Close review", aria_label: "Close review", onclick: move |_| studio.review_open.set(false), Icon { name: "close" } }
                }
                div { class: "property-row", style: "margin: 16px 0; align-items: end;",
                    div { class: "property-group property-group-half", label { class: "property-label", "Track" }
                        select { aria_label: "Review track", onkeydown: move |event| event.stop_propagation(), class: "property-select", value: "{track}", onchange: move |event| studio.dispatch(Action::SelectTrack { track: event.value() }),
                            for track in doc.content.tracks.keys() { option { value: "{track}", selected: track == &doc.selected_track, "{track}" } }
                        }
                    }
                    div { class: "property-group property-group-half", label { class: "property-label", "Reason" }
                        select { aria_label: "Review reason filter", onkeydown: move |event| event.stop_propagation(), class: "property-select", value: "{reason}", onchange: move |event| reason.set(event.value()),
                            option { value: "", selected: reason().is_empty(), "Any reason" }
                            for entry in Reason::ALL {
                                option { value: "{entry:?}", selected: format!("{entry:?}") == reason(), "{entry.label()}" }
                            }
                        }
                    }
                    label { class: "property-checkbox", style: "margin-bottom: 18px;", input { onkeydown: move |event| event.stop_propagation(), r#type: "checkbox", checked: pending_only(), onchange: move |event| pending_only.set(event.checked()) } span { "Pending only" } }
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
                                rsx! { button { class: if active { "explorer-song-item active" } else { "explorer-song-item" }, style: "width: 100%; color: inherit; background-color: transparent; text-align: left; margin-bottom: 6px;", onclick: move |_| studio.dispatch(Action::SelectRegion { region_id: id.clone() }),
                                    div { class: "explorer-song-info", div { class: "explorer-song-name", "{clock(region.start)}–{clock(region.end)}" } div { class: "explorer-song-artist", "{count} flags · {status:?} · {region.priority:?}" } }
                                } }
                            }
                        }
                    }
                    div {
                        if let Some(region) = selected {
                            div { class: "property-section-title", "{clock(region.start)}–{clock(region.end)} · {track}" }
                            {
                                let id = region.id.clone();
                                rsx! { button { class: "property-button property-button-metadata", onclick: move |_| { studio.dispatch(Action::SelectRegion { region_id: id.clone() }); studio.review_open.set(false); }, "Jump to notes and loop section" } }
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
                                div { class: "property-group", style: "margin-top: 10px;", label { class: "property-label", "Reviewer" } input { aria_label: "Reviewer", onkeydown: move |event| event.stop_propagation(), class: "property-input", value: "{studio.reviewer}", oninput: move |event| studio.reviewer.set(event.value()) } }
                                div { class: "property-group", label { class: "property-label", "Evidence or reason" } textarea { aria_label: "Review evidence or reason", onkeydown: move |event| event.stop_propagation(), class: "property-input", rows: "3", value: "{comment}", placeholder: "What did you check or change?", oninput: move |event| comment.set(event.value()) } }
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
}
