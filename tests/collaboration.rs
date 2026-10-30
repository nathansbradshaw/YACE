use chart_engine::{
    chart::Chart,
    editor::{Action, Actor, ActorKind, Content, EditorNote, Project, Request, Store, Transport},
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n 500 = N 0 0\n 2400 = N 1 0\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n";

fn fixture(root: &Path) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    fs::write(root.join("source/notes.chart"), SOURCE).unwrap();
    let audio = b"opaque recording fixture; visibility uses no decoder";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let chart = Chart::parse(SOURCE.into()).unwrap();
    let tracks = ["ExpertSingle", "ExpertDrums"]
        .into_iter()
        .map(|track| {
            (
                track.into(),
                chart
                    .notes(track)
                    .unwrap()
                    .into_iter()
                    .map(|note| EditorNote {
                        id: format!("{track}-{}", note.tick),
                        note,
                    })
                    .collect(),
            )
        })
        .collect();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Visibility fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: source_hash.clone(),
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash.clone(),
        duration: 4.0,
        review: ReviewMap::build(
            4.0,
            Policy {
                window_seconds: 2.0,
                ..Policy::default()
            },
            BTreeMap::from([
                ("chart".into(), source_hash),
                ("original_audio".into(), audio_hash),
            ]),
            Some(BTreeSet::from([
                "ExpertSingle".into(),
                "ExpertDrums".into(),
            ])),
            BTreeSet::from([Reason::FastRepeat]),
            vec![
                Issue::at(0.5, "ExpertSingle", Reason::FastRepeat, &[]),
                Issue::at(0.5, "ExpertDrums", Reason::FastRepeat, &[]),
                Issue::at(2.5, "ExpertSingle", Reason::FastRepeat, &[]),
            ],
        )
        .unwrap(),
        content: Content {
            difficulty_links: Default::default(),
            vocals: None,
            vocal_review: None,
            tracks,
            decisions: vec![],
            hidden_flags: BTreeSet::new(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: None,
        kit_audition: None,
        transport: Transport {
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
            playback_rate: 1.0,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    save(&Store::new(root), &project);
    Store::new(root)
}

fn save(store: &Store, project: &Project) {
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec_pretty(project).unwrap(),
    )
    .unwrap();
}

use chart_engine::collaboration::{self, Anchor, Operation, Proposal};
fn actor(name: &str) -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: name.into(),
    }
}
fn run(s: &Store, name: &str, operation: Operation) -> anyhow::Result<()> {
    s.apply(Request {
        expected_revision: s.load()?.revision,
        actor: actor(name),
        action: Action::Collaborate { operation },
    })?;
    Ok(())
}
fn proposal(s: &Store) -> Proposal {
    let base = s.load().unwrap();
    let mut edit = base.clone();
    edit.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .lane = 2;
    Proposal::between(
        &base,
        &edit,
        "p1".into(),
        "Fix phrase".into(),
        "Match the guitar".into(),
        actor("Alice"),
    )
    .unwrap()
}
#[test]
fn complete_review_merge_undo_and_reopen_preserve_history() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = proposal(&s);
    let fp = p.fingerprint().unwrap();
    run(
        &s,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    let anchor = Some(Anchor {
        track: "ExpertSingle".into(),
        start_tick: 480,
        end_tick: 500,
        note_id: Some(p.proposed["ExpertSingle"][0].id.clone()),
    });
    run(
        &s,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "t1".into(),
            anchor: anchor.clone(),
            body: "Check this attack".into(),
        },
    )
    .unwrap();
    run(
        &s,
        "Alice",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "t1".into(),
            anchor,
            body: "Checked against the stem".into(),
        },
    )
    .unwrap();
    run(
        &s,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "Listened and approved".into(),
        },
    )
    .unwrap();
    let merge = Operation::Merge {
        proposal_id: "p1".into(),
        fingerprint: fp.clone(),
    };
    let bytes = fs::read(s.root.join("project.json")).unwrap();
    assert!(run(&s, "Maintainer", merge.clone()).is_err());
    assert_eq!(bytes, fs::read(s.root.join("project.json")).unwrap());
    run(
        &s,
        "Bob",
        Operation::SetResolved {
            proposal_id: "p1".into(),
            thread_id: "t1".into(),
            resolved: true,
        },
    )
    .unwrap();
    run(&s, "Maintainer", merge.clone()).unwrap();
    assert_eq!(s.load().unwrap().content.tracks, p.proposed);
    assert_eq!(
        collaboration::inspect(&s.load().unwrap()).unwrap()["p1"].status,
        "merged"
    );
    assert!(run(&s, "Maintainer", merge).is_err());
    let d = s.load().unwrap();
    s.apply(Request {
        expected_revision: d.revision,
        actor: actor("Maintainer"),
        action: Action::Undo,
    })
    .unwrap();
    assert_eq!(s.load().unwrap().content.tracks, p.base);
    assert_eq!(
        collaboration::inspect(&s.load().unwrap()).unwrap()["p1"].status,
        "merged"
    );
}
#[test]
fn independent_edits_merge_but_conflicts_and_stale_reviews_do_not_write() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = proposal(&s);
    let fp = p.fingerprint().unwrap();
    let mut ours = p.base.clone();
    ours.get_mut("ExpertDrums").unwrap()[0].note.tick += 5;
    let merged = collaboration::merge_tracks(&p.base, &ours, &p.proposed).unwrap();
    assert_eq!(merged["ExpertDrums"], ours["ExpertDrums"]);
    assert_eq!(merged["ExpertSingle"], p.proposed["ExpertSingle"]);
    ours.get_mut("ExpertSingle").unwrap()[0].note.lane = 3;
    assert!(collaboration::merge_tracks(&p.base, &ours, &p.proposed).is_err());
    run(
        &s,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    let bytes = fs::read(s.root.join("project.json")).unwrap();
    assert!(
        run(
            &s,
            "Bob",
            Operation::Review {
                proposal_id: "p1".into(),
                fingerprint: "stale".into(),
                approve: true,
                body: "yes".into()
            }
        )
        .is_err()
    );
    assert_eq!(bytes, fs::read(s.root.join("project.json")).unwrap());
    run(
        &s,
        "Alice",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "self".into(),
        },
    )
    .unwrap();
    assert!(
        run(
            &s,
            "Alice",
            Operation::Merge {
                proposal_id: "p1".into(),
                fingerprint: fp.clone()
            }
        )
        .is_err()
    );
    run(
        &s,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: false,
            body: "needs work".into(),
        },
    )
    .unwrap();
    assert!(
        run(
            &s,
            "Alice",
            Operation::Merge {
                proposal_id: "p1".into(),
                fingerprint: fp
            }
        )
        .is_err()
    );
}
#[test]
fn binding_duplicate_ids_delete_edit_and_concurrent_additions_are_guarded() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = proposal(&s);
    let mut bad = p.clone();
    bad.audio_sha256 = "different".into();
    assert!(run(&s, "Alice", Operation::Submit { proposal: bad }).is_err());
    let mut bad = p.clone();
    bad.proposed
        .get_mut("ExpertSingle")
        .unwrap()
        .push(p.proposed["ExpertSingle"][0].clone());
    assert!(run(&s, "Alice", Operation::Submit { proposal: bad }).is_err());
    let mut deleted = p.base.clone();
    deleted.get_mut("ExpertSingle").unwrap().remove(0);
    assert!(collaboration::merge_tracks(&p.base, &deleted, &p.proposed).is_err());
    let mut ours = p.base.clone();
    let mut theirs = p.base.clone();
    let mut n = ours["ExpertSingle"][0].clone();
    n.id = "edit-note-1".into();
    n.note.tick = 1000;
    ours.get_mut("ExpertSingle").unwrap().push(n.clone());
    n.note.tick = 1100;
    theirs.get_mut("ExpertSingle").unwrap().push(n);
    assert!(collaboration::merge_tracks(&p.base, &ours, &theirs).is_err());
}

