//! A shared, phase-driven repair flow: configure, run, then review.
use super::state::Studio;
use crate::reanalysis::Phase;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub struct Options {
    pub sensitivity: u8,
    pub loop_only: bool,
    pub guitar_source: crate::mapping::GuitarSource,
    pub keyboard_sound: crate::keyboard_detection::KeyboardSound,
    pub piano_quality: crate::keyboard_detection::PianoQuality,
    pub language: crate::lyric_language::Language,
    pub vocal_mode: crate::vocal_generation::AnalysisMode,
    pub standard_drums: bool,
    pub full_mix_drums: bool,
}

#[component]
pub fn Controls(
    options: Options,
    instrument: String,
    loop_label: Option<String>,
    onchange: EventHandler<Options>,
    advanced: Element,
) -> Element {
    let loop_available = loop_label.is_some();
    let loop_label = loop_label.unwrap_or_else(|| "Set a loop in the editor".into());
    rsx! { div { class: "ra-config",
        div { class: "ra-field",
            span { class: "ra-label", "Repair" }
            div { class: "ra-segments", role: "group", aria_label: "Repair range",
                button { r#type: "button", aria_pressed: !options.loop_only, onclick: move |_| { let mut next=options;next.loop_only=false;onchange.call(next); }, "Whole track" }
                button { r#type: "button", aria_pressed: options.loop_only, disabled: !loop_available, title: "{loop_label}", onclick: move |_| { let mut next=options;next.loop_only=true;onchange.call(next); }, "Current loop" }
            }
            if options.loop_only { span { class: "ra-hint", "{loop_label}" } }
        }
        div { class: "ra-field",
            div { class: "ra-label-row", label { r#for: "ra-sensitivity", "Sensitivity" } strong { "{options.sensitivity}" }
                if options.sensitivity != 50 { button { class: "ra-link", onclick: move |_| { let mut next=options;next.sensitivity=50;onchange.call(next); }, "Reset" } } else { span { class: "ra-hint", "Default" } }
            }
            input { id: "ra-sensitivity", r#type: "range", min: "0", max: "100", step: "1", aria_label: "Detection sensitivity", value: "{options.sensitivity}", onkeydown: move |event| event.stop_propagation(), oninput: move |event| { if let Ok(value)=event.value().parse::<u8>() { let mut next=options;next.sensitivity=value.min(100);onchange.call(next); } } }
            div { class: "ra-endpoints", span { "Fewer notes" } span { "More notes" } }
        }
        if instrument == "guitar" { label { class: "ra-field", span { class: "ra-label", "Follow this sound" }
            select { aria_label: "Guitar sound source", value: options.guitar_source.stem().as_str(), onchange: move |event| { let mut next=options;next.guitar_source=match event.value().as_str(){"other"=>crate::mapping::GuitarSource::Other,"piano"=>crate::mapping::GuitarSource::Piano,_=>crate::mapping::GuitarSource::Guitar};onchange.call(next); }, option { value:"guitar", "Guitar" } option { value:"other", "Other melody / synth" } option { value:"piano", "Piano" } }
        } }
        if matches!(instrument.as_str(),"piano"|"pro_piano") { label { class: "ra-field", span { class:"ra-label", "Sound" }
            select { aria_label:"Keyboard sound", value:if options.keyboard_sound==crate::keyboard_detection::KeyboardSound::Synth{"synth"}else{"piano"},onchange:move|event|{let mut next=options;next.keyboard_sound=if event.value()=="synth"{crate::keyboard_detection::KeyboardSound::Synth}else{crate::keyboard_detection::KeyboardSound::Piano};onchange.call(next);},option{value:"piano","Piano"}option{value:"synth","Synth (experimental)"} }
        } }
        details { class: "ra-advanced", summary { "Advanced" }, div { class:"ra-advanced-fields",
            if instrument=="drums" {
                label { class:"ra-field", "Drum mapping",select{aria_label:"Drum mapping",value:if options.standard_drums{"standard"}else{"pro"},onchange:move|event|{let mut next=options;next.standard_drums=event.value()=="standard";onchange.call(next);},option{value:"pro","Pro Drums"}option{value:"standard","Standard drums"}} }
                label { class:"ra-check",input{r#type:"checkbox",checked:options.full_mix_drums,onchange:move|event|{let mut next=options;next.full_mix_drums=event.checked();onchange.call(next);}},"Use the full recording" }
                p { class:"ra-hint", "Full-recording detection is faster, but can pick up other instruments." }
            }
            if matches!(instrument.as_str(),"piano"|"pro_piano") && options.keyboard_sound==crate::keyboard_detection::KeyboardSound::Piano { label { class:"ra-field","Piano analysis",select{aria_label:"Piano analysis",value:if options.piano_quality==crate::keyboard_detection::PianoQuality::HighQuality{"high"}else{"standard"},onchange:move|event|{let mut next=options;next.piano_quality=if event.value()=="high"{crate::keyboard_detection::PianoQuality::HighQuality}else{crate::keyboard_detection::PianoQuality::Standard};onchange.call(next);},option{value:"standard","Standard"}option{value:"high",disabled:cfg!(target_arch="wasm32"),"Higher quality · desktop"}}} }
            if instrument=="vocals" {
                label { class:"ra-field","Lyric language",select{aria_label:"Lyric language",value:match options.language{crate::lyric_language::Language::English=>"en",crate::lyric_language::Language::Spanish=>"es",crate::lyric_language::Language::Auto=>"auto"},onchange:move|event|{let mut next=options;next.language=match event.value().as_str(){"es"=>crate::lyric_language::Language::Spanish,"auto"=>crate::lyric_language::Language::Auto,_=>crate::lyric_language::Language::English};onchange.call(next);},option{value:"en","English"}option{value:"es","Spanish"}option{value:"auto","Detect language"}} }
                label { class:"ra-field","Vocal analysis",select{aria_label:"Vocal analysis",value:if options.vocal_mode==crate::vocal_generation::AnalysisMode::Standard{"standard"}else{"high"},onchange:move|event|{let mut next=options;next.vocal_mode=if event.value()=="high"{crate::vocal_generation::AnalysisMode::HighAccuracy}else{crate::vocal_generation::AnalysisMode::Standard};onchange.call(next);},option{value:"standard","Standard"}option{value:"high","High accuracy (experimental)"}} }
            }
            {advanced}
        } }
    } }
}

#[component]
pub fn Panel(
    mut studio: Studio,
    track: String,
    recording: String,
    phase: Phase,
    activity: String,
    elapsed: String,
    quiet_for: u64,
    problem: Option<String>,
    controls: Element,
    review: Element,
    diagnostics: Element,
    can_start: bool,
    blocked_reason: Option<String>,
    onstart: EventHandler,
    oncancel: EventHandler,
) -> Element {
    let title = super::track_picker::title(&track);
    let review_phase = phase == Phase::Review;
    let problem_short = problem
        .as_deref()
        .unwrap_or("The analysis could not finish.")
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(220)
        .collect::<String>();
    rsx! { style { {include_str!("../../assets/reanalysis.css")} }
        super::sheet::ToolSheet { id:"part-reanalysis",title:"Reanalyze",description:"",close_label:"Close reanalysis",class:"reanalysis-sheet",onclose:move |_|{studio.show_analysis_preview(None);studio.generation_open.set(false);},
            actions:rsx!{
                if phase==Phase::Analyzing { button{class:"action-secondary",onclick:move |_|oncancel.call(()),"Cancel analysis"} }
                else if phase==Phase::Applying { button{class:"action-primary",disabled:true,"Applying…"} }
                else if !review_phase { button{class:"action-primary",disabled:!can_start,onclick:move |_|onstart.call(()),if phase==Phase::Setup{"Analyze"}else{"Analyze again"}} }
                else { button{class:"action-secondary",onclick:move |_|{studio.show_analysis_preview(None);studio.generation_open.set(false);},"Close"} }
            },
            div { class:"ra-target",strong{"{title}"}span{class:"ra-hint","{recording}"} }
            ol { class:"ra-steps",aria_label:"Analysis steps",li{class:if matches!(phase,Phase::Setup|Phase::Stopped|Phase::Failed|Phase::Empty){"active"}else{""},"1  Setup"}li{class:if phase==Phase::Analyzing{"active"}else{""},"2  Analyze"}li{class:if review_phase||phase==Phase::Applying{"active"}else{""},"3  Review"} }
            if matches!(phase,Phase::Analyzing|Phase::Applying) {
                div{class:"ra-status ra-working",role:"status",aria_live:"polite",span{class:"ra-spinner"}strong{if phase==Phase::Applying{"Applying your draft"}else{"Analyzing recording"}}p{if phase==Phase::Applying{"Saving one undoable edit…"}else{"{activity}"}}span{class:"ra-hint","{elapsed}"}if quiet_for>=20{span{class:"ra-hint","Waiting for the detector · last update {quiet_for}s ago"}}progress{aria_label:"Analysis in progress"}}
            } else if phase==Phase::Review {
                {review}
                details { class:"ra-adjust",summary{"Adjust and analyze again"},{controls}button{class:"action-secondary",disabled:!can_start,onclick:move |_|onstart.call(()),"Analyze again"} }
            } else {
                if phase==Phase::Stopped { div{class:"ra-status",role:"status",strong{"Analysis stopped"}p{"Your chart is unchanged. Adjust the settings or analyze again."}} }
                if phase==Phase::Empty { div{class:"ra-status",role:"status",strong{"No notes found in this range"}p{"Try a wider range or a different sensitivity. Your chart is unchanged."}} }
                if phase==Phase::Failed { div{class:"ra-status ra-failed",role:"alert",strong{"Analysis couldn’t finish"}p{"{problem_short}"}span{class:"ra-hint","Your chart and saved drafts are kept."}} }
                {controls}
                if !can_start && let Some(reason)=blocked_reason { p{class:"ra-hint",role:"status","{reason}"} }
            }
            details{class:"ra-diagnostics",summary{"Saved analyses & details"},{diagnostics}if let Some(problem)=problem{pre{class:"ra-error-detail","{problem}"}}}
        }
    }
}
