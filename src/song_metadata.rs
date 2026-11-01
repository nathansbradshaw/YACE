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
    anyhow::bail!(
        "No full-song audio found in {}; supply song or guitar audio",
        folder.display()
    )
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
/// Both clients start a manual project with the same validated source chart.
pub fn blank_song(title: &str, artist: &str, bpm: f64) -> Result<(String, String)> {
    blank_song_with_timing(title, artist, &crate::timing::TempoMap::constant(bpm)?)
}
pub fn blank_song_with_timing(
    title: &str,
    artist: &str,
    timing: &crate::timing::TempoMap,
) -> Result<(String, String)> {
    anyhow::ensure!(
        !title.trim().is_empty() && !artist.trim().is_empty(),
        "Enter a title and artist"
    );
    anyhow::ensure!(
        !title.chars().chain(artist.chars()).any(char::is_control),
        "Title and artist must be single lines"
    );
    anyhow::ensure!(
        timing
            .tempos
            .iter()
            .all(|tempo| tempo.bpm.is_finite() && (1. ..=1000.).contains(&tempo.bpm)),
        "Tempo must be 1–1000 BPM"
    );
    let metadata = BTreeMap::from([
        ("name".into(), title.trim().into()),
        ("artist".into(), artist.trim().into()),
    ]);
    let ini = update_ini("[song]\n", &metadata);
    anyhow::ensure!(
        *timing
            == crate::timing::TempoMap::new(
                timing.resolution,
                timing.offset,
                timing.tempos.clone()
            )?,
        "Invalid tempo map"
    );
    let tempos = timing
        .tempos
        .iter()
        .map(|tempo| {
            format!(
                "  {} = B {}\n",
                tempo.tick,
                (tempo.bpm * 1000.).round() as u32
            )
        })
        .collect::<String>();
    Ok((
        ini,
        format!(
            "[Song]\n{{\n  Resolution = {}\n  Offset = {}\n}}\n[SyncTrack]\n{{\n{tempos}  0 = TS 4\n}}\n[Events]\n{{\n}}\n[ExpertSingle]\n{{\n}}\n",
            timing.resolution, timing.offset
        ),
    ))
}
pub fn song_ini(folder: &Path) -> Result<String> {
    match fs::read_to_string(folder.join("song.ini")) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).context(
            "Cannot read song.ini; preserving source rather than replacing unreadable metadata",
        ),
    }
}

pub fn cover_extension(bytes: &[u8]) -> Result<&'static str> {
    anyhow::ensure!(bytes.len() <= 5_000_000, "Choose artwork smaller than 5 MB");
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok("jpg")
    } else {
        anyhow::bail!("Choose a PNG or JPEG cover image")
    }
}

/// Persist editable metadata, cover and catalogue provenance on both clients.
pub fn write_song_info(song: &Path, info: &crate::song_import::SongInfo) -> Result<()> {
    let fields = BTreeMap::from([
        ("album".into(), info.album.clone()),
        ("year".into(), info.year.clone()),
        ("genre".into(), info.genre.clone()),
    ]);
    anyhow::ensure!(
        fields
            .values()
            .all(|value| !value.chars().any(char::is_control)),
        "Song information must use single-line fields"
    );
    fs::write(song.join("song.ini"), update_ini(&song_ini(song)?, &fields))?;
    crate::song_layout::prepare(song)?;
    let mut retained = info.clone();
    if let Some(artwork) = &info.artwork {
        let bytes = fs::read(artwork).context("Cover artwork is unavailable")?;
        let name = format!("album.{}", cover_extension(&bytes)?);
        fs::write(song.join(&name), bytes)?;
        retained.artwork = Some(name.into());
    }
    fs::write(
        crate::song_layout::path(song, "import-information.json"),
        serde_json::to_vec_pretty(&retained)?,
    )?;
    Ok(())
}