#[test]
fn merge_conflict_and_expected_revision_leave_project_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = proposal(&s);
    let fp = p.fingerprint().unwrap();
    run(
        &s,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    run(
        &s,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "Approved".into(),
        },
    )
    .unwrap();
    let doc = s.load().unwrap();
    let note = &doc.content.tracks["ExpertSingle"][0];
    s.apply(Request {
        expected_revision: doc.revision,
        actor: actor("Charlie"),
        action: Action::UpdateNote {
            note_id: note.id.clone(),
            tick: note.note.tick,
            lane: 3,
            sustain: 0,
            cymbal: false,
        },
    })
    .unwrap();
    let bytes = fs::read(s.root.join("project.json")).unwrap();
    assert!(
        run(
            &s,
            "Maintainer",
            Operation::Merge {
                proposal_id: "p1".into(),
                fingerprint: fp
            }
        )
        .unwrap_err()
        .to_string()
        .contains("Conflicting notes")
    );
    assert_eq!(bytes, fs::read(s.root.join("project.json")).unwrap());
    assert!(
        s.apply(Request {
            expected_revision: 0,
            actor: actor("Bob"),
            action: Action::Collaborate {
                operation: Operation::Close {
                    proposal_id: "p1".into(),
                    reason: "old revision".into()
                }
            }
        })
        .is_err()
    );
    assert_eq!(bytes, fs::read(s.root.join("project.json")).unwrap());
}

