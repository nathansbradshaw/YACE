use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn one_cached_executable_resolves_the_selected_worktree_without_starting_a_server() {
    let base = Fixture(std::env::temp_dir().join(format!(
            "yace-worktree-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    for name in ["first", "second"] {
        let root = base.0.join(name);
        fs::create_dir_all(root.join("web/tools")).unwrap();
        fs::create_dir_all(root.join("web/dist")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("web/tools/Cargo.toml"), "").unwrap();
        fs::write(root.join("web/index.html"), "").unwrap();
        fs::write(root.join("src/editor.rs"), "").unwrap();
        fs::write(root.join("web/dist/index.html"), name).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_yace-static-web"))
            .current_dir(&root)
            .env_remove("YACE_SOURCE_ROOT")
            .args(["serve", "3030", "--check"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert!(
            output.contains(
                &root
                    .join("web/dist")
                    .canonicalize()
                    .unwrap()
                    .display()
                    .to_string()
            )
        );
        assert!(output.contains("No server started"));
        for port in ["0", "3031", "3033", "3037", "3049", "46871"] {
            let rejected = Command::new(env!("CARGO_BIN_EXE_yace-static-web"))
                .current_dir(&root)
                .env_remove("YACE_SOURCE_ROOT")
                .args(["serve", port, "--check"])
                .output()
                .unwrap();
            assert!(!rejected.status.success());
            assert!(String::from_utf8_lossy(&rejected.stderr).contains("not allowed"));
        }
    }
}
