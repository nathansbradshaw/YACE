//! Dance supplies instrument content to the normal workspace's three panes.
use super::super::state::{EditTool, Studio};
use super::*;
use sm::project::SelectionEdit;

fn beat(timing: &Timing, seconds: f64) -> f64 {
    match timing.position(seconds) {
        AudioPosition::Beat(b)
        | AudioPosition::Delay { beat: b }
        | AudioPosition::Stop { beat: b } => b,
        AudioPosition::Warp { to, .. } => to,
    }
}
fn selected_chart(s: DanceStudio) -> Option<Chart> {
    s.project
        .read()
        .as_ref()?
        .content
        .document
        .charts
        .iter()
        .find(|c| c.id == (s.selected)())
        .cloned()
}
fn timing(s: DanceStudio) -> Timing {
    s.project
        .read()
        .as_ref()
        .and_then(|p| {
            p.content
                .document
                .effective_timing(&(s.selected)())
                .ok()
                .cloned()
        })
        .unwrap_or_default()
}
#[component]
pub fn Audition(studio: Studio) -> Element {
    let s = use_context::<DanceStudio>();
    let mut cmod = use_signal(|| false);
    let mut zoom = use_signal(|| 1_f64);
    let chart = use_memo(move || selected_chart(s));
    let position = super::super::session::position(studio);
    let playing = super::super::session::transport(studio).is_some_and(|t| t.playing && t.ready);
    let timing = use_memo(move || timing(s));
    let beat = beat(&timing.read(), position);
    let revision = s.project.read().as_ref().map_or(0, |p| p.revision);
    let motion = if uses_recording(studio) {
        studio.snapshot().map_or(0, |p| p.revision)
    } else {
        (s.motion_epoch)()
    };
    let identity = format!("{}:{}:{revision}:{motion}", (s.epoch)(), (s.selected)());
    let title = tracks(studio)
        .into_iter()
        .find(|(id, _)| id == &format!("dance:{}", (s.selected)()))
        .map_or(String::new(), |(_, title)| title);
    rsx! {section{class:"audition-view", "data-preview-position":"{position}", "data-preview-playing":"{playing}",
        header{class:"desk-heading audition-heading",h2{"Audition"}span{class:"audition-track","{title}"}span{class:"audition-mode","Autoplay"}}
        div{class:"audition-surface dance-audition",
            if chart.read().as_ref().is_some_and(|c|c.stepstype=="dance-single"&&c.read_only_reason.is_none()) {
                super::preview::Preview{chart,timing,beat,seconds:position,playing,cmod:cmod(),identity,zoom:zoom()}
            }else{div{class:"studio-empty-preview","This imported chart is preserved read-only."}}
        }
        super::super::practice::PracticeControls{studio,
            div{class:"audition-settings",label{class:"form-check",input{r#type:"checkbox",checked:cmod(),onchange:move|e|cmod.set(e.checked())}span{"Constant scroll speed"}}}
            label{class:"game-preview-lookahead",span{"Arrow size"}select{aria_label:"Arrow size",value:"{zoom}",onchange:move|e|if let Ok(value)=e.value().parse::<f64>(){zoom.set(value.clamp(0.5,1.5));},for (value,label)in[(0.75,"75%"),(1.,"100%"),(1.25,"125%")]{option{value:"{value}",selected:value==zoom(),"{label}"}}}}
        }
    }}
}
#[derive(Clone)]
enum Drag {
    Box {
        row: i64,
        lane: u8,
        x: f64,
        y: f64,
        additive: bool,
    },
    Note {
        note: sm::Note,
        x: f64,
        y: f64,
        resize: bool,
        revision: u64,
    },
}
fn row_at(start: i64, span: f64, x: f64, width: f64, snap: i64) -> i64 {
    let row = start as f64 + x / width.max(1.) * span * 48.;
    ((row / snap as f64).round() as i64 * snap).clamp(0, sm::MAX_ROW)
}
fn place(s: DanceStudio, row: i64, lane: u8) {
    let kind = (s.note_kind)();
    s.apply(Action::AddNote {
        chart_id: (s.selected)(),
        row,
        lane,
        kind,
        end_row: kind.sustained().then_some(row + 48),
    });
}
#[component]
pub fn Roll(mut studio: Studio) -> Element {
    let mut s = use_context::<DanceStudio>();
    let mut width = use_signal(|| 1000.);
    let mut drag = use_signal(|| None::<Drag>);
    let mut suppress = use_signal(|| false);
    let mut delta = use_signal(|| (0., 0.));
    use_effect(move || {
        if (studio.follow)() && super::super::session::transport(studio).is_some_and(|t| t.playing)
        {
            let row =
                (beat(&timing(s), super::super::session::position(studio)) * 48.).max(0.) as i64;
            let span = ((studio.span)() * 48.).max(1.) as i64;
            if row < (s.view_row)() || row >= (s.view_row)() + span {
                s.view_row.set(row / span * span);
            }
        }
    });
    let Some(chart) = selected_chart(s) else {
        return rsx! {p{class:"selection-empty","Add a difficulty to begin charting."}};
    };
    let map = timing(s);
    let start = (s.view_row)();
    let span = (studio.span)();
    let snap = (s.snap_rows)();
    let end = (start as f64 + span * 48.).min(sm::MAX_ROW as f64) as i64;
    let max_row = s
        .project
        .read()
        .as_ref()
        .map_or(192, |p| {
            p.content
                .document
                .charts
                .iter()
                .flat_map(|c| c.notes.iter())
                .map(|n| n.end_row.unwrap_or(n.row))
                .max()
                .unwrap_or(0)
                + 192
        })
        .max(end);
    let cursor = (s.cursor)();
    let play_row = beat(&map, super::super::session::position(studio)) * 48.;
    let readonly = chart.read_only_reason.is_some();
    let revision = s.project.read().as_ref().map_or(0, |p| p.revision);
    let notes = chart.notes.clone();
    let drag_notes = notes.clone();
    let x = move |r: f64| (r - start as f64) / (span * 48.) * width();
    rsx! {
        div{class:"chart-desk-scroll",
            div{class:"chart-desk-ruler lane-surface",
                span{class:"dance-ruler-label","Beat"}
                svg{class:"midi-ruler-canvas",view_box:"0 0 {width} 32",preserve_aspect_ratio:"none",
                    onclick:move |e:MouseEvent|{let row=row_at(start,span,e.element_coordinates().x,width(),snap);s.cursor.set(row);super::super::session::seek(studio,map.seconds(row as f64/48.,sm::timing::Boundary::Note));},
                    for r in (start/48*48..=end).step_by(48){text{x:"{x(r as f64)+3.}",y:"20",fill:"var(--text-secondary)",font_size:"10",pointer_events:"none","{r/48}"}}
                }
            }
            section{class:"chart-desk-track",
                header{class:"lane-track-header",strong{if chart.profile.unwrap_or(Profile::Pad)==Profile::Pad{"Dance"}else{"4-key"}}span{"{chart.notes.len()} notes · {chart.difficulty} · {chart.meter}"}}
                if let Some(reason)=chart.read_only_reason{p{class:"field-hint","Preserved read-only: {reason}"}}
                div{class:"lane-surface",
                    ol{class:"lane-labels",for arrow in ["← Left","↓ Down","↑ Up","→ Right"]{li{class:"lane-label",style:"height:40px","{arrow}"}}}
                    div{class:"lane-plot",onresize:move|e|if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.));},
                        svg{class:"studio-note-svg",height:"160",view_box:"0 0 {width} 160",preserve_aspect_ratio:"none","aria-label":"Stable beat grid",
                            onmousedown:move|e:MouseEvent|{if (studio.tool)()==EditTool::Select{drag.set(Some(Drag::Box{row:row_at(start,span,e.element_coordinates().x,width(),1),lane:(e.element_coordinates().y/40.).floor().clamp(0.,3.)as u8,x:e.page_coordinates().x,y:e.page_coordinates().y,additive:e.modifiers().shift()}));delta.set((0.,0.));}},
                            onmousemove:move|e:MouseEvent|{if let Some(d)=drag.read().as_ref(){let (x,y)=match d{Drag::Box{x,y,..}|Drag::Note{x,y,..}=>(*x,*y)};delta.set((e.page_coordinates().x-x,e.page_coordinates().y-y));}},
                            onmouseleave:move |_|drag.set(None),
                            onmouseup:move|e:MouseEvent|{
                                let Some(d)=drag.take()else{return};let (ox,oy)=match &d{Drag::Box{x,y,..}|Drag::Note{x,y,..}=>(*x,*y)};
                                let dx=e.page_coordinates().x-ox;let dy=e.page_coordinates().y-oy;
                                if dx.abs()+dy.abs()<3. {return;}suppress.set(true);
                                match d {
                                    Drag::Box{row,lane,additive,..}=>{let last=row_at(start,span,x(row as f64)+dx,width(),1);let last_lane=(lane as f64+dy/40.).floor().clamp(0.,3.)as u8;if !additive{s.selection.set(Default::default());}for n in &drag_notes{if (row.min(last)..=row.max(last)).contains(&n.row)&&(lane.min(last_lane)..=lane.max(last_lane)).contains(&n.lane){s.selection.write().insert(n.id.clone());}}},
                                    Drag::Note{mut note,resize,revision:expected,..}=>{
                                        if readonly||s.project.read().as_ref().is_none_or(|p|p.revision!=expected){return;}
                                        let rows=((dx/width()*span*48.)/snap as f64).round()as i64*snap;
                                        if resize{note.end_row=Some((note.end_row.unwrap_or(note.row+48)+rows).max(note.row+1));s.apply(Action::UpdateNote{chart_id:(s.selected)(),note});}
                                        else if s.selection.read().len()>1{s.edit_selection(SelectionEdit::MoveLanes{rows,lanes:(dy/40.).round().clamp(-4.,4.)as i8});}
                                        else{let destination=note.row+rows;if destination<0{return;}note.end_row=note.end_row.map(|end|end+rows);note.row=destination;note.lane=(note.lane as i16+(dy/40.).round()as i16).clamp(0,3)as u8;s.apply(Action::UpdateNote{chart_id:(s.selected)(),note});}
                                    }
                                }
                            },
                            onclick:move|e:MouseEvent|{if suppress.replace(false){return;}studio.focus_editor();let row=row_at(start,span,e.element_coordinates().x,width(),snap);s.cursor.set(row);if (studio.tool)()==EditTool::Place&&!readonly{place(s,row,(e.element_coordinates().y/40.).floor().clamp(0.,3.)as u8);}else if (studio.tool)()==EditTool::Select&&!e.modifiers().shift(){s.selection.set(Default::default());}},
                            ondoubleclick:move|e:MouseEvent|{if (studio.tool)()==EditTool::Select&&!readonly{place(s,row_at(start,span,e.element_coordinates().x,width(),snap),(e.element_coordinates().y/40.).floor().clamp(0.,3.)as u8);}},
                            for lane in 0..4 {line{x1:"0",x2:"{width}",y1:"{lane*40}",y2:"{lane*40}",stroke:"var(--border-color)",pointer_events:"none"}}
                            for r in (start/12*12..=end).step_by(12){line{x1:"{x(r as f64)}",x2:"{x(r as f64)}",y1:"0",y2:"160",stroke:"var(--border-color)",stroke_opacity:if r%48==0{"1"}else{"0.35"},pointer_events:"none"}}
                            for n in notes.iter().filter(|n|n.row<=end&&n.end_row.unwrap_or(n.row)>=start){
                                g{key:"{n.id}","data-note-id":"{n.id}",
                                    onmousedown:{let n=n.clone();move|e:MouseEvent|{e.stop_propagation();if (studio.tool)()==EditTool::Select&&!e.modifiers().shift(){if !s.selection.read().contains(&n.id){s.selection.set([n.id.clone()].into());}drag.set(Some(Drag::Note{note:n.clone(),x:e.page_coordinates().x,y:e.page_coordinates().y,resize:false,revision}));}}},
                                    onclick:{let n=n.clone();move|e:MouseEvent|{e.stop_propagation();if suppress.replace(false){return;}studio.focus_editor();s.cursor.set(n.row);if (studio.tool)()==EditTool::Erase&&!readonly{s.apply(Action::DeleteNote{chart_id:(s.selected)(),note_id:n.id.clone()});}else if e.modifiers().shift(){if !s.selection.write().remove(&n.id){s.selection.write().insert(n.id.clone());}}else{s.selection.set([n.id.clone()].into());}}},
                                    if let Some(end)=n.end_row{rect{x:"{x(n.row as f64).max(0.)}",y:"{n.lane as usize*40+14}",width:"{(x(end as f64).min(width())-x(n.row as f64).max(0.)).max(1.)}",height:"12",fill:"var(--accent-color)",opacity:"0.5"}}
                                    rect{x:"{x(n.row as f64)-9.}",y:"{n.lane as usize*40+5}",width:"20",height:"30",rx:"3",fill:"var(--canvas-raised)",stroke:if s.selection.read().contains(&n.id){"var(--accent-color)"}else{"var(--border-color)"},stroke_width:"2"}
                                    text{x:"{x(n.row as f64)+1.}",y:"{n.lane as usize*40+26}",text_anchor:"middle",fill:if n.kind==NoteKind::Mine{"var(--danger-color)"}else{"var(--text-primary)"},pointer_events:"none","{note_symbol(n.kind,n.lane)}"}
                                    if let Some(end)=n.end_row{rect{x:"{x(end as f64)-4.}",y:"{n.lane as usize*40+8}",width:"8",height:"24",fill:"var(--accent-color)",cursor:"ew-resize",onmousedown:{let n=n.clone();move|e:MouseEvent|{e.stop_propagation();drag.set(Some(Drag::Note{note:n.clone(),x:e.page_coordinates().x,y:e.page_coordinates().y,resize:true,revision}));}}}}
                                }
                            }
                            if let Some(Drag::Box{row,lane,..})=drag(){rect{x:"{x(row as f64).min(x(row as f64)+delta().0)}",y:"{(lane as f64*40.).min(lane as f64*40.+delta().1)}",width:"{delta().0.abs()}",height:"{delta().1.abs()+40.}",class:"selection-box",pointer_events:"none"}}
                            line{x1:"{x(play_row)}",x2:"{x(play_row)}",y1:"0",y2:"160",stroke:"var(--danger-color)",pointer_events:"none"}
                            line{x1:"{x(cursor as f64)}",x2:"{x(cursor as f64)}",y1:"0",y2:"160",stroke:"var(--accent-color)",stroke_dasharray:"3 3",pointer_events:"none"}
                        }
                    }
                }
            }
        }
        label{class:"chart-desk-navigation",aria_label:"Timeline navigation",span{"Beat {start as f64/48.:.2}"}input{r#type:"range",min:"0",max:"{max_row}",step:"{snap}",value:"{start}",aria_label:"Timeline position",onkeydown:move|e|e.stop_propagation(),oninput:move|e|if let Ok(row)=e.value().parse(){s.view_row.set(row);}}span{"Beat {end as f64/48.:.2}"}}
    }
}

#[component]
pub fn Inspector(studio: Studio) -> Element {
    let mut s = use_context::<DanceStudio>();
    let Some(chart) = selected_chart(s) else {
        let profile = s
            .project
            .read()
            .as_ref()
            .map_or(Profile::Pad, |p| p.content.profile);
        return rsx! {aside{class:"selection-desk",aria_label:"Selection and review",header{class:"desk-heading",h2{"Selection"}}div{class:"selection-desk-content",p{"Add a difficulty to begin charting."}button{title:"Add a chart for {profile.name()}",onclick:move |_|{s.apply(Action::AddChart{difficulty:Difficulty::Edit,meter:1,description:"Manual chart".into()});if let Some(c)=s.project.read().as_ref().and_then(|p|p.content.document.charts.last()){s.selected.set(c.id.clone());}},"Add difficulty"}}}};
    };
    let id = chart.id.clone();
    let selected = chart
        .notes
        .iter()
        .find(|n| s.selection.read().contains(&n.id))
        .cloned();
    let count = s.selection.read().len();
    let kind = selected
        .as_ref()
        .filter(|_| count == 1)
        .map_or((s.note_kind)(), |n| n.kind);
    let profile = chart
        .profile
        .unwrap_or_else(|| s.project.read().as_ref().unwrap().content.profile);
    rsx! {aside{class:"selection-desk",aria_label:"Selection and review","data-chart-id":"{id}",onkeydown:move|e|e.stop_propagation(),
        header{class:"desk-heading",h2{"Selection"}}
        div{class:"selection-desk-content dance-inspector",
            if let Some(error)=(s.error)(){p{class:"field-hint",role:"alert","{error}"}button{onclick:move |_|s.error.set(None),"Dismiss"}}
            section{class:"selection-note",aria_label:"Selected note",
                if count>0{p{class:"selection-caption","{count} notes selected"}}else{p{class:"selection-empty","Click a note or drag a box. Shift-click adds notes."}}
                label{"Note type" select{aria_label:"Note type",value:"{kind.symbol()}",onchange:move |e|if let Some(kind)=NoteKind::ALL.into_iter().find(|k|k.symbol().to_string()==e.value()){s.note_kind.set(kind);if count==1&&let Some(mut note)=selected_chart(s).and_then(|c|c.notes.into_iter().find(|n|s.selection.read().contains(&n.id))){note.kind=kind;note.end_row=if kind.sustained(){Some(note.end_row.unwrap_or(note.row+48))}else{None};s.apply(Action::UpdateNote{chart_id:(s.selected)(),note});}},for k in NoteKind::ALL{option{value:"{k.symbol()}",selected:k==kind,"{k:?}"}}}}
                if count==1{if let Some(note)=selected{
                    NoteProperties{key:"{note.id}",s,note,profile}
                }}
                if count>0{div{class:"selection-actions",button{onclick:move |_|super::super::session::copy(studio,false),"Copy"}button{onclick:move |_|super::delete(),"Delete"}}}
                div{class:"selection-actions",button{disabled:count==0,onclick:move |_|s.edit_selection(SelectionEdit::Mirror),"Mirror"}button{disabled:count==0,onclick:move |_|s.edit_selection(SelectionEdit::Quantize{rows:(s.snap_rows)()}),"Quantize"}}
            }
            details{class:"context-fold",summary{"Difficulty"}for chart in [chart.clone()]{ChartProperties{key:"{chart.id}",s,chart}}
                button{title:"Add a chart for {profile.name()}",onclick:move |_|{s.apply(Action::AddChart{difficulty:Difficulty::Edit,meter:1,description:"Manual chart".into()});if let Some(c)=s.project.read().as_ref().and_then(|p|p.content.document.charts.last()){s.selected.set(c.id.clone());s.selection.set(Default::default());}},"Add difficulty"}
            }
            for epoch in [view_epoch()]{SongProperties{key:"{epoch}",s}}
            for key in [id.clone()]{TimingEditor{key:"{key}",s}}
            FootworkReview{s}
        }
    }}
}
#[component]
fn NoteProperties(s: DanceStudio, note: sm::Note, profile: Profile) -> Element {
    let position = note.clone();
    let length = note.clone();
    let lane = note.clone();
    rsx! {div{class:"selection-timing",
        label{"Beat" input{aria_label:"Note beat",r#type:"number",min:"0",step:"0.020833333",value:"{note.row as f64/48.}",oninput:move|e|{if let Ok(row)=e.value().parse::<f64>().map_err(anyhow::Error::new).and_then(sm::row_from_beat){let mut n=position.clone();n.end_row=n.end_row.map(|end|end+row-n.row);n.row=row;s.apply(Action::UpdateNote{chart_id:(s.selected)(),note:n});}}}}
        if note.kind.sustained(){label{"Length (beats)" input{aria_label:"{note.kind:?} length in beats",r#type:"number",min:"0.020833333",step:"0.020833333",value:"{(note.end_row.unwrap_or(note.row+48)-note.row)as f64/48.}",oninput:move |e|{if let Ok(rows)=e.value().parse::<f64>().map_err(anyhow::Error::new).and_then(sm::row_from_beat){let mut n=length.clone();n.end_row=Some(n.row+rows);s.apply(Action::UpdateNote{chart_id:(s.selected)(),note:n});}}}}}
    }
    label{"Lane" select{aria_label:"Note lane",value:"{note.lane}",onchange:move|e|if let Ok(l)=e.value().parse(){let mut n=lane.clone();n.lane=l;s.apply(Action::UpdateNote{chart_id:(s.selected)(),note:n});},for(i,arrow)in["←","↓","↑","→"].iter().enumerate(){option{value:"{i}",selected:i==note.lane as usize,"{arrow}"}}}}
    if profile==Profile::Pad{label{"Foot override" select{aria_label:"Foot override",value:s.project.read().as_ref().and_then(|p|p.content.foot_overrides.get(&note.id)).map_or("auto",|f|if *f==Foot::Left{"left"}else{"right"}),onchange:move |e|s.apply(Action::SetFoot{note_id:note.id.clone(),foot:match e.value().as_str(){"left"=>Some(Foot::Left),"right"=>Some(Foot::Right),_=>None}}),option{value:"auto","Automatic"}option{value:"left","Left"}option{value:"right","Right"}}}}
    }
}
#[component]
fn FootworkReview(mut s: DanceStudio) -> Element {
    let analysis = use_memo(move || {
        let p = s.project.read();
        let p = p.as_ref()?;
        let c = p
            .content
            .document
            .charts
            .iter()
            .find(|c| c.id == (s.selected)())?;
        if c.profile.unwrap_or(p.content.profile) != Profile::Pad {
            return None;
        }
        Some(sm::project::footwork(
            c,
            p.content.document.effective_timing(&c.id).ok()?,
            &p.content.foot_overrides,
        ))
    });
    rsx! {if let Some(a)=analysis.read().as_ref(){details{class:"context-fold",summary{"Footwork · {a.warnings.len()} warnings"}p{class:"field-hint","Foot assignments are estimates; review them while playing."}for w in a.warnings.iter().take(200){button{onclick:{let row=w.row;move |_|{s.cursor.set(row);s.view_row.set(row/192*192);}},"Beat {w.row as f64/48.:.3}: {w.message}"}}}}}
}
#[component]
pub fn Dialogs() -> Element {
    let mut s = use_context::<DanceStudio>();
    let mut seed = use_signal(|| "1".to_owned());
    let mut density = use_signal(|| "1".to_owned());
    let mut difficulties = use_signal(|| Difficulty::GENERATED.to_vec());
    let mut review = use_signal(String::new);
    let chart = selected_chart(s);
    let profile = chart
        .as_ref()
        .and_then(|c| c.profile)
        .or_else(|| s.project.read().as_ref().map(|p| p.content.profile))
        .unwrap_or(Profile::Pad);
    rsx! {
        if (s.generation_open)(){super::super::sheet::ToolSheet{id:"generate-chart",title:"Generate charts",description:"Add draft difficulties from this recording. Existing notes stay in place.",close_label:"Close generation",class:"generation-sheet",onclose:move |_|s.generation_open.set(false),actions:rsx!{button{onclick:move |_|s.generation_open.set(false),"Close"}},
            p{class:"field-hint","Review the first beat under Song and synchronization before generating."}
            for d in Difficulty::GENERATED{label{class:"form-check",input{r#type:"checkbox",checked:difficulties.read().contains(&d),onchange:move|e|{if e.checked(){difficulties.write().push(d);}else{difficulties.write().retain(|v|*v!=d);}}}"{d.name()}"}}
            label{"Seed" input{value:"{seed}",oninput:move|e|seed.set(e.value())}}
            label{"Density (0.25–2)" input{r#type:"number",min:"0.25",max:"2",step:"0.05",value:"{density}",oninput:move|e|density.set(e.value())}}
            button{disabled:(s.busy)(),onclick:move |_|{let request=(||Ok::<_,anyhow::Error>(GenerationRequest{profile,difficulties:difficulties(),seed:seed().parse()?,density:density().parse()?}))();match request{Ok(request)=>{dioxus::core::spawn_forever(async move{s.generate(request).await;});},Err(error)=>s.report(Err(error))}},"Generate drafts"}
            if (s.busy)(){button{onclick:move |_|s.cancel.set(true),"Cancel generation"}}
            p{role:"status","{s.status}"}if let Some(error)=(s.error)(){p{role:"alert","{error}"}}
        }}
        if (s.review_open)(){super::super::sheet::ToolSheet{id:"review-chart",title:"Review chart",description:"Check synchronization, patterns and estimated difficulty while playing.",close_label:"Close review",class:"review-sheet",onclose:move |_|s.review_open.set(false),actions:rsx!{button{onclick:move |_|s.review_open.set(false),"Close"}},
            FootworkReview{s}
            if let Some(chart)=chart{label{"Review comment" input{aria_label:"Review comment",value:"{review}",oninput:move|e|review.set(e.value())}}button{onclick:{let id=chart.id.clone();move |_|s.apply(Action::Review{chart_id:id.clone(),comment:review()})},"Mark chart reviewed"}p{if s.project.read().as_ref().is_some_and(|p|p.reviewed(&chart.id)){"Review matches current chart"}else{"Pending review"}}}
        }}
    }
}
