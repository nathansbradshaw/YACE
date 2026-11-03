//! One track selector serves editing, audition and review.
use super::state::{Studio, instrument};
use crate::editor::{Action, Project};
use dioxus::prelude::*;

pub fn title(track: &str) -> String {
    crate::track_names::title(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    #[test]
    fn choices_keep_difficulties_together_and_include_unwritten_vocals_once() {
        let root = tempfile::tempdir().unwrap();
        let mut document = fixture::fixture(root.path()).load().unwrap();
        for track in ["EasySingle", "HardSingle", "ExpertDoubleBass", "Vocals"] {
            document.content.tracks.insert(track.into(), Vec::new());
        }
        assert_eq!(
            tracks(&document),
            [
                "ExpertSingle",
                "HardSingle",
                "EasySingle",
                "ExpertDoubleBass",
                "ExpertDrums",
                "Vocals"
            ]
        );
        assert_eq!(title("ExpertSingle"), "Guitar · Expert");
        assert_eq!(title("Vocals"), "Vocals");
    }
}

pub(super) fn tracks(document: &Project) -> Vec<String> {
    let mut tracks: Vec<_> = document.content.tracks.keys().cloned().collect();
    tracks.push("Vocals".into());
    tracks.sort_by_key(|track| {
        let family = match instrument(track) {
            "Guitar" => 0,
            "Bass" => 1,
            "Drums" => 2,
            "Piano" => 3,
            "Pro Piano" => 4,
            _ => 5,
        };
        let tier = ["Expert", "Hard", "Medium", "Easy"]
            .iter()
            .position(|tier| track.starts_with(tier))
            .unwrap_or(4);
        (family, tier, track.clone())
    });
    tracks.dedup();
    tracks
}

#[component]
pub fn TrackPicker(mut studio: Studio) -> Element {
    let document = studio
        .snapshot()
        .filter(|_| super::dance::band_visible(studio));
    let mut options: Vec<(String, String)> = document
        .as_ref()
        .map(|doc| {
            tracks(doc)
                .into_iter()
                .map(|track| {
                    let label = title(&track);
                    (track, label)
                })
                .collect()
        })
        .unwrap_or_default();
    let dance = try_consume_context::<super::dance::DanceStudio>();
    options.extend(super::dance::tracks(studio));
    let value = super::dance::selected_track()
        .or_else(|| document.as_ref().map(|d| d.selected_track.clone()))
        .unwrap_or_default();
    if options.is_empty() {
        return rsx! {};
    }
    rsx! {
        label { class: "edit-track-picker",
            span { "Track" }
            select { aria_label: "Edit track", value: "{value}",
                onkeydown: move |event| event.stop_propagation(),
                onchange: move |event| {
                    if let Some(id)=event.value().strip_prefix("dance:") { if let Some(dance)=dance{super::dance::select_track(dance,id.to_owned());} }
                    else {super::dance::deactivate();studio.dispatch(Action::SelectTrack {track:event.value()});}
                },
                for (track,label) in options {
                    option { value: "{track}", selected: value == track, "{label}" }
                }
            }
        }
        if let Some(doc) = document && crate::parts::instrument(&doc.selected_track).is_ok() {
            button { class: "studio-tool", disabled: (studio.analysis_preview)().is_some(), onclick: move |_| {
                studio.generation_target.set(Some(doc.selected_track.clone()));
                studio.generation_open.set(true);
            }, "Reanalyze…" }
        }
    }
}
