//! Evaluator metadata only. Freeze before inspecting target note values.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub fn select(inventory: &Value, history: &Value, english_only: bool) -> Result<Value> {
    select_size(inventory, history, english_only, 6, 2)
}
pub fn select_size(
    inventory: &Value,
    history: &Value,
    english_only: bool,
    count: usize,
    development: usize,
) -> Result<Value> {
    ensure!(
        (1..=12).contains(&count) && development < count,
        "Invalid frozen split size"
    );
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
    ensure!(
        candidates.len() >= count,
        "Not enough fresh vocal references"
    );
    let songs: Vec<_> = candidates
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(i, mut song)| {
            song["split"] = json!(if i < development {
                "development"
            } else {
                "held_out"
            });
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
        json!({"version":1,"count":count,"development":development,"scope":"Professional YARG references; combine distinct frozen batches for the twelve-recording pitch gate. Language balance is optional; pitch category review remains required.","english_only":english_only,"selection":"direct vocal stem, nonzero vocal difficulty, unexposed distinct artists, sorted archive SHA256; no target pitch values", "songs":songs}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> (Value, Value) {
        let songs:Vec<_>=(0..8).map(|i|json!({"archive_sha256":format!("{i:064x}"),"metadata":{"artist":format!("artist{i}"),"diff_vocals":"3"},"files":[{"name":"vocals.opus"}],"vocal_language":"eng","vocal_language_reliable":true})).collect();
        (
            json!({"songs":songs}),
            json!({"excluded_artists":["artist0"]}),
        )
    }
    #[test]
    fn supplemental_frozen_holdout_keeps_unexposed_artists_and_no_development() {
        let (i, h) = inputs();
        let s = select_size(&i, &h, false, 2, 0).unwrap();
        let songs = s["songs"].as_array().unwrap();
        assert_eq!(songs.len(), 2);
        assert!(songs.iter().all(|s| s["split"] == "held_out"));
        assert_eq!(songs[0]["metadata"]["artist"], "artist1");
        assert_eq!(songs[1]["metadata"]["artist"], "artist2");
    }
    #[test]
    fn default_six_song_split_is_retained() {
        let (i, h) = inputs();
        let s = select(&i, &h, false).unwrap();
        let songs = s["songs"].as_array().unwrap();
        assert_eq!(songs.len(), 6);
        assert_eq!(
            songs.iter().filter(|s| s["split"] == "development").count(),
            2
        );
        assert_eq!(songs.iter().filter(|s| s["split"] == "held_out").count(), 4);
    }
    #[test]
    fn impossible_splits_and_insufficient_eligible_artists_fail() {
        let (mut i, h) = inputs();
        for (n, d) in [(0, 0), (13, 0), (2, 2)] {
            assert!(select_size(&i, &h, false, n, d).is_err())
        }
        for s in i["songs"].as_array_mut().unwrap() {
            s["metadata"]["artist"] = json!("artist0")
        }
        assert!(select_size(&i, &h, false, 2, 0).is_err());
    }
}
