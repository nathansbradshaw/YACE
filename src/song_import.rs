//! Recording metadata and catalogue matching; native operations stay platform-specific.
#[cfg(not(target_arch = "wasm32"))]
use crate::pipeline;
use crate::{recording_metadata::Metadata, tempo_analysis};
#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result, ensure};
#[cfg(not(target_arch = "wasm32"))]
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::{fs, io::Read, path::Path, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SongInfo {
    pub album: String,
    pub year: String,
    pub genre: String,
    pub artwork: Option<PathBuf>,
    pub artwork_source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artwork_alternatives: Vec<String>,
    pub tempo: tempo_analysis::Estimate,
    pub notes: Vec<String>,
    pub lyrics: Option<Value>,
    pub lyrics_checked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingInfo {
    pub metadata: Metadata,
    pub info: SongInfo,
}

/// Reads tags and audio locally. Online lookup is a separate, explicit operation.
#[cfg(not(target_arch = "wasm32"))]
pub fn local(path: &Path) -> Result<RecordingInfo> {
    let mut metadata = pipeline::metadata(path)?;
    let filename = path.file_stem().unwrap_or_default().to_string_lossy();
    if metadata.name.trim().is_empty() {
        if let Some((artist, title)) = filename.split_once(" - ") {
            metadata.name = title.trim().into();
            if metadata.artist.trim().is_empty() {
                metadata.artist = artist.trim().into();
            }
        } else {
            metadata.name = filename.into_owned();
        }
    }
    let mut info = SongInfo {
        album: metadata.album.clone(),
        year: metadata.year.clone(),
        genre: metadata.genre.clone(),
        tempo: tempo_analysis::analyze(path)?,
        ..Default::default()
    };
    let cache = artwork_cache(path)?;
    // Only attached pictures qualify; an MP4's video frames are not album art.
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=index:stream_disposition=attached_pic",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;
    if probe.status.success() {
        let body: Value = serde_json::from_slice(&probe.stdout)?;
        if let Some(index) = body["streams"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["disposition"]["attached_pic"] == 1)
            .and_then(|s| s["index"].as_u64())
        {
            let cover = cache.join("embedded.png");
            let output = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(path)
                .args([
                    "-map",
                    &format!("0:{index}"),
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=1024:1024:force_original_aspect_ratio=decrease",
                    "-y",
                ])
                .arg(&cover)
                .output()?;
            if output.status.success() && fs::metadata(&cover)?.len() <= 5_000_000 {
                info.artwork = Some(cover);
                info.artwork_source = "Embedded recording artwork".into();
            }
        }
    }
    Ok(RecordingInfo { metadata, info })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn artwork_cache(path: &Path) -> Result<PathBuf> {
    let parent = std::env::temp_dir().join("YACE");
    let cache = parent.join("imports").join(crate::models::hash_file(path)?);
    fs::create_dir_all(&cache)?;
    Ok(cache)
}

/// Create a blank, editable standard song folder without downloading models or
/// inventing notes. Generation can be started later from the imported recording.
#[cfg(not(target_arch = "wasm32"))]
pub fn create_project(
    audio: &Path,
    recording: &RecordingInfo,
    bpm: Option<f64>,
    output: &Path,
) -> Result<PathBuf> {
    crate::pipeline::destination(output, Some(audio))?;
    let mut metadata = recording.metadata.clone();
    let info = &recording.info;
    metadata.album = info.album.clone();
    metadata.year = info.year.clone();
    metadata.genre = info.genre.clone();
    for field in [
        &metadata.name,
        &metadata.artist,
        &metadata.album,
        &metadata.year,
        &metadata.genre,
    ] {
        ensure!(
            !field.contains(['\n', '\r']),
            "Song metadata must use one line per field"
        );
    }
    ensure!(
        !metadata.name.trim().is_empty() && !metadata.artist.trim().is_empty(),
        "Enter the song title and artist"
    );
    let timing = if let Some(bpm) = bpm {
        crate::timing::TempoMap::constant(bpm)?
    } else if !info.tempo.segments.is_empty() {
        info.tempo.tempo_map()?
    } else {
        crate::timing::TempoMap::constant(
            info.tempo
                .bpm
                .context("No stable pulse found; supply BPM after listening")?,
        )?
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    let audio = audio.canonicalize()?;
    let before = crate::models::hash_file(&audio)?;
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&audio)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-n",
        ])
        .arg(stage.path().join("song.wav"))
        .output()?;
    ensure!(
        decoded.status.success(),
        "Could not prepare song audio: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    ensure!(
        crate::models::hash_file(&audio)? == before,
        "Recording changed during import"
    );
    let fields = std::collections::BTreeMap::from([
        ("name".into(), metadata.name.clone()),
        ("artist".into(), metadata.artist.clone()),
        ("album".into(), metadata.album.clone()),
        ("year".into(), metadata.year.clone()),
        ("genre".into(), metadata.genre.clone()),
        ("charter".into(), "YACE".into()),
    ]);
    fs::write(
        stage.path().join("song.ini"),
        pipeline::update_ini("[song]\n", &fields),
    )?;
    let tracks = std::collections::BTreeMap::from([("ExpertSingle".into(), Vec::new())]);
    fs::write(
        stage.path().join("notes.chart"),
        crate::chart::new_chart(&metadata.name, &metadata.artist, &timing, &tracks),
    )?;
    crate::song_layout::prepare(stage.path())?;
    let mut retained = info.clone();
    if let Some(art) = &info.artwork {
        let name = if art
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("png"))
        {
            "album.png"
        } else {
            "album.jpg"
        };
        fs::copy(art, stage.path().join(name)).context("Cover artwork is unavailable")?;
        retained.artwork = Some(name.into());
    }
    crate::generation::write_json(
        &crate::song_layout::path(stage.path(), "import-information.json"),
        &retained,
    )?;
    let workspace = tempfile::tempdir_in(parent)?;
    let store =
        crate::editor::Store::create(stage.path(), None, &workspace.path().join("project"))?;
    let mut project = store.load()?;
    project.title = metadata.name;
    project.original_folder = std::path::absolute(output)?;
    crate::generation::write_json(&store.root.join("project.json"), &project)?;
    // Keep the full-quality source for later generation, separate from game audio.
    let extension = audio
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    ensure!(
        extension.len() <= 10 && extension.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Unsupported audio extension"
    );
    fs::create_dir_all(stage.path().join(".yace/originals/audio"))?;
    let original = stage
        .path()
        .join(format!(".yace/originals/audio/recording.{extension}"));
    fs::copy(&audio, &original)?;
    ensure!(
        crate::models::hash_file(&original)? == before,
        "Recording changed while retaining the original"
    );
    fs::rename(&store.root, stage.path().join(".yace/project"))?;
    pipeline::publish(stage.path(), output)?;
    let project = output.canonicalize()?.join(".yace/project");
    crate::editor::Store::new(&project).load()?;
    Ok(project)
}

