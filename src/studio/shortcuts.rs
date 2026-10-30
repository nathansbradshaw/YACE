//! Editor commands stay separate from layout and panel rendering.
use super::state::{EditTool, Studio};
use crate::editor::Action;
use dioxus::prelude::*;

pub fn keep_space_local(event: KeyboardEvent) {
    if event.key() == Key::Character(" ".into()) {
        event.stop_propagation();
    }
}

fn undo_action(key: &Key, shift: bool) -> Option<Action> {
    match key {
        Key::Character(value) if value.eq_ignore_ascii_case("z") => {
            Some(if shift { Action::Redo } else { Action::Undo })
        }
        _ => None,
    }
}

pub fn handle(mut studio: Studio, event: KeyboardEvent) {
    // Text fields consume their events before they reach the editor.
    let modifiers = event.modifiers();
    let key = event.key();
    if modifiers.alt() {
        return;
    }
    if modifiers.meta() || modifiers.ctrl() {
        if let Some(action) = undo_action(&key, modifiers.shift()) {
            event.prevent_default();
            studio.dispatch(action);
        }
        return;
    }
    match key {
        Key::Character(value) if value == " " => {
            event.prevent_default();
            studio.toggle_playback();
        }
        Key::Delete | Key::Backspace => {
            let Some(doc) = studio.snapshot() else { return };
            if doc.selected_track == "Vocals" {
                if let Some(index) = (studio.vocal_note)() {
                    event.prevent_default();
                    studio.dispatch(Action::DeleteVocalNote { index });
                    studio.vocal_note.set(None);
                }
            } else if let Some(note_id) = doc.selected_note {
                event.prevent_default();
                studio.dispatch(Action::DeleteNote { note_id });
            }
        }
        Key::Character(value) => {
            let tool = match value.as_str() {
                "1" => EditTool::Select,
                "2" => EditTool::Place,
                "3" => EditTool::Erase,
                _ => return,
            };
            event.prevent_default();
            studio.tool.set(tool);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_uppercase_z_redoes_instead_of_being_ignored() {
        assert!(matches!(
            undo_action(&Key::Character("Z".into()), true),
            Some(Action::Redo)
        ));
        assert!(matches!(
            undo_action(&Key::Character("z".into()), true),
            Some(Action::Redo)
        ));
        assert!(matches!(
            undo_action(&Key::Character("z".into()), false),
            Some(Action::Undo)
        ));
        assert!(undo_action(&Key::Character("x".into()), false).is_none());
    }
}
