#![cfg(feature = "browser")]
//! Exercise the actual loopback HTTP boundary, including streamed uploads.
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Host(Child);
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn browser_host_requires_session_same_origin_and_bounded_upload_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_yace-browser"))
        .args(["serve", "--workspace"])
        .arg(workspace.path())
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let _host = Host(child);
    let url = line
        .trim()
        .strip_prefix("YACE browser editor: ")
        .expect("host launch URL");
    let (base, token) = url.split_once("/?token=").unwrap();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assert_eq!(client.get(base).send().unwrap().status(), 403);
    let response = client.get(url).send().unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let cookie = format!("yace_session={token}");
    assert_eq!(
        client
            .get(base)
            .header("Cookie", &cookie)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(base)
            .header("Cookie", &cookie)
            .header("Origin", "https://untrusted.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(base)
            .header("Cookie", &cookie)
            .header("Host", "untrusted.invalid")
            .send()
            .unwrap()
            .status(),
        403
    );
    for (name, expected) in [("../escape.chart", 400), ("song/notes.chart", 200)] {
        let body = format!(
            "--yace-test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n[Song]\n{{\n Resolution = 192\n}}\r\n--yace-test--\r\n"
        );
        let response = client
            .post(format!("{base}/upload"))
            .header("Cookie", &cookie)
            .header("Origin", base)
            .header("Content-Type", "multipart/form-data; boundary=yace-test")
            .body(body)
            .send()
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == 200 {
            let uploaded: serde_json::Value = response.json().unwrap();
            let path = std::path::Path::new(uploaded["path"].as_str().unwrap());
            assert!(path.is_file());
            assert!(path.starts_with(workspace.path().canonicalize().unwrap()));
            assert!(
                std::fs::read_to_string(path)
                    .unwrap()
                    .contains("Resolution = 192")
            );
        }
    }
    assert!(!workspace.path().join("escape.chart").exists());
}
