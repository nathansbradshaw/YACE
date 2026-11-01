//! Portable recording fields shared by native probes and browser codecs.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub duration: f64,
}
