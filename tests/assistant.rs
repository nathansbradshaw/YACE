use chart_engine::editor::{Action, Actor, ActorKind, DecisionStatus, Request, Store};
use std::fs;
#[path = "support/assistant_project.rs"]
mod project_fixture;
use chart_engine::{
    ai_harness::{self, Options},
    assistant::{self, Provider, Reply, Review, Suggestion},
};
use project_fixture::{fixture, save};
use serde_json::json;

fn review(store: &Store) -> Review {
    let packet = ai_harness::review(store, Options::default()).unwrap();
    let note = packet
        .sections
        .iter()
        .flat_map(|s| &s.notes)
        .find(|n| n.lane.is_some())
        .unwrap();
    let action = Action::UpdateNote {
        note_id: note.id.clone(),
        tick: note.tick,
        lane: 2,
        sustain: 0,
        cymbal: false,
    };
    Review {
        content_sha256: assistant::content_fingerprint(&store.load().unwrap()).unwrap(),
        packet,
        reply: Reply {
            answer: "This repeat may be easier to read consistently. Listen before deciding."
                .into(),
            suggestions: vec![Suggestion {
                reason: "Keep the repeated pattern consistent".into(),
                action,
            }],
        },
        provider: Provider::Codex,
        model: "test".into(),
    }
}
#[test]
fn assistant_edits_are_explicit_attributed_undoable_and_stale_safe() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r = review(&store);
    let original = store.load().unwrap();
    let text = serde_json::to_string(&r.reply).unwrap();
    assistant::parse_reply(&text, &r.packet).unwrap();
    assert_eq!(store.load().unwrap().revision, original.revision);
    assistant::apply(&store, &r, 0).unwrap();
    let changed = store.load().unwrap();
    assert_eq!(changed.audit.last().unwrap().actor.kind, ActorKind::Ai);
    assert!(assistant::apply(&store, &r, 0).is_err());
    store
        .apply(Request {
            expected_revision: changed.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "tester".into(),
            },
            action: Action::Undo,
        })
        .unwrap();
    assert_eq!(
        store.load().unwrap().content.tracks,
        original.content.tracks
    );
}
#[test]
fn assistant_rejects_unknown_notes_approvals_exports_and_malformed_answers() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut r = review(&store);
    for action in [
        Action::DeleteNote {
            note_id: "invented".into(),
        },
        Action::Export {
            output: root.path().join("leak"),
        },
        Action::Decide {
            region_id: "anything".into(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "approved".into(),
        },
    ] {
        r.reply.suggestions[0].action = action;
        assert!(
            assistant::parse_reply(&serde_json::to_string(&r.reply).unwrap(), &r.packet).is_err()
        );
        assert!(assistant::apply(&store, &r, 0).is_err());
    }
    assert!(assistant::parse_reply("Not JSON", &r.packet).is_err());
    assert!(
        assistant::parse_reply(
            &json!({"answer":"hi","suggestions":[],"actor":"human"}).to_string(),
            &r.packet
        )
        .is_err()
    );
    assert_eq!(store.load().unwrap().revision, 0);
}
#[test]
fn provider_adapters_read_only_the_assistant_answer() {
    assert_eq!(
        assistant::decode_http(Provider::Ollama, &json!({"message":{"content":"one"}})).unwrap(),
        "one"
    );
    assert_eq!(
        assistant::decode_http(
            Provider::OpenAi,
            &json!({"choices":[{"message":{"content":"two"}}]})
        )
        .unwrap(),
        "two"
    );
    assert_eq!(assistant::decode_http(Provider::Anthropic,&json!({"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"three"}]})).unwrap(),"three");
    assert!(assistant::decode_http(Provider::OpenAi, &json!({"error":"no key"})).is_err());
}
#[test]
fn cancellation_and_missing_model_fail_before_inference() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let connection = assistant::Connection {
        provider: Provider::Ollama,
        model: String::new(),
        api_key: String::new(),
    };
    let cancel = assistant::Cancellation::default();
    assert!(
        assistant::ask(&store, &connection, "help", &[], &cancel)
            .unwrap_err_string()
            .contains("model")
    );
    cancel.cancel();
    assert!(
        assistant::ask(&store, &connection, "help", &[], &cancel)
            .unwrap_err_string()
            .contains("cancelled")
    );
}
trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl<T> ErrorString for anyhow::Result<T> {
    fn unwrap_err_string(self) -> String {
        match self {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected failure"),
        }
    }
}
#[test]
fn changed_evidence_and_same_revision_content_block_edits() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r = review(&store);
    fs::write(root.path().join("source/song.ogg"), b"changed audio").unwrap();
    assert!(assistant::apply(&store, &r, 0).is_err());
    assert_eq!(store.load().unwrap().revision, 0);
}
#[test]
#[ignore = "Requires installed, signed-in Codex CLI and network; explicit live smoke test"]
fn live_codex_chart_help() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r=assistant::ask(&store,&assistant::Connection{provider:Provider::Codex,model:String::new(),api_key:String::new()},"Explain what the flagged fast repeat means and how I should review it. Give advice only; no suggested edits.",&[],&assistant::Cancellation::default()).unwrap();
    assert!(!r.reply.answer.is_empty());
    assert!(r.reply.suggestions.is_empty());
    println!("{}", r.reply.answer);
    assert_eq!(store.load().unwrap().revision, 0);
}

#[test]
fn listening_and_navigation_preserve_suggestions_but_note_changes_do_not() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r = review(&store);
    for action in [
        Action::Seek { seconds: 1. },
        Action::SetLoop {
            range: Some(chart_engine::editor::TimeRange {
                start: 0.5,
                end: 1.5,
            }),
        },
        Action::Playback { playing: false },
    ] {
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "Listener".into(),
                },
                action,
            })
            .unwrap();
    }
    assert_eq!(
        assistant::content_fingerprint(&store.load().unwrap()).unwrap(),
        r.content_sha256
    );
    assistant::apply(&store, &r, 0).unwrap();
    assert!(assistant::apply(&store, &r, 0).is_err());
}

#[test]
#[ignore = "Requires installed, signed-in Claude Code CLI and network; explicit live smoke test"]
fn live_claude_chart_help() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r = assistant::ask(
        &store,
        &assistant::Connection {
            provider: Provider::Claude,
            model: String::new(),
            api_key: String::new(),
        },
        "Explain the flagged repeat. Advice only; return no suggested edits.",
        &[],
        &assistant::Cancellation::default(),
    )
    .unwrap();
    assert!(!r.reply.answer.is_empty());
    assert!(r.reply.suggestions.is_empty());
    println!("{}", r.reply.answer);
    assert_eq!(store.load().unwrap().revision, 0);
}

#[test]
fn same_revision_note_change_requires_fresh_advice() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let r = review(&store);
    let mut doc = store.load().unwrap();
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .lane = 1;
    save(&store, &doc);
    assert!(assistant::apply(&store, &r, 0).is_err());
    assert_eq!(store.load().unwrap().revision, 0);
}
