//! Source-bound, bounded timing repair shared by preview and the transaction.
use super::*;
use crate::snap_grid::{RepairPolicy, RepairPreview};

pub fn whole_part(project: &Project, track: &str) -> Result<Selection> {
    let mut selected = Selection {
        track: track.into(),
        ..Selection::default()
    };
    if track == "Vocals" {
        ensure!(
            project
                .content
                .vocals
                .as_ref()
                .context("No vocal chart")?
                .notes
                .len()
                <= MAX_NOTES,
            "Repair at most 100,000 notes at once; select a smaller section"
        );
        selected.vocals.extend(
            0..project
                .content
                .vocals
                .as_ref()
                .context("No vocal chart")?
                .notes
                .len(),
        );
    } else {
        selected.ids.extend(
            project
                .content
                .tracks
                .get(track)
                .context("Unknown track")?
                .iter()
                .filter(|n| project.playable(track, n.note.lane))
                .take(MAX_NOTES + 1)
                .map(|n| n.id.clone()),
        );
    }
    ensure!(
        selected.len() <= MAX_NOTES,
        "Repair at most 100,000 notes at once; select a smaller section"
    );
    Ok(selected)
}

pub fn preview(
    project: &Project,
    timing: &TempoMap,
    selection: &Snapshot,
    policy: RepairPolicy,
) -> Result<RepairPreview> {
    let mut candidate = project.clone();
    apply(&mut candidate, timing, selection, policy)
}

pub(super) fn apply(
    project: &mut Project,
    timing: &TempoMap,
    selection: &Snapshot,
    policy: RepairPolicy,
) -> Result<RepairPreview> {
    selection.validate(project)?;
    let grid = policy.grid(timing.resolution)?;
    let mut report = RepairPreview::default();
    if selection.track == "Vocals" {
        let vocals = project.content.vocals.as_ref().context("No vocal chart")?;
        for &index in &selection.vocals {
            let note = &vocals.notes[index];
            let after = policy.destination(grid, note.start)?;
            report.observe(
                note.start,
                grid.snap_tick(note.start).context("Grid timing overflow")?,
                after,
            );
        }
        if report.changed > 0 {
            edit_vocals(
                project,
                selection,
                &Edit::Repair {
                    selection: selection.clone(),
                    policy,
                },
            )?;
            let vocals = project.content.vocals.as_ref().context("No vocal chart")?;
            ensure!(
                vocals
                    .phrases
                    .iter()
                    .all(|p| timing.seconds(p.end) <= project.duration + 0.1),
                "Repaired scoring phrase extends beyond the recording"
            );
        }
        return Ok(report);
    }
    let mut moving = selection.clone();
    moving.ids.clear();
    let mut moves = BTreeMap::new();
    let mut bundles = Vec::new();
    let selected: Vec<_> = project.content.tracks[&selection.track]
        .iter()
        .filter(|n| selection.ids.contains(&n.id))
        .collect();
    for (item, mut gem) in selected.into_iter().zip(gems(project, selection)) {
        let after = policy.destination(grid, gem.note.tick)?;
        report.observe(
            gem.note.tick,
            grid.snap_tick(gem.note.tick)
                .context("Grid timing overflow")?,
            after,
        );
        if after != gem.note.tick {
            let end = after
                .checked_add(gem.note.sustain)
                .context("Repaired note timing overflow")?;
            ensure!(
                timing.seconds(end) <= project.duration + 0.1,
                "Repaired note extends beyond the recording"
            );
            moving.ids.insert(item.id.clone());
            moves.insert(item.id.clone(), after);
            gem.note.tick = after;
            bundles.push((item.id.clone(), gem));
        }
    }
    if report.changed == 0 {
        return Ok(report);
    }
    // Chord-wide attack markers cannot change an unselected or separately
    // authored attack when rounding brings two original attacks together.
    if !selection.track.ends_with("Drums") {
        let track = &project.content.tracks[&selection.track];
        let mut flags: BTreeMap<u64, BTreeSet<u8>> = BTreeMap::new();
        for n in track
            .iter()
            .filter(|n| attack_marker(&selection.track, n.note.lane))
        {
            flags.entry(n.note.tick).or_default().insert(n.note.lane);
        }
        let mut attacks = BTreeMap::new();
        for n in track
            .iter()
            .filter(|n| project.playable(&selection.track, n.note.lane))
        {
            let after = moves.get(&n.id).copied().unwrap_or(n.note.tick);
            let markers = flags.get(&n.note.tick);
            if let Some((origin, previous)) = attacks.insert(after, (n.note.tick, markers)) {
                ensure!(
                    origin == n.note.tick || previous == markers,
                    "Repair would combine attacks with different HOPO/tap flags; choose a finer grid or smaller tolerance"
                );
            }
        }
    }
    remove_gems(project, &moving);
    let mut index = DestinationIndex::new(project, &selection.track)?;
    for (id, gem) in bundles {
        insert_gem(project, &selection.track, gem, Some(id), &mut index)?;
    }
    Ok(report)
}
