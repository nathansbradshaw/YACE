//! Saved editing state and successful song exports are separate histories.
use crate::{
    editor::{Project, Store},
    export_options::Options,
    storage as fs,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const FILE: &str = ".yace/last-song-export.json";
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "export_state/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    PublishedFolder,
    PreparedZip,
    DownloadRequested,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    version: u32,
    pub source_chart_sha256: String,
    pub fingerprint: String,
    pub revision: u64,
    pub options: Options,
    pub all_parts: bool,
    pub destination: String,
    pub delivery: Delivery,
}
/// Ignore transport/selection revisions and filesystem relocation. Keep typed
/// editing content, recording identity, review evidence and source metadata.
pub fn fingerprint(store: &Store, doc: &Project) -> Result<String> {
    let mut content = doc.content.clone();
    for manifest in content
        .stems
        .iter_mut()
        .chain(content.keyboard_stems.iter_mut())
    {
        for asset in &mut manifest.stems {
            asset.path = PathBuf::from(&asset.sha256);
        }
    }
    let ini = crate::song_metadata::song_ini(&store.root.join("source"))?;
    let bytes = serde_json::to_vec(&(
        &doc.source_chart_sha256,
        &doc.title,
        &doc.audio_sha256,
        &doc.audio_filename,
        doc.duration,
        &doc.review,
        &content,
        ini,
    ))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
impl Receipt {
    pub fn capture(
        store: &Store,
        doc: &Project,
        options: &Options,
        destination: String,
        delivery: Delivery,
    ) -> Result<Self> {
        let selected = options.selected(store, doc)?;
        let mut options = options.clone();
        options.include_audio &= doc.has_recording();
        Ok(Self {
            version: 1,
            source_chart_sha256: doc.source_chart_sha256.clone(),
            fingerprint: fingerprint(store, doc)?,
            revision: doc.revision,
            all_parts: selected == crate::export_options::available(store, doc)?,
            options,
            destination,
            delivery,
        })
    }
    fn validate(&self, doc: &Project) -> Result<()> {
        ensure!(self.version == 1, "Unsupported song-export history version");
        ensure!(
            self.source_chart_sha256 == doc.source_chart_sha256,
            "Song-export history belongs to another source"
        );
        ensure!(
            self.fingerprint.len() == 64
                && self
                    .fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "Invalid song-export history fingerprint"
        );
        Ok(())
    }
    pub fn load(store: &Store, doc: &Project) -> Result<Option<Self>> {
        let path = store.root.join(FILE);
        if !fs::exists(&path) {
            return Ok(None);
        }
        let receipt: Self = serde_json::from_slice(&fs::read(&path)?)?;
        receipt.validate(doc)?;
        Ok(Some(receipt))
    }
    pub fn save(&self, store: &Store) -> Result<()> {
        // This sidecar must never recreate a closed/cleared project or replace
        // an incompatible history file. It does not mutate project revisions.
        let current = store.load()?;
        self.validate(&current)?;
        Self::load(store, &current)?;
        let path = store.root.join(FILE);
        fs::create_dir_all(
            path.parent()
                .context("Export history needs a project directory")?,
        )?;
        let bytes = serde_json::to_vec_pretty(self)?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::io::Write;
            let mut stage = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            stage.write_all(&bytes)?;
            stage.as_file().sync_all()?;
            stage.persist(&path).map_err(|e| e.error)?;
        }
        #[cfg(target_arch = "wasm32")]
        fs::write(&path, &bytes)?;
        Ok(())
    }
    pub fn request_download(&mut self) -> Result<()> {
        ensure!(
            matches!(
                self.delivery,
                Delivery::PreparedZip | Delivery::DownloadRequested
            ),
            "This export is not a browser song ZIP"
        );
        self.delivery = Delivery::DownloadRequested;
        Ok(())
    }
    pub fn summary(&self, current: &str) -> String {
        let prefix = match self.delivery {
            Delivery::PublishedFolder => "Song exported",
            Delivery::PreparedZip => "Song ZIP prepared (not downloaded)",
            Delivery::DownloadRequested => "Song ZIP download requested",
        };
        let mut text = format!("{prefix} · r{}", self.revision);
        if self.fingerprint == current {
            text.push_str(" · editing content matches");
        } else {
            text.push_str(" · project changed");
        }
        if !self.all_parts {
            text.push_str(" · selected parts");
        }
        if !self.options.include_audio {
            text.push_str(" · without audio");
        }
        text
    }
}
#[derive(Clone, Debug, Default)]
pub struct State {
    pub fingerprint: String,
    pub receipt: Option<Receipt>,
    pub error: Option<String>,
}
impl State {
    pub fn load(store: &Store, doc: &Project) -> Self {
        match fingerprint(store, doc)
            .and_then(|fingerprint| Ok((fingerprint, Receipt::load(store, doc)?)))
        {
            Ok((fingerprint, receipt)) => Self {
                fingerprint,
                receipt,
                error: None,
            },
            Err(error) => Self {
                error: Some(error.to_string()),
                ..Default::default()
            },
        }
    }
    pub fn summary(&self) -> String {
        if self.error.is_some() {
            return "Song-export history unavailable".into();
        }
        self.receipt.as_ref().map_or_else(
            || "No song export recorded".into(),
            |r| r.summary(&self.fingerprint),
        )
    }
}
