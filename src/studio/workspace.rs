//! Panel sizing uses the available workbench area, including after a resize.
use super::{
    chart_desk::ChartDesk, game_preview::Preview, selection_desk::SelectionDesk, state::Studio,
};
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Pane {
    Selection,
    Preview,
}

impl Pane {
    fn bounds(self) -> (f64, f64) {
        match self {
            Self::Selection => (23.0, 42.0),
            Self::Preview => (35.0, 78.0),
        }
    }

    fn adjust(self, value: f64, pixels: f64, available: f64) -> f64 {
        if !pixels.is_finite() || !available.is_finite() || available <= 0.0 {
            return value;
        }
        let direction = if self == Self::Selection { -1.0 } else { 1.0 };
        let (min, max) = self.bounds();
        (value + direction * pixels / available * 100.0).clamp(min, max)
    }

    fn key(self, value: f64, key: &Key, shift: bool) -> Option<f64> {
        let step = if shift { 5.0 } else { 1.0 };
        let (min, max) = self.bounds();
        match (self, key) {
            (_, Key::Home) => Some(min),
            (_, Key::End) => Some(max),
            (Self::Preview, Key::ArrowUp) | (Self::Selection, Key::ArrowRight) => {
                Some((value - step).clamp(min, max))
            }
            (Self::Preview, Key::ArrowDown) | (Self::Selection, Key::ArrowLeft) => {
                Some((value + step).clamp(min, max))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Drag {
    pane: Pane,
    origin: f64,
    value: f64,
    available: f64,
}

#[component]
fn Divider(
    pane: Pane,
    label: &'static str,
    mut value: Signal<f64>,
    available: f64,
    mut drag: Signal<Option<Drag>>,
) -> Element {
    let vertical = pane != Pane::Preview;
    let (min, max) = pane.bounds();
    rsx! {
        div {
            class: if vertical { "splitter vertical" } else { "splitter horizontal" },
            role: "separator",
            tabindex: "0",
            aria_label: label,
            aria_orientation: if vertical { "vertical" } else { "horizontal" },
            aria_valuemin: "{min}",
            aria_valuemax: "{max}",
            aria_valuenow: "{value}",
            title: "Drag or use arrow keys to resize. Hold Shift for larger steps.",
            onmousedown: move |event: MouseEvent| {
                let point = event.page_coordinates();
                drag.set(Some(Drag {
                    pane,
                    origin: if vertical { point.x } else { point.y },
                    value: value(),
                    available,
                }));
            },
            onkeydown: move |event: KeyboardEvent| {
                event.stop_propagation();
                if let Some(next) = pane.key(value(), &event.key(), event.modifiers().shift()) {
                    event.prevent_default();
                    value.set(next);
                }
            },
        }
    }
}

#[component]
pub fn Workbench(studio: Studio) -> Element {
    let mut right = use_signal(|| 28.0_f64);
    let mut top = use_signal(|| 45.0_f64);
    let mut size = use_signal(|| (1400.0_f64, 800.0_f64));
    let mut drag = use_signal(|| None::<Drag>);
    let center = 100.0 - right();
    let bottom = 100.0 - top();
    let (width, height) = size();
    rsx! {
        div {
            class: "workspace-shell",
            style: "grid-template-rows:minmax(200px,{top}fr) 4px minmax(160px,{bottom}fr)",
            onresize: move |event| {
                if let Ok(box_size) = event.get_content_box_size() {
                    size.set((box_size.width, box_size.height));
                }
            },
            onmousemove: move |event: MouseEvent| {
                if let Some(Drag { pane, origin, value, available }) = drag() {
                    let point = event.page_coordinates();
                    let delta = if pane == Pane::Preview { point.y - origin } else { point.x - origin };
                    let next = pane.adjust(value, delta, available);
                    match pane {
                        Pane::Selection => right.set(next),
                        Pane::Preview => top.set(next),
                    }
                }
            },
            onmouseup: move |_| drag.set(None),
            onmouseleave: move |_| drag.set(None),
            div { class: "audition-workspace", style: "grid-template-columns:minmax(300px,{center}fr) 4px minmax(240px,{right}fr)",
                Preview { studio }
                Divider { pane: Pane::Selection, label: "Resize selection desk", value: right, available: (width - 4.0).max(1.0), drag }
                SelectionDesk { studio }
            }
            Divider { pane: Pane::Preview, label: "Resize note editor", value: top, available: (height - 4.0).max(1.0), drag }
            ChartDesk { studio }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_resize_uses_actual_space_and_respects_each_edge() {
        assert_eq!(Pane::Selection.adjust(28.0, 50.0, 1000.0), 23.0);
        assert_eq!(Pane::Selection.adjust(28.0, 50.0, 2000.0), 25.5);
        assert_eq!(Pane::Preview.adjust(58.0, 1000.0, 600.0), 78.0);
        assert_eq!(Pane::Selection.adjust(28.0, -1000.0, 1000.0), 42.0);
        assert_eq!(Pane::Preview.adjust(58.0, 100.0, 0.0), 58.0);
    }

    #[test]
    fn keyboard_resize_follows_divider_direction_and_keeps_bounds() {
        assert_eq!(
            Pane::Selection.key(28.0, &Key::ArrowLeft, false),
            Some(29.0)
        );
        assert_eq!(
            Pane::Selection.key(26.0, &Key::ArrowRight, true),
            Some(23.0)
        );
        assert_eq!(Pane::Preview.key(58.0, &Key::Home, false), Some(35.0));
        assert_eq!(Pane::Preview.key(58.0, &Key::End, false), Some(78.0));
        assert_eq!(Pane::Preview.key(58.0, &Key::ArrowRight, false), None);
    }
}
