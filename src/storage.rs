//! Native counterpart of the browser's project working set.
pub use std::fs::*;
pub fn exists(path: impl AsRef<std::path::Path>) -> bool {
    path.as_ref().exists()
}
pub fn is_file(path: impl AsRef<std::path::Path>) -> bool {
    path.as_ref().is_file()
}
pub fn is_absolute(path: impl AsRef<std::path::Path>) -> bool {
    path.as_ref().is_absolute()
}
pub fn write_new(path: impl AsRef<std::path::Path>, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
pub fn unique_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
