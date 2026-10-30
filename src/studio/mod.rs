//! Project actions, audition, contextual review and the chart-editing desk.
mod assistant;
mod brand;
#[cfg(feature = "browser")]
mod browser_clipboard;
mod chart_desk;
mod clean;
mod collaboration;
mod diagnostics;
mod dialogs;
mod difficulty;
mod frame_pacing;
mod game_preview;
mod generation;
mod icons;
mod inline_review;
mod modal;
mod note_canvas;
mod note_gestures;
mod part_analysis;
mod parts;
mod practice;
mod presentation_clock;
mod project_bar;
mod review;
mod sections;
mod selection;
mod selection_desk;
mod sheet;
mod shortcuts;
mod star_power;
pub mod state;
mod stems;
mod timing_editor;
mod track_picker;
mod vocal_editor;
mod vocal_preview;
mod welcome;
mod workspace;
use crate::editor::Action;
use dioxus::prelude::*;
use std::time::Duration;
#[cfg(not(feature = "browser"))]
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};
#[cfg(not(feature = "browser"))]
static START_PROJECT: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();

#[cfg(not(feature = "browser"))]
pub fn open(path: &Path) -> anyhow::Result<()> {
    run(Some(path.canonicalize()?));
    Ok(())
}
#[cfg(not(feature = "browser"))]
pub fn launch() -> anyhow::Result<()> {
    run(None);
    Ok(())
}
#[cfg(not(feature = "browser"))]
fn run(project: Option<std::path::PathBuf>) {
    *START_PROJECT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = project;
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("YACE")
                    .with_inner_size(dioxus::desktop::LogicalSize::new(1400.0, 900.0))
                    .with_min_inner_size(dioxus::desktop::LogicalSize::new(1024.0, 600.0)),
            ),
        )
        .launch(App);
}
#[cfg(feature = "browser")]
pub fn open(path: &std::path::Path) -> anyhow::Result<()> {
    crate::browser::serve_project(path, 3030)
}
#[cfg(feature = "browser")]
pub fn launch() -> anyhow::Result<()> {
    crate::browser::serve(std::path::Path::new("output/browser-workspace"), 3030)
}
const CSS: &str = concat!(
    include_str!("../../assets/theme.css"),
    include_str!("../../assets/forms.css"),
    include_str!("../../assets/workspace.css"),
    include_str!("../../assets/timeline.css"),
    include_str!("../../assets/dialogs.css"),
    include_str!("../../assets/assistant.css"),
    include_str!("../../assets/details.css"),
    include_str!("../../assets/workbench.css"),
    include_str!("../../assets/part-analysis.css")
);
#[component]
pub(crate) fn App() -> Element {
    #[cfg(feature = "browser")]
    dialogs::provide_browser_dialogs();
    use_hook(diagnostics::install);
    let studio = state::use_studio();
    difficulty::use_snapshot(studio);
    let display_link = frame_pacing::use_frame_pacing(studio);
    #[cfg(feature = "browser")]
    let initial_project =
        use_signal(|| consume_context::<crate::browser::BrowserContext>().start_project);
    use_future(move || async move {
        #[cfg(not(feature = "browser"))]
        let project = START_PROJECT.get().and_then(|p| p.lock().ok()?.take());
        #[cfg(feature = "browser")]
        let project = initial_project();
        if let Some(path) = project {
            studio.open_project(path).await;
        }
        let mut count = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(16)).await;
            // Keep the clock usable if WebView suspends animation callbacks or
            // loses its display link. A resumed display link takes over again.
            studio.tick(display_link.peek().elapsed() > Duration::from_millis(100));
            count += 1;
            if count % 18 == 0 {
                studio.poll();
            }
        }
    });
    use_drop(move || {
        if let Some(doc) = studio.snapshot()
            && doc.transport.playing
        {
            studio.seek(studio.current_audio_position());
            studio.dispatch(Action::Playback { playing: false });
        }
        if let Some(store) = studio.store.read().as_ref() {
            let _ = std::fs::remove_file(store.root.join("live-state.json"));
        }
    });
    let doc = studio.snapshot();
    rsx! {
        style { "{CSS}" }
        div { class: "studio-app", tabindex: "-1",
            onkeydown: move |event| shortcuts::handle(studio, event),
            project_bar::ProjectBar { studio }
            if doc.is_some() { workspace::Workbench { studio } } else { welcome::Welcome { studio } }
            div {class:"studio-status",span{title: studio.audio_details(), "{studio.status}"} if let Some(doc)=doc {span{title: "Revision {doc.revision} · {doc.selected_track}", "Saved · {state::instrument(&doc.selected_track)}"}}}
            if let Some(error)=(studio.error)(){div {class:"studio-error",role:"alert",span{"{error}"}button{onclick:move |_|{let mut s=studio;s.error.set(None)},"Dismiss"}}}
            if (studio.review_open)(){review::ReviewPanel{studio}}
            if (studio.parts_open)(){parts::PartsPanel{studio}}
            clean::CleanPanel{studio}
        }
        generation::GenerationPanel{studio}
        collaboration::Collaboration{studio}
        assistant::Assistant{studio}
        {browser_files()}
    }
}
fn browser_files() -> Element {
    #[cfg(feature = "browser")]
    {
        rsx! {dialogs::BrowserFiles {}}
    }
    #[cfg(not(feature = "browser"))]
    {
        rsx! {}
    }
}
