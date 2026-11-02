//! Evaluator metadata only. Freeze before inspecting target note values.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub fn select(inventory: &Value, history: &Value, english_only: bool) -> Result<Value> {
    let excluded: BTreeSet<_> = history["excluded_artists"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Need exposure history"))?
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .collect();
    ensure!(!excluded.is_empty(), "Exposure history cannot be empty");
    let mut candidates: Vec<_> = inventory["songs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Need inventory songs"))?
        .iter()
        .filter(|song| {
            (!english_only
                || (song["vocal_language"] == "eng" && song["vocal_language_reliable"] == true))
                && song["files"]
                    .as_array()
                    .is_some_and(|files| files.iter().any(|f| f["name"] == "vocals.opus"))
                && song["metadata"]["artist"]
                    .as_str()
                    .is_some_and(|s| !excluded.contains(&s.trim().to_lowercase()))
                && song["metadata"]["diff_vocals"]
                    .as_str()
                    .is_some_and(|s| s != "-1" && s != "0")
        })
        .cloned()
        .collect();
    candidates.sort_by(|a, b| {
        a["archive_sha256"]
            .as_str()
            .cmp(&b["archive_sha256"].as_str())
    });
    let mut artists = BTreeSet::new();
    candidates.retain(|s| artists.insert(s["metadata"]["artist"].as_str().unwrap().to_lowercase()));
    ensure!(candidates.len() >= 6, "Not enough fresh vocal references");
    let songs: Vec<_> = candidates
        .into_iter()
        .take(6)
        .enumerate()
        .map(|(i, mut song)| {
            song["split"] = json!(if i < 2 { "development" } else { "held_out" });
            song["language"] = json!(match song["vocal_language"].as_str() {
                Some("eng") => "en",
                Some("spa") => "es",
                Some(other) => other,
                None => "und",
            });
            song
        })
        .collect();
    Ok(
        json!({"version":1,"scope":"Six professional YARG references; combine distinct frozen batches for the twelve-recording pitch gate. Language balance is optional; pitch category review remains required.","english_only":english_only,"selection":"direct vocal stem, nonzero vocal difficulty, unexposed distinct artists, sorted archive SHA256; no target pitch values", "songs":songs}),
    )
}
