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
