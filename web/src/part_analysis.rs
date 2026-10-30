//! Preserve saved kit identity without loading native model workers.
pub use crate::kit_audition::KitAudition;
pub use crate::kit_audition::source_key as kit_source_key;
pub fn select_kit_audition(
    _store: &crate::editor::Store,
    _project: &crate::editor::Project,
    _part: &str,
) -> anyhow::Result<KitAudition> {
    anyhow::bail!("Native kit separation is unavailable in the browser")
}
