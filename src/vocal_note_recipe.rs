//! Byte identity of the experimental Rust note-model frontend/converter.
use sha2::{Digest, Sha256};
pub fn implementation_hash() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("vocal_note_recipe.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/src/main.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/src/infer.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/src/network.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/src/onnx.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/src/checkpoint.rs"));
    h.update(include_bytes!("../examples/support/onnx_proto.rs"));
    h.update(include_bytes!("../tools/rosvot-convert/Cargo.toml"));
    h.update(include_bytes!("../tools/rosvot-convert/Cargo.lock"));
    format!("{:x}", h.finalize())
}
