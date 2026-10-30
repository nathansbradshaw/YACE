//! Content-addressed review DAG. Exchange files are immutable; project.json stays local.
use crate::{
    collaboration::{self, Entry, Operation},
    editor::{Action, Actor, Project, Request, Store},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

pub const MAX_EVENTS: usize = 2000;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;
type Ancestors = BTreeMap<String, BTreeSet<String>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub source_chart_sha256: String,
    pub audio_sha256: String,
    pub parents: BTreeSet<String>,
    pub actor: Actor,
    pub at_ms: u64,
    pub origin_revision: u64,
    pub operation: Operation,
}
impl Event {
    pub(crate) fn valid_checksum(&self) -> Result<bool> {
        Ok(self.id == self.digest()?)
    }
    fn digest(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &self.source_chart_sha256,
                &self.audio_sha256,
                &self.parents,
                &self.actor,
                self.at_ms,
                self.origin_revision,
                &self.operation,
            ))?)
        ))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    pub events: Vec<Event>,
}
#[derive(Default)]
pub struct Journal {
    pub events: BTreeMap<String, Event>,
    pub local_merges: BTreeSet<String>,
}
impl Journal {
    fn insert(&mut self, event: Event) -> Result<()> {
        ensure!(
            event.id == event.digest()?,
            "Review event checksum mismatch"
        );
        if let Some(old) = self.events.get(&event.id) {
            ensure!(
                serde_json::to_value(old)? == serde_json::to_value(&event)?,
                "Event ID collision"
            );
        } else {
            self.events.insert(event.id.clone(), event);
        }
        ensure!(
            self.events.len() <= MAX_EVENTS,
            "Review journal exceeds {MAX_EVENTS} events"
        );
        Ok(())
    }
    fn heads(&self) -> BTreeSet<String> {
        let mut heads: BTreeSet<_> = self.events.keys().cloned().collect();
        for e in self.events.values() {
            for p in &e.parents {
                heads.remove(p);
            }
        }
        heads
    }
    pub fn bundle(&self) -> Bundle {
        Bundle {
            version: 1,
            events: self.events.values().cloned().collect(),
        }
    }
}
pub fn journal(project: &Project) -> Result<Journal> {
    let mut result = Journal::default();
    for entry in &project.audit {
        match &entry.action {
            Action::Collaborate { operation } => {
                let mut e = Event {
                    id: String::new(),
                    source_chart_sha256: project.source_chart_sha256.clone(),
                    audio_sha256: project.audio_sha256.clone(),
                    parents: result.heads(),
                    actor: entry.actor.clone(),
                    at_ms: entry.at_ms,
                    origin_revision: entry.revision,
                    operation: operation.clone(),
                };
                e.id = e.digest()?;
                if matches!(operation, Operation::Merge { .. }) {
                    result.local_merges.insert(e.id.clone());
                }
                result.insert(e)?;
            }
            Action::ImportCollaboration { bundle } => {
                ensure!(bundle.version == 1, "Unsupported review bundle version");
                for event in &bundle.events {
                    result.insert(event.clone())?;
                }
            }
            _ => (),
        }
    }
    Ok(result)
}
// Stable topological order and complete ancestor sets. Missing dependencies never disappear silently.
fn ordered(events: &BTreeMap<String, Event>) -> Result<(Vec<&Event>, Ancestors)> {
    let mut order = Vec::new();
    let mut ancestors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    while order.len() < events.len() {
        let ready: Vec<_> = events
            .values()
            .filter(|e| {
                !ancestors.contains_key(&e.id)
                    && e.parents.iter().all(|p| ancestors.contains_key(p))
            })
            .collect();
        ensure!(
            !ready.is_empty(),
            "Review events have missing parents or a dependency cycle"
        );
        for event in ready {
            let mut seen = event.parents.clone();
            for parent in &event.parents {
                seen.extend(ancestors[parent].clone());
            }
            ancestors.insert(event.id.clone(), seen);
            order.push(event);
        }
    }
    Ok((order, ancestors))
}
fn proposal_id(operation: &Operation) -> &str {
    match operation {
        Operation::Submit { proposal } => &proposal.id,
        Operation::Comment { proposal_id, .. }
        | Operation::SetResolved { proposal_id, .. }
        | Operation::Review { proposal_id, .. }
        | Operation::Merge { proposal_id, .. }
        | Operation::Close { proposal_id, .. }
        | Operation::Suggest { proposal_id, .. }
        | Operation::AcceptSuggestion { proposal_id, .. }
        | Operation::ResolveConflicts { proposal_id, .. } => proposal_id,
    }
}
fn project_events(
    order: &[&Event],
    ancestors: &BTreeMap<String, BTreeSet<String>>,
    local_merges: &BTreeSet<String>,
) -> Result<BTreeMap<String, Entry>> {
    let mut state: BTreeMap<String, Entry> = BTreeMap::new();
    for event in order {
        let id = proposal_id(&event.operation);
        let status = state.get(id).map(|s| s.status.clone());
        if let Some(entry) = state.get_mut(id) {
            entry.status = "open".into();
        }
        match &event.operation {
            Operation::Submit { proposal } if state.contains_key(&proposal.id) => {
                ensure!(
                    serde_json::to_value(&state[&proposal.id].proposal)?
                        == serde_json::to_value(proposal)?,
                    "Concurrent proposal ID collision"
                );
                state.get_mut(&proposal.id).unwrap().status =
                    status.unwrap_or_else(|| "open".into());
            }
            Operation::Merge { .. } => {
                let entry = state.get_mut(id).context("Merge without proposal")?;
                entry.merge_receipts.push(serde_json::json!({"event_id":event.id,"actor":event.actor,"at_ms":event.at_ms,"applied_in_this_project":local_merges.contains(&event.id)}));
                entry.status = if local_merges.contains(&event.id) {
                    "merged".into()
                } else {
                    status.unwrap_or_else(|| "open".into())
                };
            }
            _ => {
                collaboration::record(
                    &mut state,
                    &event.operation,
                    &event.actor,
                    event.origin_revision,
                )?;
                if !matches!(event.operation, Operation::Close { .. })
                    && let Some(status) = status
                    && status != "open"
                {
                    state.get_mut(id).unwrap().status = status;
                }
            }
        }
    }
    // A causal successor replaces an actor's review; concurrent reviews are all retained.
    for (id, entry) in &mut state {
        let reviews: Vec<_> = order
            .iter()
            .filter(
                |e| matches!(&e.operation,Operation::Review { proposal_id,.. } if proposal_id==id),
            )
            .copied()
            .collect();
        entry.reviews.clear();
        for e in &reviews {
            if reviews
                .iter()
                .any(|later| later.actor == e.actor && ancestors[&later.id].contains(&e.id))
            {
                continue;
            }
            if let Operation::Review {
                fingerprint,
                approve,
                body,
                ..
            } = &e.operation
            {
                entry.reviews.push(collaboration::Review {
                    actor: e.actor.clone(),
                    fingerprint: fingerprint.clone(),
                    approve: *approve,
                    body: body.clone(),
                });
            }
        }
        for (thread_id, thread) in &mut entry.threads {
            let updates: Vec<_> = order
                .iter()
                .filter(|e| match &e.operation {
                    Operation::Comment {
                        proposal_id,
                        thread_id: t,
                        ..
                    }
                    | Operation::SetResolved {
                        proposal_id,
                        thread_id: t,
                        ..
                    } => proposal_id == id && t == thread_id,
                    _ => false,
                })
                .copied()
                .collect();
            thread.resolved = updates
                .iter()
                .filter(|e| {
                    !updates
                        .iter()
                        .any(|later| ancestors[&later.id].contains(&e.id))
                })
                .all(|e| matches!(e.operation, Operation::SetResolved { resolved: true, .. }));
        }
    }
    Ok(state)
}
pub fn inspect(project: &Project) -> Result<BTreeMap<String, Entry>> {
    let j = journal(project)?;
    let (order, ancestors) = ordered(&j.events)?;
    project_events(&order, &ancestors, &j.local_merges)
}
/// Validate each imported action against exactly the history its author had seen.
pub fn validate_import(project: &Project, bundle: &Bundle) -> Result<bool> {
    ensure!(
        bundle.version == 1 && bundle.events.len() <= MAX_EVENTS,
        "Unsupported/oversized review bundle"
    );
    let mut j = journal(project)?;
    let before = j.events.len();
    let known: BTreeSet<_> = j.events.keys().cloned().collect();
    for e in &bundle.events {
        ensure!(
            e.source_chart_sha256 == project.source_chart_sha256
                && e.audio_sha256 == project.audio_sha256,
            "Review event belongs to another song/recording"
        );
        ensure!(
            !e.actor.name.trim().is_empty() && e.actor.name.len() <= 200,
            "Invalid original actor"
        );
        j.insert(e.clone())?;
    }
    let (order, ancestors) = ordered(&j.events)?;
    for event in &order {
        if known.contains(&event.id) {
            continue;
        }
        let context: Vec<_> = order
            .iter()
            .filter(|p| ancestors[&event.id].contains(&p.id))
            .copied()
            .collect();
        let mut state = project_events(&context, &ancestors, &BTreeSet::new())?;
        if let Operation::Submit { proposal } = &event.operation {
            ensure!(
                proposal.source_chart_sha256 == project.source_chart_sha256
                    && proposal.audio_sha256 == project.audio_sha256,
                "Proposal belongs to another song"
            );
        }
        if let Operation::Submit { proposal } = &event.operation
            && let Some(entry) = state.get(&proposal.id)
        {
            ensure!(
                serde_json::to_value(&entry.proposal)? == serde_json::to_value(proposal)?,
                "Proposal ID collision"
            );
            continue;
        }
        collaboration::record(
            &mut state,
            &event.operation,
            &event.actor,
            event.origin_revision,
        )?;
    }
    // Also reject collisions between concurrent proposal IDs, thread anchors, or suggestion IDs.
    project_events(&order, &ancestors, &j.local_merges)?;
    Ok(j.events.len() != before)
}

