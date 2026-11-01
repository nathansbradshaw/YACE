//! Read embedded tags and pictures with Rust codecs; never inspect video frames.
use crate::recording_metadata::Metadata;
use anyhow::Result;
use std::io::Cursor;
use symphonia::core::{
    io::MediaSourceStream,
    meta::{MetadataRevision, StandardTagKey as Key, StandardVisualKey},
    probe::Hint,
};
#[derive(Default)]
pub struct Tags {
    pub metadata: Metadata,
    pub artwork: Option<Vec<u8>>,
}
impl Tags {
    fn apply(&mut self, revision: &MetadataRevision) {
        for tag in revision.tags() {
            let field = match tag.std_key {
                Some(Key::TrackTitle) => &mut self.metadata.name,
                Some(Key::Artist) => &mut self.metadata.artist,
                Some(Key::Album) => &mut self.metadata.album,
                Some(Key::Date) => &mut self.metadata.year,
                Some(Key::Genre) => &mut self.metadata.genre,
                _ => continue,
            };
            if field.is_empty() {
                *field = tag.value.to_string();
            }
        }
        if self.artwork.is_none() {
            let visuals = revision.visuals();
            if let Some(cover) = visuals
                .iter()
                .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
                .or_else(|| visuals.first())
                .filter(|v| v.data.len() <= 5_000_000)
            {
                self.artwork = Some(cover.data.to_vec());
            }
        }
    }
}
pub fn read(bytes: &[u8]) -> Result<Tags> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut probe = symphonia::default::get_probe().format(
        &Hint::new(),
        source,
        &Default::default(),
        &Default::default(),
    )?;
    let mut tags = Tags::default();
    if let Some(metadata) = probe.metadata.get()
        && let Some(revision) = metadata.current()
    {
        tags.apply(revision);
    }
    if let Some(revision) = probe.format.metadata().current() {
        tags.apply(revision);
    }
    tags.metadata.year = tags.metadata.year.chars().take(4).collect();
    Ok(tags)
}
