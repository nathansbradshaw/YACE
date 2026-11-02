//! A full-song seek control; browsing the view never changes the transport.
use super::{
    dance, session,
    state::{Studio, clock},
};
use crate::editor::{Action, TimeRange};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
struct Source {
    root: PathBuf,
    dance_epoch: Option<u64>,
}
fn source(studio: Studio) -> Option<Source> {
    if dance::active() {
        dance::overview_source().map(|(root, epoch)| Source {
            root,
            dance_epoch: Some(epoch),
        })
    } else {
        studio.store.read().as_ref().map(|s| Source {
            root: s.root.clone(),
            dance_epoch: None,
        })
    }
}
fn playback_stamp(studio: Studio) -> Option<u64> {
    if dance::active() && !dance::uses_recording(studio) {
        dance::overview_playback_stamp()
    } else {
        studio
            .snapshot()?
            .audit
            .iter()
            .rev()
            .find(|entry| matches!(entry.action, Action::Playback { .. } | Action::Pause { .. }))
            .map(|entry| entry.revision)
    }
}
#[derive(Clone, Debug)]
struct Drag {
    source: Source,
    pointer: i32,
    was_playing: bool,
    pause_stamp: Option<u64>,
}
impl Drag {
    fn resumes(
        &self,
        current: Option<&Source>,
        stamp: Option<u64>,
        playing: bool,
        position: f64,
        duration: f64,
    ) -> bool {
        self.was_playing
            && current == Some(&self.source)
            && self.pause_stamp == stamp
            && !playing
            && position < duration
    }
}
fn bounded(seconds: f64, duration: f64, range: Option<TimeRange>) -> Option<f64> {
    if !seconds.is_finite() || !duration.is_finite() || duration <= 0. {
        return None;
    }
    let (lo, hi) = range
        .filter(|r| {
            r.start.is_finite()
                && r.end.is_finite()
                && r.start >= 0.
                && r.start < r.end
                && r.end <= duration
        })
        .map_or((0., duration), |r| {
            (
                r.start,
                (r.end - 0.001_f64.min((r.end - r.start) / 2.)).max(r.start),
            )
        });
    Some(seconds.clamp(lo, hi))
}
fn window_start(position: f64, span: f64, duration: f64) -> f64 {
    (position - span * 0.25).clamp(0., (duration - span).max(0.))
}
fn seek(mut studio: Studio, seconds: f64) {
    let Some(transport) = session::transport(studio) else {
        return;
    };
    let Some(seconds) = bounded(seconds, transport.duration, transport.loop_range) else {
        return;
    };
    session::seek(studio, seconds);
    // Use the acknowledged position if persistence or validation rejected it.
    let position = session::position(studio);
    if dance::active() {
        dance::overview_reveal(studio, position);
    } else {
        studio
            .view_start
            .set(window_start(position, (studio.span)(), transport.duration));
    }
    reflect_position(position);
}
fn finish(studio: Studio, mut drag: Signal<Option<Drag>>, pointer: Option<i32>) {
    let Some(current) = drag.peek().clone() else {
        return;
    };
    if pointer.is_some_and(|id| id != current.pointer) {
        return;
    }
    drag.set(None);
    if let Some(t) = session::transport(studio)
        && current.resumes(
            source(studio).as_ref(),
            playback_stamp(studio),
            t.playing,
            session::position(studio),
            t.duration,
        )
    {
        session::playback(studio, true);
    }
}