#[test]
fn portable_cli_proposal_import_and_diff_work_between_project_folders() {
    use std::process::Command;
    let base_root = tempfile::tempdir().unwrap();
    let edited_root = tempfile::tempdir().unwrap();
    let base = fixture(base_root.path());
    let edited = fixture(edited_root.path());
    let mut doc = edited.load().unwrap();
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .lane = 2;
    save(&edited, &doc);
    let exchange = tempfile::tempdir().unwrap();
    let proposal_file = exchange.path().join("proposal.json");
    let binary = env!("CARGO_BIN_EXE_collaborate");
    let output = Command::new(binary)
        .arg("propose")
        .arg(&base.root)
        .arg(&edited.root)
        .arg(&proposal_file)
        .args([
            "--id",
            "exchange-1",
            "--title",
            "Guitar correction",
            "--description",
            "Correct this phrase",
            "--actor",
            "Alice",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&proposal_file).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("opaque recording"));
    let output = Command::new(binary)
        .arg("import")
        .arg(&base.root)
        .arg(&proposal_file)
        .args(["--actor", "Alice", "--revision", "0"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary)
        .arg("show")
        .arg(&base.root)
        .output()
        .unwrap();
    assert!(output.status.success());
    let shown: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(shown["revision"], 1);
    assert_eq!(shown["proposals"][0]["diff"].as_array().unwrap().len(), 1);
    let fp = shown["proposals"][0]["fingerprint"].as_str().unwrap();
    let review_file = exchange.path().join("review.json");
    fs::write(
        &review_file,
        serde_json::to_vec(&Operation::Review {
            proposal_id: "exchange-1".into(),
            fingerprint: fp.into(),
            approve: true,
            body: "Reviewed the note correction".into(),
        })
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(binary)
        .arg("apply")
        .arg(&base.root)
        .arg(review_file)
        .args(["--actor", "Bob", "--revision", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        collaboration::inspect(&base.load().unwrap()).unwrap()["exchange-1"].reviews[0]
            .actor
            .name,
        "Bob"
    );
}

use chart_engine::collaboration_exchange as exchange;
fn pull(s: &Store, bundle: exchange::Bundle, name: &str) -> anyhow::Result<()> {
    let bundle = serde_json::from_slice(&serde_json::to_vec(&bundle)?)?;
    s.apply(Request {
        expected_revision: s.load()?.revision,
        actor: actor(name),
        action: Action::ImportCollaboration { bundle },
    })?;
    Ok(())
}
fn bundle(s: &Store) -> exchange::Bundle {
    exchange::export_bundle(&s.load().unwrap()).unwrap()
}
#[test]
fn separate_people_complete_review_suggestion_approval_merge_and_receipt_exchange() {
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    let p = proposal(&a);
    let fp = p.fingerprint().unwrap();
    run(
        &a,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    pull(&b, bundle(&a), "Bob importer").unwrap();
    let bytes = fs::read(b.root.join("project.json")).unwrap();
    pull(&b, bundle(&a), "Bob importer").unwrap();
    assert_eq!(bytes, fs::read(b.root.join("project.json")).unwrap());
    let mut replacement = p.proposed["ExpertSingle"][0].clone();
    replacement.note.tick = 490;
    run(
        &b,
        "Bob",
        Operation::Suggest {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            suggestion_id: "suggest-1".into(),
            track: "ExpertSingle".into(),
            note_id: replacement.id.clone(),
            replacement: Some(replacement.clone()),
            body: "Move this attack ten ticks".into(),
        },
    )
    .unwrap();
    run(
        &b,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "phrase".into(),
            anchor: None,
            body: "Check timing".into(),
        },
    )
    .unwrap();
    let original = bundle(&b);
    pull(&a, original.clone(), "Alice importer").unwrap();
    assert!(original.events.iter().any(|e| e.actor.name == "Bob"));
    run(
        &a,
        "Alice",
        Operation::AcceptSuggestion {
            proposal_id: "p1".into(),
            suggestion_id: "suggest-1".into(),
            new_proposal_id: "p2".into(),
        },
    )
    .unwrap();
    let entry = collaboration::inspect(&a.load().unwrap()).unwrap()["p2"].clone();
    assert!(entry.reviews.is_empty());
    assert!(entry.proposal.proposed["ExpertSingle"].contains(&replacement));
    assert!(
        run(
            &a,
            "Alice",
            Operation::Merge {
                proposal_id: "p2".into(),
                fingerprint: entry.proposal.fingerprint().unwrap()
            }
        )
        .is_err()
    );
    pull(&b, bundle(&a), "Bob importer").unwrap();
    run(
        &b,
        "Bob",
        Operation::Review {
            proposal_id: "p2".into(),
            fingerprint: entry.proposal.fingerprint().unwrap(),
            approve: true,
            body: "Listened and approved revision".into(),
        },
    )
    .unwrap();
    pull(&a, bundle(&b), "Alice importer").unwrap();
    run(
        &a,
        "Alice",
        Operation::Merge {
            proposal_id: "p2".into(),
            fingerprint: entry.proposal.fingerprint().unwrap(),
        },
    )
    .unwrap();
    let before = b.load().unwrap().content.tracks;
    pull(&b, bundle(&a), "Bob importer").unwrap();
    let state = collaboration::inspect(&b.load().unwrap()).unwrap();
    assert_eq!(state["p2"].merge_receipts.len(), 1);
    assert_eq!(state["p2"].status, "open");
    assert_eq!(
        b.load().unwrap().content.tracks,
        before,
        "Receipt import must not overwrite local notes"
    );
    run(
        &b,
        "Bob",
        Operation::Merge {
            proposal_id: "p2".into(),
            fingerprint: entry.proposal.fingerprint().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        b.load().unwrap().content.tracks,
        a.load().unwrap().content.tracks
    );
    pull(&a, bundle(&b), "Alice importer").unwrap();
    assert_eq!(
        collaboration::inspect(&a.load().unwrap()).unwrap()["p2"]
            .merge_receipts
            .len(),
        2
    );
}
#[test]
fn concurrent_review_and_thread_updates_preserve_blockers_until_causally_acknowledged() {
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    let p = proposal(&a);
    let fp = p.fingerprint().unwrap();
    run(&a, "Alice", Operation::Submit { proposal: p }).unwrap();
    run(
        &a,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "t".into(),
            anchor: None,
            body: "Check it".into(),
        },
    )
    .unwrap();
    pull(&b, bundle(&a), "importer").unwrap();
    run(
        &a,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "yes".into(),
        },
    )
    .unwrap();
    run(
        &b,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: false,
            body: "needs work".into(),
        },
    )
    .unwrap();
    run(
        &a,
        "Alice",
        Operation::SetResolved {
            proposal_id: "p1".into(),
            thread_id: "t".into(),
            resolved: true,
        },
    )
    .unwrap();
    run(
        &b,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "t".into(),
            anchor: None,
            body: "Another concern".into(),
        },
    )
    .unwrap();
    let old = bundle(&a);
    pull(&a, bundle(&b), "importer").unwrap();
    pull(&b, old, "importer").unwrap();
    let state = collaboration::inspect(&a.load().unwrap()).unwrap();
    assert_eq!(state["p1"].reviews.len(), 2);
    assert!(!state["p1"].threads["t"].resolved);
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::to_value(collaboration::inspect(&b.load().unwrap()).unwrap()).unwrap()
    );
    assert!(
        run(
            &a,
            "Alice",
            Operation::Merge {
                proposal_id: "p1".into(),
                fingerprint: fp.clone()
            }
        )
        .is_err()
    );
    run(
        &a,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "Checked both concerns".into(),
        },
    )
    .unwrap();
    run(
        &a,
        "Alice",
        Operation::SetResolved {
            proposal_id: "p1".into(),
            thread_id: "t".into(),
            resolved: true,
        },
    )
    .unwrap();
    run(
        &a,
        "Alice",
        Operation::Merge {
            proposal_id: "p1".into(),
            fingerprint: fp,
        },
    )
    .unwrap();
}
#[test]
fn damaged_incomplete_wrong_song_and_locked_exchanges_leave_projects_unchanged() {
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    run(
        &a,
        "Alice",
        Operation::Submit {
            proposal: proposal(&a),
        },
    )
    .unwrap();
    run(
        &a,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "t".into(),
            anchor: None,
            body: "Test".into(),
        },
    )
    .unwrap();
    let initial = fs::read(b.root.join("project.json")).unwrap();
    let mut bad = bundle(&a);
    bad.events[0].actor.name = "tampered".into();
    assert!(pull(&b, bad, "importer").is_err());
    let mut bad = bundle(&a);
    bad.events
        .retain(|e| !matches!(e.operation, Operation::Submit { .. }));
    assert!(pull(&b, bad, "importer").is_err());
    let mut bad = bundle(&a);
    bad.version = 2;
    assert!(pull(&b, bad, "importer").is_err());
    assert_eq!(initial, fs::read(b.root.join("project.json")).unwrap());
    let folder = tempfile::tempdir().unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(folder.path().join("exchange.lock"))
        .unwrap();
    file.try_lock().unwrap();
    assert!(exchange::sync(&b, folder.path(), actor("Bob"), 0).is_err());
    file.unlock().unwrap();
    exchange::sync(
        &a,
        folder.path(),
        actor("Alice"),
        a.load().unwrap().revision,
    )
    .unwrap();
    exchange::sync(&b, folder.path(), actor("Bob"), 0).unwrap();
    let bytes = fs::read(b.root.join("project.json")).unwrap();
    exchange::sync(&b, folder.path(), actor("Bob"), b.load().unwrap().revision).unwrap();
    assert_eq!(bytes, fs::read(b.root.join("project.json")).unwrap());
}
#[test]
fn git_clones_exchange_review_events_without_merging_project_json() {
    use std::process::Command;
    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let root = tempfile::tempdir().unwrap();
    git(
        root.path(),
        &["init", "--bare", "--initial-branch=main", "remote.git"],
    );
    git(root.path(), &["clone", "remote.git", "alice"]);
    git(root.path(), &["clone", "remote.git", "bob"]);
    let ad = root.path().join("alice");
    let bd = root.path().join("bob");
    for dir in [&ad, &bd] {
        git(dir, &["config", "user.email", "local@example.invalid"]);
        git(dir, &["config", "user.name", "Local test"]);
    }
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    let p = proposal(&a);
    let fp = p.fingerprint().unwrap();
    run(&a, "Alice", Operation::Submit { proposal: p }).unwrap();
    exchange::sync(
        &a,
        &ad.join("reviews"),
        actor("Alice"),
        a.load().unwrap().revision,
    )
    .unwrap();
    git(&ad, &["add", "reviews"]);
    git(&ad, &["commit", "-m", "Propose phrase"]);
    git(&ad, &["push", "origin", "main"]);
    git(&bd, &["pull", "origin", "main"]);
    exchange::sync(&b, &bd.join("reviews"), actor("Bob"), 0).unwrap();
    run(
        &b,
        "Bob",
        Operation::Review {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            approve: true,
            body: "Approved via Git".into(),
        },
    )
    .unwrap();
    exchange::sync(
        &b,
        &bd.join("reviews"),
        actor("Bob"),
        b.load().unwrap().revision,
    )
    .unwrap();
    git(&bd, &["add", "reviews"]);
    git(&bd, &["commit", "-m", "Review phrase"]);
    git(&bd, &["push", "origin", "main"]);
    git(&ad, &["pull", "--ff-only", "origin", "main"]);
    exchange::sync(
        &a,
        &ad.join("reviews"),
        actor("Alice"),
        a.load().unwrap().revision,
    )
    .unwrap();
    run(
        &a,
        "Alice",
        Operation::Merge {
            proposal_id: "p1".into(),
            fingerprint: fp,
        },
    )
    .unwrap();
    let output = Command::new("git")
        .current_dir(&ad)
        .args(["ls-files"])
        .output()
        .unwrap();
    let tracked = String::from_utf8(output.stdout).unwrap();
    assert!(!tracked.contains("project.json"));
    assert!(!tracked.contains("exchange.lock"));
}

