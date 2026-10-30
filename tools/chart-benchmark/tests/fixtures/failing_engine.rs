//! Deliberate subprocess fixture: real Rust executable, no shell interpreter.
use std::{fs, path::PathBuf};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if std::env::var_os("FAKE_ENGINE_MODEL_FREE").is_some() {
        assert_eq!(args[1], "vocals");
        assert!(
            !args
                .iter()
                .any(|a| matches!(a.as_str(), "--recognize" | "--neural-pitch"))
        );
    }
    let output = PathBuf::from(&args[args.iter().position(|a| a == "--output").unwrap() + 1]);
    match args[1].as_str() {
        "transcribe" => fs::write(output, "{}").unwrap(),
        "build" => {
            fs::create_dir(&output).unwrap();
            fs::copy(
                std::env::var_os("FAKE_ENGINE_CHART").unwrap(),
                output.join("notes.chart"),
            )
            .unwrap();
        }
        "vocals" => {
            fs::create_dir(&output).unwrap();
            let mode = std::env::var("FAKE_ENGINE_VOCALS").unwrap();
            if mode != "missing-report" {
                fs::copy(
                    std::env::var_os("FAKE_ENGINE_REPORT").unwrap(),
                    output.join("generation.json"),
                )
                .unwrap();
            }
            if !matches!(
                mode.as_str(),
                "missing" | "needs-lyrics" | "no-voiced-pitch" | "model-free-abstention"
            ) {
                let bytes = if mode == "invalid" {
                    b"not JSON".to_vec()
                } else {
                    fs::read(std::env::var_os("FAKE_ENGINE_VOCAL_CHART").unwrap()).unwrap()
                };
                let bytes = if mode == "incompatible-export" {
                    String::from_utf8(bytes)
                        .unwrap()
                        .replace("\"resolution\":480", "\"resolution\":192")
                        .into_bytes()
                } else {
                    bytes
                };
                fs::write(output.join("vocals.json"), bytes).unwrap();
            }
            if mode == "failed-with-partial-chart" {
                std::process::exit(19);
            }
        }
        _ => panic!("unexpected stage"),
    }
}
