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

#[test]
fn zip_import_refuses_backslash_names_instead_of_renaming_them() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "extras/opaque\\cue.bin",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"opaque payload").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(
        decode_shared(&bytes)
            .unwrap_err()
            .to_string()
            .contains("backslash")
    );
    assert_eq!(
        zip::ZipArchive::new(Cursor::new(&bytes))
            .unwrap()
            .by_index(0)
            .unwrap()
            .name()
            .unwrap(),
        "extras/opaque\\cue.bin"
    );
}
