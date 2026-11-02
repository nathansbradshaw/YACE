//! Recording and timeline context for projects which start without audio.
//! Legacy recorded projects/checkpoints retain their existing representation.
use crate::{
    editor::{Content, Project},
    review_regions::{Policy, ReviewMap},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub fn chart_end(chart: &crate::chart::Chart) -> Result<f64> {
    let mut end = 0.;
    for section in &chart.sections {
        if section.name == "Song" {
            continue;
        }
        for line in chart.raw[section.body_start..section.body_end].lines() {
            let Some((tick, event)) = line.split_once('=') else {
                continue;
            };
            let Ok(tick) = tick.trim().parse::<u64>() else {
                continue;
            };
            let words: Vec<_> = event.split_whitespace().collect();
            let length = if matches!(words.first().copied(), Some("N" | "S")) {
                words
                    .get(2)
                    .context("Malformed chart event length")?
                    .parse::<u64>()?
            } else {
                0
            };
            let seconds = chart
                .timing
                .seconds(tick.checked_add(length).context("Chart extent overflow")?);
            ensure!(seconds.is_finite(), "Nonfinite chart timeline extent");
            end = f64::max(end, seconds);
        }
    }
    Ok(end)
}
pub fn inferred_duration(chart: &crate::chart::Chart, ini: &str) -> Result<f64> {
    let end = chart_end(chart)?;
    let declared = crate::song_metadata::ini_fields(ini)
        .get("song_length")
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(0) as f64
        / 1000.;
    let duration = if declared > 0. {
        declared.max(end)
    } else {
        60f64.max(end + 8.)
    };
    crate::timeline_clock::TimelineClock::new(duration, 0.)?;
    Ok(duration)
}
pub fn project_end(project: &Project, store: &crate::editor::Store) -> Result<f64> {
    let chart = crate::chart::Chart::parse(project.chart_text(store)?)?;
    let mut end = chart_end(&chart)?;
    if let Some(vocals) = &project.content.vocals {
        for tick in vocals
            .notes
            .iter()
            .map(|note| note.end)
            .chain(vocals.phrases.iter().map(|phrase| phrase.end))
        {
            let seconds = chart.timing.seconds(tick);
            ensure!(seconds.is_finite(), "Nonfinite vocal timeline extent");
            end = end.max(seconds);
        }
    }
    Ok(end)
}
pub fn resize(project: &mut Project, duration: f64) -> Result<()> {
    ensure!(
        !project.has_recording(),
        "Timeline length follows the attached recording"
    );
    crate::timeline_clock::TimelineClock::new(duration, 0.)?;
    let basis = project
        .recording
        .as_mut()
        .context("No chart-only timeline context")?;
    basis.duration = duration;
    basis.review.duration_seconds = duration;
    project.apply_recording_context()?;
    project.transport.position = project.transport.position.min(duration);
    if project
        .transport
        .loop_range
        .is_some_and(|range| range.end > duration)
    {
        project.transport.loop_range = None;
    }
    Ok(())
}

const AUDIO_EXTENSIONS: [&str; 8] = ["ogg", "opus", "mp3", "flac", "wav", "m4a", "aiff", "aac"];
pub fn attached_filename(name: &str) -> bool {
    let parts: Vec<_> = std::path::Path::new(name).components().collect();
    if parts.len() != 5 {
        return false;
    }
    let parts: Option<Vec<_>> = parts
        .iter()
        .map(|part| {
            if let std::path::Component::Normal(part) = part {
                part.to_str()
            } else {
                None
            }
        })
        .collect();
    parts.is_some_and(|parts| {
        parts[0] == ".yace"
            && parts[1] == "audio"
            && parts[2] == "recordings"
            && parts[3].len() == 64
            && parts[3].bytes().all(|b| b.is_ascii_hexdigit())
            && AUDIO_EXTENSIONS
                .iter()
                .any(|ext| parts[4] == format!("song.{ext}"))
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedRecording {
    pub version: u32,
    pub recording: Recording,
    pub duration: f64,
}
pub fn prepare_recording(
    source: &std::path::Path,
    root: &std::path::Path,
) -> Result<PreparedRecording> {
    use crate::{models, storage as fs};
    let sha256 = models::hash_file(source)?;
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        AUDIO_EXTENSIONS.contains(&extension.as_str()),
        "Choose a supported audio recording"
    );
    #[cfg(not(target_arch = "wasm32"))]
    let duration = crate::pipeline::metadata(source)?.duration;
    #[cfg(target_arch = "wasm32")]
    let duration = models::duration(source)?;
    crate::timeline_clock::TimelineClock::new(duration, 0.)?;
    let filename = format!(".yace/audio/recordings/{sha256}/song.{extension}");
    let target = root.join("source").join(&filename);
    let folder = target.parent().context("Recording needs a parent")?;
    fs::create_dir_all(folder)?;
    if !fs::is_file(&target) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
            std::io::copy(&mut fs::File::open(source)?, temporary.as_file_mut())?;
            temporary.as_file().sync_all()?;
            ensure!(
                models::hash_file(temporary.path())? == sha256,
                "Recording changed during retention"
            );
            temporary
                .persist_noclobber(&target)
                .map_err(|error| error.error)?;
        }
        #[cfg(target_arch = "wasm32")]
        fs::copy(source, &target)?;
    }
    ensure!(
        models::hash_file(&target)? == sha256 && models::hash_file(source)? == sha256,
        "Retained recording changed"
    );
    let prepared = PreparedRecording {
        version: 1,
        recording: Recording { filename, sha256 },
        duration,
    };
    let receipt = target.with_extension("json");
    let prepared = if fs::is_file(&receipt) {
        let previous: PreparedRecording = serde_json::from_slice(&fs::read(&receipt)?)?;
        ensure!(
            previous.version == 1
                && previous.recording.filename == prepared.recording.filename
                && previous.recording.sha256 == prepared.recording.sha256
                && (previous.duration - duration).abs() <= 0.1,
            "Recording receipt differs"
        );
        previous
    } else {
        fs::write(&receipt, serde_json::to_vec_pretty(&prepared)?)?;
        prepared
    };
    Ok(prepared)
}
pub fn verify_recording(root: &std::path::Path, prepared: &PreparedRecording) -> Result<()> {
    use crate::{models, storage as fs};
    ensure!(
        prepared.version == 1 && attached_filename(&prepared.recording.filename),
        "Invalid recording attachment"
    );
    let components: Vec<_> = std::path::Path::new(&prepared.recording.filename)
        .components()
        .collect();
    ensure!(
        components[3].as_os_str() == prepared.recording.sha256.as_str(),
        "Recording cache identity differs"
    );
    crate::timeline_clock::TimelineClock::new(prepared.duration, 0.)?;
    let path = root.join("source").join(&prepared.recording.filename);
    ensure!(
        models::hash_file(&path)? == prepared.recording.sha256,
        "Attached recording changed"
    );
    let saved: PreparedRecording = serde_json::from_slice(&fs::read(path.with_extension("json"))?)?;
    ensure!(
        saved.version == 1
            && saved.recording.filename == prepared.recording.filename
            && saved.recording.sha256 == prepared.recording.sha256
            && saved.duration == prepared.duration,
        "Recording attachment differs from its prepared receipt"
    );
    #[cfg(not(target_arch = "wasm32"))]
    ensure!(
        (crate::pipeline::metadata(&path)?.duration - prepared.duration).abs() <= 0.1,
        "Recording duration differs from its actual audio"
    );
    Ok(())
}
pub fn blank_song(title: &str, artist: &str, bpm: f64, duration: f64) -> Result<(String, String)> {
    crate::timeline_clock::TimelineClock::new(duration, 0.)?;
    ensure!(
        duration >= 1.,
        "Choose at least one second of chart timeline"
    );
    let (ini, chart) = crate::song_metadata::blank_song(title, artist, bpm)?;
    Ok((
        crate::song_metadata::update_ini(
            &ini,
            &BTreeMap::from([(
                "song_length".into(),
                ((duration * 1000.).round() as u64).to_string(),
            )]),
        ),
        chart,
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub filename: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    pub version: u32,
    pub recording: Option<Recording>,
    pub duration: f64,
    pub review: ReviewMap,
}
impl Basis {
    pub fn manual(
        source_chart_sha256: &str,
        duration: f64,
        recording: Option<Recording>,
    ) -> Result<Self> {
        let mut inputs = BTreeMap::from([("chart".into(), source_chart_sha256.into())]);
        if let Some(recording) = &recording {
            inputs.insert("original_audio".into(), recording.sha256.clone());
        }
        let mut review = ReviewMap::build(
            duration,
            Policy::default(),
            inputs,
            Some(BTreeSet::new()),
            BTreeSet::new(),
            Vec::new(),
        )?;
        review.musical_acceptance = "unverified".into();
        review.coverage_limits.push(if recording.is_some() {
            "Manual chart with an attached recording. No transcription or musical approval is implied."
        } else {
            "Chart-only timeline. No recording, audio analysis or musical approval is available."
        }.into());
        let basis = Self {
            version: 1,
            recording,
            duration,
            review,
        };
        basis.validate(source_chart_sha256)?;
        Ok(basis)
    }
    pub fn audio_hash(&self) -> &str {
        self.recording
            .as_ref()
            .map_or("", |recording| recording.sha256.as_str())
    }
    pub fn validate(&self, source_chart_sha256: &str) -> Result<()> {
        ensure!(
            source_chart_sha256.len() == 64
                && source_chart_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Recording context requires the immutable source chart hash"
        );
        ensure!(self.version == 1, "Unsupported recording context");
        crate::timeline_clock::TimelineClock::new(self.duration, 0.)?;
        ensure!(
            self.review.version == 1
                && self.review.duration_seconds == self.duration
                && self
                    .review
                    .input_sha256
                    .get("chart")
                    .is_some_and(|hash| hash == source_chart_sha256),
            "Recording context differs from the project source or timeline"
        );
        if let Some(recording) = &self.recording {
            ensure!(
                crate::stem_review::conventional::valid_audio_filename(&recording.filename)
                    && recording.sha256.len() == 64
                    && recording.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid attached recording identity"
            );
            ensure!(
                self.review.input_sha256.get("original_audio") == Some(&recording.sha256),
                "Review context belongs to another recording"
            );
        } else {
            ensure!(
                !self.review.input_sha256.contains_key("original_audio"),
                "A chart-only project cannot claim recording evidence"
            );
        }
        Ok(())
    }
    /// Keep the established public fields as a projection of this context.
    pub fn apply(&self, project: &mut Project) -> Result<()> {
        self.validate(&project.source_chart_sha256)?;
        project.audio_filename = self
            .recording
            .as_ref()
            .map_or_else(String::new, |r| r.filename.clone());
        project.audio_sha256 = self.audio_hash().into();
        project.duration = self.duration;
        project.review = self.review.clone();
        Ok(())
    }
}

pub struct Checkpoint {
    pub content: Content,
    pub recording: Option<Basis>,
}
impl Checkpoint {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let Some(recording) = &self.recording else {
            // Preserve the legacy byte encoding and content-addressed IDs.
            return Ok(serde_json::to_vec(&self.content)?);
        };
        let mut value = serde_json::to_value(&self.content)?;
        value
            .as_object_mut()
            .context("Checkpoint must be an object")?
            .insert("recording_context".into(), serde_json::to_value(recording)?);
        Ok(serde_json::to_vec(&value)?)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
        let recording = value
            .as_object_mut()
            .context("Checkpoint must be an object")?
            .remove("recording_context")
            .map(serde_json::from_value)
            .transpose()?
            .flatten();
        Ok(Self {
            content: serde_json::from_value(value)?,
            recording,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn content() -> Content {
        serde_json::from_str(r#"{"tracks":{},"decisions":[]}"#).unwrap()
    }
    #[test]
    fn legacy_checkpoint_bytes_and_ids_do_not_change() {
        let content = content();
        let old = serde_json::to_vec(&content).unwrap();
        let encoded = Checkpoint {
            content,
            recording: None,
        }
        .encode()
        .unwrap();
        assert_eq!(encoded, old);
        let decoded = Checkpoint::decode(&old).unwrap();
        assert!(decoded.recording.is_none());
        assert_eq!(decoded.encode().unwrap(), old);
    }
    #[test]
    fn timeline_checkpoint_retains_context_without_claiming_audio_evidence() {
        let basis = Basis::manual(&"a".repeat(64), 240., None).unwrap();
        assert_eq!(basis.audio_hash(), "");
        assert!(!basis.review.input_sha256.contains_key("original_audio"));
        let encoded = Checkpoint {
            content: content(),
            recording: Some(basis),
        }
        .encode()
        .unwrap();
        let decoded = Checkpoint::decode(&encoded).unwrap();
        assert_eq!(decoded.recording.as_ref().unwrap().duration, 240.);
        assert_eq!(decoded.encode().unwrap(), encoded);
        let mut invalid: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        invalid["recording_context"]["review"]["input_sha256"]["original_audio"] =
            "b".repeat(64).into();
        let decoded = Checkpoint::decode(&serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(
            decoded
                .recording
                .unwrap()
                .validate(&"a".repeat(64))
                .is_err()
        );
    }
    #[test]
    fn recording_context_rejects_wrong_source_and_traversal() {
        let recording = Recording {
            filename: "song.wav".into(),
            sha256: "b".repeat(64),
        };
        let mut basis = Basis::manual(&"a".repeat(64), 10., Some(recording)).unwrap();
        assert!(basis.validate(&"c".repeat(64)).is_err());
        basis.recording.as_mut().unwrap().filename = "../song.wav".into();
        assert!(basis.validate(&"a".repeat(64)).is_err());
        assert!(Basis::manual("", 10., None).is_err());
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), feature = "studio"))]
#[path = "project_recording/tests.rs"]
mod workflows;
