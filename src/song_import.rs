//! Editable metadata supplied alongside a local recording.
use crate::tempo_analysis;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SongInfo {
    pub album: String,
    pub year: String,
    pub genre: String,
    pub artwork: Option<PathBuf>,
    pub artwork_source: String,
    pub tempo: tempo_analysis::Estimate,
    pub notes: Vec<String>,
    pub lyrics: Option<Value>,
    pub lyrics_checked: bool,
}
