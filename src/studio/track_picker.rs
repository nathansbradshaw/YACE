//! One track selector serves editing, audition and review.
use super::state::{Studio, instrument};
use crate::editor::{Action, Project};
use dioxus::prelude::*;

pub fn title(track: &str) -> String {
    ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find(|tier| track.starts_with(tier))
        .map_or_else(
            || track.to_owned(),
            |tier| format!("{} · {tier}", instrument(track)),
        )
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
    let Some(document) = studio.snapshot() else {
        return rsx! {};
    };
    rsx! {
        label { class: "edit-track-picker",
            span { "Track" }
            select { aria_label: "Edit track", value: "{document.selected_track}",
                onkeydown: move |event| event.stop_propagation(),
                onchange: move |event| studio.dispatch(Action::SelectTrack { track: event.value() }),
                for track in tracks(&document) {
                    option { value: "{track}", selected: document.selected_track == track, "{title(&track)}" }
                }
            }
        }
    }
}
