//! Panel sizing uses the available workbench area, including after a resize.
use super::{
    chart_desk::ChartDesk, game_preview::Preview, selection_desk::SelectionDesk, state::Studio,
};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// View preferences are separate from authored content and its undo history.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Layout {
    right: f64,
    top: f64,
    preview_hidden: bool,
    inspector_hidden: bool,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            right: 28.,
            top: 45.,
            preview_hidden: false,
            inspector_hidden: false,
        }
    }
}
impl Layout {
    pub(super) fn normalized(mut self) -> Self {
        let defaults = Self::default();
        self.right = if self.right.is_finite() {
            self.right.clamp(23., 42.)
        } else {
            defaults.right
        };
        self.top = if self.top.is_finite() {
            self.top.clamp(35., 78.)
        } else {
            defaults.top
        };
        self
    }
    fn resize(&mut self, pane: Pane, value: f64) {
        match pane {
            Pane::Selection => self.right = value,
            Pane::Preview => self.top = value,
        }
    }
}

#[component]
pub(super) fn LayoutControls(studio: Studio) -> Element {
    let layout = super::track_views::layout(studio);
    rsx! {div{class:"workspace-layout-controls",role:"group",aria_label:"Workspace layout",
        button{tabindex:"0",aria_pressed:(!layout.preview_hidden).to_string(),onclick:move|_|{
            let mut next=super::track_views::layout(studio);next.preview_hidden = !next.preview_hidden;
            super::track_views::set_layout(studio,next,true);
        },if layout.preview_hidden{"Show preview"}else{"Hide preview"}}
        button{tabindex:"0",aria_pressed:(!layout.inspector_hidden).to_string(),onclick:move|_|{
            let mut next=super::track_views::layout(studio);next.inspector_hidden = !next.inspector_hidden;
            super::track_views::set_layout(studio,next,true);
        },if layout.inspector_hidden{"Show inspector"}else{"Hide inspector"}}
        button{tabindex:"0",disabled:layout==Layout::default(),onclick:move|_|super::track_views::set_layout(studio,Layout::default(),true),"Reset layout"}
    }}
}

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
    studio: Studio,
    pane: Pane,
    label: &'static str,
    value: f64,
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
                    value,
                    available,
                }));
            },
            onkeydown: move |event: KeyboardEvent| {
                event.stop_propagation();
                if let Some(next) = pane.key(value, &event.key(), event.modifiers().shift()) {
                    event.prevent_default();
                    let mut layout=super::track_views::layout(studio);
                    layout.resize(pane,next);
                    super::track_views::set_layout(studio,layout,true);
                }
            },
        }
    }
}

#[component]
pub fn Workbench(studio: Studio) -> Element {
    super::part_analysis::provide_panel();
    super::track_views::provide(studio);
    let mut size = use_signal(|| (1400.0_f64, 800.0_f64));
    let mut drag = use_signal(|| None::<Drag>);
    let layout = super::track_views::layout(studio);
    let right = layout.right;
    let top = layout.top;
    let center = 100.0 - right;
    let bottom = 100.0 - top;
    let rows = if layout.preview_hidden {
        "max-content 4px minmax(160px,1fr)".into()
    } else {
        format!("minmax(200px,{top}fr) 4px minmax(160px,{bottom}fr)")
    };
    let columns = if layout.inspector_hidden {
        "minmax(300px,1fr) 0px 0px".into()
    } else {
        format!("minmax(300px,{center}fr) 4px minmax(240px,{right}fr)")
    };
    let class = format!(
        "workspace-shell{}{}",
        if layout.preview_hidden {
            " preview-hidden"
        } else {
            ""
        },
        if layout.inspector_hidden {
            " inspector-hidden"
        } else {
            ""
        }
    );
    let (width, height) = size();
    rsx! {
        div {
            class,
            style: "grid-template-rows:{rows};grid-template-columns:{columns}",
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
                    let mut layout=super::track_views::layout(studio);
                    layout.resize(pane,next);
                    super::track_views::set_layout(studio,layout,false);
                }
            },
            onmouseup: move |_| {if drag().is_some(){super::track_views::set_layout(studio,super::track_views::layout(studio),true);drag.set(None);}},
            onmouseleave: move |_| {if drag().is_some(){super::track_views::set_layout(studio,super::track_views::layout(studio),true);drag.set(None);}},
            div { class: "audition-workspace",
                Preview { studio }
                Divider { studio, pane: Pane::Selection, label: "Resize selection desk", value: right, available: (width - 4.0).max(1.0), drag }
                SelectionDesk { studio }
            }
            Divider { studio, pane: Pane::Preview, label: "Resize note editor", value: top, available: (height - 4.0).max(1.0), drag }
            ChartDesk { studio }
        }
        // Keep the fixed drawer outside the preview's overflow clip and the
        // collapsible playback settings, on both desktop and the static web app.
        super::part_analysis::PartAnalysisPanel { studio }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_layout_bounds_and_hidden_panels_keep_restore_proportions() {
        let layout = Layout {
            right: -10.,
            top: 500.,
            preview_hidden: true,
            inspector_hidden: true,
        }
        .normalized();
        assert_eq!((layout.right, layout.top), (23., 78.));
        assert!(layout.preview_hidden && layout.inspector_hidden);
        let invalid = Layout {
            right: f64::NAN,
            top: f64::INFINITY,
            ..Layout::default()
        }
        .normalized();
        assert_eq!(invalid, Layout::default());
        let mut resized = Layout {
            right: 31.5,
            top: 62.,
            ..Layout::default()
        };
        resized.preview_hidden = true;
        resized.inspector_hidden = true;
        let reopened: Layout =
            serde_json::from_slice(&serde_json::to_vec(&resized).unwrap()).unwrap();
        assert_eq!((reopened.right, reopened.top), (31.5, 62.));
        assert!(reopened.preview_hidden && reopened.inspector_hidden);
    }

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
