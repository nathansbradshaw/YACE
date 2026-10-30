//! Check the archive behaviors covered by the ZIP dependency advisories.
use std::io::{Cursor, Write};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

fn file_archive(name: &str) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(name, SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"fixture").unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn ordinary_files_keep_their_contents() {
    let root = tempfile::tempdir().unwrap();
    let mut archive = ZipArchive::new(Cursor::new(file_archive("nested/file.txt"))).unwrap();
    archive.extract(root.path()).unwrap();
    assert_eq!(
        std::fs::read(root.path().join("nested/file.txt")).unwrap(),
        b"fixture"
    );
}

#[cfg(unix)]
#[test]
fn extraction_strips_privileged_permission_bits() {
    use std::os::unix::fs::PermissionsExt;
    let mut bytes = file_archive("file.txt");
    // Put hostile Unix attributes in the central header. The writer's public
    // permission setter strips these bits before they reach the archive.
    let central = bytes
        .windows(4)
        .position(|value| value == b"PK\x01\x02")
        .unwrap();
    bytes[central + 5] = 3;
    bytes[central + 38..central + 42].copy_from_slice(&(0o107755_u32 << 16).to_le_bytes());
    let root = tempfile::tempdir().unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(
        archive.by_index(0).unwrap().unix_mode().unwrap() & 0o7000,
        0
    );
    archive.extract(root.path()).unwrap();
    let mode = std::fs::metadata(root.path().join("file.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7000, 0);
}

#[cfg(unix)]
#[test]
fn outside_symlink_is_rejected_without_touching_its_target() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"keep me").unwrap();
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
    let output = root.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "link",
            "../victim",
            SimpleFileOptions::default().unix_permissions(0o777),
        )
        .unwrap();
    let mut archive = ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
    assert!(archive.extract(&output).is_err());
    assert!(output.join("link").symlink_metadata().is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
    assert_eq!(
        std::fs::metadata(&victim).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn symlink_permissions_do_not_change_the_target() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"keep me").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "link",
            "target",
            SimpleFileOptions::default().unix_permissions(0o777),
        )
        .unwrap();
    let mut archive = ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
    archive.extract(root.path()).unwrap();
    assert_eq!(
        std::fs::read_link(root.path().join("link")).unwrap(),
        std::path::Path::new("target")
    );
    assert_eq!(
        std::fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
