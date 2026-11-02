//! Media is an instrument backend; the visible transport is shared.
use super::*;
use crate::editor::TimeRange;

pub fn transport() -> Option<super::super::session::Transport> {
    let s = try_consume_context::<DanceStudio>()?;
    let p = s.project.read();
    let p = p.as_ref()?;
    let chart_end = p
        .content
        .document
        .charts
        .iter()
        .filter_map(|c| {
            let timing = p.content.document.effective_timing(&c.id).ok()?;
            let row = c
                .notes
                .iter()
                .map(|n| n.end_row.unwrap_or(n.row))
                .max()
                .unwrap_or(0);
            Some(timing.seconds(row as f64 / 48., sm::timing::Boundary::Note) + 4.)
        })
        .fold(8., f64::max);
    Some(super::super::session::Transport {
        duration: if (s.length)() > 0. {
            (s.length)()
        } else {
            chart_end
        },
        playing: (s.playing)(),
        playback_rate: (s.rate)(),
        loop_range: (s.loop_range)(),
        selected: None,
        ready: (s.length)() > 0.,
    })
}
pub fn position() -> Option<f64> {
    active().then(|| (consume_context::<DanceStudio>().clock)())
}
#[derive(Clone, Copy)]
enum Media {
    Seek(f64),
    Play(bool),
    Rate(f64),
    Volume(f32),
}
fn media(command: Media) {
    spawn(async move {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            if let Some(audio) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("dance-audio"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlAudioElement>().ok())
            {
                match command {
                    Media::Seek(t) => audio.set_current_time(t),
                    Media::Play(true) => {
                        if let Ok(p) = audio.play() {
                            let _ = wasm_bindgen_futures::JsFuture::from(p).await;
                        }
                    }
                    Media::Play(false) => {
                        let _ = audio.pause();
                    }
                    Media::Rate(r) => audio.set_playback_rate(r),
                    Media::Volume(v) => audio.set_volume(f64::from(v)),
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let operation = match command {
                Media::Seek(t) => format!("a.currentTime={t}"),
                Media::Play(true) => "await a.play()".into(),
                Media::Play(false) => "a.pause()".into(),
                Media::Rate(r) => format!("a.preservesPitch=true;a.playbackRate={r}"),
                Media::Volume(v) => format!("a.volume={v}"),
            };
            let _ = dioxus_document::eval(&format!(
                "const a=document.getElementById('dance-audio');if(a){{{operation};}}"
            ))
            .await;
        }
    });
}
pub fn seek(seconds: f64) {
    if !seconds.is_finite() {
        return;
    }
    let mut s = consume_context::<DanceStudio>();
    let duration = transport().map_or(0., |t| t.duration);
    let seconds = seconds.clamp(0., duration);
    s.clock.set(seconds);
    s.motion_epoch.set((s.motion_epoch)().wrapping_add(1));
    media(Media::Seek(seconds));
}
pub fn playback(playing: bool) {
    let mut s = consume_context::<DanceStudio>();
    if !playing {
        s.playing.set(false);
    }
    media(Media::Play(playing));
}
pub fn set_rate(rate: f64) {
    if !rate.is_finite() || !(0.25..=2.).contains(&rate) {
        return;
    }
    let mut s = consume_context::<DanceStudio>();
    s.rate.set(rate);
    s.motion_epoch.set((s.motion_epoch)().wrapping_add(1));
    media(Media::Rate(rate));
}
pub fn set_volume(volume: f32) {
    if volume.is_finite() {
        media(Media::Volume(volume.clamp(0., 1.)));
    }
}
pub fn set_loop(range: Option<TimeRange>) {
    let Some(t) = transport() else { return };
    if range.is_some_and(|r| {
        !r.start.is_finite()
            || !r.end.is_finite()
            || r.start < 0.
            || r.start >= r.end
            || r.end > t.duration
    }) {
        return;
    }
    consume_context::<DanceStudio>().loop_range.set(range);
}
#[component]
pub fn AudioEngine() -> Element {
    let mut s = use_context::<DanceStudio>();
    let studio = use_context::<super::super::state::Studio>();
    let sample = move || async move {
        let epoch = (s.motion_epoch)();
        #[cfg(target_arch = "wasm32")]
        let reading = {
            use wasm_bindgen::JsCast;
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("dance-audio"))
                .and_then(|e| e.dyn_into::<web_sys::HtmlAudioElement>().ok())
                .map(|a| (a.current_time(), a.duration(), a.paused()))
        };
        #[cfg(not(target_arch="wasm32"))]
        let reading = dioxus_document::eval("const a=document.getElementById('dance-audio');return a?[a.currentTime,Number.isFinite(a.duration)?a.duration:0,a.paused]:null;")
            .await.ok().and_then(|v|Some((v.get(0)?.as_f64()?,v.get(1)?.as_f64()?,v.get(2)?.as_bool()?)));
        if epoch != (s.motion_epoch)() {
            return;
        }
        if let Some((seconds, length, paused)) = reading {
            if length.is_finite() && length > 0. {
                s.length.set(length);
            }
            s.playing.set(!paused);
            if !paused
                && let Some(range) = (s.loop_range)()
                && seconds >= range.end
            {
                seek(range.start);
            } else {
                s.clock.set(seconds);
            }
        }
    };
    // Imported standalone simfiles use this media backend. Sampling its actual
    // counter avoids the coarse HTML timeupdate cadence for moving arrows.
    use_future(move || async move {
        loop {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(16).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            if (s.playing)() && (s.open)() && !uses_recording(studio) {
                sample().await;
            }
        }
    });
    rsx! {
        if !uses_recording(studio) && !(s.audio)().is_empty() {
            audio { key:"{s.epoch}", id:"dance-audio", hidden:true, src:"{s.audio}",
                onloadedmetadata:move |_| { set_volume((studio.volume)()); sample() },
                ontimeupdate:move |_|async move{if !(s.playing)(){sample().await;}}, onplay:move |_|s.playing.set(true),
                onpause:move |_|s.playing.set(false), onended:move |_|{if let Some(range)=(s.loop_range)(){seek(range.start);playback(true);}else{s.playing.set(false);}},
                onerror:move |_|{s.length.set(0.);s.playing.set(false);s.error.set(Some("Could not play the recording.".into()));}
            }
        }
    }
}