#[test]
fn independently_imported_same_proposal_coalesces_and_cli_bundles_preserve_authors() {
    use std::process::Command;
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    let p = proposal(&a);
    run(
        &a,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    run(&b, "Bob importer", Operation::Submit { proposal: p }).unwrap();
    pull(&a, bundle(&b), "Alice importer").unwrap();
    assert_eq!(collaboration::inspect(&a.load().unwrap()).unwrap().len(), 1);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("reviews.json");
    let binary = env!("CARGO_BIN_EXE_collaborate");
    let output = Command::new(binary)
        .arg("export-reviews")
        .arg(&a.root)
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rev = b.load().unwrap().revision.to_string();
    let output = Command::new(binary)
        .arg("import-reviews")
        .arg(&b.root)
        .arg(&file)
        .args(["--actor", "Bob importer", "--revision", &rev])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rev = b.load().unwrap().revision.to_string();
    let output = Command::new(binary)
        .arg("sync")
        .arg(&b.root)
        .arg(temp.path().join("events"))
        .args(["--actor", "Bob importer", "--revision", &rev])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        bundle(&b)
            .events
            .iter()
            .any(|event| event.actor.name == "Alice")
    );
    let bytes = fs::read(b.root.join("project.json")).unwrap();
    let mut bad = bundle(&a);
    bad.events[0].source_chart_sha256 = "other-song".into();
    assert!(
        pull(&b, bad, "importer")
            .unwrap_err()
            .to_string()
            .contains("another song")
    );
    assert_eq!(bytes, fs::read(b.root.join("project.json")).unwrap());
}

