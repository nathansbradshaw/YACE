//! Audition controls are client preferences, separate from authored song timing.
use super::state::Studio;
use crate::audition_clicks::{Mix, NoteClass, Plan, Settings, Singer, preferences};
use dioxus::prelude::*;
use std::sync::Arc;
#[derive(Clone, Copy)]
struct Audition {
    settings: Signal<Settings>,
    storage_warning: Signal<Option<String>>,
    audio_warning: Signal<Option<String>>,
}
pub fn provide(studio: Studio) {
    let state = use_context_provider(|| {
        let loaded = preferences::load();
        let (settings, warning) = match loaded {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!("Audition settings could not be restored: {e:#}")),
            ),
        };
        Audition {
            settings: Signal::new(settings),
            storage_warning: Signal::new(warning),
            audio_warning: Signal::new(None),
        }
    });
    use_effect(move || {
        let settings = (state.settings)();
        apply(studio, state, settings);
    });
}
fn configure(studio: Studio, settings: Settings) -> anyhow::Result<()> {
    if !settings.enabled() || super::dance::active() {
        return studio.configure_audition(None);
    }
    let Some(doc) = studio.snapshot() else {
        return studio.configure_audition(None);
    };
    let Some(store) = studio.store.read().clone() else {
        return Ok(());
    };
    let plan = Arc::new(Plan::for_project(&store, &doc, &settings)?);
    studio.configure_audition(Some(Arc::new(Mix::new(plan, settings)?)))
}
fn apply(studio: Studio, mut state: Audition, settings: Settings) {
    let result = configure(studio, settings);
    if result.is_err() {
        let _ = studio.configure_audition(None);
    }
    state.audio_warning.set(
        result
            .err()
            .map(|e| format!("Click audition unavailable: {e:#}")),
    );
}
#[cfg(target_arch = "wasm32")]
pub(super) fn report_warning(message: String) {
    if let Some(mut state) = try_consume_context::<Audition>() {
        state.audio_warning.set(Some(message));
    }
}
fn update(studio: Studio, edit: impl FnOnce(&mut Settings)) {
    let mut state = consume_context::<Audition>();
    let mut settings = state.settings.read().clone();
    edit(&mut settings);
    if let Err(error) = settings.validate() {
        state.storage_warning.set(Some(format!("{error:#}")));
        return;
    }
    state.storage_warning.set(
        preferences::save(&settings)
            .err()
            .map(|e| format!("Settings are active for this session; saving failed: {e:#}")),
    );
    state.settings.set(settings.clone());
    apply(studio, state, settings);
}
#[component]
pub fn Controls(studio: Studio) -> Element {
    let Some(state) = try_consume_context::<Audition>() else {
        return rsx! {};
    };
    let settings = (state.settings)();
    let dance = super::dance::active();
    let disabled = studio.snapshot().is_none() || dance;
    let vocal = studio
        .snapshot()
        .is_some_and(|d| d.selected_track == "Vocals");
    rsx! {fieldset{class:"audition-clicks",disabled,legend{"Metronome and note claps"}
        div{class:"audition-click-row",
            label{input{r#type:"checkbox",aria_label:"Metronome",checked:settings.metronome,onchange:move|e|update(studio,|s|s.metronome=e.checked())}"Metronome"}
            label{"Beat volume ",input{r#type:"range",min:"0",max:"100",step:"1",aria_label:"Metronome volume",value:"{settings.beat_gain*100.}",oninput:move|e|{if let Ok(value)=e.value().parse::<f32>(){update(studio,|s|s.beat_gain=value/100.);}}}}
            label{input{r#type:"checkbox",aria_label:"Note claps",checked:settings.claps,onchange:move|e|update(studio,|s|s.claps=e.checked())}"Note claps"}
            label{"Clap volume ",input{r#type:"range",min:"0",max:"100",step:"1",aria_label:"Clap volume",value:"{settings.clap_gain*100.}",oninput:move|e|{if let Ok(value)=e.value().parse::<f32>(){update(studio,|s|s.clap_gain=value/100.);}}}}
        }
        p{class:"field-hint","Claps follow the selected part, with one clap per attack. Music plays slightly quieter while clicks are enabled."}
        details{summary{"Clap note classes"}div{class:"audition-click-classes",
            for class in NoteClass::ALL{label{input{r#type:"checkbox",aria_label:"Clap {class.label()}",checked:settings.classes.contains(&class),onchange:move|e|update(studio,|s|{s.classes.retain(|c|*c!=class);if e.checked(){s.classes.push(class);}})}"{class.label()}"}}
        }}
        if vocal{label{"Singer ",select{aria_label:"Clap singer",value:match settings.singer{Singer::Lead=>"lead",Singer::Harm1=>"harm1",Singer::Harm2=>"harm2",Singer::Harm3=>"harm3"},onchange:move|e|update(studio,|s|s.singer=match e.value().as_str(){"harm1"=>Singer::Harm1,"harm2"=>Singer::Harm2,"harm3"=>Singer::Harm3,_=>Singer::Lead}),
            option{value:"lead","Lead"}option{value:"harm1","Harmony 1"}option{value:"harm2","Harmony 2"}option{value:"harm3","Harmony 3"}
        }}}
        p{class:"field-hint","Settings stay on this device or browser. Song timing and exports are unchanged."}
    }
    if dance{p{class:"field-hint","Dance click audition is not ready yet."}}
    for warning in [(state.storage_warning)(),(state.audio_warning)()].into_iter().flatten(){p{role:"alert",class:"field-hint","{warning}"}}
    }
}
