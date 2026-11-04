//! Browser storage for the current tab and the default for a fresh tab.
pub use crate::workspace_target::{Family, Target};
use anyhow::{Context, Result};

const TAB_KEY: &str = "YACE-tab-workspace-v1";
const DEFAULT_KEY: &str = "YACE-last-workspace-v1";
const FOLDER: &str = "folder";

fn storage_error(error: wasm_bindgen::JsValue) -> anyhow::Error {
    anyhow::anyhow!("Workspace recovery preferences: {error:?}")
}
fn storage(session: bool) -> Result<web_sys::Storage> {
    let window = web_sys::window().context("No browser window")?;
    if session {
        window.session_storage()
    } else {
        window.local_storage()
    }
    .map_err(storage_error)?
    .context("Browser workspace preferences are unavailable")
}
pub fn current() -> Result<Option<Target>> {
    let tab = storage(true)?.get_item(TAB_KEY).map_err(storage_error)?;
    crate::workspace_target::select(tab.as_deref(), || {
        storage(false)?.get_item(DEFAULT_KEY).map_err(storage_error)
    })
}

pub fn folder_tab() -> Result<bool> {
    Ok(storage(true)?
        .get_item(TAB_KEY)
        .map_err(storage_error)?
        .as_deref()
        == Some(FOLDER))
}

pub fn remember_folder() -> Result<()> {
    storage(true)?
        .set_item(TAB_KEY, FOLDER)
        .map_err(storage_error)
}

/// Call only for the current durable workspace, never an asynchronous save's
/// captured root. The tab identity does not modify shared chart revisions.
pub fn remember(target: &Target) -> Result<()> {
    let encoded = serde_json::to_string(target)?;
    Target::decode(&encoded)?;
    storage(true)?
        .set_item(TAB_KEY, &encoded)
        .map_err(storage_error)?;
    storage(false)?
        .set_item(DEFAULT_KEY, &encoded)
        .map_err(storage_error)?;
    Ok(())
}