/// The caller supplies the edited title/artist and a measured duration. Audio is
/// never uploaded. Existing fields and chosen artwork take priority over suggestions.
#[cfg(not(target_arch = "wasm32"))]
pub fn lookup(meta: &mut Metadata, info: &mut SongInfo, cache: &Path, lyrics: bool) -> Result<()> {
    ensure!(
        !meta.name.trim().is_empty(),
        "Enter a song title before looking up song information"
    );
    ensure!(
        meta.duration.is_finite() && meta.duration > 0.0,
        "A measured recording duration is required"
    );
    let mut suggestion = meta.clone();
    let mut found = SongInfo::default();
    enrich_catalogues(&mut suggestion, &mut found);
    if meta.artist.trim().is_empty() {
        meta.artist = suggestion.artist;
    }
    if meta.album.is_empty() {
        meta.album = suggestion.album;
    }
    if meta.year.is_empty() {
        meta.year = suggestion.year;
    }
    if meta.genre.is_empty() {
        meta.genre = suggestion.genre;
    }
    info.album = meta.album.clone();
    info.year = meta.year.clone();
    info.genre = meta.genre.clone();
    info.notes.extend(found.notes.iter().cloned());
    if info.artwork.is_none() && !found.artwork_source.is_empty() {
        match catalogue_artwork(&found, cache) {
            Ok((name, source)) => {
                info.artwork = Some(cache.join(name));
                info.artwork_source = source;
            }
            Err(error) => {
                info.notes.push(format!(
                    "Cover artwork unavailable: {}. Choose an image manually or retry.",
                    error
                ));
            }
        }
    }
    if lyrics {
        match pipeline::lyrics(meta) {
            Ok(found) => {
                info.lyrics_checked = true;
                if info.lyrics.is_none() {
                    info.lyrics = found;
                }
            }
            Err(error) => info
                .notes
                .push(format!("Lyrics lookup unavailable: {error}")),
        }
    }
    Ok(())
}

