//! Chart Engine's original, monochrome SVG icon set.
//!
//! These are UI symbols drawn on a shared 24-unit grid, not emoji, font glyphs,
//! or artwork copied from a game. Buttons supply their accessible names while
//! decorative SVGs stay out of the accessibility tree.
use dioxus::prelude::*;

fn drawing(name: &str) -> &'static str {
    match name {
        "folder" => "<path d='M3 6h6l2 2h10v11H3Z'/><path d='M3 6V4h6l2 2h8v2'/>",
        "folder-open" => "<path d='M3 9V5h6l2 2h9v3'/><path d='M3 10h19l-3 10H3Z'/>",
        "file-plus" => "<path d='M14 3H5v18h14V8Z'/><path d='M14 3v5h5M8 14h8M12 10v8'/>",
        "save" => "<path d='M3 3h15l3 3v15H3Z'/><path d='M7 3v6h10V3M7 21v-8h10v8'/>",
        "export" => "<path d='M5 13v7h14v-7M12 16V3M7 8l5-5 5 5'/>",
        "undo" => "<path d='m8 5-5 5 5 5M3 10h11a6 6 0 0 1 0 12'/>",
        "redo" => "<path d='m16 5 5 5-5 5M21 10H10a6 6 0 0 0 0 12'/>",
        "play" => "<path d='m7 4 13 8-13 8Z'/>",
        "pause" => "<path d='M7 4v16M17 4v16'/>",
        "square" | "stop" => "<rect x='5' y='5' width='14' height='14' rx='1'/>",
        "volume" => "<path d='M4 9h4l5-5v16l-5-5H4ZM17 8a6 6 0 0 1 0 8M20 5a10 10 0 0 1 0 14'/>",
        "sliders" => {
            "<path d='M3 6h7M16 6h5M3 18h13M20 18h1'/><circle cx='13' cy='6' r='3'/><circle cx='18' cy='18' r='3'/>"
        }
        "sparkles" => {
            "<path d='m12 3 2.6 6.4L21 12l-6.4 2.6L12 21l-2.6-6.4L3 12l6.4-2.6ZM20 2v4M18 4h4'/>"
        }
        "flag" => "<path d='M5 21V3M5 4c5-4 9 4 15 0v10c-6 4-10-4-15 0'/>",
        "music" => {
            "<path d='M9 17V5l11-2v12M9 9l11-2'/><ellipse cx='6' cy='18' rx='3' ry='2'/><ellipse cx='17' cy='16' rx='3' ry='2'/>"
        }
        "info" => "<circle cx='12' cy='12' r='9'/><path d='M12 11v6M12 7v.1'/>",
        "comment" => "<path d='M4 4h16v12H9l-5 4Z'/><path d='M8 8h8M8 12h5'/>",
        "drums" => {
            "<ellipse cx='12' cy='11' rx='8' ry='3'/><path d='M4 11v7c0 4 16 4 16 0v-7M8 14v6M16 14v6M4 3l7 4M20 3l-7 4'/>"
        }
        "guitar" => {
            "<path d='m14 9 6-6 2 2-6 6M17 4l3 3M12 8c-3-2-4 1-3 3-4-1-8 4-5 7 3 4 9 2 9-2 3 0 6-3 2-5'/><circle cx='10' cy='14' r='2'/>"
        }
        "bass" => {
            "<path d='m13 10 7-7 2 2-7 7M18 4l3 3M11 9c-4-2-5 3-3 4-4 0-6 4-3 6 3 3 7 0 6-3 4 2 7-2 3-5M7 17l3-3'/>"
        }
        "search" => "<circle cx='10' cy='10' r='6'/><path d='m15 15 6 6'/>",
        "chevron-down" => "<path d='m6 9 6 6 6-6'/>",
        "chevron-right" => "<path d='m9 6 6 6-6 6'/>",
        "trash" => "<path d='M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7M14 10v7'/>",
        "check" => "<path d='m4 12 5 5L20 6'/>",
        "eye-off" => {
            "<path d='m3 3 18 18M9 5c7-3 12 7 12 7a24 24 0 0 1-4 5M6 6a22 22 0 0 0-3 6s8 12 16 5M10 10a3 3 0 0 0 4 4'/>"
        }
        "eye" => "<path d='M2 12s9-14 20 0c-11 14-20 0-20 0Z'/><circle cx='12' cy='12' r='3'/>",
        "plus" => "<path d='M12 5v14M5 12h14'/>",
        "minus" => "<path d='M5 12h14'/>",
        "cursor" => "<path d='m4 3 15 10-7 1-3 7Z'/>",
        "eraser" => "<path d='m14 3 7 7-11 11H7l-5-5ZM7 11l7 7M10 21h12'/>",
        "close" => "<path d='m6 6 12 12M18 6 6 18'/>",
        "loop" => {
            "<path d='M5 8a7 7 0 0 1 12-3l3 3M20 3v5h-5M19 16a7 7 0 0 1-12 3l-3-3M4 21v-5h5'/>"
        }
        "skip-back" => "<path d='M5 5v14M19 5l-10 7 10 7Z'/>",
        "skip-forward" => "<path d='M19 5v14M5 5l10 7-10 7Z'/>",
        _ => "<circle cx='12' cy='12' r='8'/><path d='M12 8v8M8 12h8'/>",
    }
}

#[component]
pub fn Icon(
    name: &'static str,
    #[props(default = 18)] size: u16,
    #[props(default)] class: String,
) -> Element {
    rsx! {
        svg {
            class: "studio-icon {class}", width: "{size}", height: "{size}", view_box: "0 0 24 24",
            fill: "none", stroke: "currentColor", stroke_width: "1.8",
            stroke_linecap: "round", stroke_linejoin: "round",
            "aria-hidden": "true", "focusable": "false",
            dangerous_inner_html: drawing(name)
        }
    }
}
