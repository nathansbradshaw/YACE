//! A single stereo file keeps the vocal stem and MIDI-note tones sample aligned.
use super::state::Studio;
use crate::editor::{Action, TimeRange};
use dioxus::prelude::*;
#[derive(Clone)]
struct Clip {
    binding: String,
    selected: Option<Vec<usize>>,
    include_pitch: bool,
    url: String,
}
#[component]
pub fn VocalSectionAudio(studio: Studio, selected: Vec<usize>, include_pitch: bool) -> Element {
    let clip = use_signal(|| None::<Clip>);
    let busy = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(a) = doc.content.vocal_pitch_analysis.as_ref() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let range = doc
        .transport
        .loop_range
        .filter(|r| {
            r.start >= a.range.start
                && r.end <= a.range.end
                && r.start < r.end
                && r.end - r.start <= 30.
        })
        .unwrap_or(TimeRange {
            start: a.range.start,
            end: (a.range.start + 30.).min(a.range.end),
        });
    let binding = crate::vocal_section_audio::binding(&doc, &timing, range).unwrap_or_default();
    let current = clip
        .read()
        .as_ref()
        .filter(|c| {
            c.binding == binding
                && c.selected
                    .as_ref()
                    .is_none_or(|v| v == &selected && c.include_pitch == include_pitch)
        })
        .cloned();
    let valid = a.validate(&doc, &timing).is_ok();
    let chosen = selected.clone();
    rsx! {div{class:"review-evidence",
        strong{"Listen against the recording"}
        p{class:"field-hint","{range.start:.2}–{range.end:.2}s · left: vocal stem · right: note tones · original speed. Uses the current loop, or the first 30 seconds of this analysis. Quiet talkies have no pitch tone."}
        button{class:"studio-tool",disabled:busy(),onclick:move |_|prepare(studio,range,None,false,clip,busy,error),"Prepare current-note comparison"}
        button{class:"studio-tool",disabled:busy()||!valid||(chosen.is_empty()&&!include_pitch),onclick:move |_|prepare(studio,range,Some(chosen.clone()),include_pitch,clip,busy,error),"Prepare selected-repair comparison"}
        if busy(){p{role:"status","Preparing synchronized comparison…"}}
        if let Some(message)=error(){p{role:"alert","{message}"}}
        if let Some(c)=current {
            p{class:"field-hint",if c.selected.is_some(){"Selected-repair preview; the chart has not been changed."}else{"Current chart preview."}}
            audio{controls:true,src:c.url,aria_label:"Synchronized vocal stem and note comparison",onplay:move |_|studio.dispatch(Action::Playback{playing:false})}
        }
    }}
}
#[allow(clippy::too_many_arguments)]
fn prepare(
    studio: Studio,
    range: TimeRange,
    selected: Option<Vec<usize>>,
    include_pitch: bool,
    mut clip: Signal<Option<Clip>>,
    mut busy: Signal<bool>,
    mut error: Signal<Option<String>>,
) {
    let Some(project) = studio.snapshot() else {
        return;
    };
    let Some(store) = studio.store.read().clone() else {
        return;
    };
    let Some(timing) = (studio.timing)() else {
        return;
    };
    let built = (|| -> anyhow::Result<_> {
        let binding = crate::vocal_section_audio::binding(&project, &timing, range)?;
        let mut chart = project
            .content
            .vocals
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No vocal notes"))?
            .clone();
        if let Some(indices) = &selected {
            let a = project
                .content
                .vocal_pitch_analysis
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Analyze first"))?;
            a.validate(&project, &timing)?;
            let repairs = indices
                .iter()
                .map(|i| {
                    a.section
                        .repairs
                        .get(*i)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Repair changed"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            if include_pitch {
                for c in &a.corrections {
                    if !repairs
                        .iter()
                        .any(|r| r.before.contains(&chart.notes[c.index]))
                    {
                        chart.notes[c.index].tone =
                            crate::vocals::VocalTone::Pitched { midi: c.after };
                    }
                }
            }
            chart = crate::vocal_section_analysis::apply(&chart, &repairs)?;
        }
        Ok((binding, chart))
    })();
    let (binding, chart) = match built {
        Ok(v) => v,
        Err(e) => {
            error.set(Some(e.to_string()));
            return;
        }
    };
    error.set(None);
    busy.set(true);
    clip.set(None);
    spawn(async move {
        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            let p = project.clone();
            let s = store.clone();
            tokio::task::spawn_blocking(move || {
                crate::vocal_section_audio::prepare(&p, &s, &chart, range)
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|v| v)
        };
        #[cfg(target_arch = "wasm32")]
        let result = crate::worker::run_with_progress::<String>(
            crate::worker::Task::VocalSectionAudition {
                root: store.root.clone(),
                project: Box::new((*project).clone()),
                chart,
                range,
            },
            std::slice::from_ref(&store.root),
            std::rc::Rc::new(|_| {}),
        )
        .await;
        let still_current = studio
            .store
            .read()
            .as_ref()
            .is_some_and(|s| s.root == store.root)
            && studio
                .snapshot()
                .zip((studio.timing)())
                .is_some_and(|(p, t)| {
                    crate::vocal_section_audio::binding(&p, &t, range).is_ok_and(|k| k == binding)
                });
        if still_current {
            match result {
                Ok(url) => clip.set(Some(Clip {
                    binding,
                    selected,
                    include_pitch,
                    url,
                })),
                Err(e) => error.set(Some(format!("{e:#}"))),
            }
        }
        busy.set(false);
    });
}