pub(crate) fn normalize(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
pub(crate) fn quote(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(not(target_arch = "wasm32"))]
fn enrich(meta: &mut Metadata, info: &mut SongInfo) -> Result<()> {
    // MusicBrainz permits one request per second per application. UI work runs
    // on a blocking worker; serialize concurrent windows here too.
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(time) = *last {
        std::thread::sleep(Duration::from_secs(1).saturating_sub(time.elapsed()));
    }
    *last = Some(std::time::Instant::now());
    let client = Client::builder()
        .user_agent("YACE/0.1 (https://github.com/nathansbradshaw/YACE)")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    let identity = if meta.artist.is_empty() {
        format!("recording:{}", quote(&meta.name))
    } else {
        format!(
            "recording:{} AND artist:{}",
            quote(&meta.name),
            quote(&meta.artist)
        )
    };
    // Popular songs can have hundreds of equally scored edits/live recordings.
    // Filter duration at the server, before its result limit discards the album cut.
    let query = format!(
        "{identity} AND dur:[{} TO {}]",
        ((meta.duration - 5.0).max(0.0) * 1000.0).floor() as u64,
        ((meta.duration + 5.0) * 1000.0).ceil() as u64
    );
    let response = client
        .get("https://musicbrainz.org/ws/2/recording/")
        .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "10")])
        .send()?
        .error_for_status()?;
    let bytes = limited(response, 2_000_000)?;
    let body: Value = serde_json::from_slice(&bytes)?;
    if let Some(record) = matching_recording(&body, meta) {
        apply_recording(record, meta, info);
    } else {
        info.notes.push(
            "No unambiguous catalogue match; check the title, artist and recording version, then retry."
                .into(),
        );
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn enrich_catalogues(meta: &mut Metadata, info: &mut SongInfo) {
    if let Err(error) = enrich(meta, info) {
        info.notes.push(format!(
            "MusicBrainz lookup unavailable: {error}. Imported information is retained."
        ));
    }
    if (meta.album.is_empty() || meta.year.is_empty() || meta.genre.is_empty())
        && let Err(error) = apple_metadata(meta, info)
    {
        info.notes.push(format!(
            "Additional catalogue lookup unavailable: {error}. Missing fields can be edited."
        ));
    }
}

// Only factual catalogue fields are used. Apple promotional artwork and audio
// previews are deliberately not downloaded or included in game exports.
#[cfg(not(target_arch = "wasm32"))]
fn apple_metadata(meta: &mut Metadata, info: &mut SongInfo) -> Result<()> {
    let client = Client::builder()
        .user_agent("ChartEngine/0.1 (local desktop chart editor)")
        .timeout(Duration::from_secs(20))
        .build()?;
    let term = format!("{} {}", meta.artist, meta.name);
    let response = client
        .get("https://itunes.apple.com/search")
        .query(&[
            ("term", term.as_str()),
            ("entity", "song"),
            ("media", "music"),
            ("country", "US"),
            ("limit", "15"),
        ])
        .send()?
        .error_for_status()?;
    let body: Value = serde_json::from_slice(&limited(response, 2_000_000)?)?;
    apply_apple_body(&body, meta, info);
    Ok(())
}

pub(crate) fn apple_match<'a>(body: &'a Value, meta: &Metadata) -> Option<&'a Value> {
    body["results"]
        .as_array()?
        .iter()
        .filter(|record| {
            record["kind"] == "song"
                && normalize(record["trackName"].as_str().unwrap_or_default())
                    == normalize(&meta.name)
                && normalize(record["artistName"].as_str().unwrap_or_default())
                    == normalize(&meta.artist)
                && record["trackTimeMillis"].as_f64().is_some_and(|length| {
                    let seconds = length / 1000.0;
                    let extra = meta.duration - seconds;
                    extra.abs() <= 5.0
                            // Music-video intros/outros may extend the selected audio.
                            // Only factual release fields are borrowed, never its audio/timing.
                            || ((0.0..=30.0).contains(&extra) && seconds >= meta.duration * 0.85)
                })
        })
        .min_by_key(|record| {
            (
                !meta.album.is_empty()
                    && normalize(record["collectionName"].as_str().unwrap_or_default())
                        != normalize(&meta.album),
                record["releaseDate"].as_str().unwrap_or("9999"),
            )
        })
}

