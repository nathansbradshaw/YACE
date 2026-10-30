//! Expert-derived lower difficulties, local exceptions and inspectable ancestry.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::{
    difficulty_links::{self, LinkPreview, Snapshot},
    editor::Action,
};
use dioxus::prelude::*;

type SharedSnapshot = Memo<Option<Snapshot>>;
/// Mount once in App. Every badge/inspector shares one immutable revision view.
pub fn use_snapshot(studio: Studio) {
    let view = use_memo(move || {
        let project = studio.doc.read();
        let timing = studio.timing.read();
        project
            .as_ref()
            .zip(timing.as_ref())
            .map(|(project, timing)| difficulty_links::snapshot(project, timing))
    });
    use_context_provider(|| view);
}
pub fn shared_snapshot() -> SharedSnapshot {
    use_context::<SharedSnapshot>()
}
fn family(track: &str) -> Option<&str> {
    ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
        .filter(|suffix| matches!(*suffix, "Single" | "DoubleBass" | "Drums"))
}
fn jump_note(studio: Studio, note_id: String) {
    let target = {
        let project = studio.doc.read();
        let timing = studio.timing.read();
        project
            .as_ref()
            .and_then(|doc| doc.find_note(&note_id))
            .zip(timing.as_ref())
            .map(|((track, item), timing)| {
                (
                    track.to_owned(),
                    timing.seconds(item.note.tick),
                    crate::editor::playable(track, item.note.lane),
                )
            })
    };
    if let Some((track, seconds, playable)) = target {
        studio.dispatch(Action::SetLoop { range: None });
        studio.dispatch(if playable {
            Action::SelectNote {
                note_id: Some(note_id),
            }
        } else {
            Action::SelectTrack { track }
        });
        studio.seek(seconds);
    }
}
pub fn is_override(snapshot: &Snapshot, track: &str, note_id: &str) -> bool {
    snapshot
        .tracks
        .iter()
        .find(|state| state.track == track)
        .is_some_and(|state| {
            state
                .overrides
                .iter()
                .any(|item| item.note_id == note_id && !item.suppressed)
        })
}
#[component]
pub fn TrackLinkBadge(track: String) -> Element {
    let view = shared_snapshot();
    let view = view.read();
    let Some(snapshot) = view.as_ref() else {
        return rsx! {};
    };
    if track.starts_with("Expert") && family(&track).is_some() {
        return rsx! {span{style:"color:var(--text-muted);font-size:9px;margin-left:8px;font-weight:400",title:"Linked lower difficulties derive directly from this Expert track","Master"}};
    }
    let Some(state) = snapshot.tracks.iter().find(|state| state.track == track) else {
        return rsx! {};
    };
    if !state.has_link {
        return rsx! {};
    }
    let status = if state.linked {
        "Auto-linked"
    } else {
        "Link paused"
    };
    rsx! {span{style:"color:var(--text-muted);font-size:9px;margin-left:8px;font-weight:400",title:"{state.expert_track} · {state.rule_id}","{status}" if state.override_count>0{" · {state.override_count} overrides"}}}
}
#[component]
pub fn DifficultyControls(mut studio: Studio) -> Element {
    let mut expanded = use_signal(|| false);
    let mut proposal = use_signal(|| None::<(u64, LinkPreview)>);
    let view = shared_snapshot();
    let view = view.read();
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    let Some(snapshot) = view.as_ref() else {
        return rsx! {};
    };
    let Some(suffix) = family(&doc.selected_track) else {
        return rsx! {};
    };
    let master = format!("Expert{suffix}");
    let has_master = doc.content.tracks.contains_key(&master);
    let master_jump = master.clone();
    let linked = snapshot
        .tracks
        .iter()
        .filter(|state| state.expert_track == master && state.linked)
        .count();
    let disclosure_icon = if expanded() {
        "chevron-down"
    } else {
        "chevron-right"
    };
    rsx! {
        div{class:"difficulty-controls",onkeydown:move|event|event.stop_propagation(),style:"padding:6px 12px;border-top:1px solid var(--border-color);background:var(--bg-secondary);font-size:11px;flex-shrink:0",
            div{style:"display:flex;align-items:center;gap:7px;flex-wrap:wrap",
                button{class:"studio-tool",aria_expanded:expanded().to_string(),onclick:move |_|expanded.set(!expanded()),Icon{name:disclosure_icon,size:13}"Difficulty links"}
                span{style:"color:var(--text-muted);font-size:10px",title:"Linked lower tracks update from Expert; lower edits remain local","{linked}/3 linked"}
            }
            if expanded(){
                button{class:"studio-tool",style:"margin-top:8px",title:"Expert is the master for linked reductions. Lower edits never change Expert.",disabled:!has_master,onclick:move |_|studio.dispatch(Action::SelectTrack{track:master_jump.clone()}),Icon{name:"skip-back",size:12}"Expert source"}
                for tier in ["Hard","Medium","Easy"]{
                    {let track=format!("{tier}{suffix}");let state=snapshot.tracks.iter().find(|state|state.track==track);let linked=state.is_some_and(|state|state.linked);let overrides=state.map_or(0,|state|state.override_count);let count=doc.content.tracks.get(&track).map_or(0,Vec::len);let preview_track=track.clone();let pause_track=track.clone();let jump_track=track.clone();let exists=doc.content.tracks.contains_key(&track);
                    rsx!{div{key:"{track}",style:"display:flex;gap:7px;align-items:center;flex-wrap:wrap;padding-top:6px",
                        button{class:"studio-tool",disabled:!exists,onclick:move |_|studio.dispatch(Action::SelectTrack{track:jump_track.clone()}),"{tier}"}
                        span{style:"color:var(--text-secondary);font-size:10px",title:"{count} note/control events",if linked{"Linked"}else if state.is_some_and(|state|state.has_link){"Paused"}else if exists{"Independent"}else{"Missing"}}
                        if overrides>0{span{style:"color:#c889ff;font-size:10px",title:"Local lower-difficulty changes","{overrides} overrides"}}
                        button{class:"studio-tool",disabled:!has_master,onclick:move |_|{
                            let project=studio.doc.read();let timing=studio.timing.read();
                            if let (Some(doc),Some(timing))=(project.as_ref(),timing.as_ref()){
                                match difficulty_links::preview(doc,&preview_track,timing){Ok(preview)=>proposal.set(Some((doc.revision,preview))),Err(error)=>studio.error.set(Some(format!("{error:#}")))}
                            }
                        },if linked{"Refresh…"}else if exists{"Link…"}else{"Create…"}}
                        if linked{button{class:"studio-tool",title:"Pause this link while preserving its current notes",aria_label:"Pause {tier} link",onclick:move |_|studio.dispatch(Action::EnableDifficultyLink{track:pause_track.clone(),enabled:false}),Icon{name:"pause",size:12}}}
                        TrackExceptions{studio,track:track.clone()}
                    }}}
                }
            }
            if let Some((revision,preview))=proposal(){
                {let stale=revision!=doc.revision;let track=preview.track.clone();let structural_errors=preview.rules.has_errors();let creates=!doc.content.tracks.contains_key(&preview.track);
                rsx!{div{style:"border:1px solid var(--accent-color);border-radius:5px;padding:9px;margin-top:8px",
                    strong{"{preview.expert_track} → {preview.track}"}
                    div{class:"link-preview-counts",title:"Counts include chart control events","{preview.before_count} → {preview.after_count} events"}
                    div{class:"link-preview-detail","+{preview.added_count} added · −{preview.removed_count} removed · {preview.changed_count} changed"}
                    if preview.retained_overrides>0{div{class:"link-preview-detail","{preview.retained_overrides} overrides kept"}}
                    if stale{div{style:"color:var(--warning-color);margin-top:5px","Project changed. Preview this link again before applying."}}
                    if !preview.rules.findings.is_empty(){div{style:"max-height:90px;overflow:auto;margin-top:5px",for issue in &preview.rules.findings{div{style:"font-size:10px","{issue.severity:?} · {clock(issue.seconds)} · {issue.reason}"}}}}
                    div{style:"display:flex;gap:6px;margin-top:7px",
                        button{class:"studio-tool",disabled:stale||structural_errors,onclick:move |_|{
                            if studio.doc.read().as_ref().is_some_and(|doc|doc.revision==revision){studio.dispatch(Action::EnableDifficultyLink{track:track.clone(),enabled:true});proposal.set(None);}
                        },if creates{"Create and link"}else{"Apply link"}}
                        button{class:"studio-tool",onclick:move |_|proposal.set(None),"Cancel"}
                    }
                }}}
            }
        }
    }
}

