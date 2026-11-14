//! Hash-bound ordinary MIDI text editing composes with the current game MIDI.
use super::*;
use crate::{
    editor::Project,
    midi_text_events as text,
    text_events::{Document, Edit, Scope},
};

fn baseline(song: &Path) -> Result<text::Source> {
    let (_, directory) = source(song)?.context("MIDI text needs its verified original source")?;
    text::imported(&fs::read(directory.join("notes.mid"))?)
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
struct Working {
    view: text::Source,
    binding: Binding,
}
fn working(song: &Path) -> Result<Option<Working>> {
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
        "Working MIDI text binding changed; original retained"
    );
    Ok(Some(Working {
        view: text::imported(&bytes)?,
        binding,
    }))
}
fn values(document: &Document) -> Vec<(Scope, u64, &str)> {
    let mut values: Vec<_> = document
        .events
        .iter()
        .map(|e| (e.scope.clone(), e.tick, e.text.as_str()))
        .collect();
    values.sort();
    values
}
fn retain_owners(document: &mut Document, owners: &[String]) {
    document.events.retain(|e| match &e.scope {
        Scope::Global => true,
        Scope::Physical { owner } => owners.contains(owner),
        Scope::Local { .. } => false,
    });
}
pub fn read_text_events(song: &Path) -> Result<Option<Document>> {
    let path = song_layout::resolve(song, "chart-engine-working").join("text-events.json");
    if !exists(song) || !fs::exists(&path) {
        return Ok(None);
    }
    let saved: Saved = serde_json::from_slice(&fs::read(path)?)?;
    let working = working(song)?.context("MIDI text sidecar needs its working MIDI binding")?;
    let original = baseline(song)?;
    text::validate_shape(&saved.document)?;
    ensure!(
        saved.version == 1
            && saved.master_sha256 == working.binding.master_sha256
            && saved.midi_sha256 == working.binding.midi_sha256
            && saved.document.source_sha256 == original.document.source_sha256
            && values(&saved.document) == values(&working.view.document),
        "MIDI text sidecar no longer matches the verified game MIDI"
    );
    Ok(Some(saved.document))
}
fn original_state(song: &Path, original: &text::Source) -> Result<Document> {
    if let Some(document) = read_text_events(song)? {
        return Ok(document);
    }
    let mut document = original.document.clone();
    if let Some(working) = working(song)? {
        retain_owners(&mut document, &working.view.owners);
        ensure!(
            values(&document) == values(&working.view.document),
            "Working MIDI text changed without an editable text binding; retained read-only"
        );
    }
    Ok(document)
}
pub fn text_event_owners(song: &Path) -> Result<Vec<String>> {
    Ok(match working(song)? {
        Some(working) => working.view.owners,
        None => baseline(song)?.owners,
    })
}
pub fn text_events(song: &Path, project: &Project, chart: &Chart) -> Result<Document> {
    let source = baseline(song)?;
    let current = match &project.content.midi_text_events {
        Some(document) => document.clone(),
        None => original_state(song, &source)?,
    };
    text::validate(
        &source,
        &current,
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    Ok(current)
}
pub fn validate_text_events(song: &Path, project: &Project, chart: &Chart) -> Result<()> {
    if let Some(document) = &project.content.midi_text_events {
        text::validate(
            &baseline(song)?,
            document,
            &crate::sync_track::effective(project, &chart.timing)?,
            project.duration,
        )?;
    }
    Ok(())
}
pub fn edit_text_events(
    song: &Path,
    project: &mut Project,
    chart: &Chart,
    edit: &Edit,
) -> Result<()> {
    let source = baseline(song)?;
    let current = text_events(song, project, chart)?;
    if let Edit::Put {
        scope: Scope::Physical { owner },
        ..
    } = edit
    {
        ensure!(
            text_event_owners(song)?.contains(owner),
            "Choose a physical MIDI track retained in this project"
        );
    }
    let (document, next_id) = text::edited(
        &source,
        &current,
        edit,
        project.next_id,
        project.revision,
        &crate::sync_track::effective(project, &chart.timing)?,
        project.duration,
    )?;
    let (_, directory) = super::source(song)?.context("Missing MIDI text source")?;
    text::rewrite(&fs::read(directory.join("notes.mid"))?, &source, &document)?;
    project.content.midi_text_events = Some(document);
    project.next_id = next_id;
    Ok(())
}
pub fn export_text_events(song: &Path, state: Option<&Document>, bytes: &[u8]) -> Result<Vec<u8>> {
    let Some(state) = state else {
        return Ok(bytes.to_vec());
    };
    let source = baseline(song)?;
    // The established note/control export pipeline composes from the immutable
    // original MIDI. Reapply the complete text state to that baseline, even
    // when its editable state was restored from a previously exported song.
    text::rewrite(bytes, &source, state)
}
pub fn write_text_events(stage: &Path, state: Option<&Document>, bytes: &[u8]) -> Result<()> {
    let Some(state) = state else { return Ok(()) };
    let path = song_layout::resolve(stage, "chart-engine-working");
    let binding: Binding = serde_json::from_slice(&fs::read(path.join("binding.json"))?)?;
    ensure!(
        binding.version == 1 && binding.midi_sha256 == hash(bytes),
        "MIDI text export binding changed"
    );
    let actual = text::imported(bytes)?;
    let mut document = state.clone();
    retain_owners(&mut document, &actual.owners);
    ensure!(
        values(&document) == values(&actual.document),
        "Exported MIDI text no longer matches its editable state"
    );
    fs::write(
        path.join("text-events.json"),
        serde_json::to_vec_pretty(&Saved {
            version: 1,
            master_sha256: binding.master_sha256,
            midi_sha256: binding.midi_sha256,
            document,
        })?,
    )?;
    Ok(())
}
