use super::{modal::Modal, state::Studio};
use dioxus::prelude::*;

#[component]
pub fn Setup(mut studio: Studio) -> Element {
    let mut title = use_signal(|| "Untitled".to_owned());
    let mut artist = use_signal(|| "Unknown artist".to_owned());
    let mut bpm = use_signal(|| "120".to_owned());
    let mut duration = use_signal(|| "180".to_owned());
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let location = use_signal(super::new_location::Choice::default);
    let valid = crate::project_recording::blank_song(
        &title(),
        &artist(),
        bpm().parse().unwrap_or(f64::NAN),
        duration().parse().unwrap_or(f64::NAN),
    )
    .is_ok();
    rsx! {Modal{id:"new-blank-chart-dialog",label:"New blank chart",onclose:move |_|{if !busy(){studio.blank_chart.set(false);}},
        section{class:"sections-dialog",h2{"New blank chart"}
            p{"Create an empty Guitar chart without a recording. Add parts, edit timing and attach audio later."}
            super::new_location::Prompt{choice:location,busy:busy()}
            label{class:"field-stack","Title",input{class:"field-control",aria_label:"Blank chart title",value:title,disabled:busy(),oninput:move|e|title.set(e.value())}}
            label{class:"field-stack","Artist",input{class:"field-control",aria_label:"Blank chart artist",value:artist,disabled:busy(),oninput:move|e|artist.set(e.value())}}
            label{class:"field-stack","Tempo (BPM)",input{class:"field-control",r#type:"number",min:"1",max:"1000",aria_label:"Blank chart tempo",value:bpm,disabled:busy(),oninput:move|e|bpm.set(e.value())}}
            label{class:"field-stack","Timeline length (seconds)",input{class:"field-control",r#type:"number",min:"1",max:"86400",aria_label:"Blank chart length",value:duration,disabled:busy(),oninput:move|e|duration.set(e.value())}}
            if let Some(message)=error(){p{role:"alert","{message}"}}
            div{class:"dialog-actions",button{aria_label:if busy(){"Creating chart, cancellation unavailable"}else{"Cancel blank chart setup"},disabled:busy(),onclick:move |_|studio.blank_chart.set(false),"Cancel"}
                button{disabled:busy()||!valid||!location.read().ready(),onclick:move |_|{if !location.read().ready(){return;}busy.set(true);error.set(None);spawn(async move {let result=studio.finish_blank_chart(title(),artist(),bpm().parse().unwrap_or(f64::NAN),duration().parse().unwrap_or(f64::NAN),location()).await;busy.set(false);if let Err(failure)=result {error.set(Some(format!("{failure:#}")));}});},if busy(){"Creating…"}else{"Create chart"}}
            }
        }
    }}
}

#[cfg(all(test, not(feature = "browser")))]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::RefCell, rc::Rc};
    type Slot = Rc<RefCell<Option<Studio>>>;
    fn app() -> Element {
        let mut studio = super::super::state::use_studio();
        let slot = use_context::<Slot>();
        use_hook(move || {
            studio.blank_chart.set(true);
            *slot.borrow_mut() = Some(studio);
        });
        rsx! {if (studio.blank_chart)(){Setup{studio}}}
    }
    #[tokio::test]
    async fn cancel_blank_setup_keeps_the_workspace_empty_without_a_file_picker() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let slot = Slot::default();
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context(slot.clone());
        let mut changes = dom.rebuild_to_vec();
        let cancel=tokio::time::timeout(std::time::Duration::from_secs(5),async{
            loop {
                if let Some(cancel)=changes.edits.iter().find_map(|edit|match edit {
                    Mutation::SetAttribute{name:"aria-label",value:AttributeValue::Text(value),id,..} if value=="Cancel blank chart setup"=>Some(*id),_=>None,
                }) {break cancel;}
                tokio::select!{_=dom.wait_for_work()=>{},_=tokio::time::sleep(std::time::Duration::from_millis(5))=>{}}
                changes=dom.render_immediate_to_vec();
            }
        }).await.unwrap();
        dom.runtime().handle_event(
            "click",
            Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::<
                    dioxus::html::SerializedMouseData,
                >::default(
                ))) as Rc<dyn Any>,
                true,
            ),
            cancel,
        );
        dom.render_immediate_to_vec();
        let studio = slot.borrow().unwrap();
        assert!(!*studio.blank_chart.peek());
        assert!(studio.store.peek().is_none());
        assert!(studio.doc.peek().is_none());
    }
}