#[component]
pub fn NoteAncestry(studio: Studio, note_id: String) -> Element {
    let view = shared_snapshot();
    let view = view.read();
    let Some(snapshot) = view.as_ref() else {
        return rsx! {};
    };
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    let Some((track, _item)) = doc.find_note(&note_id) else {
        return rsx! {};
    };
    let Some(state) = snapshot.tracks.iter().find(|state| state.track == track) else {
        return if track.starts_with("Expert") {
            rsx! {div{class:"field-stack",label{class:"field-label","Difficulty source"}div{class:"difficulty-hint",title:"Linked lower difficulties retain or omit this attack according to their rules","Expert master"}}}
        } else {
            rsx! {}
        };
    };
    let override_view = state.overrides.iter().find(|item| item.note_id == note_id);
    let ancestry = state
        .ancestry
        .get(&note_id)
        .or_else(|| override_view.and_then(|item| item.ancestry.as_ref()));
    let reset_id = note_id.clone();
    let reset_track = track.to_owned();
    rsx! {div{class:"field-stack",onkeydown:move|event|event.stop_propagation(),
        label{class:"field-label","Expert source"}
        div{style:"font-size:11px;color:var(--text-secondary)",if state.linked{"Auto-linked"}else if state.has_link{"Link paused"}else{"Independent"} if override_view.is_some(){span{style:"color:#c889ff"," · Local override"}}}
        if let Some(ancestry)=ancestry{
            for parent_id in &ancestry.parent_ids{
                if let Some((parent_track,parent))=doc.find_note(parent_id){
                    {let id=parent_id.clone();rsx!{button{class:"field-button",style:"margin-top:5px",title:"Jump to {parent_track}, tick {parent.note.tick}, lane {parent.note.lane}",onclick:move |_|jump_note(studio,id.clone()),Icon{name:"skip-back",size:12}" Expert · tick {parent.note.tick}"}}}
                }else{div{class:"difficulty-hint","Expert source removed: {parent_id}"}}
            }
            details{class:"review-evidence",summary{"Reduction rules"}div{class:"review-evidence-body","Tick {ancestry.source_tick} · lanes {ancestry.source_lanes:?} → {ancestry.mapped_lane}" div{class:"review-evidence-values",{format!("{} · {}", ancestry.role, ancestry.rule_ids.join(", "))}}}}
        }else if state.has_link{div{class:"difficulty-hint","Manual addition"}}else{div{class:"difficulty-hint","Independent authored note"}}
        if let Some(override_view)=override_view{
            if override_view.orphaned{div{style:"font-size:11px;color:var(--warning-color)","Expert parent changed or disappeared. Review this retained override."}}
            button{class:"field-button",disabled:!state.linked,title:if state.linked{"Restore this Expert-derived reduction"}else{"Resume the link before resetting overrides"},style:"margin-top:6px",onclick:move |_|studio.dispatch(Action::ClearDifficultyOverrides{track:reset_track.clone(),note_id:Some(reset_id.clone())}),"Reset this override"}
        }
        for finding in state.findings.iter().filter(|finding|finding.note_id.as_ref()==Some(&note_id)){
            div{style:"font-size:10px;color:var(--warning-color);margin-top:5px","{finding.reason}"}
        }
    }}
}

