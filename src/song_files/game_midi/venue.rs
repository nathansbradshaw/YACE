//! Independent, hash-bound venue state; it never takes ownership of chart notes
//! or ordinary instrument text. Playable exports carry verified editable IDs.
use super::*;
use crate::{
    editor::Project,
    venue::{self, Document, Edit},
};

fn baseline(song: &Path) -> Result<venue::Source> {
    let (_, directory) =
        source(song)?.context("Venue authoring currently requires an imported MIDI source")?;
    venue::imported(&fs::read(directory.join("notes.mid"))?)
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
    let path = song_layout::resolve(song, "chart-engine-working").join("venue-events.json");
    if !exists(song) || !fs::exists(&path) {
        return Ok(None);
    }
    let saved: Saved = serde_json::from_slice(&fs::read(path)?)?;
    let (actual, binding) = working(song)?.context("Venue state needs verified working MIDI")?;
    let source = baseline(song)?;
    venue::validate_shape(&saved.document)?;
    ensure!(
        saved.version == 1
            && saved.master_sha256 == binding.master_sha256
            && saved.midi_sha256 == binding.midi_sha256
            && saved.document.source_sha256 == source.document.source_sha256
            && venue::values(&saved.document) == venue::values(&actual.document),
        "Venue sidecar no longer matches its verified MIDI"
    );
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
    let (_, directory) = super::source(song)?.context("Missing venue source")?;
    venue::rewrite(&fs::read(directory.join("notes.mid"))?, &source, &document)?;
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
pub fn write_venue(stage: &Path, state: Option<&Document>, bytes: &[u8]) -> Result<()> {
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
    fs::write(
        directory.join("venue-events.json"),
        serde_json::to_vec_pretty(&Saved {
            version: 1,
            master_sha256: binding.master_sha256,
            midi_sha256: binding.midi_sha256,
            document: document.clone(),
        })?,
    )?;
    Ok(())
}
