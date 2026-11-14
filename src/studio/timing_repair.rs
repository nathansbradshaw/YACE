//! Explicit, immutable timing previews; no editing occurs until Apply.
use super::{dance, state::Studio};
use crate::{
    editor::Action,
    note_groups::{self, Selection, Snapshot},
    snap_grid::{RepairPolicy, RepairPreview},
    vocal_harmonies::{HarmonyChart, Part},
};
use anyhow::{Context as _, Result, ensure};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
enum Context {
    Band {
        root: PathBuf,
        revision: u64,
        track: String,
        selected: Selection,
    },
    Dance(dance::RepairContext),
}
impl Context {
    fn capture(studio: Studio) -> Option<Self> {
        if dance::active() {
            return dance::repair_context().map(Self::Dance);
        }
        let project = studio.snapshot()?;
        Some(Self::Band {
            root: studio.store.read().as_ref()?.root.clone(),
            revision: project.revision,
            track: project.selected_track.clone(),
            selected: studio.selection(),
        })
    }
    fn current(&self, studio: Studio) -> bool {
        match self {
            Self::Dance(binding) => dance::active() && dance::repair_is_current(binding),
            Self::Band {
                root,
                revision,
                track,
                selected,
            } => {
                if dance::active()
                    || !studio
                        .store
                        .read()
                        .as_ref()
                        .is_some_and(|s| &s.root == root)
                    || !studio
                        .snapshot()
                        .is_some_and(|p| p.revision == *revision && &p.selected_track == track)
                {
                    return false;
                }
                let group = studio.group.read();
                if group.track == *track && !group.is_empty() {
                    &*group == selected
                } else {
                    studio.selection() == *selected
                }
            }
        }
    }
}
#[derive(Clone)]
enum Scope {
    Band(Snapshot),
    Harmony(HarmonyChart),
    Dance(Vec<String>),
}
#[derive(Clone)]
struct Preview {
    context: Context,
    policy: RepairPolicy,
    scope: Scope,
    report: RepairPreview,
}
fn preview(
    studio: Studio,
    context: Context,
    whole: bool,
    harmony: Option<Part>,
    policy: RepairPolicy,
) -> Result<Preview> {
    ensure!(
        context.current(studio),
        "The project, part or selection changed. Preview again"
    );
    let (scope, report) = match &context {
        Context::Band {
            track, selected, ..
        } => {
            let project = studio.snapshot().context("Open a project")?;
            if let Some(part) = harmony {
                let (candidate, report) = project
                    .content
                    .harmonies
                    .as_ref()
                    .context("No editable harmony chart")?
                    .preview_timing_repair(part, policy)?;
                candidate.validate_timing(
                    &(studio.timing)().context("Missing chart timing")?,
                    project.duration,
                )?;
                return Ok(Preview {
                    context,
                    policy,
                    scope: Scope::Harmony(candidate),
                    report,
                });
            }
            let selected = if whole {
                note_groups::repair::whole_part(&project, track)?
            } else {
                selected.clone()
            };
            let snapshot = Snapshot::capture(&project, &selected)?;
            let timing = (studio.timing)().context("Missing chart timing")?;
            let report = note_groups::repair::preview(&project, &timing, &snapshot, policy)?;
            (Scope::Band(snapshot), report)
        }
        Context::Dance(binding) => {
            let (ids, report) = dance::repair_preview(binding, whole, policy)?;
            (Scope::Dance(ids), report)
        }
    };
    Ok(Preview {
        context,
        policy,
        scope,
        report,
    })
}
fn apply_harmony(mut studio: Studio, context: Context, document: HarmonyChart) {
    let Some(guard) = super::activity::BusyGuard::start(studio.group_busy) else {
        return;
    };
    studio.status.set("Repairing harmony timing…".into());
    spawn(async move {
        let _guard = guard;
        #[cfg(target_arch = "wasm32")]
        gloo_timers::future::TimeoutFuture::new(20).await;
        #[cfg(not(target_arch = "wasm32"))]
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if !context.current(studio) {
            studio.error.set(Some(
                "The project, part or selection changed. Preview again; nothing was changed."
                    .into(),
            ));
            studio.status.set("Timing repair cancelled".into());
            return;
        }
        let accepted = studio.try_dispatch(Action::SetHarmonies { document });
        studio.status.set(
            if accepted {
                "Harmony timing repair applied"
            } else {
                "Timing repair failed · no edit was committed"
            }
            .into(),
        );
    });
}
fn policy(divisions: &str, maximum: &str, all: bool) -> Result<RepairPolicy> {
    let policy = RepairPolicy {
        divisions: divisions
            .parse()
            .context("Use a whole number of steps per beat")?,
        maximum_shift: if all {
            None
        } else {
            Some(
                maximum
                    .parse()
                    .context("Use a nonnegative whole-number tolerance")?,
            )
        },
    };
    policy.grid(1)?;
    Ok(policy)
}

