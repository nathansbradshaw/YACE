//! A thin marker lane and one dialog for naming song sections.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::{
    editor::{Action, TimeRange},
    sections::{Edit, Marker},
};
use dioxus::prelude::*;

fn markers(studio: Studio) -> Vec<Marker> {
    studio
        .doc
        .read()
        .as_ref()
        .and_then(|doc| doc.content.sections.clone())
        .unwrap_or_else(|| (studio.source_sections)())
}
fn jump(mut studio: Studio, marker: &Marker) {
    let Some(timing) = (studio.timing)() else {
        return;
    };
    let seconds = timing.seconds(marker.tick).max(0.);
    studio.dispatch(Action::SetLoop { range: None });
    studio.seek(seconds);
    studio.view_start.set((seconds - 0.5).max(0.));
}
fn section_range(studio: Studio, list: &[Marker], index: usize) -> Option<TimeRange> {
    let timing = (studio.timing)()?;
    let duration = studio.snapshot()?.duration;
    let start = timing.seconds(list.get(index)?.tick).max(0.);
    let end = list
        .get(index + 1)
        .map_or(duration, |next| timing.seconds(next.tick))
        .min(duration);
    (end > start).then_some(TimeRange { start, end })
}

#[component]
pub fn SectionStrip(mut studio: Studio) -> Element {
    let mut width = use_signal(|| 1000.);
    let list = markers(studio);
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    rsx! {
        div { class: "section-strip", aria_label: "Song sections",
            button { class: "section-strip-label", title: "Add, rename or move song sections", onclick: move |_| studio.sections_open.set(true), Icon { name: "flag", size: 12 } "Sections" }
            div { class: "section-strip-markers", onresize: move |event| { if let Ok(size) = event.get_content_box_size() { width.set(size.width.max(1.)); } },
                for (index, marker) in list.iter().enumerate() {
                    { let seconds = timing.seconds(marker.tick); let x = (seconds-start)/span*width();
                      let next = list.get(index+1).map_or(width(), |next| (timing.seconds(next.tick)-start)/span*width());
                      let available = (next-x-3.).min(width()-x).clamp(2.,180.); let marker = marker.clone();
                      rsx! { if seconds >= start && seconds < start+span {
                          button { class: "section-marker", style: "left:{x}px;max-width:{available}px", aria_label: "Go to section {marker.name}", title: "{marker.name} · {clock(seconds)}", onclick: move |_| jump(studio,&marker), "{marker.name}" }
                      } }
                    }
                }
            }
        }
        if (studio.sections_open)() { SectionDialog { studio } }
    }
}

#[component]
fn SectionDialog(mut studio: Studio) -> Element {
    let mut previous = use_signal(|| None::<Marker>);
    let mut name = use_signal(String::new);
    let mut tick = use_signal(|| {
        super::note_canvas::snap(studio, (studio.playhead)())
            .unwrap_or(0)
            .to_string()
    });
    // Keep typed drafts separate from values applied by explicit form resets.
    let mut defaults = use_signal(|| (name(), tick()));
    let mut form_epoch = use_signal(|| 0u64);
    let mut error = use_signal(|| None::<String>);
    let list = markers(studio);
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let mut save = move |_| {
        let Ok(position) = tick().parse::<u64>() else {
            error.set(Some("Enter a whole tick number.".into()));
            return;
        };
        studio.dispatch(Action::EditSection {
            edit: Edit::Put {
                previous: previous(),
                marker: Marker {
                    tick: position,
                    name: name().trim().into(),
                },
            },
        });
        error.set((studio.error)());
        if error().is_none() {
            previous.set(None);
            name.set(String::new());
            defaults.set((String::new(), tick()));
            form_epoch.set(form_epoch().wrapping_add(1));
        }
    };
    rsx! {
        super::modal::Modal { id: "song-sections-dialog", label: "Song sections", onclose: move |_| studio.sections_open.set(false),
            section { class: "sections-dialog",
                header { h2 { "Song sections" } button { aria_label: "Close song sections", onclick: move |_| studio.sections_open.set(false), Icon { name: "close", size: 16 } } }
                if list.is_empty() { p { class: "field-hint", "Add a section at the playhead, then name it." } }
                ul { class: "sections-list",
                    for (index, marker) in list.iter().enumerate() {
                        { let go = marker.clone(); let edit = marker.clone(); let remove = marker.clone(); let range = section_range(studio,&list,index); let seconds = timing.seconds(marker.tick);
                          rsx! { li {
                              button { class: "section-go", title: "Go to {marker.name}", onclick: move |_| { jump(studio,&go); studio.sections_open.set(false); }, span { "{marker.name}" } time { "{clock(seconds)}" } }
                              button { title: "Loop section", aria_label: "Loop section {marker.name}", disabled: range.is_none(), onclick: move |_| { if let Some(range) = range { let start = range.start; studio.dispatch(Action::SetLoop { range: Some(range) }); studio.seek(start); studio.view_start.set((start-0.5).max(0.)); studio.sections_open.set(false); } }, Icon { name: "loop", size: 14 } }
                              button { title: "Edit section", aria_label: "Edit section {marker.name}", onclick: move |_| { previous.set(Some(edit.clone())); name.set(edit.name.clone()); tick.set(edit.tick.to_string()); defaults.set((edit.name.clone(),edit.tick.to_string())); form_epoch.set(form_epoch().wrapping_add(1)); error.set(None); }, Icon { name: "sliders", size: 14 } }
                              button { title: "Delete section (Undo restores it)", aria_label: "Delete section {marker.name}", onclick: move |_| {
                                  studio.dispatch(Action::EditSection { edit: Edit::Remove { marker: remove.clone() } }); error.set((studio.error)());
                                  if error().is_none() && previous().as_ref() == Some(&remove) { previous.set(None); name.set(String::new()); defaults.set((String::new(),tick())); form_epoch.set(form_epoch().wrapping_add(1)); }
                              }, Icon { name: "trash", size: 14 } }
                          } }
                        }
                    }
                }
                for form_key in [form_epoch()] {
                form { key: "{form_key}", class: "section-form", onsubmit: move |event| { event.prevent_default(); save(()); },
                    label { "Name" input { aria_label: "Section name", maxlength: "128", placeholder: "Chorus", initial_value: defaults().0, oninput: move |event| name.set(event.value()) } }
                    label { "Start tick" input { aria_label: "Section start tick", r#type: "number", min: "0", step: "1", initial_value: defaults().1, oninput: move |event| tick.set(event.value()) } }
                    button { r#type: "button", title: "Use the snapped playhead position", onclick: move |_| { if let Some(position) = super::note_canvas::snap(studio,(studio.playhead)()) { tick.set(position.to_string()); defaults.set((name(),position.to_string())); form_epoch.set(form_epoch().wrapping_add(1)); } }, "At playhead" }
                    div { class: "section-form-actions",
                        if previous().is_some() { button { r#type: "button", onclick: move |_| { previous.set(None); name.set(String::new()); defaults.set((String::new(),tick())); form_epoch.set(form_epoch().wrapping_add(1)); error.set(None); }, "New section" } }
                        button { r#type: "submit", disabled: name().trim().is_empty(), if previous().is_some() { "Save" } else { "Add" } }
                    }
                }
                }
                if let Some(error) = error() { p { role: "alert", class: "section-error", "{error}" } }
            }
        }
    }
}
