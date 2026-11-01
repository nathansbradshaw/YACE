//! Source-bound saved kit receipts are usable without model inference.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub const ALGORITHM: &str = "larsnet-native-masks-v1";
pub const WEIGHT_LICENSE: &str =
    "CC-BY-NC-4.0; optional local LarsNet/LARS weights, not relicensed by the MIT Rust frontend";
pub const SAMPLE_RATE: u32 = 44100;
pub const RECEIPT_FILENAME: &str = "drum-parts-receipt.json";
pub const MODEL_BYTES: u64 = 39_341_756;
/// ONNX sigmoid SIMD roundoff only; never an instrument admission threshold.
pub const MASK_EPSILON: f32 = 1e-6;
pub const MODELS: [(&str, &str); 5] = [
    (
        "kick",
        "15f1dcd32d883a7c76827d374a56e960b87679fddfe9e35527811fedeb3c3351",
    ),
    (
        "snare",
        "16b8d4c7882473f3cccfdfad36583aa0cdbcb0c782a2db3f0ed4b7b870bf8555",
    ),
    (
        "toms",
        "f8f279925383d539d3fef7ab43ed024be2ca483ad0e5e855484b045e31ea0e00",
    ),
    (
        "hihat",
        "972d6bfe012436156f9fc66dc062bc774fb620fcad3f895c35f2acedd5499b32",
    ),
    (
        "cymbals",
        "69110dd7705a09935590ef9a22dfecd5e8d1af5656890b2d3dbd764015f9a727",
    ),
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputReceipt {
    pub part: String,
    pub filename: String,
    pub frames: u64,
    pub pcm_sha256: String,
    pub wav_sha256: Option<String>,
    pub rms: f64,
    pub peak: f64,
    pub mask_numeric: MaskNumeric,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaskNumeric {
    pub raw_min: f32,
    pub raw_max: f32,
    pub clipped_values: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeparationReceipt {
    pub version: u32,
    pub algorithm: String,
    pub implementation_sha256: String,
    pub source_audio_sha256: Option<String>,
    pub source_pcm_sha256: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub models: BTreeMap<String, String>,
    pub weight_license: String,
    pub outputs: Vec<OutputReceipt>,
    pub elapsed_seconds: f64,
    pub warnings: Vec<String>,
}
pub fn implementation_hash() -> String {
    format!(
        "{:x}",
        Sha256::digest(
            [
                include_bytes!("larsnet.rs").as_slice(),
                include_bytes!("larsnet_receipt.rs").as_slice()
            ]
            .concat()
        )
    )
}

/// Schema extraction left the DSP unchanged. Keep those existing local caches.
pub fn supported_implementation(hash: &str) -> bool {
    hash == implementation_hash()
        || hash == "2655740a47fc5a301df34873b7154a3b07ee5e4e3158c9a04a190b06c218f973"
}
