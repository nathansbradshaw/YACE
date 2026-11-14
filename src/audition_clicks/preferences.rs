//! Device/profile audition preferences never enter authored project history.
use super::Settings;
#[cfg(any(target_arch = "wasm32", not(feature = "browser")))]
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    settings: Settings,
}
#[cfg(any(target_arch = "wasm32", not(feature = "browser"), test))]
fn decode(bytes: &[u8]) -> Result<Settings> {
    let stored: Stored = serde_json::from_slice(bytes)?;
    ensure!(
        stored.version == 1,
        "Unsupported audition preferences version; existing settings retained"
    );
    stored.settings.validate()?;
    Ok(stored.settings)
}
fn encode(settings: &Settings) -> Result<Vec<u8>> {
    settings.validate()?;
    Ok(serde_json::to_vec_pretty(&Stored {
        version: 1,
        settings: settings.clone(),
    })?)
}
pub fn load() -> Result<Settings> {
    #[cfg(target_arch = "wasm32")]
    {
        let storage = storage()?;
        storage
            .get_item("YACE-audition-settings")
            .map_err(|e| anyhow::anyhow!("Cannot read audition settings: {e:?}"))?
            .map_or_else(|| Ok(Settings::default()), |s| decode(s.as_bytes()))
    }
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    {
        if cfg!(test) {
            return Ok(Settings::default());
        }
        load_from(&path()?)
    }
    #[cfg(all(not(target_arch = "wasm32"), feature = "browser"))]
    Ok(Settings::default())
}
pub fn save(settings: &Settings) -> Result<()> {
    let bytes = encode(settings)?;
    #[cfg(target_arch = "wasm32")]
    {
        let storage = storage()?;
        if let Some(previous) = storage
            .get_item("YACE-audition-settings")
            .map_err(|e| anyhow::anyhow!("Cannot read audition settings: {e:?}"))?
        {
            decode(previous.as_bytes())?;
        }
        storage
            .set_item("YACE-audition-settings", std::str::from_utf8(&bytes)?)
            .map_err(|e| anyhow::anyhow!("Cannot save audition settings: {e:?}"))
    }
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    {
        if cfg!(test) {
            return Ok(());
        }
        save_to(&path()?, &bytes)
    }
    #[cfg(all(not(target_arch = "wasm32"), feature = "browser"))]
    {
        let _ = bytes;
        Ok(())
    }
}
#[cfg(target_arch = "wasm32")]
fn storage() -> Result<web_sys::Storage> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .context("Browser audition settings storage is unavailable")
}
#[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
fn path() -> Result<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Some(path) = std::env::var_os("YACE_AUDITION_SETTINGS") {
        return Ok(path.into());
    }
    #[cfg(target_os = "macos")]
    let root = PathBuf::from(std::env::var_os("HOME").context("Home directory unavailable")?)
        .join("Library/Application Support");
    #[cfg(target_os = "windows")]
    let root =
        PathBuf::from(std::env::var_os("APPDATA").context("App data directory unavailable")?);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("App state directory unavailable")?;
    Ok(root.join("YACE/audition-settings.json"))
}
#[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
fn load_from(path: &std::path::Path) -> Result<Settings> {
    match std::fs::read(path) {
        Ok(bytes) => decode(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
}
#[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
fn save_to(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    // Refuse a damaged/future document before replacing anything. Its bytes are
    // recoverable even if the current client can only offer session preferences.
    load_from(path)?;
    decode(bytes)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_and_old_missing_fields_keep_safe_defaults() {
        let settings = Settings {
            claps: true,
            beat_gain: 0.75,
            classes: vec![super::super::NoteClass::Kick],
            ..Settings::default()
        };
        assert_eq!(decode(&encode(&settings).unwrap()).unwrap(), settings);
        assert_eq!(
            decode(br#"{"version":1,"settings":{}}"#).unwrap(),
            Settings::default()
        );
        assert!(decode(br#"{"version":2,"settings":{}}"#).is_err());
        assert!(decode(br#"{"version":1,"settings":{"clap_gain":2}}"#).is_err());
    }
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    #[test]
    fn atomic_persistence_retains_corrupt_and_future_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audition.json");
        assert_eq!(load_from(&path).unwrap(), Settings::default());
        let s = Settings {
            metronome: true,
            ..Settings::default()
        };
        save_to(&path, &encode(&s).unwrap()).unwrap();
        assert_eq!(load_from(&path).unwrap(), s);
        for bytes in [
            &b"{recoverable partial"[..],
            &br#"{"version":99,"settings":{}}"#[..],
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(save_to(&path, &encode(&s).unwrap()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
