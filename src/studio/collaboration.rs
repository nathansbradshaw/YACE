//! Native proposal review controls, using the same commands as portable exchange.
use super::dialogs as rfd;
use super::state::Studio;
use crate::{
    collaboration::{self, Operation, Proposal},
    collaboration_exchange as exchange,
    editor::{Action, Actor, ActorKind, EditorNote, Store},
};
use dioxus::prelude::*;

#[component]
pub fn Collaboration(mut studio: Studio) -> Element {
    let mut open = studio.collaboration_open;
    let mut body = use_signal(String::new);
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let doc = studio.snapshot();
    let entries = doc
        .as_ref()
        .and_then(|d| collaboration::inspect(d).ok())
        .unwrap_or_default();
    rsx! {
        if open() {
            super::modal::Modal { id: "collaboration-dialog", label: "Collaborative song reviews", onclose: move |_| open.set(false),
                div { class: "collaboration-modal", style: "background:var(--bg-secondary);padding:24px;width:900px;max-width:95vw;max-height:85vh;overflow:auto",
                    button { class: "field-button", onclick: move |_| open.set(false), "Close" }
                    h2 { "Collaborative reviews" }
                    if let Some(error) = (studio.error)() {
                        p { role: "alert", "{error}" }
                    }
                    p { "Share review files through Git or a shared folder. Audio stays local." }
                    details { summary { "How sharing works" } p { "Work in your own project, then exchange review files. No server is needed. Names identify contributors locally; they aren’t verified accounts." } }
                    GitReview { studio }
                    if !cfg!(feature="browser") { button { class: "field-button", onclick: move |_| { spawn(async move {
                        if let Some(folder) = rfd::AsyncFileDialog::new().set_title("Choose Git/shared review exchange folder").pick_folder().await {
                            let store = studio.store.read().clone();
                            if let (Some(store),Some(doc)) = (store,studio.snapshot()) {
                                let actor = Actor { kind:ActorKind::Human,name:studio.reviewer.read().clone() };
                                match exchange::sync(&store,folder.path(),actor,doc.revision) {
                                    Ok(_) => { studio.error.set(None); studio.status.set("Review files synchronized".into()); studio.reload(false); },
                                    Err(error) => studio.error.set(Some(error.to_string())),
                                }
                            }
                        }
                    }); }, "Sync Git / shared folder…" } }
                    button { class: "field-button", onclick: move |_| { spawn(async move {
                        if let Some(path) = rfd::AsyncFileDialog::new().set_file_name("reviews.json").save_file().await
                            && let Some(doc) = studio.snapshot() {
                            match exchange::write_bundle(&doc,path.path()) {
                                Ok(())=>rfd::download(path.path()).await,
                                Err(error)=>studio.error.set(Some(error.to_string())),
                            }
                        }
                    }); }, "Export complete review bundle…" }
                    button { class: "field-button", "data-browser-pick":"file","data-browser-accept":".json", onclick: move |_| { spawn(async move {
                        if let Some(path) = rfd::AsyncFileDialog::new().add_filter("Review bundle", &["json"]).pick_file().await {
                            match exchange::read_bundle(path.path()) {
                                Ok(bundle) => studio.dispatch(Action::ImportCollaboration { bundle }),
                                Err(error) => studio.error.set(Some(error.to_string())),
                            }
                        }
                    }); }, "Import review bundle…" }
                    label { r#for: "collaboration-reviewer", "Your name" }
                    input { id: "collaboration-reviewer", class: "field-control", value: "{studio.reviewer}", oninput: move |e| studio.reviewer.set(e.value()) }
                    label { r#for: "collaboration-title", "Proposal title" }
                    input { id: "collaboration-title", class: "field-control", value: "{title}", oninput: move |e| title.set(e.value()) }
                    label { r#for: "collaboration-description", "Proposal description / review comment" }
                    textarea { id: "collaboration-description", class: "field-control", value: "{description}", oninput: move |e| description.set(e.value()) }
                    button { class: "field-button", "data-browser-pick":"file", "data-browser-accept":".zip", onclick: move |_| { spawn(async move {
                        let result = async {
                            let base = rfd::AsyncFileDialog::new().set_title("Select unchanged base project").pick_folder().await.ok_or_else(|| anyhow::anyhow!("Cancelled"))?;
                            let doc = studio.snapshot().ok_or_else(|| anyhow::anyhow!("No project"))?;
                            let id = format!("proposal-{}", crate::storage::unique_id());
                            let p = Proposal::between(&Store::new(base.path()).load()?, &doc, id, title(), description(), Actor { kind: ActorKind::Human, name: studio.reviewer.read().clone() })?;
                            let output = rfd::AsyncFileDialog::new().set_file_name("proposal.json").save_file().await.ok_or_else(|| anyhow::anyhow!("Cancelled"))?;
                            crate::storage::write_new(output.path(),&serde_json::to_vec_pretty(&p)?)?;
                            rfd::download(output.path()).await;
                            studio.dispatch(Action::Collaborate { operation:Operation::Submit { proposal:p } });
                            Ok::<_,anyhow::Error>(())
                        }.await;
                        if let Err(error) = result { studio.error.set(Some(error.to_string())); }
                    }); }, "Export changes against base project…" }
                    button { class: "field-button", "data-browser-pick":"file","data-browser-accept":".json", onclick: move |_| { spawn(async move {
                        if let Some(path) = rfd::AsyncFileDialog::new().add_filter("Proposal JSON", &["json"]).pick_file().await {
                            match crate::storage::read(path.path()).map_err(anyhow::Error::from).and_then(|bytes| Ok(serde_json::from_slice::<Proposal>(&bytes)?)) {
                                Ok(proposal) => studio.dispatch(Action::Collaborate { operation: Operation::Submit { proposal } }),
                                Err(error) => studio.error.set(Some(error.to_string())),
                            }
                        }
                    }); }, "Import proposal…" }
                    for (id, entry) in entries {
                        {
                            let fingerprint = entry.proposal.fingerprint().unwrap_or_default();
                            let changed = entry.proposal.base.iter().map(|(t,notes)| notes.iter().filter(|n| !entry.proposal.proposed[t].contains(n)).count() + entry.proposal.proposed[t].iter().filter(|n| !notes.contains(n)).count()).sum::<usize>();
                            let json = serde_json::to_string_pretty(&entry).unwrap_or_default();
                            rsx! {
                                section { style: "border-top:1px solid #777;padding:12px 0",
                                    h3 { "{entry.proposal.title} · {entry.status}" }
                                    p { "{entry.proposal.description}" }
                                    p { "By {entry.proposal.author.name}; {changed} note records added/removed (a moved note counts twice)." }
                                    for change in collaboration::diff(&entry.proposal) {
                                        {
                                            let before = change.before.map(|n| format!("tick {}, lane {}, sustain {}", n.note.tick, n.note.lane, n.note.sustain)).unwrap_or_else(|| "absent".into());
                                            let after = change.after.map(|n| format!("tick {}, lane {}, sustain {}", n.note.tick, n.note.lane, n.note.sustain)).unwrap_or_else(|| "absent".into());
                                            let note = entry.proposal.proposed.get(&change.track).and_then(|notes| notes.iter().find(|n| n.id==change.note_id)).cloned();
                                            rsx! {
                                                p { "{change.track} / {change.note_id}: {before} → {after}" }
                                                if entry.status == "open" && let Some(note) = note {
                                                    SuggestNote { studio, proposal_id:id.clone(), fingerprint:fingerprint.clone(), track:change.track.clone(), note }
                                                }
                                            }
                                        }
                                    }
                                    details { summary { "Notes, review history and anchored discussions" } pre { style: "white-space:pre-wrap", "{json}" } }
                                    if entry.status == "open" {
                                        ResolveNotes { studio, proposal:entry.proposal.clone() }
                                    }
                                    div {
                                        for (approve,label) in [(true,"Approve"),(false,"Request changes")] {
                                            { let id = id.clone(); let fp = fingerprint.clone(); rsx! { button { class: "field-button", onclick: move |_| studio.dispatch(Action::Collaborate { operation: Operation::Review { proposal_id: id.clone(), fingerprint: fp.clone(), approve, body: description() } }), "{label}" } } }
                                        }
                                        { let id = id.clone(); rsx! { button { class: "field-button", onclick: move |_| {
                                            studio.dispatch(Action::Collaborate { operation: Operation::Comment { proposal_id: id.clone(), thread_id: unique_id("discussion"), anchor: None, body: description() } });
                                        }, "Flag / start discussion" } } }
                                        for (thread_id, thread) in entry.threads {
                                            { let id = id.clone(); let reply_id = id.clone(); let reply_thread = thread_id.clone(); let anchor=thread.anchor.clone(); rsx! {
                                                div { p { "Discussion {thread_id}" }
                                                    for message in thread.messages { p { "{message.actor.name}: {message.body}" } }
                                                    button { class:"field-button",onclick:move |_| studio.dispatch(Action::Collaborate { operation:Operation::Comment { proposal_id:reply_id.clone(),thread_id:reply_thread.clone(),anchor:anchor.clone(),body:description() } }),"Reply with comment above" }
                                                    button { class: "field-button", onclick: move |_| studio.dispatch(Action::Collaborate { operation: Operation::SetResolved { proposal_id: id.clone(), thread_id: thread_id.clone(), resolved: !thread.resolved } }), if thread.resolved { "Reopen discussion" } else { "Resolve discussion" } }
                                                }
                                            } }
                                        }
                                        for (suggestion_id,suggestion) in entry.suggestions {
                                            { let id=id.clone(); rsx! {
                                                p { "Suggestion from {suggestion.actor.name}: {suggestion.body}" }
                                                button { class:"field-button",disabled:entry.status!="open",onclick:move |_| studio.dispatch(Action::Collaborate { operation:Operation::AcceptSuggestion { proposal_id:id.clone(),suggestion_id:suggestion_id.clone(),new_proposal_id:unique_id("proposal") } }),"Accept as new proposal (requires new approval)" }
                                            } }
                                        }
                                        { let id = id.clone(); let fp = fingerprint.clone(); rsx! { button { class: "field-button",disabled:entry.status!="open", onclick: move |_| studio.dispatch(Action::Collaborate { operation: Operation::Merge { proposal_id: id.clone(), fingerprint: fp.clone() } }), "Merge approved proposal" } } }
                                        { let id = id.clone(); rsx! { button { class: "field-button",disabled:entry.status!="open", onclick: move |_| studio.dispatch(Action::Collaborate { operation: Operation::Close { proposal_id: id.clone(), reason: description() } }), "Close proposal" } } }
                                    }
                                }
                            }
                        }
                    }
                    details { summary { "Advanced: apply an anchored comment or review operation" }
                        textarea { class: "field-control", value: "{body}", oninput: move |e| body.set(e.value()) }
                        button { class: "field-button", onclick: move |_| match serde_json::from_str::<Operation>(&body()) {
                            Ok(operation) => studio.dispatch(Action::Collaborate { operation }),
                            Err(error) => studio.error.set(Some(error.to_string())),
                        }, "Apply operation JSON" }
                    }
                }
            }
        }
    }
}

fn unique_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
#[component]
fn SuggestNote(
    mut studio: Studio,
    proposal_id: String,
    fingerprint: String,
    track: String,
    note: EditorNote,
) -> Element {
    let initial = note.note.clone();
    let mut tick = use_signal(move || initial.tick.to_string());
    let mut lane = use_signal(move || initial.lane.to_string());
    let mut sustain = use_signal(move || initial.sustain.to_string());
    let mut body = use_signal(String::new);
    let flag_proposal = proposal_id.clone();
    let flag_track = track.clone();
    let flag_note = note.clone();
    rsx! {
        details { summary { "Flag or suggest an edit to this note" }
            label { "Tick" } input { class:"field-control",value:"{tick}",oninput:move |e| tick.set(e.value()) }
            label { "Lane" } input { class:"field-control",value:"{lane}",oninput:move |e| lane.set(e.value()) }
            label { "Sustain" } input { class:"field-control",value:"{sustain}",oninput:move |e| sustain.set(e.value()) }
            label { "Reason / comment" } input { class:"field-control",value:"{body}",oninput:move |e| body.set(e.value()) }
            button { class:"field-button",onclick:move |_| {
                let result = (|| -> anyhow::Result<_> {
                    let mut replacement=note.clone(); replacement.note.tick=tick().parse()?; replacement.note.lane=lane().parse()?; replacement.note.sustain=sustain().parse()?;
                    Ok(Operation::Suggest { proposal_id:proposal_id.clone(),fingerprint:fingerprint.clone(),suggestion_id:unique_id("suggestion"),track:track.clone(),note_id:note.id.clone(),replacement:Some(replacement),body:body() })
                })();
                match result { Ok(operation)=>studio.dispatch(Action::Collaborate { operation }),Err(error)=>studio.error.set(Some(error.to_string())) }
            },"Suggest edit" }
            button { class:"field-button",onclick:move |_| studio.dispatch(Action::Collaborate { operation:Operation::Comment { proposal_id:flag_proposal.clone(),thread_id:unique_id("discussion"),anchor:Some(collaboration::Anchor { track:flag_track.clone(),start_tick:flag_note.note.tick,end_tick:flag_note.note.tick.saturating_add(flag_note.note.sustain),note_id:Some(flag_note.id.clone()) }),body:body() } }),"Flag this note" }
        }
    }
}

#[component]
fn ResolveNotes(mut studio: Studio, proposal: Proposal) -> Element {
    let mut choices =
        use_signal(std::collections::BTreeMap::<String, collaboration::ConflictChoice>::new);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let mut preview_key = use_signal(String::new);
    let key = serde_json::to_string(&(&proposal.id, &doc.content.tracks)).unwrap_or_default();
    use_effect(move || {
        if preview_key() != key {
            preview_key.set(key.clone());
            choices.set(Default::default());
        }
    });
    let plan = collaboration::plan_merge(&proposal.base, &doc.content.tracks, &proposal.proposed);
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => return rsx! {p{role:"alert","{error}"}},
    };
    if plan.conflicts.is_empty() {
        return rsx! {};
    };
    let timing = studio.timing.read().clone();
    let describe = |track: &str, note: Option<EditorNote>| {
        note.map(|n| {
            let lane =
                crate::octave_adapter::lane_name(track.ends_with("Drums"), n.note.lane, false)
                    .map(String::from)
                    .unwrap_or_else(|| format!("lane/control {}", n.note.lane));
            if let Some(timing) = &timing {
                format!(
                    "{} at beat {:.2} ({:.2}s), hold {:.2} beats",
                    lane,
                    1.0 + n.note.tick as f64 / f64::from(timing.resolution),
                    timing.seconds(n.note.tick),
                    n.note.sustain as f64 / f64::from(timing.resolution)
                )
            } else {
                format!("{lane}, tick {}, sustain {}", n.note.tick, n.note.sustain)
            }
        })
        .unwrap_or_else(|| "deleted / absent".into())
    };
    let current = doc.content.tracks.clone();
    let duration = doc.duration;
    let count = plan.conflicts.len();
    rsx! {
        section { style:"border:1px solid #bd9049;padding:12px",
            h4 { "Resolve conflicting notes" }
            p { "Choose the musical result for each note. Saving creates a new proposal for review; it does not overwrite your song or carry over approval." }
            for conflict in plan.conflicts {
                { let key=conflict.key.clone();let listen=conflict.current.as_ref().or(conflict.proposed.as_ref()).map(|n|n.note.tick);let track=conflict.track.clone();let instrument=super::state::instrument(&track);let before=describe(&track,conflict.base);let ours=describe(&track,conflict.current);let theirs=describe(&track,conflict.proposed);
                    rsx!{ div {
                        strong { "{instrument} · conflicting note" }
                        p { "Before: {before}" } p { "Your current note: {ours}" } p { "Proposed note: {theirs}" }
                        button {class:"field-button",onclick:move |_| {if let Some(tick)=listen {let timing=studio.timing.read().clone();if let Some(timing)=timing {let start=(timing.seconds(tick)-0.5).clamp(0.0,(duration-0.1).max(0.0));let end=(start+1.5).min(duration);studio.dispatch(Action::SelectTrack {track:track.clone()});studio.dispatch(Action::SetLoop {range:Some(crate::editor::TimeRange {start,end})});studio.seek(start);studio.dispatch(Action::Playback {playing:true});}}},"Listen around this section"}
                        select { aria_label:"Result for conflicting note",value:choices.read().get(&key).map(|c|format!("{c:?}")).unwrap_or_default(),onchange:move |event| {
                            let choice=match event.value().as_str(){"Current"=>Some(collaboration::ConflictChoice::Current),"Proposed"=>Some(collaboration::ConflictChoice::Proposed),"Delete"=>Some(collaboration::ConflictChoice::Delete),_=>None};
                            if let Some(choice)=choice {choices.write().insert(key.clone(),choice);}else{choices.write().remove(&key);}
                        },option { value:"","Choose a result…" }option {value:"Current","Keep my current note"}option {value:"Proposed","Use the proposed note"}option {value:"Delete","Remove this note"} }
                    }}
                }
            }
            button {class:"field-button",disabled:choices.read().len()!=count,onclick:move |_| {
                let result=proposal.fingerprint();
                match result {Ok(fingerprint)=>studio.dispatch(Action::Collaborate {operation:Operation::ResolveConflicts {proposal_id:proposal.id.clone(),fingerprint,current:current.clone(),choices:choices(),new_proposal_id:unique_id("resolved")}}),Err(error)=>studio.error.set(Some(error.to_string()))}
            },"Save conflict choices"}
        }
    }
}

#[cfg(not(feature = "browser"))]
#[component]
fn GitReview(mut studio: Studio) -> Element {
    let mut folder = use_signal(|| None::<std::path::PathBuf>);
    let mut remote = use_signal(String::new);
    let mut message = use_signal(String::new);
    let mut conflicts = use_signal(Vec::<crate::review_git::FileConflict>::new);
    let mut busy = use_signal(|| false);
    rsx! {
        details {summary {"Share with Git"}
            p {"Use a separate review repo. Publishing uses your existing Git login."}
            label {r#for:"review-git-remote","Remote URL or shared Git path (optional)"}
            input {id:"review-git-remote",class:"field-control",value:"{remote}",oninput:move |event|remote.set(event.value())}
            button {class:"field-button",onclick:move |_| {spawn(async move {
                if let Some(handle)=rfd::AsyncFileDialog::new().set_title("Choose an empty folder for a new review repository").pick_folder().await {let path=handle.path().to_owned();
                    match crate::review_git::create(&path,Some(&remote())){Ok(())=>{folder.set(Some(path));message.set("Review repository created. Save / Pull / Publish to save its first review history.".into());},Err(error)=>message.set(error.to_string())}
                }
            });},"Create review repository…"}
            button {class:"field-button",disabled:busy(),onclick:move |_| {spawn(async move {
                if let Some(handle)=rfd::AsyncFileDialog::new().set_title("Choose an empty folder to clone into").pick_folder().await {let path=handle.path().to_owned();
                    let url=remote();let target=path.clone();busy.set(true);message.set("Cloning review repository…".into());
                    spawn(async move { let result=tokio::task::spawn_blocking(move ||crate::review_git::clone_repository(&url,&target)).await;busy.set(false);match result{Ok(Ok(()))=>{folder.set(Some(path));message.set("Repository cloned; load its reviews with Save / Pull / Publish.".into());},Ok(Err(error))=>message.set(error.to_string()),Err(error)=>message.set(error.to_string())} });
                }
            });},"Clone shared review repository…"}
            button {class:"field-button",onclick:move |_| {spawn(async move {
                if let Some(path)=rfd::AsyncFileDialog::new().set_title("Choose the dedicated review repository root").pick_folder().await {folder.set(Some(path.path().to_owned()));}
            });},"Choose existing review repository…"}
            if let Some(path)=folder(){p {"Repository: {path.display()}"}}
            button {class:"field-button",disabled:folder.read().is_none()||busy(),onclick:move |_| {
                let store=studio.store.read().clone();
                if let (Some(folder),Some(store),Some(doc))=(folder(),store,studio.snapshot()){
                    let actor=Actor {kind:ActorKind::Human,name:studio.reviewer.read().clone()};
                    let remote=remote();busy.set(true);message.set("Saving and synchronizing review history…".into());
                    spawn(async move {let result=tokio::task::spawn_blocking(move ||->anyhow::Result<_>{if !remote.trim().is_empty(){crate::review_git::connect(&folder,&remote)?;}crate::review_git::sync(&store,&folder,actor,doc.revision)}).await;
                        busy.set(false);match result {Ok(Ok(outcome))=>{message.set(outcome.message);conflicts.set(outcome.conflicts);},Ok(Err(error))=>message.set(format!("{error}. Your existing files and Git history are retained.")),Err(error)=>message.set(error.to_string())}studio.reload(false);
                    });
                }
            },"Save / Pull / Publish reviews"}
            p {role:"status","{message}"}
            for conflict in conflicts(){
                div {p {"Review-file conflict: {conflict.path}"}
                    for (incoming,label,valid) in [(false,"Keep valid local file",conflict.local_valid),(true,"Use valid incoming file",conflict.incoming_valid)]{
                        {let path=conflict.path.clone();rsx!{button {class:"field-button",disabled:!valid,onclick:move |_|{
                            if let Some(folder)=folder(){match crate::review_git::resolve_file(&folder,&path,incoming){Ok(())=>match crate::review_git::conflicts(&folder){Ok(remaining)=>{conflicts.set(remaining);message.set("Choice saved. Save / Pull / Publish to finish.".into());},Err(error)=>message.set(error.to_string())},Err(error)=>message.set(error.to_string())}}
                        },"{label}"}}}
                    }
                }
            }
        }
    }
}

#[cfg(feature = "browser")]
#[component]
fn GitReview(studio: Studio) -> Element {
    let _ = studio;
    rsx! {p{class:"field-hint","Export and import review bundles to exchange proposals, comments and approvals."}}
}
