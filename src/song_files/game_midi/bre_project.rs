//! Existing hash-bound working MIDI restores coda and BRE edits, including clears.
use super::*;
use crate::bre::{self as endings, State};
use std::collections::BTreeSet;
pub fn source_bre(song: &Path, chart: &Chart) -> Result<State> {
    if let Some((_, directory)) = source(song)? {
        let state = super::bre::import_bre(&fs::read(directory.join("notes.mid"))?)?;
        ensure!(
            state.resolution == chart.timing.resolution,
            "BRE/source resolution changed"
        );
        return Ok(state);
    }
    let mut state = State::empty(chart.timing.resolution);
    state.coda = endings::chart_coda(chart)?;
    state.source_sha256 = Some(hash(chart.raw.as_bytes()));
    Ok(state)
}
pub fn read_bre_state(song: &Path, chart: &Chart) -> Result<Option<State>> {
    let working = song_layout::resolve(song, "chart-engine-working");
    let path = working.join("game-notes.mid");
    if !fs::is_file(&path) {
        return Ok(None);
    }
    let original = match source_bre(song, chart) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        version: u32,
        master_sha256: String,
        midi_sha256: String,
    }
    let binding: Binding = serde_json::from_slice(&fs::read(working.join("binding.json"))?)?;
    let bytes = fs::read(path)?;
    ensure!(
        binding.version == 1
            && binding.midi_sha256 == hash(&bytes)
            && binding.master_sha256 == hash(&fs::read(working.join("notes.chart"))?),
        "Working BRE MIDI binding changed; retain source"
    );
    let mut current = super::bre::import_bre(&bytes)?;
    ensure!(
        current.resolution == original.resolution,
        "Working BRE resolution changed"
    );
    current.source_sha256 = original.source_sha256;
    if current.coda == original.coda && current.owners == original.owners {
        return Ok(None);
    }
    Ok(Some(current))
}
pub fn export_bre(
    song: &Path,
    chart: &Chart,
    state: Option<&State>,
    bytes: &[u8],
    selected: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let Some(state) = state else {
        return Ok(bytes.to_vec());
    };
    let original = source_bre(song, chart)?;
    ensure!(
        state.source_sha256 == original.source_sha256,
        "BRE source changed; export refused"
    );
    let mut before = if exists(song) {
        original.clone()
    } else {
        let mut generated = super::bre::import_bre(bytes)?;
        generated.source_sha256 = original.source_sha256.clone();
        generated
    };
    let mut after = state.clone();
    before
        .owners
        .retain(|owner, _| endings::selected(owner, selected));
    after
        .owners
        .retain(|owner, _| endings::selected(owner, selected));
    super::bre::rewrite_bre(bytes, &hash(bytes), &before, &after)
}
