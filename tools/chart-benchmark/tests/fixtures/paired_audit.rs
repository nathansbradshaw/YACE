//! Rust-only subprocess fixture. No model/reference inference occurs.
use std::{fs, io::Write, path::PathBuf};
fn main() {
    let side = if cfg!(baseline_audit) {
        "baseline"
    } else {
        "candidate"
    };
    let args: Vec<_> = std::env::args().collect();
    let get = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(args.len(), 17);
    assert_eq!(get("--bpm"), "120");
    assert_eq!(get("--guitar-source"), "guitar");
    assert_eq!(get("--title"), "Blind paired instrumental study");
    assert_eq!(get("--artist"), "YACE benchmark");
    let audio = PathBuf::from(get("--audio"));
    let label = audio
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let out = PathBuf::from(get("--output"));
    let root = out.ancestors().nth(3).unwrap();
    assert!(root.join("plan.json").is_file() && root.join("freeze.json").is_file());
    let plan = fs::read_to_string(root.join("plan.json")).unwrap();
    for i in 1..=6 {
        let name = format!("heldout-{i:02}");
        assert!(plan.contains(&name));
        if std::env::var("FAKE_PAIR_PREP_FAILED").ok().as_deref() != Some(&name) {
            assert!(root.join(name).join("inputs/stem-manifest.json").is_file());
        }
    }
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::var_os("FAKE_PAIR_CALLS").unwrap())
            .unwrap(),
        "{side}:{label}"
    )
    .unwrap();
    let mode = std::env::var("FAKE_PAIR_MODE").unwrap_or_else(|_| "clean".into());
    if mode == "failures" && side == "baseline" && label == "heldout-02" {
        eprintln!("PRIVATE baseline failure {}", audio.display());
        std::process::exit(17);
    }
    let template = PathBuf::from(std::env::var_os("FAKE_PAIR_TEMPLATE").unwrap());
    fs::create_dir(&out).unwrap();
    fs::create_dir(out.join("revised")).unwrap();
    // Poison recovery-off artifacts: the paired runner must never read them.
    fs::create_dir(out.join("baseline")).unwrap();
    fs::write(
        out.join("baseline/notes.chart"),
        "PRIVATE deliberately invalid internal baseline",
    )
    .unwrap();
    fs::write(out.join("baseline-events.json"), "not json").unwrap();
    let mut chart = side.to_owned();
    let mut events = side.to_owned();
    let mut provenance = "comparison";
    if side == "candidate" && mode == "failures" {
        match label {
            "heldout-03" => provenance = "wrong-source",
            "heldout-04" => events = "bad-bass".into(),
            "heldout-05" => chart = "moved-hand".into(),
            "heldout-06" => chart = "missing-guitar".into(),
            _ => {}
        }
    }
    if side == "candidate" && mode == "empty" && label == "heldout-06" {
        chart = "missing-guitar".into();
        events = "empty-guitar".into();
    }
    if side == "candidate" && mode == "artifact-ambiguity" && label == "heldout-03" {
        events = "omitted-drums".into();
    }
    if mode == "source-survivor" && label == "heldout-01" {
        events = format!("survivor-{side}");
        chart = events.clone();
    }
    fs::copy(
        template.join(format!("{chart}.chart")),
        out.join("revised/notes.chart"),
    )
    .unwrap();
    fs::copy(
        template.join(format!("{events}-events.json")),
        out.join("revised-events.json"),
    )
    .unwrap();
    fs::copy(
        template.join(format!("{provenance}.json")),
        out.join("comparison.json"),
    )
    .unwrap();
    if side == "candidate" && mode == "artifact-ambiguity" && label == "heldout-06" {
        fs::copy(
            template.join("bad-bass-events.json"),
            root.join("heldout-01/baseline/audit/revised-events.json"),
        )
        .unwrap();
    }
}
