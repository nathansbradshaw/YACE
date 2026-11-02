use super::*;
#[test]
fn relative_names_keep_unicode_and_native_components_without_escaping_the_project() {
    assert_eq!(
        relative_name(Path::new("source/extras/\u{97f3}\u{697d}.bin")).unwrap(),
        "source/extras/\u{97f3}\u{697d}.bin"
    );
    for path in ["", "../source/extra.bin", "/source/extra.bin"] {
        assert!(relative_name(Path::new(path)).is_err());
    }
}
#[cfg(unix)]
#[test]
fn invalid_unicode_is_refused_without_needing_an_invalid_filesystem_name() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = std::path::PathBuf::from(OsString::from_vec(b"source/opaque-\xff.bin".to_vec()));
    assert!(
        relative_name(&path)
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
}
#[cfg(not(windows))]
#[test]
fn a_literal_backslash_is_not_an_archive_directory() {
    assert!(
        relative_name(Path::new("source/opaque\\cue.bin"))
            .unwrap_err()
            .to_string()
            .contains("literal backslash")
    );
}
#[cfg(windows)]
#[test]
fn native_windows_separators_become_zip_separators() {
    assert_eq!(
        relative_name(Path::new(r"source\extras\cue.bin")).unwrap(),
        "source/extras/cue.bin"
    );
}
