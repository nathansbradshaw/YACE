//! Git helpers for dedicated review repositories.
//! Sync review files without changing project.json or rewriting Git history.
use crate::{
    collaboration_exchange as exchange,
    editor::{Actor, Store},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output},
};
const MARKER: &str = ".yace-review-repository";
fn command(folder: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .output()
        .context("Git is not installed or could not run")
}
fn git(folder: &Path, args: &[&str]) -> Result<String> {
    let out = command(folder, args)?;
    ensure!(
        out.status.success(),
        "Git {}: {}",
        args.first().unwrap_or(&"operation"),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8(out.stdout)?.trim().into())
}
fn event_name(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn allowed(name: &str) -> bool {
    event_name(name) || name == MARKER || name == ".gitignore"
}
fn check(folder: &Path) -> Result<()> {
    ensure!(
        folder.join(MARKER).is_file(),
        "Choose a dedicated YACE review repository, or create/clone one below"
    );
    let top = git(folder, &["rev-parse", "--show-toplevel"])?;
    ensure!(
        Path::new(&top).canonicalize()? == folder.canonicalize()?,
        "Select the review repository root"
    );
    let files = command(folder, &["ls-files", "-z"])?;
    ensure!(
        files.status.success(),
        "Cannot inspect tracked review files"
    );
    for file in files.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        ensure!(
            allowed(std::str::from_utf8(file)?),
            "Repository contains unrelated tracked files; use a dedicated review repository"
        );
    }
    Ok(())
}
/// Creates a new local review repository. A remote is optional and user supplied.
pub fn create(folder: &Path, remote: Option<&str>) -> Result<()> {
    fs::create_dir_all(folder)?;
    ensure!(
        fs::read_dir(folder)?.next().is_none(),
        "Choose an empty folder for the new review repository"
    );
    git(folder, &["init", "-b", "main"])?;
    fs::write(folder.join(MARKER), b"YACE immutable review events v1\n")?;
    fs::write(folder.join(".gitignore"), b"/exchange.lock\n/.tmp*\n")?;
    if let Some(remote) = remote.filter(|r| !r.trim().is_empty()) {
        connect(folder, remote)?;
    }
    Ok(())
}
pub fn clone_repository(remote: &str, folder: &Path) -> Result<()> {
    ensure!(
        !remote.trim().is_empty() && !remote.starts_with('-'),
        "Enter a repository URL or shared Git path"
    );
    ensure!(
        !folder.exists() || fs::read_dir(folder)?.next().is_none(),
        "Clone destination must be empty"
    );
    let output = Command::new("git")
        .args(["clone", "--"])
        .arg(remote)
        .arg(folder)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    ensure!(
        output.status.success(),
        "Git clone: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    check(folder)
}
pub fn connect(folder: &Path, remote: &str) -> Result<()> {
    check(folder)?;
    ensure!(
        !remote.trim().is_empty() && !remote.starts_with('-'),
        "Enter a repository URL or shared Git path"
    );
    if command(folder, &["remote", "get-url", "origin"])?
        .status
        .success()
    {
        ensure!(
            git(folder, &["remote", "get-url", "origin"])? == remote,
            "Repository already has another origin; choose its existing remote"
        );
    } else {
        git(folder, &["remote", "add", "origin", remote])?;
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct FileConflict {
    pub path: String,
    pub local_valid: bool,
    pub incoming_valid: bool,
}
pub fn conflicts(folder: &Path) -> Result<Vec<FileConflict>> {
    check(folder)?;
    let output = command(folder, &["diff", "--name-only", "--diff-filter=U", "-z"])?;
    ensure!(output.status.success(), "Could not inspect Git conflicts");
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .map(|bytes| {
            let path = std::str::from_utf8(bytes)?.to_string();
            ensure!(
                allowed(&path),
                "Conflict outside review files; repository left intact"
            );
            Ok(FileConflict {
                local_valid: side(folder, &path, 2).is_ok(),
                incoming_valid: side(folder, &path, 3).is_ok(),
                path,
            })
        })
        .collect()
}
fn side(folder: &Path, path: &str, stage: u8) -> Result<Vec<u8>> {
    ensure!(allowed(path), "Only review files can be resolved here");
    let output = command(folder, &["show", &format!(":{stage}:{path}")])?;
    ensure!(
        output.status.success(),
        "This side deleted the file; event history must be retained"
    );
    let bytes = output.stdout;
    if event_name(path) {
        let event: exchange::Event = serde_json::from_slice(&bytes)?;
        ensure!(
            event.id == path.trim_end_matches(".json") && event.valid_checksum()?,
            "This Git version contains a changed or damaged immutable event"
        );
    }
    Ok(bytes)
}
/// Explicit GUI choice. Original versions remain recoverable in the Git parents.
pub fn resolve_file(folder: &Path, path: &str, incoming: bool) -> Result<()> {
    check(folder)?;
    let bytes = side(folder, path, if incoming { 3 } else { 2 })?;
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(folder.join(path))?;
    git(folder, &["add", "--", path])?;
    Ok(())
}
fn commit(folder: &Path, actor: &Actor) -> Result<()> {
    if !command(folder, &["diff", "--cached", "--quiet"])?
        .status
        .success()
        || command(folder, &["rev-parse", "--verify", "MERGE_HEAD"])?
            .status
            .success()
    {
        git(
            folder,
            &[
                "-c",
                &format!("user.name={}", actor.name),
                "-c",
                "user.email=yace-local@example.invalid",
                "commit",
                "-m",
                "Synchronize YACE reviews",
            ],
        )?;
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub message: String,
    pub conflicts: Vec<FileConflict>,
}
/// Explicit Save/Pull/Publish operation, scoped to the dedicated review repository.
pub fn sync(store: &Store, folder: &Path, actor: Actor, revision: u64) -> Result<Outcome> {
    check(folder)?;
    let pending = conflicts(folder)?;
    if !pending.is_empty() {
        return Ok(Outcome {
            message: "Choose which valid version to keep for each review file, then sync again."
                .into(),
            conflicts: pending,
        });
    }
    // If a previously reviewed merge is staged, validate its graph before committing.
    exchange::sync(store, folder, actor.clone(), revision)?;
    let mut paths = vec![MARKER.to_string(), ".gitignore".into()];
    for item in fs::read_dir(folder)? {
        let path = item?.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if event_name(&name) {
            paths.push(name);
        }
    }
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(folder, &args)?;
    commit(folder, &actor)?;
    if !command(folder, &["remote", "get-url", "origin"])?
        .status
        .success()
    {
        return Ok(Outcome { message:"Reviews saved in local Git history. Connect an optional remote or share this repository folder.".into(),conflicts:vec![] });
    }
    git(folder, &["fetch", "origin"])?;
    if command(
        folder,
        &["rev-parse", "--verify", "refs/remotes/origin/main"],
    )?
    .status
    .success()
    {
        let tree = command(
            folder,
            &["ls-tree", "-r", "--name-only", "-z", "origin/main"],
        )?;
        ensure!(
            tree.status.success(),
            "Cannot inspect incoming review repository"
        );
        for path in tree.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
            ensure!(
                allowed(std::str::from_utf8(path)?),
                "Incoming repository contains unrelated files; no Git merge was attempted"
            );
        }
        let merge = command(
            folder,
            &[
                "-c",
                &format!("user.name={}", actor.name),
                "-c",
                "user.email=yace-local@example.invalid",
                "merge",
                "--no-edit",
                "origin/main",
            ],
        )?;
        if !merge.status.success() {
            let pending = conflicts(folder)?;
            ensure!(
                !pending.is_empty(),
                "Git could not merge: {}",
                String::from_utf8_lossy(&merge.stderr).trim()
            );
            return Ok(Outcome { message:"Git found review-file conflicts. Both versions remain in history; choose a valid copy below.".into(),conflicts:pending });
        }
    }
    check(folder)?;
    exchange::sync(store, folder, actor, store.load()?.revision)?;
    git(folder, &["push", "-u", "origin", "HEAD:main"])?;
    Ok(Outcome {
        message: "Reviews saved, incoming reviews loaded, and changes published.".into(),
        conflicts: vec![],
    })
}