pub(crate) fn artist_credit(record: &Value) -> String {
    record["artist-credit"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|credit| {
            format!(
                "{}{}",
                credit["name"]
                    .as_str()
                    .or_else(|| credit["artist"]["name"].as_str())
                    .unwrap_or_default(),
                credit["joinphrase"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

pub(crate) fn matching_recording<'a>(body: &'a Value, meta: &Metadata) -> Option<&'a Value> {
    let mut matches = body["recordings"].as_array()?.iter().filter(|record| {
        normalize(record["title"].as_str().unwrap_or_default()) == normalize(&meta.name)
            && (meta.artist.is_empty()
                || normalize(&artist_credit(record)) == normalize(&meta.artist))
            && record["length"]
                .as_f64()
                .is_some_and(|length| (length / 1000.0 - meta.duration).abs() <= 5.0)
            && record["score"]
                .as_u64()
                .or_else(|| record["score"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or_default()
                >= 95
    });
    let first = matches.next()?;
    // Multiple recording IDs with different credits are not silently conflated.
    let remaining = matches.collect::<Vec<_>>();
    if remaining
        .iter()
        .any(|record| normalize(&artist_credit(record)) != normalize(&artist_credit(first)))
    {
        return None;
    }
    std::iter::once(first).chain(remaining).min_by(|a, b| {
        let album = |record: &Value| {
            record["releases"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| {
                    normalize(r["title"].as_str().unwrap_or_default()) == normalize(&meta.album)
                })
        };
        (!meta.album.is_empty() && !album(a))
            .cmp(&(!meta.album.is_empty() && !album(b)))
            .then_with(|| {
                (a["length"].as_f64().unwrap_or_default() - meta.duration * 1000.0)
                    .abs()
                    .total_cmp(
                        &(b["length"].as_f64().unwrap_or_default() - meta.duration * 1000.0).abs(),
                    )
            })
    })
}

pub(crate) fn apply_recording(record: &Value, meta: &mut Metadata, info: &mut SongInfo) {
    meta.name = record["title"].as_str().unwrap_or(&meta.name).into();
    let artist = artist_credit(record);
    if !artist.is_empty() {
        meta.artist = artist;
    }
    meta.year = record["first-release-date"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(4)
        .collect();
    if let Some(release) = record["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["status"].as_str().is_none_or(|s| s == "Official"))
        .min_by_key(|release| {
            (
                !meta.album.is_empty()
                    && normalize(release["title"].as_str().unwrap_or_default())
                        != normalize(&meta.album),
                release["release-group"]["primary-type"].as_str() != Some("Album"),
                release["date"].as_str().unwrap_or("9999"),
            )
        })
    {
        meta.album = release["title"].as_str().unwrap_or_default().into();
        if meta.year.is_empty() {
            meta.year = release["date"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(4)
                .collect();
        }
        if let Some(id) = release["id"]
            .as_str()
            .filter(|id| id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        {
            info.artwork_source = format!("https://coverartarchive.org/release/{id}/front-500");
        }
    }
    info.artwork_alternatives = record["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| normalize(r["title"].as_str().unwrap_or_default()) == normalize(&meta.album))
        .filter_map(|r| r["id"].as_str())
        .filter(|id| valid_id(id))
        .map(|id| format!("https://coverartarchive.org/release/{id}/front-500"))
        .filter(|url| *url != info.artwork_source)
        .take(3)
        .collect();
    meta.genre = record["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .max_by_key(|tag| tag["count"].as_i64().unwrap_or_default())
        .and_then(|tag| tag["name"].as_str())
        .unwrap_or_default()
        .into();
    info.notes.push(format!("Title, artist and duration match MusicBrainz recording {}. Album is a release suggestion; editions may differ.",record["id"].as_str().unwrap_or_default()));
}

#[cfg(not(target_arch = "wasm32"))]
fn limited(response: reqwest::blocking::Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "Metadata response is too large");
    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn artwork(url: &str, root: &Path) -> Result<PathBuf> {
    let allowed = |url: &reqwest::Url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.host_str().is_some_and(|host| {
                matches!(host, "coverartarchive.org" | "archive.org" | "i.ytimg.com")
                    || host.ends_with(".archive.org")
                    || host.ends_with(".ytimg.com")
            })
    };
    ensure!(
        allowed(&reqwest::Url::parse(url)?),
        "Artwork source is unsupported"
    );
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 || !allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let response = match client.get(url).send().and_then(|r| r.error_for_status()) {
        Ok(response) => response,
        Err(original) => {
            let parsed = reqwest::Url::parse(url)?;
            let parts = parsed
                .path_segments()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if parsed.host_str() != Some("coverartarchive.org")
                || parts.len() != 3
                || parts[0] != "release"
                || !valid_id(parts[1])
            {
                return Err(original.into());
            }
            // The same CAA front-image index is hosted at Internet Archive.
            // This also works when the CAA redirect service is unavailable.
            let index = client
                .get(format!(
                    "https://archive.org/download/mbid-{}/index.json",
                    parts[1]
                ))
                .send()?
                .error_for_status()?;
            let body: Value = serde_json::from_slice(&limited(index, 2_000_000)?)?;
            let image =
                archive_front(&body).context("No front cover is available for this release")?;
            let image = reqwest::Url::parse(&image)?;
            ensure!(allowed(&image), "Unsupported archive artwork source");
            client.get(image).send()?.error_for_status()?
        }
    };
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        .unwrap_or_default();
    let name = if mime.starts_with("image/png") {
        PathBuf::from("album.png")
    } else {
        ensure!(mime.starts_with("image/jpeg"), "Unsupported artwork image");
        PathBuf::from("album.jpg")
    };
    let bytes = limited(response, 5_000_000)?;
    ensure!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "Artwork response is not a PNG or JPEG image"
    );
    fs::write(root.join(&name), bytes)?;
    Ok(name)
}

/// Try the selected release and up to three editions of the same named album.
/// Returns a relative image filename and the successful release's source URL.
#[cfg(not(target_arch = "wasm32"))]
pub fn catalogue_artwork(info: &SongInfo, root: &Path) -> Result<(PathBuf, String)> {
    let mut errors = Vec::new();
    for source in std::iter::once(&info.artwork_source)
        .chain(&info.artwork_alternatives)
        .filter(|url| !url.is_empty())
        .take(4)
    {
        match artwork(source, root) {
            Ok(path) => return Ok((path, source.clone())),
            Err(error) => errors.push(error.to_string()),
        }
    }
    anyhow::bail!(
        "{}",
        if errors.is_empty() {
            "No catalogue cover found".into()
        } else {
            errors.join("; ")
        }
    )
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

pub(crate) fn archive_front(body: &Value) -> Option<String> {
    let image = body["images"]
        .as_array()?
        .iter()
        .find(|image| image["front"] == true && image["approved"] != false)?;
    let url = image["thumbnails"]["500"]
        .as_str()
        .or_else(|| image["thumbnails"]["large"].as_str())
        .or_else(|| image["image"].as_str())?;
    Some(
        url.strip_prefix("http://")
            .map_or_else(|| url.to_owned(), |rest| format!("https://{rest}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_and_duration_can_identify_an_artist_but_conflicting_credits_are_rejected() {
        let meta = Metadata {
            name: "Song".into(),
            duration: 180.0,
            ..Default::default()
        };
        let record = serde_json::json!({"title":"Song","length":180000,"score":100,"artist-credit":[{"name":"Artist"}]});
        let mut body = serde_json::json!({"recordings":[record]});
        assert_eq!(
            artist_credit(matching_recording(&body, &meta).unwrap()),
            "Artist"
        );
        body["recordings"].as_array_mut().unwrap().push(serde_json::json!({"title":"Song","length":181000,"score":100,"artist-credit":[{"name":"Another artist"}]}));
        assert!(matching_recording(&body, &meta).is_none());
    }
    #[test]
    fn archive_index_uses_a_front_cover_and_upgrades_its_transport() {
        let body = serde_json::json!({"images":[
            {"front":false,"approved":true,"image":"https://archive.org/back.jpg"},
            {"front":true,"approved":false,"image":"https://archive.org/rejected.jpg"},
            {"front":true,"approved":true,"thumbnails":{"500":"http://archive.org/download/front.jpg"}}
        ]});
        assert_eq!(
            archive_front(&body).unwrap(),
            "https://archive.org/download/front.jpg"
        );
        assert!(archive_front(&serde_json::json!({"images":[{"front":false}]})).is_none());
    }
    #[test]
    fn catalogue_matching_preserves_recording_version_and_duration() {
        let mut meta = Metadata {
            name: "My Song (Live)".into(),
            artist: "Artist".into(),
            duration: 180.0,
            ..Default::default()
        };
        let body = serde_json::json!({"recordings":[{"id":"fixture","title":"My Song (Live)","score":100,"length":180000,"artist-credit":[{"name":"Artist"}],"first-release-date":"2001-02-03","releases":[{"title":"Live Album","status":"Official","id":"12345678-1234-1234-1234-123456789abc"}]}]});
        let record = matching_recording(&body, &meta).unwrap();
        let mut info = SongInfo::default();
        apply_recording(record, &mut meta, &mut info);
        assert_eq!(meta.album, "Live Album");
        assert_eq!(meta.year, "2001");
        meta.name = "My Song".into();
        assert!(matching_recording(&body, &meta).is_none());
        meta.name = "My Song (Live)".into();
        meta.duration = 200.0;
        assert!(matching_recording(&body, &meta).is_none());
    }

    #[test]
    fn fallback_uses_exact_recording_identity_and_prefers_the_known_album() {
        let meta = Metadata {
            name: "Song".into(),
            artist: "Artist".into(),
            album: "Original album".into(),
            duration: 200.0,
            ..Default::default()
        };
        let body = serde_json::json!({"results":[
            {"kind":"song","trackName":"Song (Remix)","artistName":"Artist","trackTimeMillis":200000},
            {"kind":"song","trackName":"Song","artistName":"Someone Else","trackTimeMillis":200000},
            {"kind":"song","trackName":"Song","artistName":"Artist","trackTimeMillis":200000,"collectionName":"Compilation","releaseDate":"1990"},
            {"kind":"song","trackName":"Song","artistName":"Artist","trackTimeMillis":201000,"collectionName":"Original album","releaseDate":"2000","primaryGenreName":"Pop"}
        ]});
        assert_eq!(
            apple_match(&body, &meta).unwrap()["collectionName"],
            "Original album"
        );
        let mut different = meta;
        different.duration = 214.0;
        assert!(apple_match(&body, &different).is_some());
        different.duration = 300.0;
        assert!(apple_match(&body, &different).is_none());
    }
}

/// Apply only matched factual release fields; never promotional audio/artwork.
pub(crate) fn apply_apple_body(body: &Value, meta: &mut Metadata, info: &mut SongInfo) {
    if let Some(record) = apple_match(body, meta) {
        if meta.album.is_empty() {
            meta.album = record["collectionName"].as_str().unwrap_or_default().into();
        }
        if meta.year.is_empty() {
            meta.year = record["releaseDate"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(4)
                .collect();
        }
        if meta.genre.is_empty() {
            meta.genre = record["primaryGenreName"]
                .as_str()
                .unwrap_or_default()
                .into();
        }
        info.notes.push(format!("Additional release metadata matched by title, artist and duration: {}. Release editions can differ.",record["trackViewUrl"].as_str().unwrap_or("Apple catalogue")));
        if record["trackTimeMillis"]
            .as_f64()
            .is_some_and(|length| (length / 1000.0 - meta.duration).abs() > 5.0)
        {
            info.notes.push("The selected video is longer than the catalogue track. Release fields are suggestions; the video's original audio and its measured timing are retained.".into());
        }
    }
}
