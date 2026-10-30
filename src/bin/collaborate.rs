//! Portable proposal exchange; all mutations use the editor's atomic command path.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    collaboration::{self, Operation, Proposal},
    collaboration_exchange as exchange,
    editor::{Action, Actor, ActorKind, Request, Store},
    review_git,
};
use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf};
#[derive(Parser)]
#[command(about = "Offline asynchronous song review and proposal exchange")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Create a dedicated review Git repository; remote is optional.
    GitCreate {
        folder: PathBuf,
        #[arg(long)]
        remote: Option<String>,
    },
    /// Clone a dedicated review repository into an empty folder.
    GitClone { remote: String, folder: PathBuf },
    /// Save, pull and publish reviews without manually invoking Git.
    GitSync {
        project: PathBuf,
        folder: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        revision: u64,
    },
    /// Resolve a review-file Git conflict using a valid saved side.
    GitResolve {
        folder: PathBuf,
        path: String,
        #[arg(long)]
        incoming: bool,
    },
    /// Export all proposals and review history into a new portable bundle.
    ExportReviews { project: PathBuf, output: PathBuf },
    /// Import a portable review bundle, preserving original authors and timestamps.
    ImportReviews {
        project: PathBuf,
        bundle: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        revision: u64,
    },
    /// Pull and publish immutable event files in a Git checkout or shared folder.
    Sync {
        project: PathBuf,
        folder: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        revision: u64,
    },
    /// Compare a saved base project to a separately edited working copy. No audio is included.
    Propose {
        base: PathBuf,
        edited: PathBuf,
        output: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: String,
        #[arg(long)]
        actor: String,
    },
    /// Import an immutable proposal JSON into the review project's audit.
    Import {
        project: PathBuf,
        proposal: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        revision: u64,
    },
    /// Show proposals, fingerprints, note differences, discussions, reviews and status.
    Show { project: PathBuf },
    /// Apply a review operation JSON using the usual expected revision guard.
    Apply {
        project: PathBuf,
        operation: PathBuf,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        revision: u64,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::GitCreate { folder, remote } => review_git::create(&folder, remote.as_deref())?,
        Command::GitClone { remote, folder } => review_git::clone_repository(&remote, &folder)?,
        Command::GitSync {
            project,
            folder,
            actor,
            revision,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&review_git::sync(
                &Store::new(&project),
                &folder,
                Actor {
                    kind: ActorKind::Human,
                    name: actor
                },
                revision
            )?)?
        ),
        Command::GitResolve {
            folder,
            path,
            incoming,
        } => review_git::resolve_file(&folder, &path, incoming)?,
        Command::ExportReviews { project, output } => {
            exchange::write_bundle(&Store::new(&project).load()?, &output)?
        }
        Command::ImportReviews {
            project,
            bundle,
            actor,
            revision,
        } => {
            let result = Store::new(&project).apply(Request {
                expected_revision: revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: actor,
                },
                action: Action::ImportCollaboration {
                    bundle: exchange::read_bundle(&bundle)?,
                },
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Sync {
            project,
            folder,
            actor,
            revision,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&exchange::sync(
                    &Store::new(&project),
                    &folder,
                    Actor {
                        kind: ActorKind::Human,
                        name: actor
                    },
                    revision
                )?)?
            );
        }
        Command::Propose {
            base,
            edited,
            output,
            id,
            title,
            description,
            actor,
        } => {
            let proposal = Proposal::between(
                &Store::new(&base).load()?,
                &Store::new(&edited).load()?,
                id,
                title,
                description,
                Actor {
                    kind: ActorKind::Human,
                    name: actor,
                },
            )?;
            ensure!(!output.exists(), "Output already exists");
            let parent = output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer_pretty(&mut temp, &proposal)?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(output)?;
        }
        Command::Show { project } => {
            let doc = Store::new(&project).load()?;
            let entries = collaboration::inspect(&doc)?;
            let proposals: Vec<_> = entries.values().map(|entry| -> Result<_> {
                let p = &entry.proposal;
                let diff = collaboration::diff(p);
                let merge = collaboration::merge_tracks(&p.base, &doc.content.tracks, &p.proposed);
                Ok(serde_json::json!({"entry":entry,"fingerprint":p.fingerprint()?,"diff":diff,"merge_conflict":merge.err().map(|e| e.to_string()),"merge_plan":collaboration::plan_merge(&p.base,&doc.content.tracks,&p.proposed).ok()}))
            }).collect::<Result<_>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"revision":doc.revision,"proposals":proposals})
                )?
            );
        }
        Command::Import {
            project,
            proposal,
            actor,
            revision,
        } => {
            let proposal = serde_json::from_slice(&fs::read(proposal)?)?;
            apply(project, Operation::Submit { proposal }, actor, revision)?;
        }
        Command::Apply {
            project,
            operation,
            actor,
            revision,
        } => {
            let operation = serde_json::from_slice(&fs::read(operation)?)
                .context("Invalid review operation")?;
            apply(project, operation, actor, revision)?;
        }
    }
    Ok(())
}
fn apply(
    project: PathBuf,
    operation: Operation,
    name: String,
    expected_revision: u64,
) -> Result<()> {
    let result = Store::new(&project).apply(Request {
        expected_revision,
        actor: Actor {
            kind: ActorKind::Human,
            name,
        },
        action: Action::Collaborate { operation },
    })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
