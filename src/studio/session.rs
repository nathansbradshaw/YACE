//! Common editor commands dispatch to the selected instrument's document.
use super::{dance, state::Studio};
use crate::editor::{Action, TimeRange};
use dioxus::prelude::*;

#[derive(Clone)]
pub struct Transport {
    pub duration: f64,
    pub playing: bool,
    pub playback_rate: f64,
    pub loop_range: Option<TimeRange>,
    pub selected: Option<(String, TimeRange)>,
    pub ready: bool,
}
pub fn transport(studio: Studio) -> Option<Transport> {
    if standalone(studio) {
        return dance::transport();
    }
    let doc = studio.snapshot()?;
    let selected = doc.selected_region.as_ref().and_then(|id| {
        doc.review.regions.iter().find(|r| &r.id == id).map(|r| {
            (
                id.clone(),
                TimeRange {
                    start: r.start,
                    end: r.end,
                },
            )
        })
    });
    Some(Transport {
        duration: doc.duration,
        playing: doc.transport.playing,
        playback_rate: doc.transport.playback_rate,
        loop_range: doc.transport.loop_range,
        selected,
        ready: (studio.audio_ready)(),
    })
}
pub fn position(studio: Studio) -> f64 {
    if standalone(studio) {
        dance::position().unwrap_or(0.)
    } else {
        (studio.playhead)()
    }
}
pub fn seek(studio: Studio, seconds: f64) {
    if standalone(studio) {
        dance::seek(seconds);
    } else {
        studio.seek(seconds);
    }
}
pub fn playback(studio: Studio, playing: bool) {
    if standalone(studio) {
        dance::playback(playing);
    } else {
        studio.dispatch(Action::Playback { playing });
    }
}
pub fn toggle_playback(studio: Studio) {
    if let Some(t) = transport(studio) {
        playback(studio, !t.playing);
    }
}
pub fn rate(studio: Studio, rate: f64) {
    if standalone(studio) {
        dance::set_rate(rate);
    } else {
        studio.set_playback_rate(rate);
    }
}
pub fn volume(mut studio: Studio, volume: f32) {
    if standalone(studio) {
        studio.volume.set(volume);
        dance::set_volume(volume);
    } else {
        studio.update_volume(volume);
    }
}
pub fn set_loop(studio: Studio, range: Option<TimeRange>) {
    if standalone(studio) {
        dance::set_loop(range);
    } else {
        studio.dispatch(Action::SetLoop { range });
    }
}
pub fn undo(studio: Studio, redo: bool) {
    if dance::active() {
        dance::history(redo);
    } else {
        studio.dispatch(if redo { Action::Redo } else { Action::Undo });
    }
}
pub fn history(studio: Studio) -> (bool, bool) {
    if dance::active() {
        dance::history_available()
    } else {
        studio
            .snapshot()
            .map_or((false, false), |p| (!p.undo.is_empty(), !p.redo.is_empty()))
    }
}
pub fn selected(studio: Studio) -> bool {
    if dance::active() {
        dance::has_selection()
    } else {
        !studio.selection().is_empty()
    }
}
pub fn copy(studio: Studio, cut: bool) {
    if dance::active() {
        dance::copy(cut);
    } else {
        studio.copy_selection(cut);
    }
}
pub fn paste(studio: Studio) {
    if dance::active() {
        dance::paste();
    } else {
        studio.paste_selection();
    }
}
pub fn delete(studio: Studio) {
    if dance::active() {
        dance::delete();
    } else {
        studio.delete_selection();
    }
}
pub fn clear(studio: Studio) {
    if dance::active() {
        dance::clear();
    } else {
        studio.clear_selection();
    }
}
pub fn select_all(studio: Studio) {
    if dance::active() {
        dance::select_all();
    } else {
        studio.select_all();
    }
}

fn standalone(studio: Studio) -> bool {
    dance::active() && !dance::uses_recording(studio)
}
