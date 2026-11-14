//! Independent, hash-bound venue state; it never takes ownership of chart notes
//! or ordinary instrument text. Playable exports carry verified editable IDs.
use super::*;
use crate::{
    editor::Project,
    venue::{self, Document, Edit},
};

fn baseline(song: &Path) -> Result<venue::Source> {
    if let Some((_, directory)) = source(song)? {
        return venue::imported(&fs::read(directory.join("notes.mid"))?);
    }
    let bytes = chart_original(song)?;
    let source_sha256 = hash(&bytes);
    let chart = Chart::parse(String::from_utf8(bytes)?)?;
    ensure!(
        !chart
            .sections
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case("VENUE")),
        "An unrecognized chart VENUE section remains read-only"
    );
    ensure!(
        (1..=32767).contains(&chart.timing.resolution),
        "Venue MIDI export needs a resolution from 1 to 32767"
    );
    Ok(venue::Source {
        document: Document {
            version: 1,
            source_sha256,
            events: Vec::new(),
        },
        resolution: chart.timing.resolution,
        present: false,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    version: u32,
    master_sha256: String,
    midi_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    master_sha256: String,
    midi_sha256: String,
    document: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chart_origin_sha256: Option<String>,
}
fn saved(song: &Path) -> Result<Option<Saved>> {
    let path = song_layout::resolve(song, "chart-engine-working").join("venue-events.json");
    if fs::exists(&path) {
        Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
    } else {
        Ok(None)
    }
}
fn chart_archive(root: &Path, sha: &str) -> PathBuf {
    root.join(".yace/originals/venue-chart")
        .join(sha)
        .join("notes.chart")
}
fn chart_original(song: &Path) -> Result<Vec<u8>> {
    if let Some(saved) = saved(song)? {
        venue::validate_shape(&saved.document)?;
        let sha = saved
            .chart_origin_sha256
            .context("Venue MIDI source receipt is missing; recover the original project")?;
        ensure!(
            sha == saved.document.source_sha256,
            "Venue chart origin changed"
        );
        let bytes = fs::read(chart_archive(song, &sha))?;
        ensure!(
            hash(&bytes) == sha,
            "Original venue chart changed; original retained"
        );
        Ok(bytes)
    } else {
        if let Some((actual, _)) = working(song)? {
            ensure!(
                !actual.present,
                "Generated MIDI VENUE lacks its editable binding; retained read-only"
            );
        }
        Ok(fs::read(song.join("notes.chart"))?)
    }
}
fn empty_midi(resolution: u32) -> Result<Vec<u8>> {
    ensure!(
        (1..=32767).contains(&resolution),
        "Invalid venue MIDI resolution"
    );
    let midi = midly::Smf {
        header: midly::Header::new(
            midly::Format::SingleTrack,
            midly::Timing::Metrical((resolution as u16).into()),
        ),
        tracks: vec![vec![
            midly::TrackEvent {
                delta: 0.into(),
                kind: midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"TEMPO")),
            },
            midly::TrackEvent {
                delta: 0.into(),
                kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
            },
        ]],
    };
    let mut bytes = Vec::new();
    midi.write_std(&mut bytes)?;
    Ok(bytes)
}
fn venue_chunk(bytes: &[u8]) -> Result<Option<&[u8]>> {
    let midi = midly::Smf::parse(bytes)?;
    let (_, chunks) = super::midi_chunks(bytes, midi.tracks.len())?;
    Ok(midi.tracks.iter().zip(chunks).find_map(|(track,chunk)| {
        let venue=track.iter().any(|event| {
            matches!(event.kind,midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(name)) if name.eq_ignore_ascii_case(b"VENUE"))
        });
        venue.then_some(chunk)
    }))
}
fn working(song: &Path) -> Result<Option<(venue::Source, Binding)>> {
    let directory = song_layout::resolve(song, "chart-engine-working");
    if !fs::exists(directory.join("binding.json")) {
        return Ok(None);
    }
    let binding: Binding = serde_json::from_slice(&fs::read(directory.join("binding.json"))?)?;
    let path = if fs::exists(directory.join("game-notes.mid")) {
        directory.join("game-notes.mid")
    } else {
        song.join("notes.mid")
    };
    let bytes = fs::read(path)?;
    ensure!(
        binding.version == 1
            && binding.midi_sha256 == hash(&bytes)
            && binding.master_sha256 == hash(&fs::read(directory.join("notes.chart"))?),
        "Working venue binding changed; original retained"
    );
    Ok(Some((venue::imported(&bytes)?, binding)))
}
pub fn read_venue(song: &Path) -> Result<Option<Document>> {
    let Some(saved) = saved(song)? else {
        return Ok(None);
    };
    let (actual, binding) = working(song)?.context("Venue state needs verified working MIDI")?;
    let source = baseline(song)?;
    venue::validate_shape(&saved.document)?;
    ensure!(
        saved.version == 1
            && saved.master_sha256 == binding.master_sha256
            && saved.midi_sha256 == binding.midi_sha256
            && actual.resolution == source.resolution
            && saved.document.source_sha256 == source.document.source_sha256
            && venue::values(&saved.document) == venue::values(&actual.document),
        "Venue sidecar no longer matches its verified MIDI"
    );
    if saved.chart_origin_sha256.is_some() {
        ensure!(
            !exists(song),
            "Chart-origin venue needs its original editable chart authority"
        );
        let expected = venue::rewrite(&empty_midi(source.resolution)?, &source, &saved.document)?;
        let directory = song_layout::resolve(song, "chart-engine-working");
        let path = if fs::exists(directory.join("game-notes.mid")) {
            directory.join("game-notes.mid")
        } else {
            song.join("notes.mid")
        };
        let bytes = fs::read(path)?;
        ensure!(
            venue_chunk(&expected)? == venue_chunk(&bytes)?,
            "Chart-origin VENUE has unbound commands; retained read-only"
        );
    }
    Ok(Some(saved.document))
}
pub fn venue_events(song: &Path, project: &Project, chart: &Chart) -> Result<Document> {
    let source = baseline(song)?;
    let current = if let Some(document) = &project.content.venue_events {
        document.clone()
    } else if let Some(document) = read_venue(song)? {
        document
    } else {
        if let Some((actual, _)) = working(song)? {
            ensure!(
                venue::values(&source.document) == venue::values(&actual.document),
                "Working venue changed without editable binding; retained read-only"
            );
        }
        source.document.clone()
    };
    venue::validate(
        &source,
        &current,
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    Ok(current)
}
pub fn validate_venue(song: &Path, project: &Project, chart: &Chart) -> Result<()> {
    if let Some(document) = &project.content.venue_events {
        venue::validate(
            &baseline(song)?,
            document,
            &crate::sync_track::effective(project, &chart.timing)?,
            project.duration,
        )?;
    }
    Ok(())
}
pub fn edit_venue(song: &Path, project: &mut Project, chart: &Chart, edit: &Edit) -> Result<()> {
    let source = baseline(song)?;
    let current = venue_events(song, project, chart)?;
    let (document, next) = venue::edited(
        &source,
        &current,
        edit,
        project.next_id,
        project.revision,
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    let bytes = if let Some((_, directory)) = super::source(song)? {
        fs::read(directory.join("notes.mid"))?
    } else {
        empty_midi(source.resolution)?
    };
    venue::rewrite(&bytes, &source, &document)?;
    project.content.venue_events = Some(document);
    project.next_id = next;
    Ok(())
}
pub fn export_venue(song: &Path, state: Option<&Document>, bytes: &[u8]) -> Result<Vec<u8>> {
    match state {
        Some(document) => venue::rewrite(bytes, &baseline(song)?, document),
        None => Ok(bytes.to_vec()),
    }
}
pub fn write_venue(
    song: &Path,
    stage: &Path,
    state: Option<&Document>,
    bytes: &[u8],
) -> Result<()> {
    let Some(document) = state else { return Ok(()) };
    let directory = song_layout::resolve(stage, "chart-engine-working");
    let binding: Binding = serde_json::from_slice(&fs::read(directory.join("binding.json"))?)?;
    ensure!(
        binding.version == 1 && binding.midi_sha256 == hash(bytes),
        "Venue export binding changed"
    );
    let actual = venue::imported(bytes)?;
    ensure!(
        venue::values(document) == venue::values(&actual.document),
        "Exported venue no longer matches editable state"
    );
    let chart_origin_sha256 = if !exists(song) {
        venue::validate_shape(document)?;
        let original = chart_original(song)?;
        ensure!(
            hash(&original) == document.source_sha256,
            "Original venue chart binding changed"
        );
        let path = chart_archive(stage, &document.source_sha256);
        if fs::exists(&path) {
            ensure!(
                fs::read(&path)? == original,
                "Existing venue source archive changed"
            );
        } else {
            fs::create_dir_all(path.parent().context("Venue source archive parent")?)?;
            fs::write(path, original)?;
        }
        Some(document.source_sha256.clone())
    } else {
        None
    };
    fs::write(
        directory.join("venue-events.json"),
        serde_json::to_vec_pretty(&Saved {
            version: 1,
            master_sha256: binding.master_sha256,
            midi_sha256: binding.midi_sha256,
            document: document.clone(),
            chart_origin_sha256,
        })?,
    )?;
    Ok(())
}
