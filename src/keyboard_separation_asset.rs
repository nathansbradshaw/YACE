//! Pinned catalogue entry shared by clients.
use crate::models;
pub const MODEL_ID: &str = "keyboard-separation-bs-roformer-sw";
pub const MODEL_FILENAME: &str = "bs_roformer_sw.onnx";
pub const MODEL_REVISION: &str = "440487b8300dcd61453cc52ec244a38150b03456";
pub const MODEL_URL: &str = "https://huggingface.co/scragnog/HOT-Step-CPP-SuperSep/resolve/440487b8300dcd61453cc52ec244a38150b03456/bs_roformer_sw.onnx";
pub const MODEL_SHA256: &str = "d2a13ad34eb26a8aa3135314b2cf1efb41e70289ffb9d03124a4201736a56df3";
pub const MODEL_BYTES: u64 = 704_536_949;
pub const WEIGHT_LICENSE_NOTICE: &str = "Original BS-RoFormer SW weight license unresolved (checkpoint mirror declares unknown). MIT covers conversion/packaging only and does not relicense weights. Not included with source/default model set; optional local model under its own unresolved terms. See BS-RoFormer-SW-NOTICE.md.";
pub fn asset() -> models::ModelAsset {
    models::ModelAsset {
        id: MODEL_ID.into(),
        filename: MODEL_FILENAME.into(),
        url: MODEL_URL.into(),
        sha256: MODEL_SHA256.into(),
        license: WEIGHT_LICENSE_NOTICE.into(),
    }
}
