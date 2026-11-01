use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn one_cached_executable_serves_the_selected_worktree_not_its_compile_directory() {
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut server = Server(
            Command::new(env!("CARGO_BIN_EXE_yace-static-web"))
                .current_dir(&root)
                .env_remove("YACE_SOURCE_ROOT")
                .args(["serve", &port.to_string()])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let mut ready = String::new();
        let mut output = BufReader::new(server.0.stdout.take().unwrap());
        output.read_line(&mut ready).unwrap();
        assert!(
            ready.contains(&format!(":{port}/")),
            "server did not start: {ready}"
        );
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.ends_with(name),
            "cached tool served a different worktree for {name}"
        );
    }
}
