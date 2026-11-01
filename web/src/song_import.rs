//! Local browser preparation and optional direct catalogue requests.
pub use crate::import_metadata::{RecordingInfo, SongInfo};
use crate::{import_metadata as common, recording_metadata::Metadata};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

pub fn artwork_kind(bytes: &[u8]) -> Result<&'static str> {
    crate::song_metadata::cover_extension(bytes)
}
pub fn local(audio: &Path, output: &Path) -> Result<RecordingInfo> {
    let bytes = crate::storage::shared(audio)?;
    let mut notes = Vec::new();
    let tags = match crate::recording_tags::read(&bytes) {
        Ok(tags) => tags,
        Err(_) => {
            notes.push(
                "No embedded recording metadata was available; enter the song fields.".into(),
            );
            Default::default()
        }
    };
    let mut metadata = tags.metadata;
    metadata.duration = crate::models::duration(audio)?;
    let mut info = SongInfo {
        album: metadata.album.clone(),
        year: metadata.year.clone(),
        genre: metadata.genre.clone(),
        tempo: crate::tempo_analysis::analyze(audio)?,
        notes,
        ..Default::default()
    };
    if let Some(bytes) = tags.artwork {
        match artwork_kind(&bytes) {
            Ok(extension) => {
                let path = output.join(format!("embedded.{extension}"));
                crate::storage::write_owned(&path, bytes)?;
                info.artwork = Some(path);
                info.artwork_source = "Embedded recording artwork".into();
            }
            Err(error) => info.notes.push(format!(
                "Embedded cover unavailable: {error}; choose a cover image manually."
            )),
        }
    }
    Ok(RecordingInfo { metadata, info })
}
async fn artwork(url: &str, root: &Path) -> Result<PathBuf> {
    let response = crate::http::get(url, 5_000_000).await?;
    ensure!(
        (200..300).contains(&response.status),
        "Artwork HTTP {}",
        response.status
    );
    ensure!(
        response.mime.starts_with("image/png") || response.mime.starts_with("image/jpeg"),
        "Artwork response is not a PNG or JPEG"
    );
    let extension = artwork_kind(&response.bytes)?;
    let path = root.join(format!("catalogue.{extension}"));
    crate::storage::write_owned(&path, response.bytes)?;
    Ok(path)
}
async fn catalogue_artwork(info: &SongInfo, root: &Path) -> Result<(PathBuf, String)> {
    let mut error = String::from("No catalogue cover found");
    for source in std::iter::once(&info.artwork_source)
        .chain(&info.artwork_alternatives)
        .filter(|s| !s.is_empty())
        .take(4)
    {
        match artwork(source, root).await {
            Ok(path) => return Ok((path, source.clone())),
            Err(failure) => error = failure.to_string(),
        }
        if let Some(release) = source
            .strip_prefix("https://coverartarchive.org/release/")
            .and_then(|s| s.split('/').next())
            .filter(|id| common::valid_id(id))
        {
            let index = crate::http::json(&format!(
                "https://archive.org/download/mbid-{release}/index.json"
            ))
            .await;
            if let Ok(index) = index
                && let Some(image) = common::archive_front(&index)
            {
                match artwork(&image, root).await {
                    Ok(path) => return Ok((path, image)),
                    Err(failure) => error = failure.to_string(),
                }
            }
        }
    }
    anyhow::bail!("{error}")
}
thread_local! {static NEXT_REQUEST:std::cell::Cell<f64>=const{std::cell::Cell::new(0.)};}
pub async fn lookup(
    mut metadata: Metadata,
    mut info: SongInfo,
    root: &Path,
    lyrics: bool,
) -> Result<RecordingInfo> {
    ensure!(
        !metadata.name.trim().is_empty() && metadata.duration.is_finite() && metadata.duration > 0.,
        "Enter a title and use a measured recording duration"
    );
    let mut suggestion = metadata.clone();
    let mut found = SongInfo::default();
    let wait = NEXT_REQUEST.with(|next| {
        let now = js_sys::Date::now();
        let at = next.get().max(now);
        next.set(at + 1000.);
        (at - now) as u32
    });
    if wait > 0 {
        gloo_timers::future::TimeoutFuture::new(wait).await;
    }
    let identity = if metadata.artist.is_empty() {
        format!("recording:{}", common::quote(&metadata.name))
    } else {
        format!(
            "recording:{} AND artist:{}",
            common::quote(&metadata.name),
            common::quote(&metadata.artist)
        )
    };
    let query = format!(
        "{identity} AND dur:[{} TO {}]",
        ((metadata.duration - 5.).max(0.) * 1000.).floor() as u64,
        ((metadata.duration + 5.) * 1000.).ceil() as u64
    );
    let url = crate::http::query(
        "https://musicbrainz.org/ws/2/recording/",
        &[
            ("query", query),
            ("fmt", "json".into()),
            ("limit", "10".into()),
        ],
    );
    match crate::http::json(&url).await {
        Ok(body) => {
            if let Some(record) = common::matching_recording(&body, &suggestion) {
                common::apply_recording(record, &mut suggestion, &mut found);
            } else {
                found.notes.push(
                    "No unambiguous catalogue match; check title, artist and recording version."
                        .into(),
                );
            }
        }
        Err(error) => found
            .notes
            .push(format!("MusicBrainz lookup unavailable: {error}")),
    }
    if (suggestion.album.is_empty() || suggestion.year.is_empty() || suggestion.genre.is_empty())
        && !suggestion.artist.is_empty()
    {
        let url = crate::http::query(
            "https://itunes.apple.com/search",
            &[
                ("term", format!("{} {}", suggestion.artist, suggestion.name)),
                ("entity", "song".into()),
                ("media", "music".into()),
                ("country", "US".into()),
                ("limit", "15".into()),
            ],
        );
        match crate::http::json(&url).await {
            Ok(body) => common::apply_apple_body(&body, &mut suggestion, &mut found),
            Err(error) => found
                .notes
                .push(format!("Additional catalogue lookup unavailable: {error}")),
        }
    }
    if metadata.artist.is_empty() {
        metadata.artist = suggestion.artist;
    }
    if metadata.album.is_empty() {
        metadata.album = suggestion.album;
    }
    if metadata.year.is_empty() {
        metadata.year = suggestion.year;
    }
    if metadata.genre.is_empty() {
        metadata.genre = suggestion.genre;
    }
    info.album = metadata.album.clone();
    info.year = metadata.year.clone();
    info.genre = metadata.genre.clone();
    info.notes.extend(found.notes.iter().cloned());
    if info.artwork.is_none() && !found.artwork_source.is_empty() {
        match catalogue_artwork(&found, root).await {
            Ok((path, source)) => {
                info.artwork = Some(path);
                info.artwork_source = source;
            }
            Err(error) => info.notes.push(format!(
                "Cover artwork unavailable: {error}. Choose an image manually or retry."
            )),
        }
    }
    if lyrics {
        let url = crate::http::query(
            "https://lrclib.net/api/get",
            &[
                ("track_name", metadata.name.clone()),
                ("artist_name", metadata.artist.clone()),
                ("album_name", metadata.album.clone()),
                ("duration", metadata.duration.round().to_string()),
            ],
        );
        match crate::http::get(&url, 2_000_000).await {
            Ok(response) if response.status == 404 => info.lyrics_checked = true,
            Ok(response) if (200..300).contains(&response.status) => {
                let record: serde_json::Value = serde_json::from_slice(&response.bytes)?;
                let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
                if record["artistName"]
                    .as_str()
                    .is_some_and(|s| same(s, &metadata.artist))
                    && record["trackName"]
                        .as_str()
                        .is_some_and(|s| same(s, &metadata.name))
                    && record["duration"]
                        .as_f64()
                        .is_some_and(|d| (d - metadata.duration).abs() <= 2.)
                {
                    info.lyrics = Some(record);
                }
                info.lyrics_checked = true;
            }
            Ok(response) => info.notes.push(format!(
                "Lyrics lookup unavailable: HTTP {}",
                response.status
            )),
            Err(error) => info
                .notes
                .push(format!("Lyrics lookup unavailable: {error}")),
        }
    }
    Ok(RecordingInfo { metadata, info })
}
