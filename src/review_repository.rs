//! The dedicated, immutable review repository protocol used by both clients.
use crate::collaboration_exchange::{self as exchange, Bundle, Event};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MARKER: &str = ".yace-review-repository";
pub const MARKER_CONTENT: &str = "YACE immutable review events v1\n";
pub const IGNORE_CONTENT: &str = "/exchange.lock\n/.tmp*\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub owner: String,
    pub name: String,
}
impl Repository {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let value = input
            .strip_prefix("https://github.com/")
            .or_else(|| input.strip_prefix("git@github.com:"))
            .unwrap_or(input);
        let value = value
            .trim_end_matches('/')
            .strip_suffix(".git")
            .unwrap_or(value.trim_end_matches('/'));
        let (owner, name) = value
            .split_once('/')
            .context("Enter owner/repository or a GitHub repository URL")?;
        ensure!(
            !owner.is_empty()
                && owner.len() <= 39
                && !owner.starts_with('-')
                && !owner.ends_with('-')
                && owner
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "Invalid GitHub owner"
        );
        ensure!(
            !name.is_empty()
                && name.len() <= 100
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
            "Invalid GitHub repository name"
        );
        Ok(Self {
            owner: owner.into(),
            name: name.into(),
        })
    }
    pub fn api(&self) -> String {
        format!("/repos/{}/{}", self.owner, self.name)
    }
    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}
pub fn event_name(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()))
}
pub fn allowed(name: &str) -> bool {
    event_name(name) || name == MARKER || name == ".gitignore"
}
pub fn event(path: &str, bytes: &[u8]) -> Result<Event> {
    ensure!(
        event_name(path) && bytes.len() as u64 <= exchange::MAX_FILE_BYTES,
        "Invalid review event file"
    );
    let event: Event = serde_json::from_slice(bytes)?;
    ensure!(
        path == format!("{}.json", event.id) && event.valid_checksum()?,
        "Review event checksum or filename mismatch"
    );
    Ok(event)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Blob {
    pub path: String,
    pub mode: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub sha: String,
    pub size: Option<u64>,
}
#[derive(Debug, Deserialize)]
pub struct Tree {
    pub sha: String,
    pub truncated: bool,
    pub tree: Vec<Blob>,
}
pub fn object_id(value: &str) -> Result<&str> {
    ensure!(
        value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid Git object identity"
    );
    Ok(value)
}
impl Tree {
    pub fn validate(&self) -> Result<()> {
        object_id(&self.sha)?;
        ensure!(
            !self.truncated && self.tree.len() <= exchange::MAX_EVENTS + 2,
            "Review repository is too large or incomplete"
        );
        let mut seen = BTreeSet::new();
        let mut total = 0;
        for blob in &self.tree {
            ensure!(
                allowed(&blob.path)
                    && blob.mode == "100644"
                    && blob.kind == "blob"
                    && seen.insert(&blob.path),
                "Repository contains unrelated, duplicate or unsafe files; choose a dedicated YACE review repository"
            );
            object_id(&blob.sha)?;
            let size = blob.size.context("Missing review file size")?;
            ensure!(size <= exchange::MAX_FILE_BYTES, "Review file is too large");
            total += size;
            ensure!(
                total <= exchange::MAX_DIRECTORY_BYTES,
                "Review repository exceeds 64 MiB"
            );
        }
        ensure!(
            seen.contains(&MARKER.to_string()),
            "This is not a YACE review repository"
        );
        Ok(())
    }
}
pub fn bundle(tree: &Tree, files: &BTreeMap<String, Vec<u8>>) -> Result<Bundle> {
    tree.validate()?;
    ensure!(
        files
            .get(MARKER)
            .is_some_and(|v| v == MARKER_CONTENT.as_bytes()),
        "Unsupported review repository marker"
    );
    let mut events = Vec::new();
    for blob in &tree.tree {
        let bytes = files.get(&blob.path).context("Review file is missing")?;
        ensure!(
            Some(bytes.len() as u64) == blob.size,
            "Review file size changed"
        );
        if event_name(&blob.path) {
            events.push(event(&blob.path, bytes)?);
        }
    }
    Ok(Bundle { version: 1, events })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_github_repository_identifiers_can_form_api_paths() {
        for input in [
            "alice/chart-reviews",
            "https://github.com/alice/chart-reviews.git",
            "git@github.com:alice/chart-reviews.git",
        ] {
            assert_eq!(
                Repository::parse(input).unwrap().api(),
                "/repos/alice/chart-reviews"
            );
        }
        for input in [
            "https://evil.example/alice/repo",
            "alice/repo/extra",
            "alice/..",
            "alice/repo?token=x",
            "alice/repo#fragment",
            "alice/repo%2fother",
            "alice/repo\nAuthorization:x",
        ] {
            assert!(Repository::parse(input).is_err(), "{input}");
        }
    }
    #[test]
    fn tree_rejects_unrelated_files_links_subdirectories_and_truncation() {
        let mut tree = Tree {
            sha: "a".repeat(40),
            truncated: false,
            tree: vec![Blob {
                path: MARKER.into(),
                mode: "100644".into(),
                kind: "blob".into(),
                sha: "b".repeat(40),
                size: Some(MARKER_CONTENT.len() as u64),
            }],
        };
        tree.validate().unwrap();
        tree.truncated = true;
        assert!(tree.validate().is_err());
        tree.truncated = false;
        for (path, mode, kind) in [
            ("notes.chart", "100644", "blob"),
            (MARKER, "120000", "blob"),
            (MARKER, "040000", "tree"),
            ("nested/file.json", "100644", "blob"),
        ] {
            let original = tree.tree[0].clone();
            tree.tree[0].path = path.into();
            tree.tree[0].mode = mode.into();
            tree.tree[0].kind = kind.into();
            assert!(tree.validate().is_err());
            tree.tree[0] = original;
        }
        tree.tree.push(tree.tree[0].clone());
        assert!(tree.validate().is_err());
    }
}
