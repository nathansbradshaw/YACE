//! GitHub's Git database API transports immutable review events only.
//! The caller owns authorization. This module never serializes credentials.
use crate::{
    collaboration_exchange as exchange,
    editor::Project,
    review_repository::{self as repository, Repository, Tree},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct Reply {
    pub status: u16,
    pub body: Value,
}
impl Reply {
    fn success(self) -> Result<Value> {
        ensure!(
            (200..300).contains(&self.status),
            "GitHub HTTP {}. Check repository access, token permissions, rate limits and branch protection. Local charts and review history are retained.",
            self.status
        );
        Ok(self.body)
    }
}
#[allow(async_fn_in_trait)]
pub trait Api {
    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Reply>;
}
pub struct Snapshot {
    pub head: String,
    pub tree: Tree,
    pub bundle: exchange::Bundle,
}
fn sha(value: &Value) -> Result<String> {
    Ok(repository::object_id(
        value["sha"]
            .as_str()
            .context("GitHub omitted an object identity")?,
    )?
    .into())
}
/// Read immutable objects by hash, so a moving branch cannot mix two snapshots.
pub async fn pull(api: &impl Api, repo: &Repository, project: &Project) -> Result<Snapshot> {
    let base = repo.api();
    let reference = api
        .request("GET", &format!("{base}/git/ref/heads/main"), None)
        .await?
        .success()?;
    ensure!(
        reference["object"]["type"] == "commit",
        "Review main must point to a commit"
    );
    let head = sha(&reference["object"])?;
    let commit = api
        .request("GET", &format!("{base}/git/commits/{head}"), None)
        .await?
        .success()?;
    let tree_sha = sha(&commit["tree"])?;
    let tree: Tree = serde_json::from_value(
        api.request("GET", &format!("{base}/git/trees/{tree_sha}"), None)
            .await?
            .success()?,
    )?;
    ensure!(
        tree.sha == tree_sha,
        "GitHub returned a different review tree"
    );
    tree.validate()?;
    let mut files = BTreeMap::new();
    // Bounded event count and byte limits are checked before requesting blobs.
    for blob in &tree.tree {
        let body = api
            .request("GET", &format!("{base}/git/blobs/{}", blob.sha), None)
            .await?
            .success()?;
        ensure!(
            sha(&body)? == blob.sha && body["encoding"] == "base64",
            "Invalid review blob response"
        );
        let content = body["content"]
            .as_str()
            .context("Missing review blob content")?;
        ensure!(
            content.len() <= (exchange::MAX_FILE_BYTES as usize) * 2,
            "Review blob is too large"
        );
        let bytes = decode_blob(content)?;
        files.insert(blob.path.clone(), bytes);
    }
    let bundle = repository::bundle(&tree, &files)?;
    exchange::validate_import(project, &bundle)?;
    Ok(Snapshot { head, tree, bundle })
}

// Keeping base64 decoding in the shared protocol also makes native mock tests
// exercise exactly the same bytes as the browser transport.
fn decode_blob(content: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    let compact: Vec<_> = content
        .bytes()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let output = base64::engine::general_purpose::STANDARD.decode(compact)?;
    ensure!(
        output.len() as u64 <= exchange::MAX_FILE_BYTES,
        "Review blob is too large"
    );
    Ok(output)
}

/// Publish missing immutable files on top of the observed head. A competing
/// push causes a fresh validated pull; a ref is never forced or deleted.
pub async fn synchronize(
    api: &impl Api,
    repo: &Repository,
    project: &Project,
    publish: bool,
) -> Result<exchange::Bundle> {
    let local = exchange::export_bundle(project)?;
    for _ in 0..3 {
        let remote = pull(api, repo, project).await?;
        if !publish {
            return Ok(remote.bundle);
        }
        let existing: BTreeSet<_> = remote.bundle.events.iter().map(|e| e.id.as_str()).collect();
        let mut additions = Vec::new();
        let mut bytes = remote.tree.tree.iter().filter_map(|b| b.size).sum::<u64>();
        for event in &local.events {
            if !existing.contains(event.id.as_str()) {
                let content = serde_json::to_string_pretty(event)?;
                ensure!(
                    content.len() as u64 <= exchange::MAX_FILE_BYTES,
                    "Review event is too large"
                );
                bytes += content.len() as u64;
                ensure!(
                    bytes <= exchange::MAX_DIRECTORY_BYTES,
                    "Review repository would exceed 64 MiB; publish no further events here"
                );
                additions.push(json!({"path":format!("{}.json",event.id),"mode":"100644","type":"blob","content":content}));
            }
        }
        // Validate the union before writing anything. This catches competing
        // approvals, source mismatch, cycles and event/proposal ID collisions.
        let mut union = remote.bundle.clone();
        union.events.extend(
            local
                .events
                .iter()
                .filter(|e| !existing.contains(e.id.as_str()))
                .cloned(),
        );
        exchange::validate_import(project, &union)?;
        if additions.is_empty() {
            return Ok(remote.bundle);
        }
        let base = repo.api();
        let tree = api
            .request(
                "POST",
                &format!("{base}/git/trees"),
                Some(json!({"base_tree":remote.tree.sha,"tree":additions})),
            )
            .await?
            .success()?;
        let commit=api.request("POST",&format!("{base}/git/commits"),Some(json!({"message":"Synchronize immutable YACE review events [skip ci]","tree":sha(&tree)?,"parents":[remote.head]}))).await?.success()?;
        let update = api
            .request(
                "PATCH",
                &format!("{base}/git/refs/heads/main"),
                Some(json!({"sha":sha(&commit)?,"force":false})),
            )
            .await?;
        if matches!(update.status, 409 | 422) {
            continue;
        }
        update.success()?;
        return Ok(union);
    }
    anyhow::bail!(
        "Review main advanced during three attempts. Pull again; existing remote commits and your local reviews are retained."
    )
}

/// Only a repository newly created by this action is initialized. The initial
/// README commit remains an ancestor; replacing its tree never rewrites history.
pub async fn create(api: &impl Api, name: &str) -> Result<Repository> {
    let user = api.request("GET", "/user", None).await?.success()?;
    let owner = user["login"].as_str().context("Missing GitHub account")?;
    let repo = Repository::parse(&format!("{owner}/{name}"))?;
    let created=api.request("POST","/user/repos",Some(json!({"name":repo.name,"private":true,"auto_init":true,"description":"YACE immutable review events; audio and editable projects stay local"}))).await?.success()?;
    ensure!(
        created["full_name"].as_str().is_some_and(
            |value| value.eq_ignore_ascii_case(&format!("{}/{}", repo.owner, repo.name))
        ),
        "GitHub created a different repository"
    );
    let branch = created["default_branch"]
        .as_str()
        .context("Missing initial branch")?;
    ensure!(
        branch
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
        "Unsupported initial branch name"
    );
    let base = repo.api();
    let reference = api
        .request("GET", &format!("{base}/git/ref/heads/{branch}"), None)
        .await?
        .success()?;
    let head = sha(&reference["object"])?;
    let initial = api
        .request("GET", &format!("{base}/git/commits/{head}"), None)
        .await?
        .success()?;
    ensure!(
        initial["parents"]
            .as_array()
            .is_some_and(|parents| parents.is_empty()),
        "New repository changed before initialization; no files were replaced"
    );
    let tree: Tree = serde_json::from_value(
        api.request(
            "GET",
            &format!("{base}/git/trees/{}", sha(&initial["tree"])?),
            None,
        )
        .await?
        .success()?,
    )?;
    ensure!(
        !tree.truncated
            && tree.tree.len() <= 1
            && tree
                .tree
                .iter()
                .all(|b| b.path == "README.md" && b.mode == "100644" && b.kind == "blob"),
        "New repository has unexpected files; no files were replaced"
    );
    let tree=api.request("POST",&format!("{base}/git/trees"),Some(json!({"tree":[{"path":repository::MARKER,"mode":"100644","type":"blob","content":repository::MARKER_CONTENT},{"path":".gitignore","mode":"100644","type":"blob","content":repository::IGNORE_CONTENT}]}))).await?.success()?;
    let commit=api.request("POST",&format!("{base}/git/commits"),Some(json!({"message":"Initialize YACE review repository [skip ci]","tree":sha(&tree)?,"parents":[head]}))).await?.success()?;
    // A new main ref is atomic. If someone creates main first this request
    // fails, retaining their branch. Never force a competing initialization.
    if branch == "main" {
        api.request(
            "PATCH",
            &format!("{base}/git/refs/heads/main"),
            Some(json!({"sha":sha(&commit)?,"force":false})),
        )
        .await?
        .success()?;
    } else {
        api.request(
            "POST",
            &format!("{base}/git/refs"),
            Some(json!({"ref":"refs/heads/main","sha":sha(&commit)?})),
        )
        .await?
        .success()?;
        api.request("PATCH", &base, Some(json!({"default_branch":"main"})))
            .await?
            .success()?;
    }
    Ok(repo)
}
