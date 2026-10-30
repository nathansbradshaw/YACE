//! Loopback browser host for the shared Rust editor. No chart logic lives in JS.
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as RoutePath, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
};
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub struct BrowserContext {
    pub root: PathBuf,
    pub assets: Arc<RwLock<BTreeMap<String, PathBuf>>>,
    pub start_project: Option<PathBuf>,
}
impl BrowserContext {
    pub fn resolve(&self, path: &Path) -> Result<PathBuf> {
        let joined = if path.is_absolute() {
            path.to_owned()
        } else {
            self.root.join(path)
        };
        ensure!(
            joined.starts_with(&self.root),
            "Choose a location inside the browser workspace"
        );
        ensure!(
            !joined
                .components()
                .any(|p| matches!(p, Component::ParentDir)),
            "Parent paths are not allowed"
        );
        let mut ancestor = joined.as_path();
        while !ancestor.exists() {
            ancestor = ancestor.parent().context("Invalid location")?;
        }
        ensure!(
            ancestor.canonicalize()?.starts_with(&self.root),
            "Location leaves the browser workspace"
        );
        Ok(joined)
    }
    /// Only explicitly registered audio/download assets can be requested by URL.
    pub fn register(&self, path: &Path) -> Result<String> {
        let id = random_id()?;
        self.assets
            .write()
            .map_err(|_| anyhow::anyhow!("Asset lock poisoned"))?
            .insert(id.clone(), path.to_owned());
        Ok(format!("/asset/{id}"))
    }
    pub fn register_download(&self, path: &Path) -> Result<String> {
        Ok(self.register(path)?.replacen("/asset/", "/download/", 1))
    }
}
fn random_id() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("OS random source: {error}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
/// Store compressed music without recompressing it. Stream all files through
/// the ZIP writer, so a full song export does not require a full archive in RAM.
pub fn archive_folder(folder: &Path, output: &Path) -> Result<()> {
    let lock = if folder.join("project.json").is_file() {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(folder.join("editor.lock"))?;
        file.try_lock()
            .context("The project is saving; retry the download after it finishes")?;
        Some(file)
    } else {
        None
    };
    let result = archive_unlocked(folder, output);
    if let Some(lock) = lock {
        lock.unlock()?;
    }
    result
}
fn archive_unlocked(folder: &Path, output: &Path) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut zip = zip::ZipWriter::new(file);
    fn visit(zip: &mut zip::ZipWriter<std::fs::File>, root: &Path, folder: &Path) -> Result<()> {
        let mut entries = std::fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            ensure!(
                !entry.file_type()?.is_symlink(),
                "Symbolic links cannot be exported"
            );
            if path.is_dir() {
                visit(zip, root, &path)?;
            } else {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "json" | "chart" | "ini" | "txt" | "log"));
                let method = if text {
                    zip::CompressionMethod::Deflated
                } else {
                    zip::CompressionMethod::Stored
                };
                zip.start_file(
                    relative,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(method)
                        .compression_level(text.then_some(1)),
                )?;
                std::io::copy(&mut std::fs::File::open(path)?, zip)?;
            }
        }
        Ok(())
    }
    visit(&mut zip, folder, folder)?;
    zip.finish()?.sync_all()?;
    Ok(())
}
fn unpack_song(path: &Path, folder: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    ensure!(!archive.is_empty(), "The song archive is empty");
    ensure!(
        archive.len() <= 1024,
        "A song archive may contain at most 1024 entries"
    );
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        total = total
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(total <= 1024 * 1024 * 1024, "Expanded song exceeds 1 GiB");
        ensure!(
            !entry.is_symlink(),
            "Archive symbolic links are not supported"
        );
        let relative = entry
            .enclosed_name()
            .context("Archive path leaves the song folder")?;
        let target = folder.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(target)?;
        } else {
            std::fs::create_dir_all(target.parent().unwrap())?;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(target)?;
            std::io::copy(&mut entry, &mut file)?;
            file.sync_all()?;
        }
    }
    Ok(())
}
fn imported_root(folder: &Path) -> Result<PathBuf> {
    let mut root = folder.to_owned();
    for _ in 0..32 {
        if ["project.json", "notes.chart", "notes.mid"]
            .iter()
            .any(|name| root.join(name).is_file())
        {
            break;
        }
        let entries = std::fs::read_dir(&root)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                name != "__MACOSX" && !name.to_string_lossy().starts_with('.')
            })
            .collect::<Vec<_>>();
        if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
            break;
        }
        root = entries[0].path();
    }
    Ok(root)
}
#[derive(Clone)]
struct Host {
    context: BrowserContext,
    pool: dioxus_liveview::LiveViewPool,
    token: String,
    authority: String,
}
pub fn serve(workspace: &Path, port: u16) -> Result<()> {
    serve_inner(workspace, port, None)
}
pub fn serve_project(project: &Path, port: u16) -> Result<()> {
    let project = project.canonicalize()?;
    let root = project
        .parent()
        .context("Project has no parent")?
        .to_owned();
    serve_inner(&root, port, Some(project))
}
fn serve_inner(workspace: &Path, port: u16, start_project: Option<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(workspace)?;
    let root = workspace.canonicalize()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        let authority = format!("127.0.0.1:{}", listener.local_addr()?.port());
        let host = Host {
            context: BrowserContext {
                root,
                assets: Arc::default(),
                start_project,
            },
            pool: dioxus_liveview::LiveViewPool::new(),
            token: random_id()?,
            authority,
        };
        println!(
            "YACE browser editor: http://{}/?token={}",
            host.authority, host.token
        );
        println!("Workspace: {}", host.context.root.display());
        let app = Router::new()
            .route("/", get(index))
            .route("/ws", get(websocket))
            .route("/upload", post(upload))
            .route("/asset/{id}", get(asset))
            .route("/download/{id}", get(download_asset))
            .layer(DefaultBodyLimit::max(1024 * 1024 * 1024))
            .layer(middleware::from_fn_with_state(host.clone(), guard))
            .with_state(host);
        axum::serve(listener, app).await?;
        Ok(())
    })
}
async fn guard(State(host): State<Host>, request: Request<Body>, next: Next) -> Response {
    let headers = request.headers();
    let same_host =
        headers.get("host").and_then(|v| v.to_str().ok()) == Some(host.authority.as_str());
    let origin = format!("http://{}", host.authority);
    let same_origin = headers
        .get("origin")
        .is_none_or(|value| value == origin.as_str());
    let cookie = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let authenticated = cookie
        .split(';')
        .any(|v| v.trim() == format!("yace_session={}", host.token));
    let first = request.uri().path() == "/"
        && request.uri().query() == Some(format!("token={}", host.token).as_str());
    if !same_host || !same_origin || !(authenticated || first) {
        return (
            StatusCode::FORBIDDEN,
            "Open the browser link printed by the YACE server.",
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    if first {
        response.headers_mut().insert(
            "set-cookie",
            HeaderValue::from_str(&format!(
                "yace_session={}; HttpOnly; SameSite=Strict; Path=/",
                host.token
            ))
            .unwrap(),
        );
    }
    response
}
async fn index() -> Html<String> {
    Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>YACE</title></head><body style="margin:0"><div id="main"></div></body>{}<script>history.replaceState(null,'','/');{}</script></html>"#,
        dioxus_liveview::interpreter_glue("/ws"),
        concat!(
            include_str!("../assets/browser-host.js"),
            include_str!("../assets/browser-files.js")
        )
    ))
}
async fn websocket(State(host): State<Host>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.max_message_size(4 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            let context = host.context;
            let _ = host
                .pool
                .launch_virtualdom(dioxus_liveview::axum_socket(socket), move || {
                    let mut dom = dioxus::prelude::VirtualDom::new(crate::studio::App);
                    dom.insert_any_root_context(Box::new(context));
                    dom
                })
                .await;
        })
}
#[derive(Serialize)]
struct Uploaded {
    path: PathBuf,
    directory: PathBuf,
    files: usize,
}
async fn upload(
    State(host): State<Host>,
    mut multipart: Multipart,
) -> Result<axum::Json<Uploaded>, (StatusCode, String)> {
    let result = async {
        let folder = host.context.root.join("imports").join(random_id()?);
        tokio::fs::create_dir_all(&folder).await?;
        let mut count = 0;
        let mut only = None;
        while let Some(mut field) = multipart.next_field().await? {
            ensure!(count < 1024, "A song upload can contain at most 1024 files");
            let name = field
                .file_name()
                .context("Missing upload filename")?
                .to_owned();
            let relative = Path::new(&name);
            ensure!(
                !relative.is_absolute()
                    && relative
                        .components()
                        .all(|p| matches!(p, Component::Normal(_))),
                "Invalid upload path"
            );
            let path = folder.join(relative);
            tokio::fs::create_dir_all(path.parent().unwrap()).await?;
            let mut file = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .await?;
            while let Some(chunk) = field.chunk().await? {
                file.write_all(&chunk).await?;
            }
            file.sync_all().await?;
            only = Some(path);
            count += 1;
        }
        ensure!(count > 0, "Choose at least one file");
        if count == 1
            && only.as_ref().is_some_and(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            })
        {
            let archive = only.take().unwrap();
            let target = folder.join("song");
            let output = target.clone();
            tokio::task::spawn_blocking(move || unpack_song(&archive, &output)).await??;
            let target = imported_root(&target)?;
            return Ok(Uploaded {
                path: target.clone(),
                directory: target,
                files: count,
            });
        }
        Ok::<_, anyhow::Error>(Uploaded {
            path: if count == 1 {
                only.unwrap()
            } else {
                folder.clone()
            },
            directory: folder,
            files: count,
        })
    }
    .await;
    result
        .map(axum::Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, format!("{error:#}")))
}
async fn asset(
    State(host): State<Host>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let path = host
        .context
        .assets
        .read()
        .ok()
        .and_then(|assets| assets.get(&id).cloned());
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // ServeFile supplies streaming, MIME types and byte-range support without
    // retaining another full PCM copy for every audio request.
    let mut request = Request::new(Body::empty());
    *request.headers_mut() = headers;
    match tower_http::services::ServeFile::new(path)
        .try_call(request)
        .await
    {
        Ok(response) => response.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn download_asset(
    State(host): State<Host>,
    RoutePath(id): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    let name = host.context.assets.read().ok().and_then(|assets| {
        assets
            .get(&id)
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    });
    let mut response = asset(State(host), RoutePath(id), headers).await;
    if response.status().is_success()
        && let Some(name) = name
    {
        let encoded: String = name
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect();
        response.headers_mut().insert(
            "content-disposition",
            HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}"))
                .expect("percent-encoded filename"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_cannot_escape_workspace_or_follow_external_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let context = BrowserContext {
            root: root.path().canonicalize().unwrap(),
            assets: Arc::default(),
            start_project: None,
        };
        assert!(context.resolve(Path::new("songs/new")).is_ok());
        assert!(context.resolve(Path::new("../outside")).is_err());
        assert!(context.resolve(Path::new("/etc/passwd")).is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
            assert!(context.resolve(Path::new("escape/new")).is_err());
        }
    }
    #[test]
    fn project_backup_waits_for_store_and_keeps_checkpoints_with_wrapped_imports() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("wrapped/project");
        std::fs::create_dir_all(project.join("checkpoints")).unwrap();
        std::fs::write(project.join("project.json"), b"project revision").unwrap();
        std::fs::write(project.join("checkpoints/saved.json"), b"undo content").unwrap();
        let lock = std::fs::File::create(project.join("editor.lock")).unwrap();
        lock.try_lock().unwrap();
        let zip = root.path().join("project.zip");
        assert!(archive_folder(&project, &zip).is_err());
        assert!(!zip.exists());
        lock.unlock().unwrap();
        archive_folder(&root.path().join("wrapped"), &zip).unwrap();
        let imported = root.path().join("imported");
        unpack_song(&zip, &imported).unwrap();
        let restored = imported_root(&imported).unwrap();
        assert_eq!(restored, imported.join("project"));
        assert_eq!(
            std::fs::read(restored.join("checkpoints/saved.json")).unwrap(),
            b"undo content"
        );
    }
    #[test]
    fn song_archive_round_trip_preserves_audio_and_notes_and_rejects_traversal() {
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song");
        std::fs::create_dir(&song).unwrap();
        std::fs::write(song.join("notes.chart"), b"source notes\r\n").unwrap();
        std::fs::write(song.join("song.ogg"), [0, 1, 2, 255]).unwrap();
        let path = root.path().join("song.zip");
        archive_folder(&song, &path).unwrap();
        let copied = root.path().join("copied");
        unpack_song(&path, &copied).unwrap();
        assert_eq!(
            std::fs::read(copied.join("notes.chart")).unwrap(),
            b"source notes\r\n"
        );
        assert_eq!(
            std::fs::read(copied.join("song.ogg")).unwrap(),
            [0, 1, 2, 255]
        );
        let bad = root.path().join("bad.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        writer
            .start_file(
                "../escape",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.finish().unwrap();
        assert!(unpack_song(&bad, &root.path().join("unsafe")).is_err());
        assert!(!root.path().join("escape").exists());
    }
}
