//! Fetch exactly the pinned optimizer; the API fallback changes only delivery.
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path, process::Command};
pub const SHA256: &str = "195ddc94f9bc89f45abdabb0b9eea86023d727ba90eac8b35b80f2544fc30572";
const URL: &str = "https://github.com/WebAssembly/binaryen/releases/download/version_132/binaryen-version_132-x86_64-linux.tar.gz";
const ASSET: &str = "repos/WebAssembly/binaryen/releases/assets/512132895";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn hash(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn verify(path: &Path, expected: &str) -> Result<()> {
    if hash(path)? != expected {
        return Err("Pinned optimizer checksum mismatch".into());
    }
    Ok(())
}
pub fn fetch(output: &Path) -> Result<()> {
    if output.exists() {
        verify(output, SHA256)?;
        println!("Reusing verified Binaryen 132 archive");
        return Ok(());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let partial = parent.join(format!(".binaryen-132-{}.part", std::process::id()));
    let owned = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    drop(owned);
    let result = (|| -> Result<()> {
        let direct = Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--retry",
                "3",
                "--max-time",
                "120",
                URL,
                "--output",
            ])
            .arg(&partial)
            .status();
        if !direct.is_ok_and(|status| status.success()) || verify(&partial, SHA256).is_err() {
            eprintln!(
                "Direct optimizer download unavailable; trying the same official release asset through GitHub's API"
            );
            let file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&partial)?;
            let status = Command::new("gh")
                .args(["api", "-H", "Accept: application/octet-stream", ASSET])
                .stdout(file)
                .status()?;
            if !status.success() {
                return Err("Official optimizer asset API failed".into());
            }
        }
        verify(&partial, SHA256)?;
        // Never overwrite an artifact that appeared while the download ran.
        fs::hard_link(&partial, output)?;
        Ok(())
    })();
    fs::remove_file(&partial)?;
    result?;
    println!("Verified Binaryen 132 archive SHA-256 {SHA256}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integrity_failure_preserves_existing_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.tar.gz");
        fs::write(&path, b"incorrect response body").unwrap();
        assert!(fetch(&path).unwrap_err().to_string().contains("checksum"));
        assert_eq!(fs::read(&path).unwrap(), b"incorrect response body");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn checksum_verification_reads_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample");
        fs::write(&path, b"abc").unwrap();
        verify(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        assert!(verify(&path, SHA256).is_err());
    }
}
