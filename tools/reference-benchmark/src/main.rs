//! Optional private reference-pack extraction and predeclared split inventory.
//! LGPL-3.0-only; see NOTICE.md. Never writes to original song archives.
mod archive;
mod mix;
mod split;
use anyhow::{Context, Result, ensure};
use archive::{Archive, sha};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Serialize)]
struct Song {
    archive: PathBuf,
    archive_sha256: String,
    metadata: BTreeMap<String, String>,
    files: Vec<archive::Entry>,
    midi_tracks: Vec<String>,
    split: &'static str,
}
fn split(artist: &str) -> &'static str {
    if ["good kid", "pinegrove", "jonathan coulton"]
        .contains(&artist.trim().to_lowercase().as_str())
    {
        "development"
    } else {
        "held_out"
    }
}
fn song(path: &Path) -> Result<Song> {
    let a = Archive::open(path)?;
    let midi_tracks = if let Some(bytes) = a.get("notes.mid") {
        let midi = midly::Smf::parse(&bytes)?;
        midi.tracks
            .iter()
            .flat_map(|t| {
                t.iter().filter_map(|e| match &e.kind {
                    midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(n)) => {
                        Some(String::from_utf8_lossy(n).into_owned())
                    }
                    _ => None,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let s = split(a.metadata.get("artist").map(String::as_str).unwrap_or(""));
    Ok(Song {
        archive: path.to_path_buf(),
        archive_sha256: a.sha256,
        metadata: a.metadata,
        files: a.entries,
        midi_tracks,
        split: s,
    })
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("select-fresh") => {
            ensure!(
                args.len() == 4,
                "select-fresh INVENTORY EXPOSURE_HISTORY NEW_MANIFEST"
            );
            let inventory = std::fs::read(&args[1])?;
            let history = std::fs::read(&args[2])?;
            let selection = split::fresh(
                &serde_json::from_slice(&inventory)?,
                &sha(&inventory),
                &serde_json::from_slice(&history)?,
                &sha(&history),
            )?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[3])?
                .write_all(&serde_json::to_vec_pretty(&selection)?)?;
            println!(
                "{}",
                serde_json::json!({"songs":6,"manifest":args[3],"note_values_read":false})
            );
        }
        Some("prepare-heldout") => {
            ensure!(
                args.len() == 3,
                "prepare-heldout FROZEN_BATCH NEW_DIRECTORY"
            );
            let bytes = std::fs::read(&args[1])?;
            let manifest: serde_json::Value = serde_json::from_slice(&bytes)?;
            let songs = manifest["songs"]
                .as_array()
                .context("missing frozen songs")?;
            ensure!(songs.len() == 6, "expected six-song frozen batch");
            let destination = Path::new(&args[2]);
            ensure!(!destination.exists(), "refusing existing batch directory");
            // Verify every original identity before creating any output.
            for song in songs {
                ensure!(song["split"] == "held_out", "non-held-out entry");
                let path = song["archive"].as_str().context("missing archive path")?;
                ensure!(
                    Archive::open(Path::new(path))?.sha256 == song["archive_sha256"],
                    "frozen archive changed"
                );
            }
            std::fs::create_dir(destination)?;
            let executable = std::env::current_exe()?;
            for (index, song) in songs.iter().enumerate() {
                let path = song["archive"].as_str().unwrap();
                let output = destination.join(format!("heldout-{:02}", index + 1));
                ensure!(
                    std::process::Command::new(&executable)
                        .arg("extract")
                        .arg(path)
                        .arg(&output)
                        .status()?
                        .success(),
                    "extraction failed"
                );
                ensure!(
                    std::process::Command::new(&executable)
                        .arg("mix")
                        .arg(&output)
                        .arg(output.join("source.wav"))
                        .status()?
                        .success(),
                    "mix failed"
                );
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination.join("frozen-batch.json"))?
                .write_all(&bytes)?;
            println!(
                "{}",
                serde_json::json!({"prepared":destination,"songs":songs.len(),"frozen_batch_sha256":sha(&bytes),"inference_performed":false})
            );
        }
        Some("select-heldout") => {
            ensure!(
                args.len() == 3,
                "select-heldout FROZEN_INVENTORY NEW_MANIFEST_JSON"
            );
            let bytes = std::fs::read(&args[1])?;
            let inventory: serde_json::Value = serde_json::from_slice(&bytes)?;
            let selection = split::heldout(&inventory, &sha(&bytes))?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[2])?
                .write_all(&serde_json::to_vec_pretty(&selection)?)?;
            println!(
                "{}",
                serde_json::json!({"selected":selection["songs"].as_array().unwrap().len(),"selection":args[2]})
            );
        }
        Some("mix") => {
            ensure!(args.len() == 3, "mix EXTRACTED_DIRECTORY NEW_OUTPUT_WAV");
            mix::create(Path::new(&args[1]), Path::new(&args[2]))?;
        }
        Some("inventory") => {
            ensure!(args.len() == 3, "inventory PACK_DIRECTORY OUTPUT_JSON");
            let mut paths = std::fs::read_dir(&args[1])?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|x| x.to_str()),
                        Some("yargsong" | "sng")
                    )
                })
                .collect::<Vec<_>>();
            paths.sort();
            let mut songs = Vec::new();
            for path in paths {
                songs.push(song(&path).with_context(|| format!("reading {}", path.display()))?);
            }
            let manifest = serde_json::json!({"version":1,"split_rule":"Before generator evaluation, all Good Kid, Pinegrove, Jonathan Coulton songs are development; every other artist held_out. Exclude generated drafts. Eligibility requires playable source chart and matching bundled audio; no scores used in split.","inference_leakage_rule":"No authored note timing, vocal word timing, fret or MIDI targets enter generation. Optional supplied lyric TEXT is a separate reference-text condition.","songs":songs});
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[2])?
                .write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            println!(
                "{}",
                serde_json::json!({"songs":manifest["songs"].as_array().unwrap().len(),"inventory":args[2]})
            );
        }
        Some("extract") => {
            ensure!(args.len() == 3, "extract ARCHIVE NEW_DESTINATION");
            let input = Path::new(&args[1]);
            let output = Path::new(&args[2]);
            let a = Archive::open(input)?;
            ensure!(
                !a.entries
                    .iter()
                    .any(|e| e.name.eq_ignore_ascii_case("reference-source.json")),
                "archive entry collides with reserved extraction receipt"
            );
            a.extract(output)?;
            let assets=a.entries.iter().map(|e|serde_json::json!({"name":e.name,"bytes":e.length,"sha256":sha(&a.data(e))})).collect::<Vec<_>>();
            let metadata_ini = output.join("song.ini");
            let manifest = serde_json::json!({"version":1,"source_archive":input,"source_archive_sha256":a.sha256,"private_assets_do_not_commit":true,"metadata":a.metadata,"assets":assets,"song_ini_sha256":sha(&std::fs::read(metadata_ini)?),"identity":"Chart and audio extracted from the same immutable archive. This binds the recording/version; metadata alone is not identity proof.","split":split(a.metadata.get("artist").map(String::as_str).unwrap_or(""))});
            std::fs::write(
                output.join("reference-source.json"),
                serde_json::to_vec_pretty(&manifest)?,
            )?;
            println!(
                "{}",
                serde_json::json!({"extracted":output,"files":a.entries.len(),"archive_sha256":a.sha256})
            );
        }
        _ => anyhow::bail!(
            "Use inventory PACK_DIRECTORY NEW_OUTPUT_JSON, extract ARCHIVE NEW_DESTINATION, mix EXTRACTED_DIRECTORY NEW_OUTPUT_WAV, select-heldout FROZEN_INVENTORY NEW_MANIFEST_JSON, select-fresh INVENTORY EXPOSURE_HISTORY NEW_MANIFEST, or prepare-heldout FROZEN_BATCH NEW_DIRECTORY"
        ),
    };
    Ok(())
}
