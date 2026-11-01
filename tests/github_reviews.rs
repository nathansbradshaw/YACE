use anyhow::Result;
use base64::Engine;
use chart_engine::{
    collaboration::{Operation, Proposal},
    collaboration_exchange as exchange,
    editor::{Action, Actor, ActorKind, Request},
    github_reviews::{self, Api, Reply},
    review_repository::{self, Repository},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    task::{Context, Poll, Waker},
};
mod fixture {
    include!("support/assistant_project.rs");
}
struct Mock {
    queue: RefCell<VecDeque<(&'static str, String, u16, Value)>>,
    calls: RefCell<Vec<(String, String, Option<Value>)>>,
}
impl Mock {
    fn new() -> Self {
        Self {
            queue: RefCell::default(),
            calls: RefCell::default(),
        }
    }
    fn expect(&self, method: &'static str, path: impl Into<String>, status: u16, body: Value) {
        self.queue
            .borrow_mut()
            .push_back((method, path.into(), status, body));
    }
    fn snapshot(&self, bundle: &exchange::Bundle, head: char) {
        let base = "/repos/alice/reviews";
        self.expect(
            "GET",
            format!("{base}/git/ref/heads/main"),
            200,
            json!({"object":{"type":"commit","sha":head.to_string().repeat(40)}}),
        );
        self.expect(
            "GET",
            format!("{base}/git/commits/{}", head.to_string().repeat(40)),
            200,
            json!({"tree":{"sha":"a".repeat(40)}}),
        );
        let files = std::iter::once((
            review_repository::MARKER.to_string(),
            review_repository::MARKER_CONTENT.as_bytes().to_vec(),
        ))
        .chain(bundle.events.iter().map(|e| {
            (
                format!("{}.json", e.id),
                serde_json::to_vec_pretty(e).unwrap(),
            )
        }))
        .collect::<Vec<_>>();
        let blobs:Vec<_>=files.iter().enumerate().map(|(i,(path,bytes))|json!({"path":path,"mode":"100644","type":"blob","sha":format!("{i:040x}"),"size":bytes.len()})).collect();
        self.expect(
            "GET",
            format!("{base}/git/trees/{}", "a".repeat(40)),
            200,
            json!({"sha":"a".repeat(40),"truncated":false,"tree":blobs}),
        );
        for (i, (_, bytes)) in files.iter().enumerate() {
            self.expect("GET",format!("{base}/git/blobs/{i:040x}"),200,json!({"sha":format!("{i:040x}"),"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
    }
    fn publish(&self, status: u16) {
        self.expect(
            "POST",
            "/repos/alice/reviews/git/trees",
            201,
            json!({"sha":"c".repeat(40)}),
        );
        self.expect(
            "POST",
            "/repos/alice/reviews/git/commits",
            201,
            json!({"sha":"d".repeat(40)}),
        );
        self.expect(
            "PATCH",
            "/repos/alice/reviews/git/refs/heads/main",
            status,
            json!({}),
        );
    }
}
impl Api for Mock {
    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Reply> {
        self.calls
            .borrow_mut()
            .push((method.into(), path.into(), body));
        let (expected_method, expected_path, status, body) = self
            .queue
            .borrow_mut()
            .pop_front()
            .expect("Unexpected API operation");
        assert_eq!((method, path), (expected_method, expected_path.as_str()));
        Ok(Reply { status, body })
    }
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("Mock unexpectedly waited for external state"),
    }
}
fn reviewed_project() -> (
    tempfile::TempDir,
    chart_engine::editor::Store,
    chart_engine::editor::Project,
) {
    let root = tempfile::tempdir().unwrap();
    let store = fixture::fixture(root.path());
    let base = store.load().unwrap();
    let mut changed = base.clone();
    changed.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 1;
    let actor = Actor {
        kind: ActorKind::Human,
        name: "Alice".into(),
    };
    let proposal = Proposal::between(
        &base,
        &changed,
        "fix-a".into(),
        "Correct note timing".into(),
        "Reviewed source recording".into(),
        actor.clone(),
    )
    .unwrap();
    store
        .apply(Request {
            expected_revision: base.revision,
            actor,
            action: Action::Collaborate {
                operation: Operation::Submit { proposal },
            },
        })
        .unwrap();
    let doc = store.load().unwrap();
    (root, store, doc)
}
#[test]
fn public_pull_preserves_notes_and_rejects_events_from_another_song() {
    let (_root, store, doc) = reviewed_project();
    let bundle = exchange::export_bundle(&doc).unwrap();
    let api = Mock::new();
    api.snapshot(&bundle, 'b');
    let pulled = ready(github_reviews::synchronize(
        &api,
        &Repository::parse("alice/reviews").unwrap(),
        &doc,
        false,
    ))
    .unwrap();
    assert_eq!(pulled.events.len(), 1);
    assert_eq!(store.load().unwrap().revision, doc.revision);
    assert_eq!(store.load().unwrap().content.tracks, doc.content.tracks);
    assert!(api.calls.borrow().iter().all(|c| c.0 == "GET"));
    let api = Mock::new();
    api.snapshot(&bundle, 'b');
    let mut other = doc.clone();
    other.audio_sha256 = "0".repeat(64);
    assert!(
        ready(github_reviews::synchronize(
            &api,
            &Repository::parse("alice/reviews").unwrap(),
            &other,
            true
        ))
        .is_err()
    );
    assert!(api.calls.borrow().iter().all(|c| c.0 == "GET"));
}
#[test]
fn competing_push_retries_with_new_parent_and_never_forces_or_replaces_files() {
    let (_root, _store, doc) = reviewed_project();
    let empty = exchange::Bundle {
        version: 1,
        events: vec![],
    };
    let api = Mock::new();
    api.snapshot(&empty, 'b');
    api.publish(422);
    api.snapshot(&empty, 'e');
    api.publish(200);
    let result = ready(github_reviews::synchronize(
        &api,
        &Repository::parse("alice/reviews").unwrap(),
        &doc,
        true,
    ))
    .unwrap();
    assert_eq!(result.events.len(), 1);
    assert!(api.queue.borrow().is_empty());
    let calls = api.calls.borrow();
    let commits: Vec<_> = calls
        .iter()
        .filter(|c| c.0 == "POST" && c.1.ends_with("/commits"))
        .map(|c| c.2.as_ref().unwrap())
        .collect();
    assert_eq!(commits[0]["parents"], json!(["b".repeat(40)]));
    assert_eq!(commits[1]["parents"], json!(["e".repeat(40)]));
    for (_, path, body) in calls.iter().filter(|c| c.0 != "GET") {
        let body = body.as_ref().unwrap();
        if path.ends_with("/main") {
            assert_eq!(body["force"], false);
        }
        if path.ends_with("/trees") {
            assert_eq!(body["base_tree"], "a".repeat(40));
            assert_eq!(body["tree"].as_array().unwrap().len(), 1);
            assert!(
                body["tree"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("Correct note timing")
            );
            assert!(body["tree"][0].get("sha").is_none());
        }
    }
}
#[test]
fn existing_events_make_publish_idempotent_and_permissions_errors_do_not_write() {
    let (_root, _store, doc) = reviewed_project();
    let api = Mock::new();
    api.snapshot(&exchange::export_bundle(&doc).unwrap(), 'b');
    ready(github_reviews::synchronize(
        &api,
        &Repository::parse("alice/reviews").unwrap(),
        &doc,
        true,
    ))
    .unwrap();
    assert!(api.calls.borrow().iter().all(|c| c.0 == "GET"));
    let api = Mock::new();
    api.expect(
        "GET",
        "/repos/alice/reviews/git/ref/heads/main",
        403,
        json!({"message":"secret response must not be echoed"}),
    );
    let error = ready(github_reviews::synchronize(
        &api,
        &Repository::parse("alice/reviews").unwrap(),
        &doc,
        true,
    ))
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("403"));
    assert!(!error.contains("secret response"));
    assert_eq!(api.calls.borrow().len(), 1);
}
#[test]
fn corrupted_remote_event_is_rejected_before_publish() {
    let (_root, _store, doc) = reviewed_project();
    let mut bundle = exchange::export_bundle(&doc).unwrap();
    bundle.events[0].actor.name = "Tampered".into();
    let api = Mock::new();
    api.snapshot(&bundle, 'b');
    assert!(
        ready(github_reviews::synchronize(
            &api,
            &Repository::parse("alice/reviews").unwrap(),
            &doc,
            true
        ))
        .is_err()
    );
    assert!(api.calls.borrow().iter().all(|c| c.0 == "GET"));
}

#[test]
fn creating_a_repository_retains_the_initial_commit_and_uses_no_force() {
    let api = Mock::new();
    api.expect("GET", "/user", 200, json!({"login":"alice"}));
    api.expect(
        "POST",
        "/user/repos",
        201,
        json!({"full_name":"alice/reviews","default_branch":"main"}),
    );
    api.expect(
        "GET",
        "/repos/alice/reviews/git/ref/heads/main",
        200,
        json!({"object":{"sha":"b".repeat(40)}}),
    );
    api.expect(
        "GET",
        format!("/repos/alice/reviews/git/commits/{}", "b".repeat(40)),
        200,
        json!({"tree":{"sha":"a".repeat(40)},"parents":[]}),
    );
    api.expect("GET",format!("/repos/alice/reviews/git/trees/{}","a".repeat(40)),200,json!({"sha":"a".repeat(40),"truncated":false,"tree":[{"path":"README.md","mode":"100644","type":"blob","sha":"e".repeat(40),"size":10}]}));
    api.publish(200);
    assert_eq!(
        ready(github_reviews::create(&api, "reviews"))
            .unwrap()
            .api(),
        "/repos/alice/reviews"
    );
    let calls = api.calls.borrow();
    assert_eq!(calls[1].2.as_ref().unwrap()["private"], true);
    let commit = calls
        .iter()
        .find(|c| c.0 == "POST" && c.1.ends_with("/commits"))
        .unwrap()
        .2
        .as_ref()
        .unwrap();
    assert_eq!(commit["parents"], json!(["b".repeat(40)]));
    assert_eq!(calls.last().unwrap().2.as_ref().unwrap()["force"], false);
    assert!(api.queue.borrow().is_empty());
}

#[test]
fn creation_refuses_a_repository_that_changed_during_initialization() {
    let api = Mock::new();
    api.expect("GET", "/user", 200, json!({"login":"alice"}));
    api.expect(
        "POST",
        "/user/repos",
        201,
        json!({"full_name":"alice/reviews","default_branch":"main"}),
    );
    api.expect(
        "GET",
        "/repos/alice/reviews/git/ref/heads/main",
        200,
        json!({"object":{"sha":"b".repeat(40)}}),
    );
    api.expect(
        "GET",
        format!("/repos/alice/reviews/git/commits/{}", "b".repeat(40)),
        200,
        json!({"tree":{"sha":"a".repeat(40)},"parents":[{"sha":"e".repeat(40)}]}),
    );
    assert!(ready(github_reviews::create(&api, "reviews")).is_err());
    assert_eq!(
        api.calls.borrow().iter().filter(|c| c.0 != "GET").count(),
        1
    );
}
