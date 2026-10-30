//! Embed the surface photos so the desktop theme also works offline.
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::OnceLock;

pub fn stylesheet(base: &'static str) -> &'static str {
    static STYLE: OnceLock<String> = OnceLock::new();
    STYLE.get_or_init(|| {
        let paint = STANDARD.encode(include_bytes!(
            "../../assets/surfaces/blue_metal_plate_diff_1k.jpg"
        ));
        let grime = STANDARD.encode(include_bytes!(
            "../../assets/surfaces/green_metal_rust_diff_1k.jpg"
        ));
        let control_edges = STANDARD.encode(include_bytes!(
            "../../assets/surfaces/control-edges.svg"
        ));
        format!(
            "{base}\n:root{{--surface-paint:url(\"data:image/jpeg;base64,{paint}\");--surface-grime:url(\"data:image/jpeg;base64,{grime}\");--control-edges:url(\"data:image/svg+xml;base64,{control_edges}\");}}\n{}",
            include_str!("../../assets/surfaces.css")
        )
    })
}
