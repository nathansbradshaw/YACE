#[path = "../../../src/studio/brand.rs"]
mod brand;
#[path = "../../../src/studio/chart_desk.rs"]
mod chart_desk;
#[path = "../../../src/studio/collaboration.rs"]
mod collaboration;
mod dialogs;
#[path = "../../../src/studio/difficulty.rs"]
mod difficulty;
#[path = "../../../src/studio/game_preview.rs"]
mod game_preview;
mod header;
#[path = "../../../src/studio/icons.rs"]
mod icons;
#[path = "../../../src/studio/inline_review.rs"]
mod inline_review;
mod media;
#[path = "../../../src/studio/modal.rs"]
mod modal;
#[path = "../../../src/studio/note_canvas.rs"]
mod note_canvas;
#[path = "../../../src/studio/note_gestures.rs"]
mod note_gestures;
#[path = "../../../src/studio/parts.rs"]
mod parts;
#[path = "../../../src/studio/practice.rs"]
mod practice;
#[path = "../../../src/studio/review.rs"]
mod review;
#[path = "../../../src/studio/sections.rs"]
mod sections;
#[path = "../../../src/studio/selection.rs"]
mod selection;
#[path = "../../../src/studio/selection_desk.rs"]
mod selection_desk;
#[path = "../../../src/studio/sheet.rs"]
mod sheet;
#[path = "../../../src/studio/shortcuts.rs"]
mod shortcuts;
mod state;
#[path = "../../../src/studio/stems.rs"]
mod stems;
#[path = "../../../src/studio/timing_editor.rs"]
mod timing_editor;
#[path = "../../../src/studio/track_picker.rs"]
mod track_picker;
#[path = "../../../src/studio/vocal_editor.rs"]
mod vocal_editor;
#[path = "../../../src/studio/vocal_preview.rs"]
mod vocal_preview;
#[path = "../../../src/studio/workspace.rs"]
mod workspace;
use dioxus::prelude::*;
const CSS: &str = concat!(
    include_str!("../../../assets/theme.css"),
    include_str!("../../../assets/forms.css"),
    include_str!("../../../assets/workspace.css"),
    include_str!("../../../assets/timeline.css"),
    include_str!("../../../assets/dialogs.css"),
    include_str!("../../../assets/details.css"),
    include_str!("../../../assets/workbench.css"),
    include_str!("../../../assets/part-analysis.css")
);
#[component]
pub fn App() -> Element {
    let studio = state::use_studio();
    difficulty::use_snapshot(studio);
    use_future(move || async move {
        let mut studio = studio;
        match crate::persistence::restore().await {
            Ok(Some((root, revision))) if studio.store.read().is_none() => {
                if let Err(error) = studio
                    .open_project_with_revision(root, Some(revision))
                    .await
                {
                    studio.error.set(Some(format!(
                        "Could not restore the local project: {error:#}"
                    )));
                } else {
                    studio.saved_revision.set(Some(revision));
                }
            }
            Ok(_) => (),
            Err(error) => studio
                .error
                .set(Some(format!("Browser recovery storage: {error:#}"))),
        }
    });
    use_future(move || async move {
        let mut studio = studio;
        loop {
            gloo_timers::future::TimeoutFuture::new(500).await;
            if !(studio.dirty)() || (studio.persistence_busy)() {
                continue;
            }
            let Some(store) = studio.store.read().clone() else {
                continue;
            };
            let Some(doc) = studio.snapshot() else {
                continue;
            };
            let revision = doc.revision;
            let expected = (studio.saved_revision)();
            studio.persistence_busy.set(true);
            let result = crate::persistence::save(&store.root, revision, expected).await;
            studio.persistence_busy.set(false);
            if studio
                .store
                .read()
                .as_ref()
                .is_none_or(|current| current.root != store.root)
            {
                continue;
            }
            match result {
                Ok(()) => {
                    studio.saved_revision.set(Some(revision));
                    if studio
                        .snapshot()
                        .is_some_and(|doc| doc.revision == revision)
                    {
                        studio.dirty.set(false);
                    }
                }
                Err(error) => {
                    studio.dirty.set(false);
                    studio
                        .error
                        .set(Some(format!("Autosave failed: {error:#}")));
                }
            }
        }
    });
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(16).await;
            studio.tick();
        }
    });
    rsx! {style{"{CSS}"}div{class:"studio-app",tabindex:"-1",onkeydown:move|event|shortcuts::handle(studio,event),
    header::Header{studio}
    if studio.snapshot().is_some(){workspace::Workbench{studio}}else{header::Welcome{studio}}
    div{class:"studio-status",span{"{studio.status}"}span{if (studio.dirty)()||(studio.persistence_busy)(){"Saving locally…"}else if studio.snapshot().is_some_and(|doc|(studio.saved_revision)()!=Some(doc.revision)){"Not saved locally · export project ZIP"}else{"Saved locally · Rust/WASM"}}}
    if let Some(error)=(studio.error)(){div{class:"studio-error",role:"alert",span{"{error}"}button{onclick:move |_|{let mut studio=studio;studio.error.set(None);},"Dismiss"}}}
    if (studio.review_open)(){review::ReviewPanel{studio}}
    if (studio.parts_open)(){parts::PartsPanel{studio}}
    if (studio.collaboration_open)(){collaboration::Collaboration{studio}}
    if (studio.generation_open)(){modal::Modal{id:"browser-generation",label:"Browser generation",onclose:move |_|{let mut s=studio;s.generation_open.set(false);},h2{"Generation unavailable"}p{"The current model backends require native runtimes. Generation is disabled in this static build. Open a chart or add parts and notes manually."}button{onclick:move |_|{let mut s=studio;s.generation_open.set(false);},"Close generation"}}}
    }}
}
mod part_analysis {
    use super::state::Studio;
    use dioxus::prelude::*;
    #[component]
    pub fn PartAnalysisButton(studio: Studio) -> Element {
        let _ = studio;
        rsx! {button{class:"studio-tool",disabled:true,title:"Native analysis backends are unavailable in the static build","Part analysis"}}
    }
}
