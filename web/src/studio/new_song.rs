use super::{modal::Modal, state::Studio};
use dioxus::prelude::*;
use std::path::PathBuf;

#[component]
pub fn Setup(mut studio: Studio, root: PathBuf, suggested_title: String) -> Element {
    let mut title = use_signal(|| suggested_title);
    let mut artist = use_signal(String::new);
    let mut tempo = use_signal(|| "120".to_owned());
    let mut busy = use_signal(|| false);
    let cancel_root = root.clone();
    let close = use_callback(move |_: ()| {
        if !busy() {
            crate::storage::clear(&cancel_root);
            studio.new_song.set(None);
            studio.error.set(None);
        }
    });
    let valid = !title().trim().is_empty()
        && !artist().trim().is_empty()
        && tempo()
            .parse::<f64>()
            .is_ok_and(|bpm| bpm.is_finite() && (1. ..=1000.).contains(&bpm));
    rsx! {
        Modal { id:"new-song-dialog", label:"New chart", onclose:close,
            section { class:"sections-dialog",
                h2 { "New chart" }
                p { "Start with an empty Guitar chart. Add other parts in the editor." }
                label { class:"field-stack", "Title", input { class:"field-control", aria_label:"Song title", value:title, oninput:move |event|title.set(event.value()), disabled:busy() } }
                label { class:"field-stack", "Artist", input { class:"field-control", aria_label:"Song artist", value:artist, oninput:move |event|artist.set(event.value()), disabled:busy() } }
                label { class:"field-stack", "Starting tempo (BPM)", input { class:"field-control", aria_label:"Starting tempo BPM", r#type:"number", min:"1", max:"1000", step:"0.001", value:tempo, oninput:move |event|tempo.set(event.value()), disabled:busy() } }
                p { class:"field-hint", "You can change tempo and time signatures later with Timing." }
                if let Some(error)=(studio.error)(){p{role:"alert","{error}"}}
                button { disabled:busy(), onclick:move |_|close.call(()), "Cancel" }
                button { disabled:busy() || !valid, onclick:move |_|{
                    let root=root.clone();let title=title();let artist=artist();let tempo=tempo().parse().unwrap_or(120.);
                    busy.set(true);
                    spawn(async move{if let Err(error)=studio.finish_new_chart(root,title,artist,tempo).await{studio.error.set(Some(format!("{error:#}")));busy.set(false);}});
                }, if busy(){"Preparing…"}else{"Create chart"} }
            }
        }
    }
}
