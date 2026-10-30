//! Instrument audition uses the same persisted mix actions as headless callers.
use super::{icons::Icon, state::Studio};
use crate::{
    editor::Action,
    stem_review::{MixMode, StemId, StemMix},
};
use dioxus::prelude::*;

fn label(id: StemId) -> &'static str {
    match id {
        StemId::Drums => "Drums",
        StemId::Bass => "Bass",
        StemId::Vocals => "Vocals",
        StemId::Guitar => "Guitar",
        StemId::Piano => "Piano",
        StemId::Other => "Other",
    }
}
fn preset(studio: Studio, quiet_backing: bool) {
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return;
    };
    let vocals = studio.is_vocal_track();
    let Some(manifest) = super::state::review_manifest(doc, vocals) else {
        return;
    };
    let selected = studio
        .guitar_source
        .read()
        .audition_stem(&doc.selected_track, vocals);
    if !manifest.stems.iter().any(|stem| stem.id == selected) {
        return;
    }
    let mut mix = StemMix {
        mode: MixMode::Stems,
        ..StemMix::default()
    };
    for stem in &manifest.stems {
        let channel = mix.channels.entry(stem.id).or_default();
        channel.gain = if stem.id == selected {
            1.0
        } else if quiet_backing {
            0.18
        } else {
            0.0
        };
        channel.mute = false;
        channel.solo = false;
    }
    drop(project);
    studio.dispatch(Action::SetStemMix { mix });
}
fn channel(studio: Studio, id: StemId, gain: Option<f32>, mute: Option<bool>, solo: Option<bool>) {
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return;
    };
    let mut mix = doc.content.stem_mix.clone();
    mix.mode = MixMode::Stems;
    let channel = mix.channels.entry(id).or_default();
    if let Some(value) = gain {
        channel.gain = value.clamp(0.0, 2.0);
    }
    if let Some(value) = mute {
        channel.mute = value;
    }
    if let Some(value) = solo {
        channel.solo = value;
    }
    drop(project);
    studio.dispatch(Action::SetStemMix { mix });
}
#[component]
pub fn StemControls(studio: Studio) -> Element {
    let mut expanded = use_signal(|| false);
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    let available = super::state::review_manifest(doc, studio.is_vocal_track());
    let selected = studio
        .guitar_source
        .read()
        .audition_stem(&doc.selected_track, studio.is_vocal_track());
    let selected_available = available.is_some_and(|m| m.stems.iter().any(|s| s.id == selected));
    let kit = doc.kit_audition.as_ref();
    let original = kit.is_none() && doc.content.stem_mix.mode == MixMode::Original;
    let matches_preset = |backing: f32| {
        kit.is_none()
            && !original
            && available.is_some_and(|manifest| {
                manifest.stems.iter().all(|stem| {
                    let settings = doc
                        .content
                        .stem_mix
                        .channels
                        .get(&stem.id)
                        .cloned()
                        .unwrap_or_default();
                    !settings.mute
                        && !settings.solo
                        && (settings.gain - if stem.id == selected { 1.0 } else { backing }).abs()
                            < 0.001
                })
            })
    };
    let solo_active = matches_preset(0.0);
    let backing_active = matches_preset(0.18);
    let attachment_notice = doc
        .review
        .coverage_limits
        .iter()
        .find(|notice| notice.starts_with("Stem attachment skipped:"));
    rsx! {
        div{class:"stem-controls",onkeydown:move|event|event.stop_propagation(),
            div{class:"stem-controls-toolbar",
                span{class:"stem-controls-label",title:"Instrument audition",Icon{name:"volume",size:14}}
                button{class:if original{"studio-tool active"}else{"studio-tool"},onclick:move |_|{
                    let project=studio.doc.read();if let Some(doc)=project.as_ref(){let mut mix=doc.content.stem_mix.clone();mix.mode=MixMode::Original;drop(project);studio.dispatch(Action::SetStemMix{mix});}
                },"Original"}
                button{class:if solo_active{"studio-tool active"}else{"studio-tool"},disabled:!selected_available,title:"Listen only to {label(selected)}",onclick:move |_|preset(studio,false),"{label(selected)} only"}
                button{class:if backing_active{"studio-tool active"}else{"studio-tool"},disabled:!selected_available,title:"{label(selected)} at full gain, other stems at 18%",onclick:move |_|preset(studio,true),"With backing"}
                if available.is_some(){button{class:if expanded(){"studio-tool active stem-mixer-toggle"}else{"studio-tool stem-mixer-toggle"},aria_label:"Open stem mixer",aria_expanded:expanded().to_string(),title:"Stem mixer and alignment details",onclick:move |_|expanded.set(!expanded()),Icon{name:"sliders",size:13}}}
                else{button{class:"studio-tool stem-mixer-toggle",title:"Attach aligned local stems to isolate instruments",onclick:move |_|studio.attach_stems_dialog(),Icon{name:"folder-open",size:13}"Attach stems"}}
                span{class:"stem-runtime-status",aria_label:"Audition source",title:"{studio.stem_status}",if let Some(kit)=kit{"Kit · {kit.label}"}else if available.is_none(){"No stems"}else if !original{"Isolated estimate"}}
                super::part_analysis::PartAnalysisButton{studio}
            }
            if expanded(){
                if let Some(manifest)=available{
                    div{style:"display:flex;gap:12px;flex-wrap:wrap;padding-top:8px",
                        for stem in &manifest.stems{
                            {let id=stem.id;let settings=doc.content.stem_mix.channels.get(&id).cloned().unwrap_or_default();
                            rsx!{div{key:"{label(id)}",style:"display:flex;align-items:center;gap:5px",
                                strong{"{label(id)}"}
                                label{style:"display:flex;gap:3px;align-items:center",input{r#type:"checkbox",checked:settings.mute,onkeydown:move|e|e.stop_propagation(),onchange:move|e|channel(studio,id,None,Some(e.checked()),None)}"Mute"}
                                label{style:"display:flex;gap:3px;align-items:center",input{r#type:"checkbox",checked:settings.solo,onkeydown:move|e|e.stop_propagation(),onchange:move|e|channel(studio,id,None,None,Some(e.checked()))}"Solo"}
                                input{r#type:"number",min:"0",max:"2",step:"0.05",value:"{settings.gain}",style:"width:56px;background:var(--bg-tertiary);color:var(--text-primary);border:1px solid var(--border-color);border-radius:3px;padding:3px",aria_label:"{label(id)} gain",title:"Stem gain (0–2)",onkeydown:move|e|e.stop_propagation(),onchange:move|e|{if let Ok(gain)=e.value().parse::<f32>(){channel(studio,id,Some(gain),None,None)}}}
                            }}}
                        }
                    }
                    div{class:"stem-mixer-footer",
                        button{class:"studio-tool",onclick:move |_|studio.attach_stems_dialog(),Icon{name:"folder-open",size:13}"Replace stems"}
                        span{title:"{manifest.alignment.details}","Alignment: {manifest.alignment.method}"}
                    }
                    div{class:"stem-detailed-status","{studio.stem_status}"}
                }
            }
            if let Some(notice)=attachment_notice{div{style:"font-size:10px;color:var(--warning-color);padding-top:4px","{notice}"}}
        }
    }
}
