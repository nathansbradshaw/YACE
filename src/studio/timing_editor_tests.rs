use super::*;
use dioxus::core::{AttributeValue, ElementId, Mutation};
use std::{any::Any, cell::RefCell, rc::Rc};
mod fixture {
    use crate as chart_engine;
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/assistant_project.rs"
    ));
}
type Slot = Rc<RefCell<Option<Studio>>>;
fn loaded() -> Element {
    let mut studio = super::super::state::use_studio();
    let (store, slot) = use_context::<(crate::editor::Store, Slot)>();
    use_hook(move || {
        let doc = store.load().unwrap();
        studio.timing.set(Some(doc.timing(&store).unwrap()));
        studio.doc.set(Some(std::sync::Arc::new(doc)));
        studio.store.set(Some(store));
        *slot.borrow_mut() = Some(studio);
    });
    rsx! {OffsetControl{studio}}
}
fn form_event(dom: &VirtualDom, name: &str, id: ElementId, value: &str) {
    dom.runtime().handle_event(
        name,
        Event::new(
            Rc::new(dioxus::html::PlatformEventData::new(Box::new(
                dioxus::html::SerializedFormData::new(value.into(), vec![]),
            ))) as Rc<dyn Any>,
            true,
        ),
        id,
    );
}
#[tokio::test]
async fn offset_form_applies_signed_values_and_refreshes_after_undo() {
    dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let slot = Slot::default();
    let mut dom = VirtualDom::new(loaded);
    dom.provide_root_context((store.clone(), slot.clone()));
    let changes = dom.rebuild_to_vec();
    let listener = |kind: &str| {
        changes
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::NewEventListener { name, id } if *name == kind => Some(*id),
                _ => None,
            })
            .unwrap()
    };
    let input = listener("input");
    let submit = listener("submit");
    form_event(&dom, "input", input, "250");
    dom.render_immediate_to_vec();
    form_event(&dom, "submit", submit, "");
    dom.render_immediate_to_vec();
    assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, 0.25);
    let studio = slot.borrow().unwrap();
    studio.dispatch(Action::Undo);
    tokio::time::timeout(std::time::Duration::from_secs(5),async{
        loop {
            let changes=dom.render_immediate_to_vec();
            if changes.edits.iter().any(|edit|matches!(edit,Mutation::SetAttribute{name:"value",value:AttributeValue::Text(value),id,..} if value=="0" && *id==input)){break;}
            tokio::select!{_=dom.wait_for_work()=>{},_=tokio::time::sleep(std::time::Duration::from_millis(5))=>{}}
        }
    }).await.unwrap();
    form_event(&dom, "input", input, "-125");
    dom.render_immediate_to_vec();
    form_event(&dom, "submit", submit, "");
    dom.render_immediate_to_vec();
    assert_eq!(store.load().unwrap().timing(&store).unwrap().offset, -0.125);
    assert!(studio.error.peek().is_none());
}
