//! Add a part without changing the recording or the chart's timing.
use crate::{
    editor::{Content, EditorNote, Project, Store},
    timing::TempoMap,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::{generation, models};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const PARTS: [(&str, &str, &str); 6] = [
    ("Guitar", "Single", "guitar"),
    ("Bass", "DoubleBass", "bass"),
    ("Drums", "Drums", "drums"),
    ("Piano (Keys)", "Keyboard", "piano"),
    ("Pro Piano (Pro Keys)", "ProKeys", "pro_piano"),
    ("Vocals", "Vocals", "vocals"),
];
pub fn instrument(track: &str) -> Result<&'static str> {
    if track == "Vocals" {
        return Ok("vocals");
    }
    let suffix = ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
        .context("Choose a supported difficulty")?;
    PARTS
        .iter()
        .find(|(_, part, _)| *part == suffix && suffix != "Vocals")
        .map(|(_, _, instrument)| *instrument)
        .context("Unsupported chart part")
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Origin {
    Authored,
    Generated { job: PathBuf, chart_sha256: String },
}

pub fn create(project: &mut Project, track: &str) -> Result<()> {
    instrument(track)?;
    if track == "Vocals" {
        ensure!(
            project.content.vocals.is_none() && !project.content.track_origins.contains_key(track),
            "Vocals already exist"
        );
        // The first note creates its scoring phrase; an empty lane has no chart yet.
    } else {
        ensure!(
            !project.content.tracks.contains_key(track),
            "This track already exists"
        );
        project.content.tracks.insert(track.into(), vec![]);
    }
    project
        .content
        .track_origins
        .insert(track.into(), Origin::Authored);
    project.selected_track = track.into();
    project.selected_note = None;
    project.selected_region = None;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    fn fingerprint(content: &Content, track: &str) -> Result<String> {
        if track == "Vocals" {
            digest(&(&content.vocals, &content.vocal_review))
        } else {
            // A link change can alter the target even if its visible notes match.
            digest(&(content.tracks.get(track), &content.difficulty_links))
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Target {
        pub version: u32,
        pub project: PathBuf,
        pub track: String,
        pub audio_sha256: String,
        pub source_chart_sha256: String,
        pub timing: TempoMap,
        pub fingerprint: String,
    }
    impl Target {
        pub fn capture(store: &Store, project: &Project, track: &str) -> Result<Self> {
            instrument(track)?;
            Ok(Self {
                version: 1,
                project: store.root.canonicalize()?,
                track: track.into(),
                audio_sha256: project.audio_sha256.clone(),
                source_chart_sha256: project.source_chart_sha256.clone(),
                timing: project.timing(store)?,
                fingerprint: fingerprint(&project.content, track)?,
            })
        }
        pub fn validate(&self, store: &Store, project: &Project) -> Result<()> {
            instrument(&self.track)?;
            ensure!(
                self.version == 1
                    && self.project == store.root.canonicalize()?
                    && self.audio_sha256 == project.audio_sha256
                    && self.source_chart_sha256 == project.source_chart_sha256
                    && self.timing == project.timing(store)?,
                "This draft belongs to a different project or recording"
            );
            ensure!(
                fingerprint(&project.content, &self.track)? == self.fingerprint,
                "This part changed during generation. Your edits and the generated draft were kept; start a new draft to replace this part."
            );
            Ok(())
        }
    }

    pub fn target(job: &Path) -> Result<Option<Target>> {
        let path = job.join("editor-target.json");
        if path.exists() {
            let mut target: Target = serde_json::from_slice(&fs::read(path)?)?;
            if target.project.is_relative() {
                target.project = job.join(&target.project).canonicalize()?;
            }
            Ok(Some(target))
        } else {
            Ok(None)
        }
    }
    pub fn timing(job: &Path, bpm: f64) -> Result<TempoMap> {
        if let Some(target) = target(job)? {
            let store = Store::new(&target.project);
            let project = store.load()?;
            target.validate(&store, &project)?;
            Ok(target.timing)
        } else {
            TempoMap::constant(bpm)
        }
    }

    pub fn create_job(
        request: generation::Request,
        output: &Path,
        target: &Target,
    ) -> Result<PathBuf> {
        let store = Store::new(&target.project);
        let project = store.load()?;
        target.validate(&store, &project)?;
        let generation::Source::Audio { path } = &request.source;
        ensure!(
            models::hash_file(path)? == target.audio_sha256,
            "Recording changed before generation"
        );
        ensure!(
            request.instruments == [instrument(&target.track)?.to_owned()],
            "Generate only the requested part"
        );
        ensure!(
            instrument(&target.track)? != "guitar"
                || request.guitar_source == store.guitar_source(&project)?,
            "Keep the chart's existing Guitar sound source"
        );
        let job = generation::create(request, output)?;
        let mut saved_target = target.clone();
        if job.parent().and_then(Path::parent) == Some(target.project.as_path()) {
            saved_target.project = PathBuf::from("../..");
        }
        generation::write_json(&job.join("editor-target.json"), &saved_target)?;
        Ok(job)
    }

    pub struct Prepared {
        target: Target,
        generated: Project,
        origin: Origin,
    }
    impl Prepared {
        pub fn load(store: &Store, project: &Project, job: &Path) -> Result<Self> {
            ensure!(
                generation::status(job)?.state == generation::State::Complete,
                "Draft is not finished"
            );
            let target = target(job)?.context("This job was not created for an open chart")?;
            target.validate(store, project)?;
            ensure!(
                models::hash_file(&store.audio_path(project)?)? == target.audio_sha256,
                "Recording changed"
            );
            let request: generation::Request =
                serde_json::from_slice(&fs::read(job.join("request.json"))?)?;
            ensure!(
                models::hash_file(&job.join("request.json"))?
                    == generation::status(job)?.request_sha256,
                "Saved generation settings changed"
            );
            ensure!(
                request.instruments == [instrument(&target.track)?.to_owned()],
                "Draft names a different part"
            );
            ensure!(
                instrument(&target.track)? != "guitar"
                    || request.guitar_source == store.guitar_source(project)?,
                "Draft uses a different Guitar sound source"
            );
            let generated_store = Store::new(&job.join("project"));
            let generated = generated_store.load()?;
            ensure!(
                generated.audio_sha256 == target.audio_sha256
                    && generated.timing(&generated_store)? == target.timing
                    && (generated.duration - project.duration).abs() <= 0.1,
                "Generated part does not match the recording and timing"
            );
            ensure!(
                models::hash_file(&generated_store.audio_path(&generated)?)? == target.audio_sha256,
                "Generated recording snapshot changed"
            );
            if target.track == "Vocals" {
                ensure!(
                    generated
                        .content
                        .vocals
                        .as_ref()
                        .is_some_and(|chart| !chart.notes.is_empty()),
                    "No vocal notes were detected; the current part was kept"
                );
            } else {
                ensure!(
                    generated
                        .content
                        .tracks
                        .get(&target.track)
                        .is_some_and(|notes| notes
                            .iter()
                            .any(|note| crate::editor::playable(&target.track, note.note.lane))),
                    "No notes were detected for this difficulty; the current part was kept"
                );
            }
            let origin = Origin::Generated {
                job: job.canonicalize()?,
                chart_sha256: generated.chart_hash(&generated_store)?,
            };
            Ok(Self {
                target,
                generated,
                origin,
            })
        }
        pub fn apply(mut self, store: &Store, project: &mut Project) -> Result<()> {
            self.target.validate(store, project)?;
            let track = self.target.track;
            if track == "Vocals" {
                project.content.vocals = self.generated.content.vocals;
                project.content.vocal_review = self.generated.content.vocal_review;
            } else {
                let notes = self
                    .generated
                    .content
                    .tracks
                    .remove(&track)
                    .context("Generated part is missing")?;
                let mut imported = Vec::with_capacity(notes.len());
                for item in notes {
                    let id = format!("edit-note-{}", project.next_id);
                    project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
                    imported.push(EditorNote {
                        id,
                        note: item.note,
                    });
                }
                project.content.tracks.insert(track.clone(), imported);
            }
            project
                .content
                .track_origins
                .insert(track.clone(), self.origin);
            project.selected_track = track;
            project.selected_note = None;
            project.selected_region = None;
            Ok(())
        }
    }

    /// Vocal-only jobs need an editable song container, but no instrumental inference.
    pub fn vocal_song(
        job: &Path,
        audio: &Path,
        request: &generation::Request,
        timing: &TempoMap,
    ) -> Result<()> {
        let stage = tempfile::tempdir_in(job)?;
        let extension = audio
            .extension()
            .and_then(|value| value.to_str())
            .context("Missing audio extension")?;
        ensure!(
            ["wav", "mp3", "ogg", "opus"].contains(&extension),
            "Expected prepared recording"
        );
        fs::copy(audio, stage.path().join(format!("song.{extension}")))?;
        fs::write(
            stage.path().join("notes.chart"),
            crate::chart::new_chart(
                &request.title,
                &request.artist,
                timing,
                &BTreeMap::from([("ExpertSingle".into(), vec![])]),
            ),
        )?;
        fs::write(
            stage.path().join("song.ini"),
            format!(
                "[song]\nname = {}\nartist = {}\n",
                request.title, request.artist
            ),
        )?;
        if job.join("isolation.json").exists() {
            fs::copy(
                job.join("isolation.json"),
                stage.path().join("chart-engine-isolation.json"),
            )?;
        }
        let supplied = request
            .song_info
            .as_ref()
            .and_then(|info| info.lyrics.clone());
        let lyrics = if supplied.is_some() || !request.lyrics {
            supplied
        } else {
            let mut meta = crate::pipeline::metadata(audio)?;
            meta.name = request.title.clone();
            meta.artist = request.artist.clone();
            crate::pipeline::lyrics(&meta)?
        };
        if let Some(lyrics) = lyrics {
            generation::write_json(&stage.path().join("chart-engine-lyrics.json"), &lyrics)?;
        }
        crate::pipeline::publish(stage.path(), &job.join("song"))
    }
}
