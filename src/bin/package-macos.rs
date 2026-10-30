//! Build a local macOS app. Replacements keep a recovery ZIP of the previous app.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{fs, path::PathBuf, process::Command};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "target/release/chart-engine")]
    executable: PathBuf,
    #[arg(short, long, default_value = "output/YACE.app")]
    output: PathBuf,
    /// Directory used only when including private local model files.
    #[arg(long, default_value = "models")]
    model_dir: PathBuf,
    /// Include verified weights for a private local bundle; review licenses before sharing.
    #[arg(long)]
    include_models: bool,
    /// Include an already verified optional HQ Piano model in this local bundle.
    #[arg(long, requires = "include_models")]
    high_quality_piano: bool,
    /// Include the optional Spanish/automatic-language lyric model when installed.
    #[arg(long, requires = "include_models")]
    multilingual_lyrics: bool,
    /// Developer ID identity, or '-' for a local ad-hoc signature.
    #[arg(long, default_value = "-")]
    signing_identity: String,
    /// Update the same app path/identity, retaining the previous bundle as a ZIP.
    #[arg(long)]
    replace: bool,
}
fn bundle_id(path: &std::path::Path) -> Result<String> {
    let result = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw"])
        .arg(path.join("Contents/Info.plist"))
        .output()?;
    ensure!(
        result.status.success(),
        "Existing app has no readable bundle identity"
    );
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}
fn publish_bundle(stage: &std::path::Path, output: &std::path::Path, replace: bool) -> Result<()> {
    if !output.exists() {
        return chart_engine::pipeline::publish(stage, output);
    }
    ensure!(
        replace && output.is_dir() && !output.is_symlink(),
        "Use --replace to update the existing app"
    );
    ensure!(
        bundle_id(stage)? == bundle_id(output)?,
        "App updates must preserve the existing bundle identifier"
    );
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let recovery = parent.join("YACE-recovery");
    fs::create_dir_all(&recovery)?;
    let archive = tempfile::Builder::new()
        .prefix("YACE-")
        .suffix(".zip")
        .tempfile_in(&recovery)?;
    let result = Command::new("ditto")
        .args(["-c", "-k", "--keepParent"])
        .arg(output)
        .arg(archive.path())
        .output()?;
    ensure!(
        result.status.success(),
        "Previous bundle could not be archived: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (_, archive_path) = archive.keep()?;
    eprintln!("Previous bundle archived: {}", archive_path.display());
    // The temporary old folder has no .app suffix and is never launched.
    let holding = tempfile::Builder::new()
        .prefix(".yace-replace-")
        .tempdir_in(parent)?
        .keep();
    let previous = holding.join("previous");
    fs::rename(output, &previous)?;
    if let Err(error) = chart_engine::pipeline::publish(stage, output) {
        fs::rename(&previous, output).with_context(|| {
            format!(
                "Update failed; previous bytes remain at {} and {}",
                previous.display(),
                archive_path.display()
            )
        })?;
        let _ = fs::remove_dir(&holding);
        return Err(error);
    }
    if let Err(error) = fs::remove_dir_all(&holding) {
        eprintln!(
            "Previous unpacked bytes also remain at {}: {error}",
            holding.display()
        );
    }
    Ok(())
}
fn copy_notices(source: &std::path::Path, output: &std::path::Path) -> Result<()> {
    fs::create_dir_all(output)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let kind = entry.file_type()?;
        ensure!(
            !kind.is_symlink(),
            "Notice/source resources must not be symlinks"
        );
        if kind.is_dir() {
            copy_notices(&entry.path(), &output.join(name))?;
        } else if kind.is_file() {
            fs::copy(entry.path(), output.join(name))?;
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    let output = build_bundle(&Args::parse())?;
    println!("{}", output.display());
    Ok(())
}

fn build_bundle(args: &Args) -> Result<PathBuf> {
    ensure!(
        cfg!(target_os = "macos"),
        "macOS app packaging requires macOS"
    );
    ensure!(
        args.include_models || (!args.high_quality_piano && !args.multilingual_lyrics),
        "Optional models require --include-models"
    );
    ensure!(
        !args.signing_identity.trim().is_empty(),
        "Choose a signing identity or '-' for local use"
    );
    let parent = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    fs::create_dir_all(parent)?;
    if args.output.exists() {
        ensure!(
            args.replace && args.output.is_dir() && !args.output.is_symlink(),
            "Use --replace to update the existing app"
        );
    } else {
        chart_engine::pipeline::destination(&args.output, None)?;
    }
    let stage = tempfile::Builder::new().suffix(".app").tempdir_in(parent)?;
    fs::create_dir_all(stage.path().join("Contents/MacOS"))?;
    fs::copy(
        &args.executable,
        stage.path().join("Contents/MacOS/chart-engine"),
    )?;
    let model_output = stage.path().join("Contents/Resources/models");
    fs::create_dir_all(&model_output)?;
    fs::write(
        stage.path().join("Contents/Resources/YACE.icns"),
        include_bytes!("../../packaging/macos/YACE.icns"),
    )?;
    let sources = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let notices = stage.path().join("Contents/Resources/third-party");
    copy_notices(&sources.join("licenses"), &notices.join("licenses"))?;
    fs::write(
        notices.join("THIRD_PARTY.md"),
        include_bytes!("../../THIRD_PARTY.md"),
    )?;
    fs::write(notices.join("LICENSE"), include_bytes!("../../LICENSE"))?;
    #[cfg(feature = "vocal-alignment")]
    copy_notices(
        &sources.join("vendor/practice-acoustic"),
        &notices.join("practice-acoustic"),
    )?;
    for vendor in ["candle-core", "candle-core-legacy"] {
        let destination = notices.join(vendor);
        fs::create_dir_all(&destination)?;
        for file in ["LICENSE", "YACE-NOTICE.md"] {
            fs::copy(
                sources.join("vendor").join(vendor).join(file),
                destination.join(file),
            )?;
        }
    }
    if args.include_models {
        for asset in chart_engine::models::assets() {
            let model = chart_engine::models::verified(&args.model_dir, &asset.id)?;
            fs::copy(model, model_output.join(asset.filename))?;
        }
        if args.high_quality_piano {
            let asset = chart_engine::keyboard_separation::asset();
            let model = chart_engine::keyboard_separation::verified(&args.model_dir)?;
            fs::copy(model, model_output.join(asset.filename))?;
        }
        if args.multilingual_lyrics {
            let asset = chart_engine::lyric_language::multilingual_asset();
            let model = chart_engine::clean_recognition::prepare_model(
                &args.model_dir,
                false,
                chart_engine::lyric_language::Language::Auto,
                &Default::default(),
            )?;
            let destination = model_output.join(asset.filename);
            fs::copy(model, &destination)?;
            ensure!(
                chart_engine::models::hash_file(&destination)? == asset.sha256,
                "Packaged multilingual lyric model checksum mismatch"
            );
        }
        // The supplied executable can be newer than this packager's embedded
        // catalog. Verify through the actual worker before publishing a bundle.
        let verified = Command::new(stage.path().join("Contents/MacOS/chart-engine"))
            .args(["models", "verify", "--directory"])
            .arg(&model_output)
            .output()?;
        ensure!(
            verified.status.success(),
            "Packaged worker rejected its models; rebuild the packager against the same catalog: {}",
            String::from_utf8_lossy(&verified.stderr)
        );
        #[cfg(feature = "vocal-alignment")]
        if args.model_dir.join("vocal-alignment/en").exists() {
            let input = chart_engine::vocal_alignment::verified(&args.model_dir)?;
            let output = model_output.join("vocal-alignment/en");
            fs::create_dir_all(&output)?;
            for asset in chart_engine::vocal_alignment::assets() {
                fs::copy(input.join(asset.filename), output.join(asset.filename))?;
            }
        }
    }
    fs::write(
        stage.path().join("Contents/Info.plist"),
        include_str!("../../packaging/macos/Info.plist"),
    )?;
    let source_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&sources)
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_owned());
    let source_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&sources)
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| !r.stdout.is_empty());
    fs::write(
        stage.path().join("Contents/Resources/build-info.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "application":"YACE", "bundle_identifier":"dev.yace.studio", "source_commit":source_commit,
            "source_dirty":source_dirty, "executable_sha256":chart_engine::models::hash_file(&args.executable)?,
            "models_included":args.include_models,
            "signing":if args.signing_identity == "-" {"ad-hoc; not notarized"} else {"Developer ID requested; notarization required"},
            "signing_identity":args.signing_identity,
            "high_quality_piano_included":args.high_quality_piano,
            "multilingual_lyrics_included":args.multilingual_lyrics
        }))?,
    )?;
    let mut signing = Command::new("codesign");
    signing.args(["--force", "--sign", &args.signing_identity]);
    if args.signing_identity != "-" {
        signing.args(["--options", "runtime", "--timestamp"]);
    }
    let result = signing.arg(stage.path()).output()?;
    ensure!(
        result.status.success(),
        "Signing failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let verification = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(stage.path())
        .output()?;
    ensure!(
        verification.status.success(),
        "Bundle signature verification failed: {}",
        String::from_utf8_lossy(&verification.stderr)
    );
    publish_bundle(stage.path(), &args.output, args.replace)?;
    Ok(args.output.canonicalize()?)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn default_bundle_excludes_weights_and_keeps_required_notices() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("new-output/YACE.app");
        let args = Args::try_parse_from([
            "package-macos",
            "--executable",
            "/usr/bin/true",
            "--output",
            output.to_str().unwrap(),
            "--model-dir",
            root.path().join("missing-models").to_str().unwrap(),
        ])
        .unwrap();
        assert!(!args.include_models);
        build_bundle(&args).unwrap();
        assert_eq!(
            fs::read_dir(output.join("Contents/Resources/models"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(bundle_id(&output).unwrap(), "dev.yace.studio");
        let info: serde_json::Value = serde_json::from_slice(
            &fs::read(output.join("Contents/Resources/build-info.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(info["models_included"], false);
        assert!(
            output
                .join("Contents/Resources/third-party/candle-core/LICENSE")
                .is_file()
        );
        assert!(
            output
                .join("Contents/Resources/third-party/candle-core-legacy/YACE-NOTICE.md")
                .is_file()
        );
        #[cfg(feature = "vocal-alignment")]
        assert!(
            output
                .join("Contents/Resources/third-party/practice-acoustic/src/lib.rs")
                .is_file()
        );
    }
    #[test]
    fn optional_model_flags_need_explicit_inclusion() {
        assert!(Args::try_parse_from(["package-macos", "--high-quality-piano"]).is_err());
        assert!(Args::try_parse_from(["package-macos", "--multilingual-lyrics"]).is_err());
        assert!(
            Args::try_parse_from(["package-macos", "--include-models", "--high-quality-piano"])
                .is_ok()
        );
    }
    fn app(path: &std::path::Path, id: &str, marker: &str) {
        fs::create_dir_all(path.join("Contents")).unwrap();
        fs::write(path.join("Contents/Info.plist"), format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string></dict></plist>")).unwrap();
        fs::write(path.join("marker"), marker).unwrap();
    }
    #[test]
    fn replacement_keeps_identity_and_recoverable_previous_bytes() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("YACE.app");
        let stage = root.path().join("staged");
        app(&output, "dev.yace.studio", "old");
        app(&stage, "dev.yace.studio", "new");
        publish_bundle(&stage, &output, true).unwrap();
        assert_eq!(fs::read(output.join("marker")).unwrap(), b"new");
        let archives = fs::read_dir(root.path().join("YACE-recovery"))
            .unwrap()
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(archives.len(), 1);
        let old = Command::new("unzip")
            .arg("-p")
            .arg(archives[0].path())
            .arg("YACE.app/marker")
            .output()
            .unwrap();
        assert!(old.status.success());
        assert_eq!(old.stdout, b"old");
    }
    #[test]
    fn changed_identity_and_implicit_replacement_fail_without_touching_old_app() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("YACE.app");
        let stage = root.path().join("staged");
        app(&output, "dev.yace.studio", "old");
        app(&stage, "different", "new");
        assert!(publish_bundle(&stage, &output, false).is_err());
        assert!(publish_bundle(&stage, &output, true).is_err());
        assert_eq!(fs::read(output.join("marker")).unwrap(), b"old");
        assert!(!root.path().join("YACE-recovery").exists());
    }
}