#[test]
fn musical_conflict_choices_create_fresh_proposals_or_keep_current_without_losing_history() {
    use chart_engine::collaboration::ConflictChoice;
    let ar = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let p = proposal(&a);
    let fp = p.fingerprint().unwrap();
    run(
        &a,
        "Alice",
        Operation::Submit {
            proposal: p.clone(),
        },
    )
    .unwrap();
    let mut doc = a.load().unwrap();
    let note = doc.content.tracks["ExpertSingle"][0].clone();
    a.apply(Request {
        expected_revision: doc.revision,
        actor: actor("Charlie"),
        action: Action::UpdateNote {
            note_id: note.id,
            tick: note.note.tick,
            lane: 3,
            sustain: 0,
            cymbal: false,
        },
    })
    .unwrap();
    doc = a.load().unwrap();
    let plan = collaboration::plan_merge(&p.base, &doc.content.tracks, &p.proposed).unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    let mut choices = BTreeMap::new();
    choices.insert(plan.conflicts[0].key.clone(), ConflictChoice::Proposed);
    run(
        &a,
        "Charlie",
        Operation::ResolveConflicts {
            proposal_id: "p1".into(),
            fingerprint: fp.clone(),
            current: doc.content.tracks.clone(),
            choices,
            new_proposal_id: "resolved-1".into(),
        },
    )
    .unwrap();
    let state = collaboration::inspect(&a.load().unwrap()).unwrap();
    let next = &state["resolved-1"];
    assert!(next.reviews.is_empty());
    assert_eq!(next.proposal.base, doc.content.tracks);
    assert_eq!(next.proposal.proposed, p.proposed);
    assert_eq!(
        a.load().unwrap().content.tracks,
        doc.content.tracks,
        "Conflict choices must not change the live song"
    );
    assert!(
        run(
            &a,
            "Charlie",
            Operation::Merge {
                proposal_id: "resolved-1".into(),
                fingerprint: next.proposal.fingerprint().unwrap()
            }
        )
        .is_err()
    );
    let mut choices = BTreeMap::new();
    choices.insert(plan.conflicts[0].key.clone(), ConflictChoice::Current);
    run(
        &a,
        "Charlie",
        Operation::ResolveConflicts {
            proposal_id: "p1".into(),
            fingerprint: fp,
            current: doc.content.tracks.clone(),
            choices,
            new_proposal_id: "unneeded".into(),
        },
    )
    .unwrap();
    let state = collaboration::inspect(&a.load().unwrap()).unwrap();
    assert_eq!(state["p1"].status, "closed");
    assert!(!state.contains_key("unneeded"));
    assert_eq!(a.load().unwrap().content.tracks, doc.content.tracks);
    let br = tempfile::tempdir().unwrap();
    let b = fixture(br.path());
    pull(&b, bundle(&a), "Bob importer").unwrap();
    let imported = collaboration::inspect(&b.load().unwrap()).unwrap();
    assert_eq!(imported["p1"].status, "closed");
    assert_eq!(
        imported["resolved-1"].proposal.proposed,
        state["resolved-1"].proposal.proposed
    );
}
#[test]
fn guided_git_handles_divergent_reviews_and_file_conflicts_without_terminal_commands() {
    use chart_engine::review_git as guided;
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let bare = root.path().join("remote.git");
    assert!(
        Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&bare)
            .output()
            .unwrap()
            .status
            .success()
    );
    let ar = tempfile::tempdir().unwrap();
    let br = tempfile::tempdir().unwrap();
    let a = fixture(ar.path());
    let b = fixture(br.path());
    let ad = root.path().join("alice");
    let bd = root.path().join("bob");
    guided::create(&ad, Some(bare.to_str().unwrap())).unwrap();
    run(
        &a,
        "Alice",
        Operation::Submit {
            proposal: proposal(&a),
        },
    )
    .unwrap();
    assert!(
        guided::sync(&a, &ad, actor("Alice"), a.load().unwrap().revision)
            .unwrap()
            .conflicts
            .is_empty()
    );
    guided::clone_repository(bare.to_str().unwrap(), &bd).unwrap();
    guided::sync(&b, &bd, actor("Bob"), 0).unwrap();
    run(
        &a,
        "Alice",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "alice-thread".into(),
            anchor: None,
            body: "Alice detail".into(),
        },
    )
    .unwrap();
    fs::write(
        ad.join(".gitignore"),
        "/exchange.lock\n/.tmp*\n/alice-only\n",
    )
    .unwrap();
    guided::sync(&a, &ad, actor("Alice"), a.load().unwrap().revision).unwrap();
    run(
        &b,
        "Bob",
        Operation::Comment {
            proposal_id: "p1".into(),
            thread_id: "bob-thread".into(),
            anchor: None,
            body: "Bob detail".into(),
        },
    )
    .unwrap();
    fs::write(bd.join(".gitignore"), "/exchange.lock\n/.tmp*\n/bob-only\n").unwrap();
    let outcome = guided::sync(&b, &bd, actor("Bob"), b.load().unwrap().revision).unwrap();
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(outcome.conflicts[0].path, ".gitignore");
    guided::resolve_file(&bd, ".gitignore", true).unwrap();
    assert!(guided::conflicts(&bd).unwrap().is_empty());
    guided::sync(&b, &bd, actor("Bob"), b.load().unwrap().revision).unwrap();
    guided::sync(&a, &ad, actor("Alice"), a.load().unwrap().revision).unwrap();
    let state = collaboration::inspect(&a.load().unwrap()).unwrap();
    assert!(state["p1"].threads.contains_key("alice-thread"));
    assert!(state["p1"].threads.contains_key("bob-thread"));
    assert_eq!(
        a.load().unwrap().content.tracks,
        b.load().unwrap().content.tracks
    );
    fs::write(ad.join("unrelated.txt"), "keep this").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&ad)
            .args(["add", "unrelated.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(guided::sync(&a, &ad, actor("Alice"), a.load().unwrap().revision).is_err());
    assert_eq!(
        fs::read_to_string(ad.join("unrelated.txt")).unwrap(),
        "keep this"
    );
}

#[test]
fn game_note_collisions_and_cross_track_id_collisions_have_explicit_gui_choices() {
    use chart_engine::collaboration::ConflictChoice;
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let base = s.load().unwrap().content.tracks;
    let mut ours = base.clone();
    let mut theirs = base.clone();
    let mut open = base["ExpertSingle"][0].clone();
    open.id = "alice-add".into();
    open.note.tick = 1000;
    open.note.lane = 7;
    ours.get_mut("ExpertSingle").unwrap().push(open.clone());
    let mut colored = open.clone();
    colored.id = "bob-add".into();
    colored.note.lane = 2;
    theirs.get_mut("ExpertSingle").unwrap().push(colored);
    let plan = collaboration::plan_merge(&base, &ours, &theirs).unwrap();
    assert_eq!(plan.conflicts.len(), 2);
    let choices = plan
        .conflicts
        .iter()
        .map(|c| (c.key.clone(), ConflictChoice::Current))
        .collect();
    let resolved = collaboration::resolve_tracks(&base, &ours, &theirs, &choices).unwrap();
    assert!(resolved["ExpertSingle"].contains(&open));
    assert!(!resolved["ExpertSingle"].iter().any(|n| n.id == "bob-add"));
    let mut theirs = base.clone();
    let mut drum = base["ExpertDrums"][0].clone();
    drum.id = "alice-add".into();
    drum.note.tick = 1000;
    theirs.get_mut("ExpertDrums").unwrap().push(drum);
    let plan = collaboration::plan_merge(&base, &ours, &theirs).unwrap();
    assert_eq!(
        plan.conflicts.len(),
        2,
        "Cross-track duplicate IDs must appear in the conflict GUI"
    );
}