#[component]
fn TrackExceptions(studio: Studio, track: String) -> Element {
    let mut expanded = use_signal(|| false);
    let view = shared_snapshot();
    let view = view.read();
    let Some(snapshot) = view.as_ref() else {
        return rsx! {};
    };
    let Some(state) = snapshot.tracks.iter().find(|state| state.track == track) else {
        return rsx! {};
    };
    let count = state.overrides.len() + state.findings.len() + state.rules.findings.len();
    if count == 0 {
        return rsx! {};
    }
    let project = studio.doc.read();
    let Some(doc) = project.as_ref() else {
        return rsx! {};
    };
    rsx! {div{style:"flex-basis:100%;padding-left:6px",
        button{class:"studio-tool",onclick:move |_|expanded.set(!expanded()),Icon{name:"flag",size:12}"{count} checks"}
        if expanded(){div{style:"max-height:160px;overflow:auto;padding:5px 0;font-size:10px",
            for item in &state.overrides{
                {let reset_id=item.note_id.clone();let reset_track=track.clone();let select_id=item.note_id.clone();let exists=doc.find_note(&item.note_id).is_some();
                rsx!{div{style:"display:flex;gap:6px;align-items:center;padding:3px 0;flex-wrap:wrap",
                    span{style:"color:#c889ff",if item.suppressed{"Deleted lower attack"}else{"Local note override"} if item.orphaned{" · Expert source changed"}}
                    if exists{button{class:"studio-tool",onclick:move |_|jump_note(studio,select_id.clone()),"Jump to note"}}
                    button{class:"studio-tool",disabled:!state.linked,title:if state.linked{"Discard this local change and resume Expert-derived reduction"}else{"Resume the link before resetting overrides"},onclick:move |_|studio.dispatch(Action::ClearDifficultyOverrides{track:reset_track.clone(),note_id:Some(reset_id.clone())}),"Reset override"}
                }}}
            }
            for finding in &state.findings{div{style:"padding:3px 0;color:var(--warning-color)","{finding.code}: {finding.reason}"}}
            for finding in &state.rules.findings{
                {let seconds=finding.seconds;let issue_track=track.clone();rsx!{div{style:"padding:3px 0",
                    button{class:"studio-tool",onclick:move |_|{studio.dispatch(Action::SelectTrack{track:issue_track.clone()});studio.seek(seconds);},"{clock(seconds)}"}
                    span{style:"margin-left:5px;color:var(--warning-color)","{finding.severity:?}: {finding.reason}"}
                }}}
            }
        }}
    }}
}
