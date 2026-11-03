//! The same measured progress presentation in desktop and browser generation.
use crate::generation_feedback::{self, Feedback};
use dioxus::prelude::*;

#[component]
pub fn GenerationStatus(
    parts: Vec<String>,
    recording: String,
    step: String,
    activity: String,
    elapsed_ms: u64,
    feedback: Feedback,
    quiet_for: u64,
    #[props(default = false)] stopping: bool,
) -> Element {
    let elapsed = generation_feedback::elapsed(elapsed_ms);
    let step = feedback.operation.clone().unwrap_or(step);
    rsx! {
        style{{include_str!("../../assets/generation-status.css")}}
        section{class:"generation-status",aria_label:"Generation progress",
            div{class:"generation-status-heading",strong{"Generating your draft"}span{"Elapsed {elapsed}"}}
            if !recording.is_empty(){p{class:"generation-status-recording","{recording}"}}
            ul{class:"generation-status-parts",aria_label:"Parts being generated",for part in parts{li{"{generation_feedback::part_label(&part)}"}}}
            div{class:"generation-status-heading",strong{"{step}"}if let Some(percent)=feedback.percent{span{"{percent}% of this step"}}}
            progress{aria_label:"Current generation step",max:"100",value:feedback.percent.map(|percent|percent.to_string())}
            p{class:"generation-status-activity",role:"status",aria_live:"polite","{activity}"}
            p{class:"generation-status-estimate","{feedback.estimate}"}
            if quiet_for>=20{p{class:"generation-status-hint","Still working · last engine update {quiet_for}s ago"}}
            if stopping{p{class:"generation-status-hint","Finishing and saving this step before stopping."}}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    fn harness() -> Element {
        let percent = use_context::<Option<u32>>();
        rsx! {GenerationStatus{parts:vec!["vocals".into(),"pro_piano".into()],recording:"Progress fixture",step:"Detecting notes",activity:"Detecting vocal contour · chunk 3 of 8",elapsed_ms:61_000,feedback:Feedback{operation:Some("Detecting vocal notes".into()),percent,estimate:"About 2–4 min left in this step".into()},quiet_for:0}}
    }
    #[test]
    fn displays_frozen_parts_elapsed_eta_and_only_measured_bar_values() {
        for percent in [None, Some(25u32)] {
            let mut dom = VirtualDom::new(harness);
            dom.provide_root_context(percent);
            let changes = dom.rebuild_to_vec();
            let texts = changes
                .edits
                .iter()
                .filter_map(|change| match change {
                    Mutation::CreateTextNode { value, .. } => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for required in [
                "Vocals",
                "Pro Piano (Pro Keys)",
                "Progress fixture",
                "About 2–4 min left in this step",
            ] {
                assert!(
                    texts.iter().any(|text| text.contains(required)),
                    "{texts:?}"
                );
            }
            assert!(texts.iter().any(|text| text.contains("1:01")));
            let values = changes
                .edits
                .iter()
                .filter_map(|change| match change {
                    Mutation::SetAttribute {
                        name: "value",
                        value: AttributeValue::Text(value),
                        ..
                    } => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                values,
                if percent.is_some() {
                    vec!["25"]
                } else {
                    vec![]
                }
            );
        }
    }
}
