//! Song and track navigation. Component ancestry: THIRD_PARTY.md.
use super::{
    icons::Icon,
    state::{Studio, clock, instrument},
};
use crate::{
    editor::{Action, DecisionStatus, Project},
    review_regions::Region,
    timing::TempoMap,
};
use dioxus::prelude::*;

fn pending_regions<'a>(
    doc: &'a Project,
    track: &str,
    timing: Option<&TempoMap>,
) -> Vec<&'a Region> {
    let Some(timing) = timing else {
        return Vec::new();
    };
    doc.review
        .regions
        .iter()
        .filter(|region| {
            region.tracks.contains(track)
                && !doc.is_flag_hidden(&region.id, track)
                && doc
                    .decision(&region.id, track, timing)
                    .is_none_or(|decision| decision.status == DecisionStatus::Pending)
        })
        .collect()
}
#[component]
pub fn Explorer(studio: Studio) -> Element {
    let doc = studio.snapshot();
    let song_name = studio.metadata_value("name");
    let artist = studio.metadata_value("artist");
    let art = (studio.album_art)();
    let title = doc.as_ref().map_or(String::new(), |d| {
        if song_name.is_empty() {
            d.title.clone()
        } else {
            song_name.clone()
        }
    });
    let timing = (studio.timing)();
    rsx! {
        div { class: "project-explorer",
            div { class: "panel-header",
                span { class: "panel-header-title", span { Icon { name: "music" } } span { "Song" } }
                div { class: "panel-header-actions",
                    button { class: "icon-button", title: "Create project from song folder", aria_label: "Create project from song folder", onclick: move |_| studio.create_dialog(), Icon { name: "plus", size: 16 } }
                    button { class: "icon-button", title: "Open review project", aria_label: "Open review project", onclick: move |_| studio.open_dialog(), Icon { name: "folder-open" } }
                }
            }
            div { class: "panel-content",
                if let Some(doc) = doc {
                    div { class: "song-identity", title: "{doc.original_folder.display()}",
                                div { class: "explorer-song-icon",
                                    if let Some(art) = art { img { class: "explorer-song-art", src: "{art}", alt: "Album artwork" } }
                                    else { span { class: "explorer-song-art-placeholder", Icon { name: "music" } } }
                                }
                                div { class: "explorer-song-info",
                                    div { class: "explorer-song-name", title: "{title}", "{title}" }
                                    div { class: "explorer-song-artist", "{artist}" }
                                }
                    }
                    div { class: "explorer-section-heading", "Tracks" }
                    div { class: "track-navigation",
                        button { class: if doc.selected_track == "Vocals" { "track-navigation-row track-vocals active" } else { "track-navigation-row track-vocals" }, aria_label: "Select Vocals", aria_pressed: (doc.selected_track == "Vocals").to_string(), onclick: move |_| {studio.dispatch(Action::SelectTrack {track: "Vocals".into()}); studio.property_tab.set("Note".into());}, "Vocals" }
                        for name in ["Guitar", "Bass", "Drums", "Piano", "Pro Piano"] {
                            if doc.content.tracks.keys().any(|track| instrument(track) == name) {
                              div { class: "track-family",
                                div { class: "track-family-label", Icon { name: match name { "Bass" => "bass", "Drums" => "drums", _ => "guitar" }, size: 14 } "{name}" }
                                div { class: "track-tiers",
                                for tier in ["Expert", "Hard", "Medium", "Easy"] {
                                    for track in doc.content.tracks.keys().filter(|track| instrument(track) == name && track.starts_with(tier)) {
                                        { let target=track.clone(); let active=track==&doc.selected_track;
                                          let count = pending_regions(&doc, track, timing.as_ref()).len();
                                          rsx! { button { class: if active { "track-navigation-row active" } else { "track-navigation-row" }, aria_label: "Select {name} {tier}", aria_pressed: active.to_string(), onclick: move |_| studio.dispatch(Action::SelectTrack { track: target.clone() }),
                                              span { class: "track-tier", "{tier}" }
                                              if count>0 { span { class: "flag-count", title: "{count} pending review sections", Icon { name: "flag", size: 10 } "{count}" } }
                                          } }
                                        }
                                    }
                                }
                                }
                              }
                            }
                        }
                    }
                    div { class: "explorer-section-heading", "Review queue" }
                    div { class: "review-queue",
                        { let pending = pending_regions(&doc, &doc.selected_track, timing.as_ref());
                          rsx! {
                            if pending.is_empty() { div { class: "review-queue-empty", "No pending flags", span { "Listen through unflagged parts too." } } }
                            for region in pending {
                                { let id=region.id.clone(); let reasons=region.issues.iter().filter(|issue|issue.track==doc.selected_track).map(|issue|issue.reason.label()).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ");
                                  rsx! { button { class: if doc.selected_region.as_ref()==Some(&region.id) { "review-queue-row active" } else { "review-queue-row" }, title: "{clock(region.start)}–{clock(region.end)} · {reasons}", onclick: move |_| studio.dispatch(Action::SelectRegion { region_id: id.clone() }),
                                      Icon { name: "flag", size: 12 } span { class: "review-queue-time", "{clock(region.start)}" } span { class: "review-queue-reason", "{reasons}" }
                                  } }
                                }
                            }
                          }
                        }
                    }
                } else {
                    div { class: "empty-state",
                        div { class: "empty-state-icon", Icon { name: "folder-open", size: 40 } }
                        div { class: "empty-state-title", "No Project Open" }
                        div { class: "empty-state-description", "Open a saved review project or create one from a song folder." }
                        button { class: "explorer-open-button", onclick: move |_| studio.create_dialog(), "Open Song Folder" }
                        button { class: "explorer-open-button", onclick: move |_| { let mut s = studio; s.generation_open.set(true); }, "Import audio" }
                    }
                }
            }
        }
    }
}
