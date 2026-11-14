use super::*;
use crate::snap_grid::{RepairPolicy, RepairPreview};

#[derive(Clone, Debug, PartialEq)]
pub struct RepairContext {
    root: Option<PathBuf>,
    revision: u64,
    source: String,
    chart: String,
    selected: std::collections::BTreeSet<String>,
    epoch: u64,
}
fn context(s: DanceStudio) -> Option<RepairContext> {
    if !(s.open)() {
        return None;
    }
    let project = s.project.read();
    let project = project.as_ref()?;
    Some(RepairContext {
        root: (s.root)(),
        revision: project.revision,
        source: project.source_sha256.clone(),
        chart: (s.selected)(),
        selected: (s.selection)(),
        epoch: (s.epoch)(),
    })
}
pub fn repair_context() -> Option<RepairContext> {
    context(try_consume_context::<DanceStudio>()?)
}
fn current(s: DanceStudio, expected: &RepairContext) -> bool {
    (s.open)()
        && s.root.read().as_ref() == expected.root.as_ref()
        && s.project
            .read()
            .as_ref()
            .is_some_and(|p| p.revision == expected.revision && p.source_sha256 == expected.source)
        && *s.selected.read() == expected.chart
        && *s.selection.read() == expected.selected
        && *s.epoch.read() == expected.epoch
}
pub fn repair_is_current(expected: &RepairContext) -> bool {
    try_consume_context::<DanceStudio>().is_some_and(|s| current(s, expected))
}
pub fn repair_preview(
    expected: &RepairContext,
    whole: bool,
    policy: RepairPolicy,
) -> Result<(Vec<String>, RepairPreview)> {
    let s = try_consume_context::<DanceStudio>().context("Open a Dance project")?;
    ensure!(
        current(s, expected),
        "The project, part or selection changed. Preview again"
    );
    let project = s.project.read();
    let project = project.as_ref().context("Open a Dance project")?;
    let ids: Vec<String> = if whole {
        project
            .content
            .document
            .charts
            .iter()
            .find(|c| c.id == expected.chart)
            .context("Choose a difficulty")?
            .notes
            .iter()
            .take(100_001)
            .map(|n| n.id.clone())
            .collect()
    } else {
        ensure!(
            expected.selected.len() <= 100_000,
            "Repair at most 100,000 notes at once; select a smaller section"
        );
        expected.selected.iter().cloned().collect()
    };
    let report = project.preview_timing_repair(&expected.chart, &ids, policy)?;
    Ok((ids, report))
}
pub fn apply_timing_repair(
    expected: RepairContext,
    ids: Vec<String>,
    policy: RepairPolicy,
    mut studio: super::super::state::Studio,
) {
    let Some(guard) = super::super::activity::BusyGuard::start(studio.group_busy) else {
        return;
    };
    let Some(mut s) = try_consume_context::<DanceStudio>() else {
        return;
    };
    s.status.set("Repairing note timing…".into());
    studio.status.set("Repairing note timing…".into());
    spawn(async move {
        let _guard = guard;
        #[cfg(target_arch = "wasm32")]
        gloo_timers::future::TimeoutFuture::new(20).await;
        #[cfg(not(target_arch = "wasm32"))]
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if !current(s, &expected) {
            studio.error.set(Some("The project, part or selection changed. Preview timing repair again; nothing was changed.".into()));
            s.status.set("Timing repair cancelled".into());
            studio.status.set("Timing repair cancelled".into());
            return;
        }
        s.apply(Action::EditNotes {
            chart_id: expected.chart,
            note_ids: ids,
            edit: sm::project::SelectionEdit::Repair { policy },
        });
        if s.project
            .read()
            .as_ref()
            .is_some_and(|p| p.revision == expected.revision + 1)
        {
            s.status.set("Timing repair applied".into());
            studio.status.set("Timing repair applied".into());
        } else {
            s.status
                .set("Timing repair failed · no edit was committed".into());
            studio
                .status
                .set("Timing repair failed · no edit was committed".into());
            studio.error.set(Some((s.error)().unwrap_or_else(|| {
                "Timing repair was refused; no edit was committed".into()
            })));
        }
    });
}
