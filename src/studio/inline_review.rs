//! Section review controls share the note grid's recording-time coordinates.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::{
    editor::{Action, DecisionStatus, Project},
    review_regions::Region,
    timing::TempoMap,
};
use dioxus::prelude::*;

pub fn flag_color(doc: &Project, region: &str, track: &str, timing: &TempoMap) -> &'static str {
    if doc.is_flag_hidden(region, track) {
        "#77818c"
    } else if doc.decision(region, track, timing).is_some_and(|d| {
        matches!(
            d.status,
            DecisionStatus::Reviewed | DecisionStatus::Resolved
        )
    }) {
        "#41c6b8"
    } else {
        "#f2a65a"
    }
}
fn select(studio: Studio, track: String, region_id: String) {
    studio.dispatch(Action::SelectTrack { track });
    studio.dispatch(Action::SelectRegion { region_id });
}
fn approve(studio: Studio, track: String, region_id: String) {
    studio.dispatch(Action::Decide {
        region_id,
        track,
        status: DecisionStatus::Reviewed,
        comment: "Approved current section notes from inline timeline".into(),
    });
}
#[component]
fn Actions(
    studio: Studio,
    track: String,
    region_id: String,
    hidden: bool,
    approved: bool,
) -> Element {
    let approve_track = track.clone();
    let approve_id = region_id.clone();
    let visibility_icon = if hidden { "eye" } else { "eye-off" };
    rsx! {
        button { class:"review-approve", title:"Approve current notes in this section and track",disabled:approved,
            onclick:move |_|approve(studio,approve_track.clone(),approve_id.clone()),
            Icon{name:"check",size:12} if approved { "Approved" } else { "Approve" }
        }
        button { class:"review-hide", title:if hidden{"Show this section flag again"}else{"Hide this flag without approving its notes"},
            onclick:move |_|studio.dispatch(Action::SetFlagVisibility{region_id:region_id.clone(),track:track.clone(),hidden:!hidden}),
            Icon{name:visibility_icon,size:12} if hidden { "Unhide" } else { "Hide" }
        }
    }
}
#[component]
pub fn InlineReview(studio: Studio, track: String, show_hidden: bool, plot_width: f64) -> Element {
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    let regions: Vec<_> = doc
        .review
        .regions
        .iter()
        .filter(|r| {
            r.tracks.contains(&track)
                && r.end > start
                && r.start < start + span
                && (show_hidden || !doc.is_flag_hidden(&r.id, &track))
        })
        .collect();
    let selected = regions
        .iter()
        .copied()
        .find(|r| doc.selected_region.as_ref() == Some(&r.id) && doc.selected_track == track)
        .cloned();
    rsx! {
        div { style:"display:flex;min-height:32px;border-bottom:1px solid var(--border-color);background:var(--bg-secondary)",
            onkeydown: move |event: KeyboardEvent| event.stop_propagation(),
            div { style:"width:140px;flex-shrink:0;display:flex;align-items:center;padding-left:12px;gap:6px;font-size:10px;color:var(--text-muted)",Icon{name:"flag",size:12} "Review" }
            div { style:"position:relative;flex:1;min-width:0;height:32px;overflow:hidden",
                for region in regions {
                    {let hidden=doc.is_flag_hidden(&region.id,&track);let color=flag_color(doc,&region.id,&track,&timing);
                    let a=((region.start-start)/span).clamp(0.0,1.0);let b=((region.end-start)/span).clamp(0.0,1.0);
                    let wide=(b-a)*plot_width>=280.0;let active=doc.selected_region.as_ref()==Some(&region.id)&&doc.selected_track==track;
                    let approved=doc.decision(&region.id,&track,&timing).is_some_and(|d|d.status==DecisionStatus::Reviewed);
                    let label=region.issues.iter().filter(|i|i.track==track).map(|i|i.reason.label()).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ");
                    let region_id=region.id.clone();let select_track=track.clone();let outline=if active{"1px solid white"}else{"none"};let opacity=if hidden{".65"}else{"1"};
                    rsx! { div { key:"{region.id}", style:"position:absolute;left:{a*100.0}%;width:{((b-a)*100.0).max(0.2)}%;top:3px;height:26px;display:flex;align-items:center;gap:4px;border:1px solid {color};border-radius:4px;background:color-mix(in srgb,{color} 16%,var(--bg-secondary));outline:{outline};overflow:hidden;padding:0 3px;opacity:{opacity}",
                        button { style:"flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:left;font-size:10px;color:{color};border:0;background:transparent;padding:3px 1px",
                            title:"{clock(region.start)}–{clock(region.end)}: {label}. Select to listen and review this track.","aria-label":"Review {track} section at {clock(region.start)}: {label}",
                            onclick:move |_|select(studio,select_track.clone(),region_id.clone()),"{clock(region.start)} {label}"
                        }
                        if wide {Actions{studio,track:track.clone(),region_id:region.id.clone(),hidden,approved}}
                    }} }
                }
            }
        }
        if let Some(region)=selected { Details{studio,track,region} }
    }
}
#[component]
fn Details(studio: Studio, track: String, region: Region) -> Element {
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let hidden = doc.is_flag_hidden(&region.id, &track);
    let status = doc
        .decision(&region.id, &track, &timing)
        .map(|d| d.status)
        .unwrap_or(DecisionStatus::Pending);
    rsx! {div { style:"padding:8px 12px 8px 152px;background:var(--bg-secondary);border-bottom:1px solid var(--border-color);font-size:11px;display:flex;gap:10px;align-items:flex-start",
        onkeydown: move |event: KeyboardEvent| event.stop_propagation(),
        div { style:"flex:1;min-width:0",
            div {style:"font-weight:600;margin-bottom:4px","{clock(region.start)}–{clock(region.end)}" if hidden {span{class:"review-state-badge","Hidden"}}}
            if doc.review.input_sha256.get("chart").is_some_and(|hash|hash!=&*studio.chart_hash.read()){
                span{class:"review-stale-badge",title:"Notes changed since these checks. Evidence describes the original draft; approval applies to the current section notes.","Checks need refresh"}
            }
            div {style:"max-height:92px;overflow:auto",
                for issue in region.issues.iter().filter(|i|i.track==track) {
                    {let evidence=issue.evidence.iter().map(|(k,v)|format!("{k}: {v:.4}")).collect::<Vec<_>>().join(", ");
                    rsx!{details{class:"review-evidence",summary{span{"{clock(issue.start)}"} strong{"{issue.reason.label()}"}} div{class:"review-evidence-body","{issue.reason.description()}" if !evidence.is_empty(){div{class:"review-evidence-values","{evidence}"}}}}}}
                }
            }
        }
        Actions{studio,track,region_id:region.id,hidden,approved:status==DecisionStatus::Reviewed}
    }}
}
