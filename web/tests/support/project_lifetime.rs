//! Disposable completed-generation fixture. No model or user project is used.
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    generation, storage,
};
use anyhow::Result;
use dioxus::prelude::*;
use std::path::PathBuf;
thread_local! {
    static OTHER: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static READY: std::cell::RefCell<std::collections::BTreeSet<PathBuf>> = std::cell::RefCell::default();
}
pub fn ready_to_commit(root: &std::path::Path) {
    READY.with(|ready| ready.borrow_mut().insert(root.to_owned()));
}
#[component]
pub fn Controls(mut studio: crate::studio::state::Studio) -> Element {
    let mut running = use_signal(|| false);
    let mut result = use_signal(|| {
        "Open the retained completed vocal draft, then run the overlap reproduction.".to_string()
    });
    rsx! {section { style:"padding:12px;background:#382b16;color:white", aria_label:"Lifecycle fixture controls",
        button {disabled:running(),onclick:move |_|{if *running.peek(){return;}running.set(true);spawn(async move {
            let test = async {
                let other = OTHER.with(|root| root.borrow().clone()).expect("Prepared other project");
                crate::persistence::load(&other).await?;
                while (studio.dirty)() || (studio.persistence_busy)() {
                    gloo_timers::future::TimeoutFuture::new(20).await;
                }
                let root = studio.store.read().as_ref().expect("Open fixture").root.clone();
                let revision = studio.snapshot().expect("Open fixture").revision;
                READY.with(|ready| ready.borrow_mut().clear());
                studio.persistence_busy.set(true);
                let mut reopening = Box::pin(studio.open_project_with_revision(root.clone(), Some(revision)));
                let mut ready = false;
                for _ in 0..1000 {
                    if let std::task::Poll::Ready(value) = futures_util::poll!(&mut reopening) {
                        value?;
                        anyhow::bail!("The old open finished before the simulated save completed");
                    }
                    if READY.with(|ready| ready.borrow().contains(&root)) {ready = true;break;}
                    gloo_timers::future::TimeoutFuture::new(10).await;
                }
                anyhow::ensure!(ready, "Old open did not reach its save wait");
                // Keep the old future unpolled, as a delayed/background callback.
                // Run the competing real open to completion before resuming it.
                studio.persistence_busy.set(false);
                studio.open_project_with_revision(other.clone(), Some(1)).await?;
                anyhow::ensure!(crate::storage::is_file(other.join("source/notes.chart")), "New project did not open");
                let old_result = reopening.await;
                let active = studio.store.read().as_ref().expect("Active project").root.clone();
                anyhow::ensure!(crate::storage::is_file(active.join("project.json")) && crate::storage::is_file(active.join("source/notes.chart")), "FILES DISAPPEARED: delayed reopen returned {old_result:?}, current root {} has no document or source", active.display());
                anyhow::ensure!(active == other, "The stale open replaced the newer project");
                anyhow::ensure!(old_result.is_err(), "The stale open falsely reported success");
                let doc = Store::new(&active).load()?;
                anyhow::ensure!(doc.content.vocals.as_ref().is_some_and(|v| v.notes.len() == 1), "Generated vocals changed");
                anyhow::ensure!(crate::models::hash_file(&active.join("source").join(&doc.audio_filename))? == doc.audio_sha256, "Recording changed");
                for id in doc.undo.iter().chain(&doc.redo) {
                    anyhow::ensure!(crate::models::hash_file(&active.join("checkpoints").join(format!("{id}.json")))? == *id, "Checkpoint was lost or changed");
                }
                Ok::<_, anyhow::Error>(format!("PASS: newer project kept with all files; stale open returned {old_result:?}"))
            }.await;
            studio.persistence_busy.set(false);
            result.set(match test {Ok(value)=>value,Err(error)=>format!("FAIL: {error:#}")});
            running.set(false);
        });}, "Reproduce overlapping project opens"}
        button {disabled:running(),onclick:move |_|{if *running.peek(){return;}running.set(true);spawn(async move {
            let cleanup = async {
                while (studio.dirty)() || (studio.persistence_busy)() {
                    gloo_timers::future::TimeoutFuture::new(20).await;
                }
                let mut roots: Vec<_> = crate::persistence::projects().await?.into_iter().filter(|entry| entry.title.contains("Synthetic — Vocal lifetime fixture")).map(|entry| entry.path).collect();
                roots.extend(crate::persistence::jobs().await?.into_iter().filter(|(_,title)| title == "Synthetic — Vocal lifetime fixture").map(|(root,_)|root));
                crate::persistence::remove_lifetime_fixtures(&roots).await?;
                crate::workspace_recovery::forget_lifetime_fixtures(&roots)?;
                storage::activate_project(None);
                for root in &roots {storage::clear(root);}
                Ok::<_,anyhow::Error>(format!("Removed {} disposable fixture snapshots; close this fixture tab.", roots.len()))
            }.await;
            result.set(match cleanup {Ok(value)=>value,Err(error)=>format!("Cleanup failed: {error:#}")});
            running.set(false);
        });}, "Remove disposable lifecycle fixtures"}
        p {"{result}"}
    }}
}
pub async fn prepare() -> Result<()> {
    let root = crate::files::unique_root();
    let source = root.join("source");
    let (ini, chart) =
        crate::song_metadata::blank_song("Vocal lifetime fixture", "Synthetic", 120.)?;
    storage::write(source.join("song.ini"), ini)?;
    storage::write(source.join("notes.chart"), chart)?;
    let samples: Vec<f32> = (0..44100 * 4)
        .map(|n| (n as f32 * 330. * std::f32::consts::TAU / 44100.).sin() * 0.1)
        .collect();
    let audio = source.join("song.wav");
    crate::models::write_audio(&audio, &samples, &samples)?;
    Store::new(&root).initialize(&source, None, 4.)?;
    crate::persistence::save(&root, 0, None).await?;
    crate::workspace_recovery::remember(&crate::workspace_recovery::Target::new(
        crate::workspace_recovery::Family::Band,
        root.clone(),
    ))?;
    let job = PathBuf::from("/generation-jobs").join(storage::unique_id());
    generation::create_job(
        generation::Request {
            guitar_source: Default::default(),
            vocal_language: Default::default(),
            vocal_analysis_mode: Default::default(),
            detection: Default::default(),
            source: generation::Source::Audio { path: audio },
            bpm: 120.,
            tempo_map: None,
            title: "Vocal lifetime fixture".into(),
            artist: "Synthetic".into(),
            instruments: vec!["vocals".into()],
            existing_stems: None,
            keyboard_sound: Default::default(),
            piano_quality: Default::default(),
            model_dir: PathBuf::from("/unused-models"),
            fetch_models: false,
            full_mix_drums: false,
            standard_drums: false,
            lyrics: false,
            song_info: None,
        },
        &job,
    )?;
    generation::freeze(
        &job,
        crate::pipeline::Metadata {
            name: "Vocal lifetime fixture".into(),
            artist: "Synthetic".into(),
            album: String::new(),
            year: String::new(),
            genre: String::new(),
            duration: 4.,
        },
    )?;
    let generated = generation::project_path(&job);
    crate::pipeline::copy_folder(&root, &generated)?;
    Store::new(&generated).apply(Request {
        expected_revision: 0,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Synthetic completed vocal output".into(),
        },
        action: Action::SetVocals {
            document: crate::vocals::VocalChart {
                version: 1,
                resolution: 480,
                source: "Synthetic lifetime fixture, no inference".into(),
                phrases: vec![crate::vocals::VocalPhrase {
                    start: 0,
                    end: 1920,
                    player: 1,
                }],
                notes: vec![crate::vocals::VocalNote {
                    start: 480,
                    end: 960,
                    tone: crate::vocals::VocalTone::Pitched { midi: 60 },
                    lyric: crate::vocals::Syllable::Missing,
                    midi: None,
                }],
            },
        },
    })?;
    generation::write_json(
        &job.join("summary.json"),
        &generation::Summary {
            vocal_notes: 1,
            ..Default::default()
        },
    )?;
    generation::update(
        &job,
        generation::State::Complete,
        generation::Stage::Build,
        "Synthetic completed output for lifetime verification".into(),
        None,
    )?;
    let competing = crate::files::unique_root();
    crate::pipeline::copy_folder(&generated, &competing)?;
    crate::persistence::save(&competing, 1, None).await?;
    OTHER.with(|root| *root.borrow_mut() = Some(competing));
    generation::checkpoint(&job, None).await?;
    storage::clear(&job);
    storage::clear(&root);
    Ok(())
}
