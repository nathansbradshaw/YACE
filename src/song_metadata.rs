//! Lossless song metadata helpers shared by native and WebAssembly.
use std::collections::BTreeMap;
pub fn audio_file(folder: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    for name in ["song", "guitar"] {
        for extension in ["ogg", "opus", "mp3", "flac", "wav", "m4a"] {
            let path = folder.join(format!("{name}.{extension}"));
            if crate::storage::is_file(&path) {
                return Ok(path);
            }
        }
    }
    anyhow::bail!("No full-song audio found in {}; supply song or guitar audio", folder.display())
}
pub fn ini_fields(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut song = false;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') {
            song = line.eq_ignore_ascii_case("[song]");
        } else if song && let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    fields
}

pub fn update_ini(original: &str, updates: &BTreeMap<String, String>) -> String {
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut result = String::new();
    let mut in_song = false;
    let mut found = false;
    for line in original.lines() {
        if line.trim().starts_with('[') {
            if in_song {
                for (k, v) in updates {
                    if seen.insert(k.clone()) {
                        result.push_str(&format!("{k} = {v}{newline}"));
                    }
                }
            }
            in_song = line.trim().eq_ignore_ascii_case("[song]");
            found |= in_song;
        }
        if in_song && let Some((key, _)) = line.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            if let Some(value) = updates.get(&key) {
                if seen.insert(key.clone()) {
                    result.push_str(&format!("{key} = {value}{newline}"));
                }
                continue;
            }
        }
        result.push_str(line);
        result.push_str(newline);
    }
    if !found {
        result.push_str(&format!("[song]{newline}"));
    }
    for (k, v) in updates {
        if seen.insert(k.clone()) {
            result.push_str(&format!("{k} = {v}{newline}"));
        }
    }
    result
}

use crate::storage as fs;
use anyhow::{Context, Result};
use std::path::Path;
pub fn song_ini(folder: &Path) -> Result<String> {
    match fs::read_to_string(folder.join("song.ini")) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).context(
            "Cannot read song.ini; preserving source rather than replacing unreadable metadata",
        ),
    }
}
