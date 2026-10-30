//! Exchange chart proposals and reviews without a shared server.
//! The editor audit log stores the review history.
use crate::editor::{Actor, EditorNote, Project};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub type Tracks = BTreeMap<String, Vec<EditorNote>>;

fn same_notes(a: &Tracks, b: &Tracks) -> bool {
    a.keys().eq(b.keys())
        && a.iter().all(|(track, notes)| {
            let mut left = notes.clone();
            let mut right = b[track].clone();
            let sort = |notes: &mut Vec<EditorNote>| {
                notes.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)))
            };
            sort(&mut left);
            sort(&mut right);
            left == right
        })
}

#[derive(Clone, Debug, Serialize)]
pub struct NoteChange {
    pub track: String,
    pub note_id: String,
    pub before: Option<EditorNote>,
    pub after: Option<EditorNote>,
}

pub fn diff(proposal: &Proposal) -> Vec<NoteChange> {
    let mut changes = Vec::new();
    for (track, notes) in &proposal.base {
        let old: BTreeMap<_, _> = notes.iter().map(|n| (&n.id, n)).collect();
        let new: BTreeMap<_, _> = proposal
            .proposed
            .get(track)
            .into_iter()
            .flatten()
            .map(|n| (&n.id, n))
            .collect();
        let ids: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
        for id in ids {
            if old.get(id) != new.get(id) {
                changes.push(NoteChange {
                    track: track.clone(),
                    note_id: id.clone(),
                    before: old.get(id).map(|n| (*n).clone()),
                    after: new.get(id).map(|n| (*n).clone()),
                });
            }
        }
    }
    changes
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub description: String,
    pub author: Actor,
    pub source_chart_sha256: String,
    pub audio_sha256: String,
    pub base: Tracks,
    pub proposed: Tracks,
}
impl Proposal {
    /// Content identity deliberately excludes editable title/description and actor claims.
    pub fn fingerprint(&self) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &self.source_chart_sha256,
                &self.audio_sha256,
                &self.base,
                &self.proposed,
            ))?)
        ))
    }
    pub fn between(
        base: &Project,
        edited: &Project,
        id: String,
        title: String,
        description: String,
        author: Actor,
    ) -> Result<Self> {
        ensure!(
            base.source_chart_sha256 == edited.source_chart_sha256
                && base.audio_sha256 == edited.audio_sha256,
            "Projects must share the same source chart and recording"
        );
        ensure!(
            serde_json::to_value(&base.content.vocals)?
                == serde_json::to_value(&edited.content.vocals)?,
            "Vocal changes are not supported in proposal v1; export note changes separately"
        );
        let proposal = Self {
            version: 1,
            id,
            title,
            description,
            author,
            source_chart_sha256: base.source_chart_sha256.clone(),
            audio_sha256: base.audio_sha256.clone(),
            base: base.content.tracks.clone(),
            proposed: edited.content.tracks.clone(),
        };
        record(
            &mut BTreeMap::new(),
            &Operation::Submit {
                proposal: proposal.clone(),
            },
            &proposal.author,
            0,
        )?;
        Ok(proposal)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub track: String,
    pub start_tick: u64,
    pub end_tick: u64,
    pub note_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    ResolveConflicts {
        proposal_id: String,
        fingerprint: String,
        current: Tracks,
        choices: BTreeMap<String, ConflictChoice>,
        new_proposal_id: String,
    },
    Suggest {
        proposal_id: String,
        fingerprint: String,
        suggestion_id: String,
        track: String,
        note_id: String,
        replacement: Option<EditorNote>,
        body: String,
    },
    AcceptSuggestion {
        proposal_id: String,
        suggestion_id: String,
        new_proposal_id: String,
    },
    Submit {
        proposal: Proposal,
    },
    Comment {
        proposal_id: String,
        thread_id: String,
        anchor: Option<Anchor>,
        body: String,
    },
    SetResolved {
        proposal_id: String,
        thread_id: String,
        resolved: bool,
    },
    Review {
        proposal_id: String,
        fingerprint: String,
        approve: bool,
        body: String,
    },
    Merge {
        proposal_id: String,
        fingerprint: String,
    },
    Close {
        proposal_id: String,
        reason: String,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub actor: Actor,
    pub body: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Thread {
    pub anchor: Option<Anchor>,
    pub messages: Vec<Message>,
    pub resolved: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Review {
    pub actor: Actor,
    pub fingerprint: String,
    pub approve: bool,
    pub body: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub proposal: Proposal,
    pub threads: BTreeMap<String, Thread>,
    pub reviews: Vec<Review>,
    pub status: String,
    pub suggestions: BTreeMap<String, Suggestion>,
    pub merge_receipts: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Suggestion {
    pub actor: Actor,
    pub track: String,
    pub note_id: String,
    pub replacement: Option<EditorNote>,
    pub body: String,
}

pub fn inspect(project: &Project) -> Result<BTreeMap<String, Entry>> {
    crate::collaboration_exchange::inspect(project)
}
fn nonempty(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 16_384,
        "Text must be nonempty and at most 16384 bytes"
    );
    Ok(())
}
pub(crate) fn record(
    entries: &mut BTreeMap<String, Entry>,
    op: &Operation,
    actor: &Actor,
    revision: u64,
) -> Result<()> {
    if let Operation::Submit { proposal } = op {
        ensure!(proposal.version == 1, "Unsupported proposal version");
        nonempty(&proposal.id)?;
        nonempty(&proposal.author.name)?;
        nonempty(&proposal.title)?;
        nonempty(&proposal.description)?;
        ensure!(
            !entries.contains_key(&proposal.id),
            "Proposal ID already exists; use a new ID for a revised proposal"
        );
        ensure!(
            proposal.base.keys().eq(proposal.proposed.keys()),
            "Track creation/removal is not supported in proposal v1"
        );
        for tracks in [&proposal.base, &proposal.proposed] {
            let mut ids = BTreeSet::new();
            for notes in tracks.values() {
                for note in notes {
                    nonempty(&note.id)?;
                    ensure!(ids.insert(&note.id), "Duplicate note ID");
                }
            }
        }
        ensure!(
            !same_notes(&proposal.base, &proposal.proposed),
            "Proposal has no note changes"
        );
        entries.insert(
            proposal.id.clone(),
            Entry {
                proposal: proposal.clone(),
                threads: BTreeMap::new(),
                reviews: vec![],
                status: "open".into(),
                suggestions: BTreeMap::new(),
                merge_receipts: Vec::new(),
            },
        );
        return Ok(());
    }
    let id = match op {
        Operation::Comment { proposal_id, .. }
        | Operation::SetResolved { proposal_id, .. }
        | Operation::Review { proposal_id, .. }
        | Operation::Merge { proposal_id, .. }
        | Operation::Close { proposal_id, .. } => proposal_id,
        Operation::Suggest { proposal_id, .. }
        | Operation::AcceptSuggestion { proposal_id, .. }
        | Operation::ResolveConflicts { proposal_id, .. } => proposal_id,
        Operation::Submit { .. } => unreachable!(),
    };
    let entry = entries.get_mut(id).context("Unknown proposal")?;
    ensure!(
        entry.status == "open"
            || matches!(
                op,
                Operation::Comment { .. }
                    | Operation::SetResolved { .. }
                    | Operation::Review { .. }
            ),
        "Proposal is closed"
    );
    match op {
        Operation::ResolveConflicts {
            fingerprint,
            current,
            choices,
            new_proposal_id,
            ..
        } => {
            ensure!(
                *fingerprint == entry.proposal.fingerprint()?,
                "Stale conflict resolution fingerprint"
            );
            let mut proposal = entry.proposal.clone();
            proposal.id = new_proposal_id.clone();
            proposal.author = actor.clone();
            proposal.proposed =
                resolve_tracks(&proposal.base, current, &proposal.proposed, choices)?;
            proposal.base = current.clone();
            proposal.description = format!(
                "{}\nResolved note conflicts from {}",
                proposal.description, entry.proposal.id
            );
            if same_notes(&proposal.proposed, &proposal.base) {
                let proposal_id = entry.proposal.id.clone();
                return record(
                    entries,
                    &Operation::Close {
                        proposal_id,
                        reason: "Conflict choices retain the current song; no changes to apply"
                            .into(),
                    },
                    actor,
                    revision,
                );
            }
            record(entries, &Operation::Submit { proposal }, actor, revision)?;
        }
        Operation::Suggest {
            fingerprint,
            suggestion_id,
            track,
            note_id,
            replacement,
            body,
            ..
        } => {
            ensure!(
                *fingerprint == entry.proposal.fingerprint()?,
                "Stale suggestion fingerprint"
            );
            nonempty(suggestion_id)?;
            nonempty(body)?;
            ensure!(
                !entry.suggestions.contains_key(suggestion_id),
                "Suggestion ID already exists"
            );
            let before = entry
                .proposal
                .proposed
                .get(track)
                .and_then(|notes| notes.iter().find(|n| &n.id == note_id))
                .context("Unknown suggested note")?;
            ensure!(
                replacement.as_ref() != Some(before),
                "Suggestion has no changes"
            );
            if let Some(after) = replacement {
                ensure!(
                    after.id == *note_id,
                    "Suggestion must retain stable note ID"
                );
            }
            entry.suggestions.insert(
                suggestion_id.clone(),
                Suggestion {
                    actor: actor.clone(),
                    track: track.clone(),
                    note_id: note_id.clone(),
                    replacement: replacement.clone(),
                    body: body.clone(),
                },
            );
        }
        Operation::AcceptSuggestion {
            suggestion_id,
            new_proposal_id,
            ..
        } => {
            let suggestion = entry
                .suggestions
                .get(suggestion_id)
                .context("Unknown suggestion")?
                .clone();
            let mut proposal = entry.proposal.clone();
            proposal.id = new_proposal_id.clone();
            proposal.author = actor.clone();
            proposal.description = format!(
                "{}\nSuggestion from {}: {}",
                proposal.description, suggestion.actor.name, suggestion.body
            );
            let notes = proposal
                .proposed
                .get_mut(&suggestion.track)
                .context("Missing suggestion track")?;
            notes.retain(|n| n.id != suggestion.note_id);
            if let Some(note) = suggestion.replacement {
                notes.push(note);
            }
            notes.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
            record(entries, &Operation::Submit { proposal }, actor, revision)?;
        }
        Operation::Comment {
            thread_id,
            anchor,
            body,
            ..
        } => {
            nonempty(thread_id)?;
            nonempty(body)?;
            if let Some(a) = anchor {
                ensure!(
                    a.start_tick <= a.end_tick && entry.proposal.proposed.contains_key(&a.track),
                    "Invalid comment range/track"
                );
                if let Some(id) = &a.note_id {
                    ensure!(
                        entry.proposal.proposed[&a.track]
                            .iter()
                            .any(|n| &n.id == id),
                        "Unknown anchored note"
                    );
                }
            }
            let thread = entry
                .threads
                .entry(thread_id.clone())
                .or_insert_with(|| Thread {
                    anchor: anchor.clone(),
                    messages: vec![],
                    resolved: false,
                });
            ensure!(
                serde_json::to_value(&thread.anchor)? == serde_json::to_value(anchor)?,
                "Reply must retain the thread anchor"
            );
            thread.messages.push(Message {
                actor: actor.clone(),
                body: body.clone(),
                revision,
            });
            // Any new discussion needs acknowledgement before merge.
            thread.resolved = false;
        }
        Operation::SetResolved {
            thread_id,
            resolved,
            ..
        } => {
            entry
                .threads
                .get_mut(thread_id)
                .context("Unknown thread")?
                .resolved = *resolved
        }
        Operation::Review {
            fingerprint,
            approve,
            body,
            ..
        } => {
            nonempty(body)?;
            ensure!(
                *fingerprint == entry.proposal.fingerprint()?,
                "Stale review fingerprint"
            );
            entry.reviews.retain(|r| r.actor != *actor);
            entry.reviews.push(Review {
                actor: actor.clone(),
                fingerprint: fingerprint.clone(),
                approve: *approve,
                body: body.clone(),
            });
        }
        Operation::Merge { fingerprint, .. } => {
            ensure!(
                *fingerprint == entry.proposal.fingerprint()?,
                "Stale merge fingerprint"
            );
            ensure!(
                entry.threads.values().all(|t| t.resolved),
                "Unresolved review threads"
            );
            ensure!(
                !entry.reviews.iter().any(|r| !r.approve),
                "Reviewer requested changes"
            );
            ensure!(
                entry
                    .reviews
                    .iter()
                    .any(|r| r.approve && r.actor != entry.proposal.author),
                "Independent approval required"
            );
            entry.status = "merged".into();
        }
        Operation::Close { reason, .. } => {
            nonempty(reason)?;
            entry.status = "closed".into();
        }
        Operation::Submit { .. } => unreachable!(),
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    Current,
    Proposed,
    Delete,
}
#[derive(Clone, Debug, Serialize)]
pub struct NoteConflict {
    pub key: String,
    pub track: String,
    pub note_id: String,
    pub base: Option<EditorNote>,
    pub current: Option<EditorNote>,
    pub proposed: Option<EditorNote>,
}
#[derive(Clone, Debug, Serialize)]
pub struct MergePlan {
    pub tracks: Tracks,
    pub conflicts: Vec<NoteConflict>,
}

/// Conflict plans preserve current notes until the user makes an explicit choice.
pub fn plan_merge(base: &Tracks, ours: &Tracks, theirs: &Tracks) -> Result<MergePlan> {
    ensure!(
        base.keys().eq(ours.keys()) && base.keys().eq(theirs.keys()),
        "Track layout changed; rebase proposal"
    );
    let mut merged = BTreeMap::new();
    let mut conflicts = vec![];
    for (track, notes) in base {
        let map = |notes: &Vec<EditorNote>| {
            notes
                .iter()
                .map(|n| (n.id.clone(), n.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let b = map(notes);
        let o = map(&ours[track]);
        let t = map(&theirs[track]);
        let ids: BTreeSet<_> = b.keys().chain(o.keys()).chain(t.keys()).collect();
        let mut result = vec![];
        for id in ids {
            let (bv, ov, tv) = (b.get(id), o.get(id), t.get(id));
            let value = if ov == tv || tv == bv {
                ov
            } else if ov == bv {
                tv
            } else {
                conflicts.push(NoteConflict {
                    key: format!("{track}/{id}"),
                    track: track.clone(),
                    note_id: id.clone(),
                    base: bv.cloned(),
                    current: ov.cloned(),
                    proposed: tv.cloned(),
                });
                ov
            };
            if let Some(note) = value {
                result.push(note.clone());
            }
        }
        result.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
        // Independently valid edits can collide musically even with different IDs.
        let mut attacks: BTreeMap<u64, Vec<&EditorNote>> = BTreeMap::new();
        for note in &result {
            if crate::editor::playable(track, note.note.lane) {
                attacks.entry(note.note.tick).or_default().push(note);
            }
        }
        for attack in attacks.values() {
            for (index, left) in attack.iter().enumerate() {
                for right in &attack[index + 1..] {
                    let collision = left.note.lane == right.note.lane
                        || (!track.ends_with("Drums")
                            && ((left.note.lane == 7 && right.note.lane <= 4)
                                || (right.note.lane == 7 && left.note.lane <= 4)));
                    let already_together = |map: &BTreeMap<String, EditorNote>| {
                        map.get(&left.id) == Some(*left) && map.get(&right.id) == Some(*right)
                    };
                    if collision && !already_together(&o) && !already_together(&t) {
                        for note in [*left, *right] {
                            let key = format!("{track}/{}", note.id);
                            if !conflicts.iter().any(|c| c.key == key) {
                                conflicts.push(NoteConflict {
                                    key,
                                    track: track.clone(),
                                    note_id: note.id.clone(),
                                    base: b.get(&note.id).cloned(),
                                    current: o.get(&note.id).cloned(),
                                    proposed: t.get(&note.id).cloned(),
                                });
                            }
                        }
                    }
                }
            }
        }
        merged.insert(track.clone(), result);
    }
    // Independent working copies can allocate the same ID on different tracks.
    let mut locations: BTreeMap<&str, Vec<(&str, &EditorNote)>> = BTreeMap::new();
    for (track, notes) in &merged {
        for note in notes {
            locations.entry(&note.id).or_default().push((track, note));
        }
    }
    for duplicates in locations.values().filter(|entries| entries.len() > 1) {
        for (track, note) in duplicates {
            let key = format!("{track}/{}", note.id);
            if !conflicts.iter().any(|c| c.key == key) {
                let find =
                    |tracks: &Tracks| tracks[*track].iter().find(|n| n.id == note.id).cloned();
                conflicts.push(NoteConflict {
                    key,
                    track: (*track).into(),
                    note_id: note.id.clone(),
                    base: find(base),
                    current: find(ours),
                    proposed: find(theirs),
                });
            }
        }
    }
    Ok(MergePlan {
        tracks: merged,
        conflicts,
    })
}
pub fn resolve_tracks(
    base: &Tracks,
    current: &Tracks,
    proposed: &Tracks,
    choices: &BTreeMap<String, ConflictChoice>,
) -> Result<Tracks> {
    let mut plan = plan_merge(base, current, proposed)?;
    ensure!(!plan.conflicts.is_empty(), "No note conflicts to resolve");
    ensure!(
        plan.conflicts.len() == choices.len(),
        "Choose a result for every conflicting note"
    );
    for conflict in plan.conflicts {
        let choice = choices
            .get(&conflict.key)
            .context("A conflicting note has no choice")?;
        let replacement = match choice {
            ConflictChoice::Current => conflict.current,
            ConflictChoice::Proposed => conflict.proposed,
            ConflictChoice::Delete => None,
        };
        let notes = plan.tracks.get_mut(&conflict.track).unwrap();
        notes.retain(|n| n.id != conflict.note_id);
        if let Some(note) = replacement {
            notes.push(note);
        }
        notes.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
    }
    Ok(plan.tracks)
}
/// Pure three-way merge; independent note edits combine, overlapping edits fail atomically.
pub fn merge_tracks(base: &Tracks, ours: &Tracks, theirs: &Tracks) -> Result<Tracks> {
    let plan = plan_merge(base, ours, theirs)?;
    ensure!(
        plan.conflicts.is_empty(),
        "Conflicting notes: {}. Resolve them in Collaborate.",
        plan.conflicts
            .iter()
            .map(|c| c.key.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(plan.tracks)
}

pub fn apply(project: &mut Project, operation: &Operation, actor: &Actor) -> Result<()> {
    ensure!(
        crate::collaboration_exchange::journal(project)?
            .events
            .len()
            < crate::collaboration_exchange::MAX_EVENTS,
        "Review journal is at its event limit; no history was removed"
    );
    let mut entries = inspect(project)?;
    if let Operation::ResolveConflicts { current, .. } = operation {
        ensure!(
            *current == project.content.tracks,
            "Current notes changed; refresh the conflict preview"
        );
    }
    if let Operation::Submit { proposal } = operation {
        ensure!(
            proposal.source_chart_sha256 == project.source_chart_sha256
                && proposal.audio_sha256 == project.audio_sha256,
            "Proposal belongs to another source chart or recording"
        );
    }
    record(&mut entries, operation, actor, project.revision + 1)?;
    if let Operation::Merge { proposal_id, .. } = operation {
        let p = &entries[proposal_id].proposal;
        project.content.tracks = merge_tracks(&p.base, &project.content.tracks, &p.proposed)?;
        // Imported IDs may overlap the local counter. Reserve all numeric suffixes.
        for note in project.content.tracks.values().flatten() {
            if let Some(number) = note
                .id
                .strip_prefix("edit-note-")
                .and_then(|n| n.parse::<u64>().ok())
            {
                project.next_id = project
                    .next_id
                    .max(number.checked_add(1).context("Note ID overflow")?);
            }
        }
    }
    Ok(())
}
