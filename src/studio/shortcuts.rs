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
            super::session::undo(studio, matches!(action, Action::Redo));
        } else if let Key::Character(value) = &key {
            match value.to_ascii_lowercase().as_str() {
                "a" => {
                    event.prevent_default();
                    super::session::select_all(studio);
                }
                "c" => {
                    event.prevent_default();
                    super::session::copy(studio, false);
                }
                "x" => {
                    event.prevent_default();
                    super::session::copy(studio, true);
                }
                "v" => {
                    event.prevent_default();
                    super::session::paste(studio);
                }
                _ => {}
            }
        }
        return;
    }
    if super::dance::active() && super::dance::handle_navigation(&event) {
        return;
    }
    match key {
        Key::Character(value) if value == " " => {
            event.prevent_default();
            super::session::toggle_playback(studio);
        }
        Key::Delete | Key::Backspace => {
            event.prevent_default();
            super::session::delete(studio);
        }
        Key::Escape => super::session::clear(studio),
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
