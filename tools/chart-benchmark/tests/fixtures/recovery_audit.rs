//! Synthetic Rust subprocess fixture; never loads a model or reference chart.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
fn copy(from: &Path, to: &Path) {
    fs::copy(from, to).unwrap();
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let get = |name: &str| args[args.iter().position(|a| a == name).unwrap() + 1].clone();
    assert_eq!(get("--bpm"), "120");
    assert_eq!(get("--guitar-source"), "guitar");
    assert_eq!(get("--title"), "Blind recovery ablation");
    assert_eq!(get("--artist"), "YACE benchmark");
    assert_eq!(args.len(), 17); // audio, manifest, models, clock, blind metadata, source mode, output
    let audio = PathBuf::from(get("--audio"));
    let label = audio
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::var_os("FAKE_RECOVERY_CALLS").unwrap())
            .unwrap(),
        "{label}"
    )
    .unwrap();
    if label == "heldout-02" {
        eprintln!(
            "Deliberate inference failure at private fixture path {}",
            audio.display()
        );
        std::process::exit(17);
    }
    let template = PathBuf::from(std::env::var_os("FAKE_RECOVERY_TEMPLATE").unwrap());
    let output = PathBuf::from(get("--output"));
    fs::create_dir(&output).unwrap();
    for variant in ["baseline", "revised"] {
        fs::create_dir(output.join(variant)).unwrap();
        copy(
            &template.join(format!("{variant}.chart")),
            &output.join(variant).join("notes.chart"),
        );
        let name = if variant == "revised" && label == "heldout-04" {
            "bad-bass-events.json".to_owned()
        } else {
            format!("{variant}-events.json")
        };
        copy(
            &template.join(name),
            &output.join(format!("{variant}-events.json")),
        );
    }
    copy(
        &template.join(if label == "heldout-06" {
            "wrong-source-comparison.json"
        } else {
            "comparison.json"
        }),
        &output.join("comparison.json"),
    );
}
