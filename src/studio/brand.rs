//! The branching Y joins note lanes into one editing timeline.
use dioxus::prelude::*;

#[component]
pub(super) fn BrandMark(#[props(default = 32)] size: u16) -> Element {
    rsx! {
        svg {
            class: "yace-mark", width: "{size}", height: "{size}", view_box: "0 0 32 32",
            "aria-hidden": "true", "focusable": "false", fill: "none",
            path { d: "M6 5L16 16L26 5M16 16V28", stroke: "currentColor", stroke_width: "4", stroke_linecap: "square", stroke_linejoin: "miter" }
            path { d: "M5 15L10 20M27 15L22 20", stroke: "currentColor", stroke_width: "2" }
        }
    }
}
