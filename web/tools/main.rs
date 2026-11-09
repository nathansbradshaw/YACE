//! Build static WebAssembly assets or serve those bytes for local acceptance.
use std::{
    env, fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[cfg(test)]
#[path = "../../src/archive_codec.rs"]
mod archive_codec;
#[cfg(test)]
#[path = "../src/audio_decode.rs"]
mod audio_decode;
#[cfg(test)]
#[path = "../src/audio_identity.rs"]
mod audio_identity;
#[cfg(test)]
#[path = "../src/audio_resample.rs"]
mod audio_resample;
#[cfg(test)]
#[path = "../../src/compact_backup.rs"]
mod compact_backup;
#[cfg(test)]
#[path = "../../src/media_clock.rs"]
mod media_clock;
mod optimizer_download;
#[cfg(test)]
#[path = "../src/playback_activity.rs"]
mod playback_activity;
#[path = "../../src/preview_ports.rs"]
mod preview_ports;
mod recognition_build;
#[cfg(test)]
#[path = "../../src/recording_metadata.rs"]
mod recording_metadata;
#[cfg(test)]
#[path = "../src/recording_tags.rs"]
mod recording_tags;
mod static_release;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "../src/workspace_target.rs"]
mod workspace_target;
fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("Build command failed: {command:?}").into());
    }
    Ok(())
}
fn copy_source(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_source(&entry.path(), &to.join(entry.file_name()))?;
        } else if kind.is_file() {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        } else {
            return Err("Covered source cannot contain symbolic links".into());
        }
    }
    Ok(())
}
fn compatible_optimizer_version(version: &str) -> bool {
    let mut fields = version.split_whitespace();
    fields.next() == Some("wasm-opt")
        && fields.next() == Some("version")
        && fields.next() == Some("132")
}
fn build(root: &Path) -> Result<()> {
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/web-static"));
    run(Command::new("cargo")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", &target)
        .args([
            "build",
            "--locked",
            "--release",
            "--manifest-path",
            "web/Cargo.toml",
            "--target",
            "wasm32-unknown-unknown",
        ]))?;
    let dist = root.join("web/dist");
    fs::create_dir_all(&dist)?;
    run(Command::new("wasm-bindgen")
        .arg(target.join("wasm32-unknown-unknown/release/yace_web.wasm"))
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(&dist))?;
    let optimizer = Command::new("wasm-opt").arg("--version").output()?;
    let optimizer_version = String::from_utf8_lossy(&optimizer.stdout);
    if !optimizer.status.success() || !compatible_optimizer_version(&optimizer_version) {
        return Err(format!(
            "Static builds require Binaryen wasm-opt 132; received {optimizer_version:?}"
        )
        .into());
    }
    let wasm = dist.join("yace_web_bg.wasm");
    let optimized = dist.join("yace_web_bg.optimized.wasm");
    run(Command::new("wasm-opt")
        .arg(&wasm)
        .args([
            "-Oz",
            "--enable-bulk-memory",
            "--enable-multivalue",
            "--enable-mutable-globals",
            "--enable-nontrapping-float-to-int",
            "--enable-reference-types",
            "--enable-sign-ext",
            "--strip-debug",
            "-o",
        ])
        .arg(&optimized))?;
    if fs::metadata(&optimized)?.len() > 25 * 1024 * 1024 {
        return Err("Optimized WebAssembly exceeds the static host's 25 MiB asset limit".into());
    }
    fs::rename(optimized, wasm)?;
    fs::copy(root.join("web/index.html"), dist.join("index.html"))?;
    fs::copy(root.join("web/worker.js"), dist.join("worker.js"))?;
    copy_source(&root.join("web/runtime"), &dist.join("runtime"))?;
    for package in [
        "symphonia-codec-vorbis",
        "symphonia-bundle-mp3",
        "practice-acoustic",
        "candle-core",
    ] {
        copy_source(
            &root.join("vendor").join(package),
            &dist.join("sources").join(package),
        )?;
    }
    for name in ["LICENSE", "THIRD_PARTY.md"] {
        fs::copy(root.join(name), dist.join(name))?;
    }
    fs::create_dir_all(dist.join("licenses"))?;
    for entry in fs::read_dir(root.join("licenses"))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), dist.join("licenses").join(entry.file_name()))?;
        }
    }
    println!("Static browser app: {}", dist.display());
    Ok(())
}
fn response(
    stream: &mut TcpStream,
    status: &str,
    mime: &str,
    body: &[u8],
    head: bool,
) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        stream.write_all(body)?;
    }
    Ok(())
}
fn serve_file(mut stream: TcpStream, root: &Path) -> Result<()> {
    let mut buffer = [0u8; 8192];
    let count = stream.read(&mut buffer)?;
    let request = std::str::from_utf8(&buffer[..count])?;
    let mut words = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = words.next().unwrap_or_default();
    let uri = words
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    if !matches!(method, "GET" | "HEAD") {
        return response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"Static files only",
            false,
        );
    }
    let relative = if uri == "/" {
        "index.html"
    } else {
        uri.trim_start_matches('/')
    };
    let path = Path::new(relative);
    if !path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return response(
            &mut stream,
            "400 Bad Request",
            "text/plain",
            b"Invalid path",
            method == "HEAD",
        );
    }
    let path = root.join(path);
    let Ok(path) = path.canonicalize() else {
        return response(
            &mut stream,
            "404 Not Found",
            "text/plain",
            b"No file",
            method == "HEAD",
        );
    };
    if !path.starts_with(root) || !path.is_file() {
        return response(
            &mut stream,
            "404 Not Found",
            "text/plain",
            b"No file",
            method == "HEAD",
        );
    }
    let mime = match path.extension().and_then(|s| s.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    let bytes = fs::read(&path)?;
    println!("{method} {uri}");
    response(&mut stream, "200 OK", mime, &bytes, method == "HEAD")
}
fn source_root() -> Result<PathBuf> {
    // A shared target cache can reuse this executable across Git worktrees.
    // Compile-time CARGO_MANIFEST_DIR would silently select the last builder's
    // checkout instead of the source requested by the current invocation.
    let start = env::var_os("YACE_SOURCE_ROOT")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(env::current_dir)?;
    start
        .ancestors()
        .find(|root| {
            root.join("web/tools/Cargo.toml").is_file()
                && root.join("web/index.html").is_file()
                && root.join("src/editor.rs").is_file()
        })
        .map(Path::to_owned)
        .ok_or_else(|| {
            "Run inside the YACE source checkout or set YACE_SOURCE_ROOT to its root".into()
        })
}
fn main() -> Result<()> {
    let root = source_root()?;
    let args: Vec<_> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fetch-optimizer") => {
            if args.len() != 2 { return Err("Use fetch-optimizer OUTPUT_ARCHIVE".into()); }
            optimizer_download::fetch(Path::new(&args[1]))
        }
        Some("build") => build(&root),
        Some("recognition-build") => recognition_build::build(&root, &args[1..]),
        Some("stage") => {
            let target=args.get(1).ok_or("Use stage HOST_DIST SOURCE_REVISION")?;
            let revision=args.get(2).ok_or("Use stage HOST_DIST SOURCE_REVISION")?;
            if args.len()>4 || args.get(3).is_some_and(|v|v!="--reuse-root-runtime") { return Err("Use stage HOST_DIST SOURCE_REVISION [--reuse-root-runtime]".into()); }
            let release=if args.get(3).is_some() {
                static_release::stage_reusing_runtime(&root.join("web/dist"),Path::new(target),revision)?
            } else { static_release::stage(&root.join("web/dist"),Path::new(target),revision)? };
            println!("Immutable static release: {}",release.display());Ok(())
        }
        Some("serve") => {
            let check_only = args.iter().any(|arg| arg == "--check");
            let positional: Vec<_> = args.iter().skip(1).filter(|arg| arg.as_str() != "--check").collect();
            if positional.len() > 2 { return Err("Use serve [3030|3032] [dist] [--check]".into()); }
            let port = positional
                .first()
                .map(|s| s.parse())
                .transpose()?
                .unwrap_or(preview_ports::PRIMARY);
            preview_ports::validate(port)?;
            let dist = positional.get(1).map(|path| PathBuf::from(path.as_str())).unwrap_or_else(||root.join("web/dist")).canonicalize()?;
            println!("YACE preview source: {}", root.display());
            println!("YACE preview files: {}", dist.display());
            if check_only {
                println!("Allowed preview port: {port}. No server started.");
                return Ok(());
            }
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).map_err(|error| format!("Cannot start YACE preview on {port}: {error}. Preserve the existing owner; reuse, coordinate a handoff, or wait. Do not choose another port."))?;
            println!("Static YACE: http://127.0.0.1:{port}/");
            for stream in listener.incoming() {
                let stream = stream?;
                let dist = dist.clone();
                std::thread::spawn(move || {
                    if let Err(error) = serve_file(stream, &dist) {
                        eprintln!("Static response: {error}");
                    }
                });
            }
            Ok(())
        }
        _ => Err(
            "Use: yace-static-web build | serve [3030|3032] [dist] [--check] | stage HOST_DIST SOURCE_REVISION | recognition-build SDK CRATE WORK".into(),
        ),
    }
}
