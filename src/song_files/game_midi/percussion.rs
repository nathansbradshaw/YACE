//! Trusted local working MIDI for explicit percussion edits; originals are immutable.
use super::*;
use crate::{
    vocal_harmonies::HarmonyChart,
    vocal_percussion::{self as pc, Part, State},
};
use std::collections::BTreeSet;

const SIDECAR: &str = "chart-engine-vocal-percussion.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bound {
    version: u32,
    midi_sha256: String,
    owners: BTreeSet<Part>,
    state: State,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_bound(song: &Path) -> Result<Option<Bound>> {
    let path = song_layout::resolve(song, SIDECAR);
    if !fs::exists(&path) {
        return Ok(None);
    }
    let archived = source(song)?;
    let bound: Bound = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(bound.version == 1, "Unsupported percussion working binding");
    let root = song.join("notes.mid");
    let working = song_layout::resolve(song, "chart-engine-working").join("game-notes.mid");
    let midi = if fs::is_file(&root) {
        root
    } else if fs::is_file(&working) {
        working
    } else {
        archived
            .context("Percussion working binding has no game MIDI")?
            .1
            .join("notes.mid")
    };
    let bytes = fs::read(midi)?;
    ensure!(
        hash(&bytes) == bound.midi_sha256,
        "Percussion working MIDI changed; recover its bound project"
    );
    let actual = pc::import_midi(&bytes)?;
    ensure!(
        actual.as_ref() == bound.state.document(),
        "Percussion working state differs from its game MIDI"
    );
    ensure!(
        actual
            .as_ref()
            .is_none_or(|chart| chart.tracks.keys().all(|part| bound.owners.contains(part))),
        "Percussion working owners are incomplete"
    );
    Ok(Some(bound))
}
pub(crate) fn read_percussion_state(song: &Path) -> Result<Option<State>> {
    Ok(read_bound(song)?.map(|bound| bound.state))
}
pub(crate) fn percussion_owners(song: &Path, state: Option<&State>) -> Result<BTreeSet<Part>> {
    let Some(state) = state else {
        return Ok(BTreeSet::new());
    };
    let mut owners = read_bound(song)?.map_or_else(BTreeSet::new, |bound| bound.owners);
    if let Some(original) = super::percussion_view(song)? {
        owners.extend(original.tracks.keys().copied());
    }
    if let Some(document) = state.document() {
        owners.extend(document.tracks.keys().copied());
    }
    Ok(owners)
}
pub(crate) fn logical_owner(name: Option<&str>, owners: &BTreeSet<Part>) -> Option<&'static str> {
    let name = name?;
    if owners.contains(&Part::Lead) && Part::from_midi(name) == Some(Part::Lead) {
        Some("Vocals")
    } else if owners.contains(&Part::Harm1)
        && crate::vocal_harmonies::Part::from_midi(name).is_some()
    {
        Some("Harmonies")
    } else {
        None
    }
}
fn working_source(song: &Path) -> Result<Option<Vec<u8>>> {
    let Some((receipt, directory)) = source(song)? else {
        return Ok(None);
    };
    let bytes = fs::read(directory.join("notes.mid"))?;
    let before = pc::import_midi(&bytes)?;
    let stripped = pc::rewrite_midi(&bytes, &receipt.midi_sha256, before.as_ref(), None)?;
    let all = original_extra_parts(song, false)?;
    let active = extra_parts(song)?;
    let parsed = midly::Smf::parse(&stripped)?;
    let (header, raw) = midi_chunks(&stripped, parsed.tracks.len())?;
    let mut chunks = Vec::new();
    for (index, (track, raw)) in parsed.tracks.iter().zip(raw).enumerate() {
        let name = decode::label(track);
        let vocal = name.as_deref().is_some_and(|name| {
            Part::from_midi(name).is_some()
                || crate::vocal_harmonies::Part::from_midi(name).is_some()
        });
        let omitted = vocal
            && extra_key(track, index)
                .is_some_and(|key| all.contains(&key) && !active.contains(&key));
        // Stripping the final PC notes can leave a metadata-only owner, so use
        // its original name as well as its post-strip note presence for scope.
        let named_key = name.as_ref().map(|name| format!("MIDI: {name}"));
        let omitted = omitted
            || (vocal
                && named_key
                    .as_ref()
                    .is_some_and(|key| all.contains(key) && !active.contains(key)));
        if omitted {
            ensure!(
                !track.iter().any(|event| matches!(
                    event.kind,
                    midly::TrackEventKind::Meta(
                        midly::MetaMessage::Tempo(_) | midly::MetaMessage::TimeSignature(..)
                    )
                )),
                "An omitted vocal owner contains timing; source retained"
            );
        } else {
            chunks.push(raw);
        }
    }
    let mut output = stripped[..header].to_vec();
    output[10..12].copy_from_slice(&u16::try_from(chunks.len())?.to_be_bytes());
    for chunk in chunks {
        output.extend(chunk);
    }
    Ok(Some(output))
}
pub(crate) fn validate_harmony_authoring(
    song: &Path,
    state: Option<&State>,
    after: &HarmonyChart,
) -> Result<()> {
    ensure!(
        state.is_some(),
        "Original harmony tracks need an imported editable baseline before authoring; source retained"
    );
    let owners = percussion_owners(song, state)?;
    ensure!(
        owners.contains(&Part::Harm1),
        "Original harmony tracks are not managed percussion owners; source retained"
    );
    if let Some(working) = working_source(song)? {
        harmonies::export_harmonies_inner(song, working, Some(after), true, true)?;
    }
    Ok(())
}
pub(crate) fn can_begin_harmonies(song: &Path, state: Option<&State>) -> Result<bool> {
    let inherited = if state.is_none() {
        super::percussion_view(song)?.map(|document| State::Authored { document })
    } else {
        None
    };
    let state = state.or(inherited.as_ref());
    if !percussion_owners(song, state)?.contains(&Part::Harm1) {
        return Ok(false);
    }
    Ok(working_source(song)?.is_none_or(|bytes| {
        crate::vocal_harmonies::import_midi(&bytes).is_ok_and(|view| view.is_none())
    }))
}
pub(crate) struct VocalExport<'a> {
    pub lead: Option<&'a VocalChart>,
    pub harmonies: Option<&'a HarmonyChart>,
    pub percussion: Option<&'a State>,
    pub selected: &'a BTreeSet<String>,
}
pub(crate) fn export_vocals(
    song: &Path,
    stage: &Path,
    chart: &Chart,
    tracks: &BTreeMap<String, Vec<EditorNote>>,
    generated: &[u8],
    vocals: VocalExport<'_>,
) -> Result<Vec<u8>> {
    let owners = percussion_owners(song, vocals.percussion)?;
    let working = if vocals.percussion.is_some() {
        working_source(song)?
    } else {
        None
    };
    let bytes = export_inner(
        song,
        stage,
        chart,
        vocals.lead,
        tracks,
        generated,
        working.as_deref(),
    )?
    .unwrap_or_else(|| generated.to_vec());
    let bytes = harmonies::export_harmonies_inner(
        song,
        bytes,
        vocals.harmonies,
        vocals.selected.contains("Harmonies"),
        vocals.percussion.is_some(),
    )?;
    let bytes = select_export_with_owners(song, stage, bytes, vocals.selected, &owners)?;
    let Some(state) = vocals.percussion else {
        return Ok(bytes);
    };
    let mut after = state.document().cloned();
    if let Some(after) = &mut after {
        after.tracks.retain(|part, _| {
            vocals.selected.contains(match part {
                Part::Lead => "Vocals",
                Part::Harm1 => "Harmonies",
            })
        });
    }
    let after = after.filter(|chart| !chart.tracks.is_empty());
    let before = pc::import_midi(&bytes)?;
    ensure!(
        before.is_none(),
        "The working singer stage unexpectedly retained managed percussion"
    );
    let output = pc::rewrite_midi(&bytes, &hash(&bytes), None, after.as_ref())?;
    Ok(output)
}
/// Call after the editor has replaced its old editable-master directory.
pub(crate) fn write_percussion_state(
    song: &Path,
    stage: &Path,
    state: Option<&State>,
) -> Result<()> {
    if state.is_none() {
        return Ok(());
    }
    let owners = percussion_owners(song, state)?;
    let output = fs::read(stage.join("notes.mid"))?;
    let actual = pc::import_midi(&output)?;
    let bound = Bound {
        version: 1,
        midi_sha256: hash(&output),
        owners,
        state: actual.map_or(State::Cleared, |document| State::Authored { document }),
    };
    fs::write(
        song_layout::path(stage, SIDECAR),
        serde_json::to_vec_pretty(&bound)?,
    )?;
    Ok(())
}