// Native Dioxus serializes pointer events, so pointer capture needs its DOM
// bridge. WASM calls the same browser operation directly from Rust.
fn capture(pointer: i32) {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen::JsCast;
    #[cfg(target_arch = "wasm32")]
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("song-overview-seek"))
    {
        if let Some(input) = element.dyn_ref::<web_sys::HtmlInputElement>() {
            let _ = input.focus();
        }
        let _ = element.set_pointer_capture(pointer);
    }
    #[cfg(not(target_arch = "wasm32"))]
    spawn(async move {
        let _=dioxus_document::eval(&format!("const e=document.getElementById('song-overview-seek');if(e){{e.focus({{preventScroll:true}});try{{e.setPointerCapture({pointer});}}catch(_){{}}}}" )).await;
    });
}
fn reflect_position(seconds: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        if let Some(input) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("song-overview-seek"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            input.set_value(&seconds.to_string());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    spawn(async move {
        let _ = dioxus_document::eval(&format!(
            "const e=document.getElementById('song-overview-seek');if(e)e.value={seconds};"
        ))
        .await;
    });
}

pub fn pan(mut studio: Studio, event: WheelEvent) {
    let delta = event.delta().strip_units();
    let amount = if event.modifiers().shift() {
        delta.y
    } else {
        delta.x
    };
    if !amount.is_finite() || amount == 0. {
        return;
    }
    event.prevent_default();
    if dance::active() {
        dance::overview_pan(studio, amount);
    } else if let Some(t) = session::transport(studio) {
        studio.view_start.set(
            ((studio.view_start)() + amount * (studio.span)() / 600.)
                .clamp(0., (t.duration - (studio.span)()).max(0.)),
        );
    }
}

#[component]
pub fn SongOverview(studio: Studio) -> Element {
    let mut drag = use_signal(|| None::<Drag>);
    let mut cancelled = use_signal(|| false);
    use_effect(move || {
        let key = source(studio);
        if drag
            .read()
            .as_ref()
            .is_some_and(|d| key.as_ref() != Some(&d.source))
        {
            drag.set(None);
        }
    });
    use_drop(move || {
        finish(studio, drag, None);
    });
    let Some(t) = session::transport(studio) else {
        return rsx! {};
    };
    let duration = t.duration;
    let position = session::position(studio).clamp(0., duration.max(0.));
    let (start, end) = if dance::active() {
        dance::overview_window(studio)
    } else {
        (
            (studio.view_start)(),
            ((studio.view_start)() + (studio.span)()).min(duration),
        )
    };
    let percent = move |seconds: f64| {
        if duration > 0. {
            (seconds / duration * 100.).clamp(0., 100.)
        } else {
            0.
        }
    };
    let left = percent(start);
    let width = (percent(end) - left).max(0.);
    let playhead = percent(position);
    let help = if t.loop_range.is_some() {
        "Drag to seek inside the active loop. Arrow keys seek 2 seconds; Shift seeks 10 seconds. Shift-wheel pans without seeking."
    } else {
        "Drag to seek the recording and timeline together. Arrow keys seek 2 seconds; Shift seeks 10 seconds. Shift-wheel pans without seeking."
    };
    rsx! {section{class:"song-overview",aria_label:"Song overview",onwheel:move|event|pan(studio,event),
        div{class:"song-overview-heading",span{"Song overview"}output{aria_live:"off","{clock(position)} / {clock(duration)}"}}
        div{class:"song-overview-rail",
            div{class:"song-overview-window",style:"left:{left}%;width:{width}%",aria_hidden:"true"}
            if let Some(range)=t.loop_range {div{class:"song-overview-loop",style:"left:{percent(range.start)}%;width:{percent(range.end)-percent(range.start)}%",aria_hidden:"true"}}
            div{class:"song-overview-playhead",style:"left:{playhead}%",aria_hidden:"true"}
            input{id:"song-overview-seek",r#type:"range",tabindex:"0",min:"0",max:"{duration}",step:"0.001",value:"{position}",disabled:duration<=0.,aria_label:"Seek song and timeline",aria_valuetext:"{clock(position)} of {clock(duration)}",title:help,
                onpointerdown:move|event:PointerEvent|{
                    if !event.is_primary()||event.trigger_button()!=Some(dioxus::html::input_data::MouseButton::Primary)||drag.peek().is_some(){return;}
                    let Some(key)=source(studio) else{return;};let Some(t)=session::transport(studio)else{return;};
                    cancelled.set(false);
                    if t.playing {
                        if dance::active() && !dance::uses_recording(studio) {session::playback(studio,false);}else{
                            studio.dispatch(Action::Pause {seconds:studio.current_audio_position().clamp(0.,t.duration)});
                        }
                    }
                    if session::transport(studio).is_some_and(|t|t.playing){return;}
                    drag.set(Some(Drag{source:key,pointer:event.pointer_id(),was_playing:t.playing,pause_stamp:playback_stamp(studio)}));capture(event.pointer_id());
                },
                oninput:move|event|{if cancelled(){reflect_position(session::position(studio));}else if let Ok(seconds)=event.value().parse(){seek(studio,seconds);}},
                onpointerup:move|event:PointerEvent|{cancelled.set(false);finish(studio,drag,Some(event.pointer_id()));},
                onpointercancel:move|event:PointerEvent|{cancelled.set(false);finish(studio,drag,Some(event.pointer_id()));},
                onlostpointercapture:move|event:PointerEvent|{cancelled.set(false);finish(studio,drag,Some(event.pointer_id()));},
                onblur:move|_|{cancelled.set(drag.peek().is_some());finish(studio,drag,None);},
                onkeydown:move|event:KeyboardEvent|{
                    event.stop_propagation();
                    if event.modifiers().alt()||event.modifiers().ctrl()||event.modifiers().meta(){return;}
                    let step=if event.modifiers().shift(){10.}else{2.};
                    let at=session::position(studio);
                    let next=match event.key(){Key::ArrowLeft|Key::ArrowDown=>Some(at-step),Key::ArrowRight|Key::ArrowUp=>Some(at+step),Key::Home=>Some(0.),Key::End=>Some(duration),Key::PageUp=>Some(at-duration*0.1),Key::PageDown=>Some(at+duration*0.1),Key::Escape=>{event.prevent_default();cancelled.set(drag.peek().is_some());finish(studio,drag,None);None},Key::Character(s)if s==" "=>{event.prevent_default();session::toggle_playback(studio);None},_=>None};
                    if let Some(next)=next{event.prevent_default();seek(studio,next);}
                }
            }
        }
        div{class:"song-overview-labels",span{"0:00.0"}span{"View {clock(start)}–{clock(end)}"}span{"{clock(duration)}"}}
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gesture() -> Drag {
        Drag {
            source: Source {
                root: "/project-a".into(),
                dance_epoch: None,
            },
            pointer: 1,
            was_playing: true,
            pause_stamp: Some(7),
        }
    }
    #[test]
    fn cancel_and_release_resume_only_the_unchanged_playback_intent() {
        let g = gesture();
        assert!(g.resumes(Some(&g.source), Some(7), false, 42., 180.));
        assert!(!g.resumes(Some(&g.source), Some(9), false, 42., 180.)); // explicit Play then Pause
        assert!(!g.resumes(Some(&g.source), Some(7), true, 42., 180.));
        assert!(!g.resumes(Some(&g.source), Some(7), false, 180., 180.));
        assert!(!g.resumes(None, Some(7), false, 42., 180.));
        let other = Source {
            root: "/project-b".into(),
            dance_epoch: None,
        };
        assert!(!g.resumes(Some(&other), Some(7), false, 42., 180.));
        let mut paused = g.clone();
        paused.was_playing = false;
        assert!(!paused.resumes(Some(&g.source), Some(7), false, 42., 180.));
    }
    #[test]
    fn seek_bounds_keep_loops_stable_and_reject_invalid_input() {
        assert_eq!(bounded(-5., 180., None), Some(0.));
        assert_eq!(bounded(900., 180., None), Some(180.));
        assert_eq!(bounded(f64::NAN, 180., None), None);
        assert_eq!(bounded(1., 0., None), None);
        let r = Some(TimeRange {
            start: 20.,
            end: 25.,
        });
        assert_eq!(bounded(0., 180., r), Some(20.));
        assert_eq!(bounded(180., 180., r), Some(24.999));
        assert_eq!(bounded(21., 180., r), Some(21.));
    }
    #[test]
    fn long_distance_seeks_preserve_zoom_and_keep_the_playhead_visible() {
        assert_eq!(window_start(100., 12., 180.), 97.);
        assert_eq!(window_start(179., 12., 180.), 168.);
        assert_eq!(window_start(0., 12., 180.), 0.);
        assert_eq!(window_start(2., 12., 5.), 0.);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    mod saved_transport {
        use super::*;
        use crate::editor::Store;
        use std::{cell::RefCell, rc::Rc, sync::Arc};
        type Slot = Rc<RefCell<Option<(Studio, Signal<Option<Drag>>)>>>;
        fn app() -> Element {
            dance::provide();
            let mut studio = super::super::super::state::use_studio();
            let drag = use_signal(|| None::<Drag>);
            let slot = use_context::<Slot>();
            let root = use_context::<PathBuf>();
            use_hook(move || {
                let store = Store::new(&root);
                let project = store.load().unwrap();
                studio.timing.set(Some(project.timing(&store).unwrap()));
                studio.doc.set(Some(Arc::new(project)));
                studio.store.set(Some(store));
                studio.audio_ready.set(true);
                studio.span.set(12.);
                *slot.borrow_mut() = Some((studio, drag));
            });
            rsx! {SongOverview{studio}}
        }
        #[tokio::test]
        async fn scrub_moves_the_saved_transport_without_touching_notes_selection_or_history() {
            let root = tempfile::tempdir().unwrap();
            let song = root.path().join("song");
            std::fs::create_dir(&song).unwrap();
            std::fs::write(song.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 240\n}\n").unwrap();
            std::fs::write(
                song.join("song.ini"),
                "[song]\nname=Overview fixture\nsong_length=180000\n",
            )
            .unwrap();
            let store = Store::create(&song, None, &root.path().join("project")).unwrap();
            let before = store.load().unwrap();
            let original = serde_json::to_value(&before.content).unwrap();
            let slot: Slot = Rc::new(RefCell::new(None));
            let mut dom = VirtualDom::new(app);
            dom.provide_root_context(slot.clone());
            dom.provide_root_context(store.root.clone());
            dom.rebuild_in_place();
            dom.in_scope(ScopeId::ROOT, || {
                let (mut studio, mut drag) = slot.borrow().unwrap();
                seek(studio, 120.);
                assert_eq!(session::position(studio), 120.);
                assert_eq!((studio.span)(), 12.);
                assert_eq!((studio.view_start)(), 117.);
                let saved = store.load().unwrap();
                assert_eq!(saved.transport.position, 120.);
                assert_eq!(serde_json::to_value(&saved.content).unwrap(), original);
                assert_eq!(saved.selected_track, before.selected_track);
                assert_eq!(saved.selected_note, before.selected_note);
                assert_eq!(saved.undo, before.undo);
                assert_eq!(saved.redo, before.redo);
                // The same finish path handles pointer-up, cancellation and blur.
                let key = source(studio).unwrap();
                drag.set(Some(Drag {
                    source: key.clone(),
                    pointer: 4,
                    was_playing: true,
                    pause_stamp: playback_stamp(studio),
                }));
                finish(studio, drag, Some(99));
                assert!(drag.peek().is_some());
                assert!(!session::transport(studio).unwrap().playing);
                finish(studio, drag, Some(4));
                assert!(drag.peek().is_none());
                assert!(session::transport(studio).unwrap().playing);
                session::playback(studio, false);
                drag.set(Some(Drag {
                    source: key,
                    pointer: 5,
                    was_playing: true,
                    pause_stamp: playback_stamp(studio),
                }));
                session::playback(studio, true);
                session::playback(studio, false);
                finish(studio, drag, None);
                assert!(
                    !session::transport(studio).unwrap().playing,
                    "a later explicit Pause wins"
                );
                studio.doc.set(Some(Arc::new(store.load().unwrap())));
            });
        }
    }
}
