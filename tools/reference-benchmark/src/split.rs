//! Metadata-only blind batch selection; no audio/model/reference note values read.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Select a new full-band batch using metadata only. Previously exposed artists
/// are excluded explicitly, never by looking at their model scores.
pub fn fresh(
    inventory: &Value,
    inventory_hash: &str,
    history: &Value,
    history_hash: &str,
) -> Result<Value> {
    let excluded: BTreeSet<String> = history["excluded_artists"]
        .as_array()
        .context("History requires excluded_artists")?
        .iter()
        .map(|v| {
            v.as_str()
                .context("Artist must be text")
                .map(|s| s.trim().to_lowercase())
        })
        .collect::<Result<_>>()?;
    ensure!(
        !excluded.is_empty() && !excluded.contains(""),
        "Explicit nonempty exposure history required"
    );
    let mut eligible = inventory["songs"]
        .as_array()
        .context("missing inventory songs")?
        .iter()
        .filter(|song| song["split"] == "held_out")
        .filter(|song| {
            !excluded.contains(
                &song["metadata"]["artist"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase(),
            )
        })
        .filter(|song| song["metadata"]["pro_drums"] == "1")
        .filter(|song| {
            [
                "bass.opus",
                "guitar.opus",
                "drums.opus",
                "vocals.opus",
                "notes.mid",
            ]
            .iter()
            .all(|name| {
                song["files"]
                    .as_array()
                    .is_some_and(|files| files.iter().any(|f| f["name"] == *name))
            })
        })
        .filter(|song| {
            ["PART BASS", "PART GUITAR", "PART DRUMS", "PART VOCALS"]
                .iter()
                .all(|name| {
                    song["midi_tracks"]
                        .as_array()
                        .is_some_and(|tracks| tracks.iter().any(|t| t == *name))
                })
        })
        .collect::<Vec<_>>();
    for song in &eligible {
        ensure!(
            song["archive_sha256"]
                .as_str()
                .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid archive identity"
        );
    }
    eligible.sort_by_key(|song| song["archive_sha256"].as_str().unwrap());
    let (mut artists, mut genres) = (BTreeSet::new(), BTreeSet::new());
    let mut selected = Vec::new();
    for song in eligible {
        let artist = song["metadata"]["artist"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let genre = song["metadata"]["genre"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if artist.is_empty()
            || genre.is_empty()
            || artists.contains(&artist)
            || genres.contains(&genre)
        {
            continue;
        }
        artists.insert(artist);
        genres.insert(genre);
        selected.push(song.clone());
        if selected.len() == 6 {
            break;
        }
    }
    ensure!(
        selected.len() == 6,
        "Not enough fresh full-band artists/genres; do not silently relax eligibility"
    );
    Ok(
        json!({"version":2,"frozen_inventory_sha256":inventory_hash,"exposure_history_sha256":history_hash,
        "scope":"Expert Guitar, Expert Bass, Expert Drums, Expert Pro Drums, Expert lead Vocals only. No Easy/Medium/Hard targets or tuning.",
        "rule":"Metadata only: exclude all history artists and development entries; require direct four instrument stems, four MIDI track labels and pro_drums=1; sort archive SHA256, take first six distinct artists and genres. Retain unsupported/failing cases without substitution.",
        "conditions":{"oracle":"Provided four source-clock instrument stems; plain reference lyric text only, no authored note/word/phrase times, pitches, lanes or tempo enter generation; fixed120 BPM.",
            "full_mix":"First two selected cases, normal native separation from source.wav; same fixed120 BPM and supplied-text condition, separately reported.",
            "model_free":"Report supported DSP route and explicit missing-word-timing/model abstentions separately; never count them as completed ML-free full-band generation."},
        "blinding":"Freeze candidate source before evaluating this batch. Algorithm authors receive only development targets until policy locked. Any later tuning requires another fresh batch.","songs":selected}),
    )
}
pub fn heldout(inventory: &Value, hash: &str) -> Result<Value> {
    let mut songs = inventory["songs"]
        .as_array()
        .context("missing inventory songs")?
        .iter()
        .filter(|s| s["split"] == "held_out")
        .filter(|s| {
            ["bass.opus", "vocals.opus", "notes.mid"]
                .iter()
                .all(|name| {
                    s["files"]
                        .as_array()
                        .is_some_and(|files| files.iter().any(|f| f["name"] == *name))
                })
        })
        .filter(|s| {
            ["PART BASS", "PART VOCALS"].iter().all(|name| {
                s["midi_tracks"]
                    .as_array()
                    .is_some_and(|tracks| tracks.iter().any(|t| t == *name))
            })
        })
        .collect::<Vec<_>>();
    songs.sort_by_key(|s| s["archive_sha256"].as_str().unwrap_or(""));
    let (mut artists, mut genres) = (BTreeSet::new(), BTreeSet::new());
    let mut selected = Vec::new();
    for song in songs {
        let artist = song["metadata"]["artist"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let genre = song["metadata"]["genre"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        ensure!(
            song["archive_sha256"]
                .as_str()
                .is_some_and(|h| h.len() == 64),
            "missing archive hash"
        );
        if artist.is_empty()
            || genre.is_empty()
            || artists.contains(&artist)
            || genres.contains(&genre)
        {
            continue;
        }
        artists.insert(artist);
        genres.insert(genre);
        selected.push(song.clone());
        if selected.len() == 6 {
            break;
        }
    }
    ensure!(
        selected.len() == 6,
        "not enough distinct artist/genre eligible held-out songs"
    );
    Ok(
        json!({"version":1,"frozen_inventory_sha256":hash,"rule":"Before candidate held-out inference: require held_out artist partition, bundled notes.mid + direct bass.opus + direct vocals.opus, PART BASS + PART VOCALS labels; sort ascending archive SHA256; select first six distinct normalized artists and genres. No note targets/audio/model scores read. Keep every selected outcome, including unsupported/talkie-only cases, and never replace based on scores.","blinding":"Algorithm authors receive development targets only until policy locked. This manifest and outcomes belong to independent evaluator.","songs":selected}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fresh_fixture() -> Value {
        json!({"songs":(0..10).rev().map(|i|json!({
            "split":if i==0{"development"}else{"held_out"},"archive_sha256":format!("{i:064x}"),
            "metadata":{"artist":format!("Artist{i}"),"genre":format!("Genre{i}"),"pro_drums":"1"},
            "files":(["notes.mid","bass.opus","guitar.opus","drums.opus","vocals.opus"].map(|name|json!({"name":name}))),
            "midi_tracks":["PART BASS","PART GUITAR","PART DRUMS","PART VOCALS"]
        })).collect::<Vec<_>>()})
    }
    #[test]
    fn fresh_selection_excludes_exposed_artists_and_retains_expert_pro_scope() {
        let mut inventory = fresh_fixture();
        let history = json!({"excluded_artists":[" ARTIST1 ","Artist2"]});
        let a = fresh(&inventory, "inventory", &history, "history").unwrap();
        inventory["songs"].as_array_mut().unwrap().reverse();
        assert_eq!(
            a,
            fresh(&inventory, "inventory", &history, "history").unwrap()
        );
        assert_eq!(a["songs"][0]["metadata"]["artist"], "Artist3");
        assert_eq!(a["songs"].as_array().unwrap().len(), 6);
        assert!(a["scope"].as_str().unwrap().contains("No Easy/Medium/Hard"));
        assert_eq!(a["exposure_history_sha256"], "history");
    }
    #[test]
    fn fresh_selection_refuses_implicit_exposure_or_insufficient_eligible_cases() {
        assert!(fresh(&fresh_fixture(), "i", &json!({}), "h").is_err());
        assert!(fresh(&fresh_fixture(), "i", &json!({"excluded_artists":[]}), "h").is_err());
        let mut inventory = fresh_fixture();
        for song in inventory["songs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .take(5)
        {
            song["metadata"]["pro_drums"] = json!("0");
        }
        assert!(
            fresh(
                &inventory,
                "i",
                &json!({"excluded_artists":["unused"]}),
                "h"
            )
            .is_err()
        );
    }
    #[test]
    fn fresh_selection_rejects_invalid_eligible_archive_identity() {
        let mut inventory = fresh_fixture();
        inventory["songs"][0]["archive_sha256"] = json!("not-a-checksum");
        assert!(
            fresh(
                &inventory,
                "i",
                &json!({"excluded_artists":["unused"]}),
                "h"
            )
            .is_err()
        );
    }
    #[test]
    fn metadata_only_selection_is_deterministic_and_excludes_development() {
        let mut songs=(0..8).rev().map(|i|json!({"split":if i==0{"development"}else{"held_out"},"archive_sha256":format!("{i:064x}"),"metadata":{"artist":format!("Artist{i}"),"genre":format!("Genre{i}")},"files":[{"name":"bass.opus"},{"name":"vocals.opus"},{"name":"notes.mid"}],"midi_tracks":["PART BASS","PART VOCALS"]})).collect::<Vec<_>>();
        let a = heldout(&json!({"songs":songs}), "frozen").unwrap();
        songs.reverse();
        let b = heldout(&json!({"songs":songs}), "frozen").unwrap();
        assert_eq!(a, b);
        assert_eq!(a["songs"].as_array().unwrap().len(), 6);
        assert_eq!(a["songs"][0]["archive_sha256"], format!("{:064x}", 1));
    }
}