pub fn export_bundle(project: &Project) -> Result<Bundle> {
    Ok(journal(project)?.bundle())
}
pub fn write_bundle(project: &Project, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let bytes = serde_json::to_vec_pretty(&export_bundle(project)?)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "Review bundle exceeds 16 MiB; use the event-directory exchange instead"
    );
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    Ok(())
}
pub fn read_bundle(path: &Path) -> Result<Bundle> {
    ensure!(
        fs::metadata(path)?.len() <= MAX_FILE_BYTES,
        "Review bundle is too large"
    );
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
struct FolderLock(fs::File);
impl Drop for FolderLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
/// Pull immutable events, then publish local events. Never edits project files in the exchange folder.
pub fn sync(
    store: &Store,
    folder: &Path,
    actor: Actor,
    expected_revision: u64,
) -> Result<serde_json::Value> {
    fs::create_dir_all(folder)?;
    ensure!(
        folder.canonicalize()? != store.root.canonicalize()?,
        "Choose a separate exchange folder, not the live project folder"
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("exchange.lock"))?;
    lock.try_lock()
        .context("Review exchange is busy; retry after the other writer finishes")?;
    let _lock = FolderLock(lock);
    let mut events = vec![];
    let mut total_bytes = 0u64;
    for item in fs::read_dir(folder)? {
        let path = item?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy();
        ensure!(
            stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()),
            "Unexpected JSON file in review exchange"
        );
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "Review events must be ordinary files"
        );
        let size = fs::metadata(&path)?.len();
        total_bytes = total_bytes
            .checked_add(size)
            .context("Exchange size overflow")?;
        ensure!(
            total_bytes <= MAX_DIRECTORY_BYTES,
            "Review directory exceeds 64 MiB"
        );
        ensure!(size <= MAX_FILE_BYTES, "Review event is too large");
        let event: Event = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(event.id == stem, "Review filename does not match event ID");
        events.push(event);
        ensure!(events.len() <= MAX_EVENTS, "Too many review event files");
    }
    let applied = store.apply(Request {
        expected_revision,
        actor,
        action: Action::ImportCollaboration {
            bundle: Bundle { version: 1, events },
        },
    })?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join(".gitignore"))
    {
        Ok(mut ignore) => {
            ignore.write_all(b"/exchange.lock\n/.tmp*\n")?;
            ignore.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let doc = store.load()?;
    let bundle = export_bundle(&doc)?;
    for event in &bundle.events {
        let path = folder.join(format!("{}.json", event.id));
        let bytes = serde_json::to_vec_pretty(event)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "A review event exceeds 16 MiB; event history was retained locally"
        );
        if path.exists() {
            continue;
        }
        let mut temp = tempfile::NamedTempFile::new_in(folder)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => (),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    serde_json::from_slice::<serde_json::Value>(&fs::read(&path)?)?
                        == serde_json::to_value(event)?,
                    "Concurrent event file differs"
                );
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(
        serde_json::json!({"revision":applied.revision,"events":bundle.events.len(),"folder":folder}),
    )
}
