#[path = "../../../src/studio/brand.rs"]
mod brand;
#[path = "../../../src/studio/chart_desk.rs"]
mod chart_desk;
#[path = "../../../src/studio/clean.rs"]
mod clean;
#[path = "../../../src/studio/collaboration.rs"]
mod collaboration;
mod dialogs;
#[path = "../../../src/studio/difficulty.rs"]
mod difficulty;
#[path = "../../../src/studio/game_preview.rs"]
mod game_preview;
mod generation_jobs;
pub mod github;
#[path = "../../../src/studio/icons.rs"]
mod icons;
#[path = "../../../src/studio/inline_review.rs"]
mod inline_review;
mod media;
#[path = "../../../src/studio/modal.rs"]
mod modal;
#[path = "../../../src/studio/new_blank.rs"]
mod new_blank;
#[path = "../../../src/studio/new_song.rs"]
mod new_song;
#[path = "../../../src/studio/note_canvas.rs"]
mod note_canvas;
#[path = "../../../src/studio/note_gestures.rs"]
mod note_gestures;
#[path = "../../../src/studio/note_types.rs"]
mod note_types;
#[path = "../../../src/studio/part_analysis.rs"]
mod part_analysis;
#[path = "../../../src/studio/parts.rs"]
mod parts;
#[path = "../../../src/studio/practice.rs"]
mod practice;
#[path = "../../../src/studio/project_bar.rs"]
mod project_bar;
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
#[path = "../../../src/studio/source_choice.rs"]
mod source_choice;
#[path = "../../../src/studio/star_power.rs"]
mod star_power;
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
#[path = "../../../src/studio/welcome.rs"]
mod welcome;
#[path = "../../../src/studio/workspace.rs"]
mod workspace;
use dioxus::prelude::*;
pub(crate) async fn decode_local(bytes: &[u8]) -> anyhow::Result<crate::audio_decode::Pcm> {
    let context = media::context()?;
    let result = media::decode(&context, bytes).await;
    let _ = context.close();
    result
}
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
    let github = github::use_session();
    use_context_provider(|| github);
    let studio = state::use_studio();
    use_context_provider(|| studio);
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
                    if expected.is_none() {
                        studio.remember_recent(store.root.clone());
                    }
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
    project_bar::ProjectBar{studio}
    if studio.snapshot().is_some(){workspace::Workbench{studio}}else{welcome::Welcome{studio}}
    div{class:"studio-status",span{"{studio.status}"}a{href:"https://github.com/nathansbradshaw/YACE",target:"_blank",rel:"noopener noreferrer",aria_label:"YACE on GitHub",style:"color:var(--accent-color)","GitHub"}span{if (studio.dirty)()||(studio.persistence_busy)(){"Saving locally…"}else if studio.snapshot().is_some_and(|doc|(studio.saved_revision)()!=Some(doc.revision)){"Not saved locally · export project ZIP"}else{"Saved locally · Rust/WASM"}}}
    if let Some(error)=(studio.error)(){div{class:"studio-error",role:"alert",span{"{error}"}button{onclick:move |_|{let mut studio=studio;studio.error.set(None);},"Dismiss"}}}
    if (studio.review_open)(){review::ReviewPanel{studio}}
    if (studio.parts_open)(){parts::PartsPanel{studio}}
    if (studio.collaboration_open)(){collaboration::Collaboration{studio}}
    generation_jobs::GenerationPanel{studio}
    if let Some((path,name,kind))=(studio.export_ready)(){modal::Modal{id:"song-export",label:"File download",onclose:move |_|studio.close_export(),h2{match kind{state::DownloadKind::Project=>"Project backup ready",state::DownloadKind::Song=>"Song export ready",state::DownloadKind::File=>"File ready to download"}}
        p{"Download the prepared copy. Your editing project stays saved in this browser."}
        p{style:"overflow-wrap:anywhere","{name}"}
        button{onclick:move |_|{let result=crate::files::download_path(&name,&path);if let Err(error)=result{let mut s=studio;s.error.set(Some(format!("{error:#}")));}},match kind{state::DownloadKind::Project=>"Download project ZIP",state::DownloadKind::Song=>"Download song ZIP",state::DownloadKind::File=>"Download file"}}
        button{onclick:move |_|studio.close_export(),"Close"}
    }}
    if let Some(entries)=(studio.saved_projects)(){modal::Modal{id:"saved-projects",label:"Saved in this browser",onclose:move |_|{let mut s=studio;s.saved_projects.set(None);},h2{"Saved in this browser"}
        if entries.is_empty(){p{"No saved projects yet. Open a chart or import a project ZIP."}}
        for entry in entries {button{class:"welcome-action",disabled:(studio.library_busy)(),onclick:move |_|{let root=entry.path.clone();dioxus::core::spawn_forever(async move{studio.open_saved_project(root).await;});},"{entry.title}"}}
        button{onclick:move |_|{let mut s=studio;s.saved_projects.set(None);},"Close"}
    }}
    clean::CleanPanel{studio}
    source_choice::SourceChoice{studio}
    if let Some((root,title))=(studio.new_song)(){new_song::Setup{studio,root,suggested_title:title}}
    if (studio.blank_chart)(){new_blank::Setup{studio}}
    }}
}
