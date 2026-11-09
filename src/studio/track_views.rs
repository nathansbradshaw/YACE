//! Project-local view choices; these never dispatch chart editing actions.
use super::{dance, state::Studio, track_picker};
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Focused,
    Selected,
    All,
}
impl Mode {
    fn value(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Selected => "selected",
            Self::All => "all",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "selected" => Self::Selected,
            "all" => Self::All,
            _ => Self::Focused,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    pub mode: Mode,
    pub chosen: BTreeSet<String>,
}
impl View {
    pub fn resolve(&self, available: &[String], active: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut tracks: Vec<_> = available
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .filter(|id| match self.mode {
                Mode::Focused => id.as_str() == active,
                Mode::Selected => self.chosen.contains(id.as_str()),
                Mode::All => true,
            })
            .cloned()
            .collect();
        if tracks.is_empty() && available.iter().any(|id| id == active) {
            tracks.push(active.to_owned());
        }
        tracks
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    version: u32,
    preview: View,
    roll: View,
    collapsed: BTreeSet<String>,
    difficulty: String,
    layout: super::workspace::Layout,
    active_dance: Option<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            preview: View::default(),
            roll: View::default(),
            collapsed: Default::default(),
            difficulty: "Expert".into(),
            layout: Default::default(),
            active_dance: None,
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Preview,
    Roll,
}
impl Pane {
    fn name(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Roll => "Piano roll",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    root: PathBuf,
    dance: bool,
}
impl Key {
    fn name(&self) -> String {
        format!(
            "workspace-{}-view.json",
            if self.dance { "dance" } else { "band" }
        )
    }
    #[cfg(target_arch = "wasm32")]
    fn storage_key(&self) -> String {
        format!("YACE-view:{}:{}", self.root.display(), self.name())
    }
}
#[derive(Clone, Default)]
struct Loaded {
    key: Option<Key>,
    prefs: Preferences,
    warning: Option<String>,
}
#[derive(Clone, Copy)]
pub struct TrackViews {
    value: Signal<Loaded>,
}
fn key(studio: Studio) -> Option<Key> {
    if dance::active() {
        dance::overview_source().map(|(root, _)| Key { root, dance: true })
    } else {
        studio.store.read().as_ref().map(|s| Key {
            root: s.root.clone(),
            dance: false,
        })
    }
}
fn decode(bytes: &[u8]) -> Result<Preferences, String> {
    let mut p: Preferences = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if p.version != 1 {
        return Err("Unsupported workspace preferences version".into());
    }
    p.layout = p.layout.normalized();
    Ok(p)
}
fn load(key: &Key) -> Result<Preferences, String> {
    #[cfg(target_arch = "wasm32")]
    let bytes = {
        let storage = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("Browser view storage is unavailable")?;
        storage
            .get_item(&key.storage_key())
            .map_err(|_| "Cannot read browser view storage")?
            .map(String::into_bytes)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let bytes = match std::fs::read(key.root.join(key.name())) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    bytes.map_or_else(|| Ok(Preferences::default()), |bytes| decode(&bytes))
}
fn save(key: &Key, prefs: &Preferences) -> Result<(), String> {
    let bytes = serde_json::to_vec(prefs).map_err(|e| e.to_string())?;
    #[cfg(target_arch = "wasm32")]
    {
        let storage = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or("Browser view storage is unavailable")?;
        storage
            .set_item(
                &key.storage_key(),
                std::str::from_utf8(&bytes).map_err(|e| e.to_string())?,
            )
            .map_err(|_| "Cannot save browser view preferences".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new_in(&key.root).map_err(|e| e.to_string())?;
        tmp.write_all(&bytes)
            .and_then(|()| tmp.as_file().sync_all())
            .map_err(|e| e.to_string())?;
        tmp.persist(key.root.join(key.name()))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
pub fn provide(studio: Studio) {
    let mut views = use_context_provider(|| TrackViews {
        value: Signal::new(Loaded::default()),
    });
    use_effect(move || {
        let current = key(studio);
        if views.value.peek().key != current {
            let (prefs, warning) = current.as_ref().map_or_else(
                || (Preferences::default(), None),
                |key| match load(key) {
                    Ok(p) => (p, None),
                    Err(e) => (
                        Preferences::default(),
                        Some(format!("View preferences could not be restored: {e}")),
                    ),
                },
            );
            views.value.set(Loaded {
                key: current,
                prefs,
                warning,
            });
        }
    });
    use_effect(move || {
        if !dance::active()
            && let Some(doc) = studio.snapshot()
            && let Some(difficulty) = tier(&doc.selected_track)
            && views.value.peek().key == key(studio)
            && views.value.peek().prefs.difficulty != difficulty
        {
            update(studio, |p| p.difficulty = difficulty.into());
        }
    });
    let linked = use_memo(move || {
        // Standalone charts must not replace the previous band's choice.
        let root = dance::linked_root(studio)?;
        let selected = dance::selected_track()
            .and_then(|track| track.strip_prefix("dance:").map(str::to_owned));
        Some((Key { root, dance: false }, selected))
    });
    use_effect(move || {
        if let Some((key, selected)) = linked() {
            remember_linked_selection(key, selected);
        }
    });
}

fn remember_selection(key: &Key, selected: Option<String>) -> Result<bool, String> {
    let mut prefs = load(key)?;
    if prefs.active_dance == selected {
        return Ok(false);
    }
    prefs.active_dance = selected;
    save(key, &prefs)?;
    Ok(true)
}

fn remembered_dance(prefs: &Preferences, available: &[(String, String)]) -> Option<String> {
    let id = prefs.active_dance.as_ref()?;
    let selected = format!("dance:{id}");
    available
        .iter()
        .any(|(track, _)| track == &selected)
        .then(|| id.clone())
}

fn remember_linked_selection(key: Key, selected: Option<String>) {
    let result = remember_selection(&key, selected.clone());
    if let Some(mut views) = try_consume_context::<TrackViews>() {
        let loaded_here = views.value.peek().key.as_ref() == Some(&key);
        match result {
            Ok(true) if loaded_here => {
                views.value.write().prefs.active_dance = selected;
            }
            Err(error) => {
                views.value.write().warning =
                    Some(format!("Workspace selection is temporary: {error}"));
            }
            _ => {}
        }
    }
}

pub fn restore_linked_selection(studio: Studio) {
    let Some(root) = dance::linked_root(studio) else {
        return;
    };
    let key = Key { root, dance: false };
    let Some(id) = load(&key)
        .ok()
        .and_then(|prefs| remembered_dance(&prefs, &dance::tracks(studio)))
    else {
        return;
    };
    if let Some(state) = try_consume_context::<dance::DanceStudio>() {
        dance::select_track(state, id);
    }
}
fn current(studio: Studio) -> Preferences {
    let Some(views) = try_consume_context::<TrackViews>() else {
        return Preferences::default();
    };
    let loaded = views.value.read();
    if loaded.key == key(studio) {
        loaded.prefs.clone()
    } else {
        key(studio)
            .and_then(|key| load(&key).ok())
            .unwrap_or_default()
    }
}
fn update(studio: Studio, edit: impl FnOnce(&mut Preferences)) {
    update_inner(studio, true, edit);
}
fn update_inner(studio: Studio, persist: bool, edit: impl FnOnce(&mut Preferences)) {
    let Some(key) = key(studio) else {
        return;
    };
    let Some(mut views) = try_consume_context::<TrackViews>() else {
        return;
    };
    let mut prefs = current(studio);
    if let Some(doc) = studio.snapshot()
        && let Some(tier) = tier(&doc.selected_track)
    {
        prefs.difficulty = tier.into();
    }
    edit(&mut prefs);
    let warning = if persist {
        save(&key, &prefs)
            .err()
            .map(|e| format!("View changes are temporary: {e}"))
    } else {
        views.value.peek().warning.clone()
    };
    views.value.set(Loaded {
        key: Some(key),
        prefs,
        warning,
    });
}
pub fn layout(studio: Studio) -> super::workspace::Layout {
    current(studio).layout.normalized()
}
pub fn set_layout(studio: Studio, layout: super::workspace::Layout, persist: bool) {
    update_inner(studio, persist, |p| p.layout = layout.normalized());
}
fn tier(track: &str) -> Option<&str> {
    ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find(|tier| track.starts_with(tier))
}
pub fn available(studio: Studio) -> Vec<String> {
    // A Dance session is one editable chart at a time. Its difficulty selector
    // remains authoritative; modes never overlay different difficulties.
    if dance::active() {
        return dance::selected_track().into_iter().collect();
    }
    let Some(doc) = studio.snapshot() else {
        return Vec::new();
    };
    let prefs = current(studio);
    let difficulty = tier(&doc.selected_track).unwrap_or(&prefs.difficulty);
    let mut tracks: Vec<_> = track_picker::tracks(&doc)
        .into_iter()
        .filter(|id| tier(id) == Some(difficulty))
        .collect();
    if doc.content.vocals.is_some()
        || doc.content.harmonies.is_some()
        || doc.selected_track == "Vocals"
    {
        tracks.push("Vocals".into());
    }
    tracks
}
pub fn active(studio: Studio) -> String {
    dance::selected_track()
        .or_else(|| studio.snapshot().map(|d| d.selected_track.clone()))
        .unwrap_or_default()
}
pub fn visible(studio: Studio, pane: Pane) -> Vec<String> {
    let prefs = current(studio);
    let view = match pane {
        Pane::Preview => prefs.preview,
        Pane::Roll => prefs.roll,
    };
    view.resolve(&available(studio), &active(studio))
}
pub fn collapsed(studio: Studio, track: &str) -> bool {
    current(studio).collapsed.contains(track)
}
pub fn select(studio: Studio, track: String) {
    if let Some(id) = track.strip_prefix("dance:") {
        if let Some(s) = try_consume_context::<dance::DanceStudio>() {
            dance::select_track(s, id.into());
        }
    } else {
        dance::deactivate();
        studio.dispatch(crate::editor::Action::SelectTrack { track });
    }
}
pub fn label(studio: Studio, track: &str) -> String {
    if track.starts_with("dance:") {
        dance::tracks(studio)
            .into_iter()
            .find(|(id, _)| id == track)
            .map_or_else(|| "Dance".into(), |(_, label)| label)
    } else {
        track_picker::title(track)
    }
}
#[component]
pub fn Controls(studio: Studio, pane: Pane) -> Element {
    let prefs = current(studio);
    let view = match pane {
        Pane::Preview => prefs.preview,
        Pane::Roll => prefs.roll,
    };
    let name = pane.name();
    let tracks = available(studio);
    let active = active(studio);
    let chosen = view.resolve(&tracks, &active);
    let warning =
        try_consume_context::<TrackViews>().and_then(|views| views.value.read().warning.clone());
    rsx! {div{class:"track-view-controls",onkeydown:move|event:KeyboardEvent|event.stop_propagation(),
        label{span{"{name}"}select{aria_label:"{name} track visibility",value:view.mode.value(),onkeydown:move|e|e.stop_propagation(),onchange:move|e|update(studio,|p|{
            let view=match pane{Pane::Preview=>&mut p.preview,Pane::Roll=>&mut p.roll};
            let next=Mode::parse(&e.value());
            if next==Mode::Selected&&view.chosen.is_empty(){view.chosen.insert(self::active(studio));}
            view.mode=next;
        }),option{value:"focused","Focused"}option{value:"selected","Selected tracks"}option{value:"all","All tracks"}}}
        if view.mode==Mode::Selected {details{class:"track-view-choices",summary{"Choose tracks"}fieldset{legend{"{name} tracks at the active difficulty"}
            p{"Keep at least one track visible."}
            for track in tracks {{let caption=label(studio,&track);let id=track.clone();rsx!{label{key:"{track}",input{r#type:"checkbox",tabindex:"0",checked:chosen.contains(&track),disabled:chosen.len()==1&&chosen.contains(&track),aria_label:"{name}: {caption}",onchange:move|e|update(studio,|p|{
                let view=match pane{Pane::Preview=>&mut p.preview,Pane::Roll=>&mut p.roll};
                if e.checked(){view.chosen.insert(id.clone());}else{view.chosen.remove(&id);}
            })}span{"{caption}"}}}}
        }}}}
        if let Some(warning)=warning {span{role:"status",class:"track-view-warning","{warning}"}}
        if pane==Pane::Roll {super::workspace::LayoutControls{studio}}
    }}
}
#[component]
pub fn RollHeading(studio: Studio, track: String) -> Element {
    let hidden = collapsed(studio, &track);
    let target = track.clone();
    let toggle = track.clone();
    let title = label(studio, &track);
    let count = studio.snapshot().and_then(|doc| {
        if track == "Vocals" {
            doc.content.vocals.as_ref().map(|v| v.notes.len())
        } else {
            doc.content.tracks.get(&track).map(|notes| {
                notes
                    .iter()
                    .filter(|n| doc.playable(&track, n.note.lane))
                    .count()
            })
        }
    });
    rsx! {header{class:"track-view-heading",onkeydown:move|event:KeyboardEvent|event.stop_propagation(),button{tabindex:"0",aria_label:"Select {title} for editing",aria_pressed:(active(studio)==track).to_string(),onclick:move|_|select(studio,target.clone()),"{title}"}
        if !track.starts_with("dance:")&&track!="Vocals"{super::difficulty::TrackLinkBadge{track:track.clone()}}
        if let Some(count)=count {span{class:"track-view-count","{count} notes"}}
        button{tabindex:"0",aria_label:if hidden{format!("Expand {title} piano roll")}else{format!("Collapse {title} piano roll")},aria_expanded:(!hidden).to_string(),onclick:move|_|update(studio,|p|{if !p.collapsed.remove(&toggle){p.collapsed.insert(toggle.clone());}}),if hidden{"Expand"}else{"Collapse"}}
    }}
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tracks() -> Vec<String> {
        ["ExpertSingle", "ExpertDoubleBass", "ExpertDrums", "Vocals"]
            .into_iter()
            .map(Into::into)
            .collect()
    }
    #[test]
    fn modes_are_stable_and_selected_empty_or_removed_tracks_fall_back() {
        let tracks = tracks();
        let mut view = View::default();
        assert_eq!(view.resolve(&tracks, "ExpertDrums"), ["ExpertDrums"]);
        view.mode = Mode::All;
        assert_eq!(view.resolve(&tracks, "ExpertDrums"), tracks);
        view.mode = Mode::Selected;
        view.chosen.extend([
            "ExpertDrums".into(),
            "ExpertSingle".into(),
            "missing".into(),
        ]);
        assert_eq!(
            view.resolve(&tracks, "ExpertDoubleBass"),
            ["ExpertSingle", "ExpertDrums"]
        );
        view.chosen.clear();
        assert_eq!(
            view.resolve(&tracks, "ExpertDoubleBass"),
            ["ExpertDoubleBass"]
        );
        assert!(view.resolve(&[], "ExpertSingle").is_empty());
    }
    #[test]
    fn preferences_keep_modes_and_collapses_independent_and_validate_versions() {
        let mut p = Preferences::default();
        p.preview.mode = Mode::All;
        p.roll.chosen.insert("ExpertDrums".into());
        p.collapsed.insert("ExpertSingle".into());
        assert_eq!(decode(&serde_json::to_vec(&p).unwrap()).unwrap(), p);
        assert_eq!(p.roll.mode, Mode::Focused);
        assert!(decode(br#"{"version":2}"#).is_err());
        assert!(decode(b"bad json").is_err());
        assert_eq!(decode(b"{}").unwrap(), Preferences::default());
    }
    #[test]
    fn layout_preferences_upgrade_existing_views_and_preserve_other_choices() {
        let old = br#"{"version":1,"preview":{"mode":"all"},"collapsed":["ExpertSingle"],"difficulty":"Hard"}"#;
        let mut prefs = decode(old).unwrap();
        assert_eq!(prefs.layout, Default::default());
        assert_eq!(prefs.active_dance, None);
        prefs.layout = serde_json::from_str(
            r#"{"right":32.5,"top":65,"preview_hidden":true,"inspector_hidden":true}"#,
        )
        .unwrap();
        let reopened = decode(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(reopened, prefs);
        assert_eq!(reopened.preview.mode, Mode::All);
        assert!(reopened.collapsed.contains("ExpertSingle"));
        assert_eq!(reopened.difficulty, "Hard");
        let clamped = decode(br#"{"layout":{"right":-10,"top":900}}"#).unwrap();
        let value = serde_json::to_value(clamped).unwrap();
        assert_eq!(value["layout"]["right"], 23.);
        assert_eq!(value["layout"]["top"], 78.);
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn saving_preferences_is_atomic_and_does_not_touch_chart_history() {
        let root = tempfile::tempdir().unwrap();
        let chart = root.path().join("project.json");
        std::fs::write(&chart, b"retained chart and undo history").unwrap();
        let key = Key {
            root: root.path().into(),
            dance: false,
        };
        assert_eq!(load(&key).unwrap(), Preferences::default());
        let mut p = Preferences::default();
        p.roll.mode = Mode::All;
        save(&key, &p).unwrap();
        assert_eq!(load(&key).unwrap(), p);
        assert!(remember_selection(&key, Some("keyboard-chart".into())).unwrap());
        p.active_dance = Some("keyboard-chart".into());
        assert_eq!(load(&key).unwrap(), p);
        assert!(!remember_selection(&key, p.active_dance.clone()).unwrap());
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&Key {
                root: other.path().into(),
                dance: false
            })
            .unwrap()
            .active_dance,
            None
        );
        assert!(remember_selection(&key, None).unwrap());
        p.active_dance = None;
        assert_eq!(load(&key).unwrap(), p);
        assert_eq!(
            std::fs::read(chart).unwrap(),
            b"retained chart and undo history"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn remembered_chart_must_still_exist_in_the_linked_project() {
        let mut prefs = Preferences::default();
        let available = vec![
            ("dance:pad".into(), "Dance".into()),
            ("dance:keys".into(), "4-key".into()),
        ];
        assert_eq!(remembered_dance(&prefs, &available), None);
        prefs.active_dance = Some("keys".into());
        assert_eq!(
            remembered_dance(&prefs, &available).as_deref(),
            Some("keys")
        );
        assert_eq!(remembered_dance(&prefs, &[]), None);
        prefs.active_dance = Some("removed-chart".into());
        assert_eq!(remembered_dance(&prefs, &available), None);
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn remembering_selection_does_not_overwrite_damaged_preferences() {
        let root = tempfile::tempdir().unwrap();
        let key = Key {
            root: root.path().into(),
            dance: false,
        };
        let path = root.path().join(key.name());
        std::fs::write(&path, b"{retained invalid preferences").unwrap();
        assert!(remember_selection(&key, Some("keys".into())).is_err());
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"{retained invalid preferences"
        );
    }
}
