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
#[path = "../../../src/studio/export.rs"]
mod export;
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
    div{class:"studio-status",span{"{studio.status}"}a{href:"https://github.com/nathansbradshaw/YACE",target:"_blank",rel:"noopener noreferrer",aria_label:"YACE on GitHub",style:"color:var(--accent-color)","GitHub"}span{role:"status",title:"Browser saves stay in this browser profile. Download a project backup to keep an independent copy.",if let Some(doc)=studio.snapshot(){if (studio.dirty)()||(studio.persistence_busy)(){"Project r{doc.revision} · saving in this browser…"}else if (studio.saved_revision)()!=Some(doc.revision){"Project r{doc.revision} not saved · download backup"}else{"Project saved in this browser · r{doc.revision}"}}else{"Projects save automatically in this browser"}} project_bar::ExportSummary{studio}}
    if let Some(error)=(studio.error)(){div{class:"studio-error",role:"alert",span{"{error}"}button{onclick:move |_|{let mut studio=studio;studio.error.set(None);},"Dismiss"}}}
    if (studio.review_open)(){review::ReviewPanel{studio}}
    if (studio.export_open)(){export::ExportDialog{studio}}
    if (studio.parts_open)(){parts::PartsPanel{studio}}
    if (studio.collaboration_open)(){collaboration::Collaboration{studio}}
    generation_jobs::GenerationPanel{studio}
    if let Some((path,name,kind))=(studio.export_ready)(){sheet::ToolSheet{id:"song-export",title:match kind{state::DownloadKind::Project=>"Project backup ready",state::DownloadKind::Song=>"Song export ready",state::DownloadKind::File=>"File ready to download"},description:"This download is separate from browser autosave.",close_label:"Close download",class:"export-sheet",onclose:move |_|studio.close_export(),
        actions:rsx!{
            button{onclick:move |_|studio.close_export(),"Close"}
            button{onclick:{let name=name.clone();let path=path.clone();move |_|{let result=crate::files::download_path(&name,&path);if let Err(error)=result{let mut s=studio;s.error.set(Some(format!("{error:#}")));}else if kind==state::DownloadKind::Song{studio.song_download_requested();}}},match kind{state::DownloadKind::Project=>"Download project backup",state::DownloadKind::Song=>"Download song ZIP",state::DownloadKind::File=>"Download file"}}
        },
        p{style:"overflow-wrap:anywhere","{name}"}
        if let Some(blob)=crate::files::retained_download(&path){p{class:"field-hint",{format!("ZIP size: {:.1} MiB",blob.size()/1_048_576.)}}}
    }}
    if let Some(entries)=(studio.saved_projects)(){sheet::ToolSheet{id:"saved-projects",title:"Saved in this browser",description:"Reopen an automatically saved editing project.",close_label:"Close saved projects",class:"export-sheet",onclose:move |_|{let mut s=studio;s.saved_projects.set(None);},
        actions:rsx!{button{onclick:move |_|{let mut s=studio;s.saved_projects.set(None);},"Close"}},
        p{"Projects save automatically on this browser profile and device. Browser saves do not sync to other browsers. Clearing site data removes them."}
        p{"Use Download project backup in the YACE menu to keep a complete editable copy, including undo history, or move a project to another device."}
        if entries.is_empty(){p{"No saved projects yet. Open a chart or import a project ZIP."}}
        for entry in entries {button{class:"welcome-action",disabled:(studio.library_busy)(),onclick:move |_|{let root=entry.path.clone();dioxus::core::spawn_forever(async move{studio.open_saved_project(root).await;});},"{entry.title}"}}
    }}
    clean::CleanPanel{studio}
    source_choice::SourceChoice{studio}
    if let Some((root,title))=(studio.new_song)(){new_song::Setup{studio,root,suggested_title:title}}
    if (studio.blank_chart)(){new_blank::Setup{studio}}
    }}
}