#[component]
pub fn Controls(mut studio: Studio) -> Element {
    let mut open = use_signal(|| false);
    let mut whole = use_signal(|| false);
    let mut harmony = use_signal(|| None::<Part>);
    let mut divisions = use_signal(|| "4".to_string());
    let mut tolerance = use_signal(|| "5".to_string());
    let mut all = use_signal(|| false);
    let mut result = use_signal(|| None::<Preview>);
    let mut error = use_signal(|| None::<String>);
    let busy = (studio.group_busy)();
    let available = dance::active() || studio.snapshot().is_some();
    let dance = dance::active();
    let units = if dance { "rows" } else { "ticks" };
    let unit = if dance { "row" } else { "tick" };
    let vocals = !dance
        && studio
            .snapshot()
            .is_some_and(|p| p.selected_track == "Vocals");
    let resolution = if dance {
        48
    } else {
        studio.timing.read().as_ref().map_or(480, |t| t.resolution)
    };
    let current = open()
        && result
            .read()
            .as_ref()
            .is_some_and(|p| p.context.current(studio));
    let count = result.read().as_ref().map_or(0, |p| p.report.changed);
    let report = result.read().as_ref().map(|p| p.report.clone());
    rsx! {
        button{class:"studio-tool",aria_label:"Repair note timing",disabled:!available||busy,onclick:move|_|{
            divisions.set((studio.snap)().max(1).to_string());
            tolerance.set(if dance {"1".into()}else{(studio.timing.read().as_ref().map_or(480,|t|t.resolution)/96).max(1).to_string()});
            harmony.set(None);result.set(None);error.set(None);open.set(true);
        },"Repair timing…"}
        if open(){super::modal::Modal{id:"timing-repair-dialog",label:"Repair note timing",onclose:move|_|open.set(false),
            div{class:"snap-custom-panel",onkeydown:super::shortcuts::keep_space_local,
                h2{"Repair note timing"}
                p{"Preview changes to note starts in the selected part. Note lengths and pitches stay unchanged."}
                p{class:"field-hint","This chart has {resolution} {units} per beat."}
                if !dance && studio.snapshot().is_some_and(|p|p.content.harmonies.is_some()) {
                    label{"Part" select{aria_label:"Timing repair part",value:harmony().map_or("canvas",|p|p.track()),disabled:busy,onchange:move|e|{harmony.set(Part::from_midi(&e.value()));result.set(None);error.set(None);},
                        option{value:"canvas","Active canvas part"}
                        for part in Part::ALL {option{value:part.track(),"{part.track()}"}}
                    }}
                }
                label{"Scope" select{aria_label:"Timing repair scope",value:if whole()||harmony().is_some(){"part"}else{"selection"},disabled:busy||harmony().is_some(),onchange:move|e|{whole.set(e.value()=="part");result.set(None);error.set(None);},
                    option{value:"selection","Current selection"}option{value:"part","Whole part"}
                }}
                label{"Steps per beat" input{aria_label:"Timing repair steps per beat",r#type:"number",min:"1",max:"1024",value:"{divisions}",disabled:busy,oninput:move|e|{divisions.set(e.value());result.set(None);error.set(None);}}}
                label{"Policy" select{aria_label:"Timing repair policy",value:if all(){"all"}else{"near"},disabled:busy,onchange:move|e|{all.set(e.value()=="all");result.set(None);error.set(None);},
                    option{value:"near","Near-grid notes only"}option{value:"all","Snap every note in scope"}
                }}
                label{"Maximum shift ({units})" input{aria_label:"Timing repair tolerance",r#type:"number",min:"0",value:"{tolerance}",disabled:busy||all(),oninput:move|e|{tolerance.set(e.value());result.set(None);error.set(None);}}}
                p{class:"field-hint","Near-grid mode keeps notes farther away than the chosen tolerance. Collisions refuse the complete edit."}
                if harmony().is_some() {p{class:"field-hint","Repair the whole selected harmony voice. Other singers and shared scoring/lyric ranges stay unchanged; moves outside the original ranges refuse."}}
                else if vocals {p{class:"field-hint","Complete vocal phrases carry their scoring padding. Partial phrases keep or extend their boundaries."}}
                else if !dance {p{class:"field-hint","Repair refuses moves that would combine incompatible note techniques."}}
                if dance {p{class:"field-hint","Dance has 48 rows per beat. Fine steps can share a row."}}
                else if harmony().is_none() {p{class:"field-hint","Enabled difficulty links can update their dependent parts. Undo restores the complete edit."}}
                if let Some(message)=error(){p{role:"alert","{message}"}}
                if busy{p{role:"status","Checking note timing…"}}
                if let Some(value)=report{
                    if !current{p{role:"status","The project, part or selection changed. Preview again before applying."}}
                    p{role:"status","{value.changed} of {value.examined} notes will move · {value.already_on_grid} already on grid · {value.kept_off_grid} off-grid notes kept · largest shift {value.largest_shift} {unit}" if value.largest_shift!=1 {"s"}}
                    if !value.examples.is_empty(){
                        p{class:"field-hint","First changes ({units}):"}
                        ul{for (before,after) in value.examples {li{"{before} → {after}"}}}
                    }
                }
                button{class:"action-primary",disabled:busy,onclick:move|_|{
                    let request=(||->Result<_>{Ok((Context::capture(studio).context("Open a project")?,policy(&divisions(),&tolerance(),all())?))})();
                    let(context,policy)=match request{Ok(value)=>value,Err(e)=>{error.set(Some(e.to_string()));return}};
                    let Some(guard)=super::activity::BusyGuard::start(studio.group_busy)else{return};
                    result.set(None);error.set(None);
                    spawn(async move{
                        let _guard=guard;
                        #[cfg(target_arch="wasm32")]gloo_timers::future::TimeoutFuture::new(20).await;
                        #[cfg(not(target_arch="wasm32"))]tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        match preview(studio,context,whole()||harmony().is_some(),harmony(),policy){Ok(value)=>result.set(Some(value)),Err(e)=>error.set(Some(format!("{e:#}")))}
                    });
                },"Preview timing repair"}
                button{class:"action-primary",disabled:busy||!current||count==0,onclick:move|_|{
                    let Some(value)=result()else{return};
                    if !value.context.current(studio){error.set(Some("The project, part or selection changed. Preview again; nothing was changed.".into()));return;}
                    match(value.context,value.scope){
                        (Context::Band{..},Scope::Band(selection))=>studio.edit_group(note_groups::Edit::Repair{selection,policy:value.policy}),
                        (context @ Context::Band{..},Scope::Harmony(document))=>apply_harmony(studio,context,document),
                        (Context::Dance(context),Scope::Dance(ids))=>dance::apply_timing_repair(context,ids,value.policy,studio),
                        _=>{error.set(Some("The workspace changed. Preview again.".into()));return;}
                    }
                    result.set(None);open.set(false);
                },"Apply timing repair"}
                button{onclick:move|_|open.set(false),"Close timing repair"}
            }
        }}
    }
}
