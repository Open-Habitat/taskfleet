//! Reducer folds of a worker's question to its parent (ADR 0004): one open
//! question per node, first closing event wins, stale ids never close a newer
//! question, the deadline only moves later, and the node ending cancels it.

use serde_json::{json, Value};
use taskfleet_core::{
    append_and_apply_event, ensure_root, read_node_opt, run_dir, AskOutcome, Node, NodeId, RunId,
    RunPaths,
};
use tempfile::TempDir;

const RUN: &str = "01jxsnap000000000000000000";

struct Run {
    _tmp: TempDir,
    paths: RunPaths,
}

impl Run {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        ensure_root(tmp.path()).unwrap();
        let dir = run_dir(tmp.path(), &RunId::parse_str(RUN).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let run = Self {
            paths: RunPaths::new(dir, RUN).unwrap(),
            _tmp: tmp,
        };
        run.append(
            "run.created",
            json!({"kind": "spinoff", "lifecycle": "autonomous", "title": "t"}),
        )
        .unwrap();
        run.append("node.created", json!({"kind": "spinoff"}))
            .unwrap();
        run
    }

    fn append(&self, kind: &str, data: Value) -> taskfleet_core::Result<u64> {
        let node = (kind != "run.created").then(|| NodeId::parse_str("n-0001").unwrap());
        append_and_apply_event(&self.paths, kind, node.as_ref(), None, data).map(|o| o.seq)
    }

    fn ask(&self, key: &str) -> u64 {
        self.append(
            "node.asked",
            json!({"attempt": 0, "question": "q?", "options": ["a", "b"], "default": "a",
                   "idempotency_key": key, "timeout_secs": 60}),
        )
        .unwrap()
    }

    fn node(&self) -> Node {
        read_node_opt(&self.paths, &NodeId::parse_str("n-0001").unwrap())
            .unwrap()
            .unwrap()
    }
}

#[test]
fn ask_opens_once_and_first_closing_event_wins() {
    let run = Run::new();
    let id = run.ask("k1");
    let open = run.node().open_ask.unwrap();
    assert_eq!(open.ask_id, id);
    assert_eq!((open.expires_at - open.asked_at).num_seconds(), 60);
    // A second question while one is open is a no-op.
    run.ask("k2");
    assert_eq!(run.node().open_ask.unwrap().ask_id, id);

    run.append(
        "node.answered",
        json!({"ask_id": id, "choice": "b", "decided_by": "agent"}),
    )
    .unwrap();
    run.append("node.ask_timed_out", json!({"ask_id": id}))
        .unwrap();
    let node = run.node();
    assert!(node.open_ask.is_none());
    let last = node.last_ask.unwrap();
    assert_eq!(last.outcome, AskOutcome::Answered);
    assert_eq!(last.choice.as_deref(), Some("b"));
    assert_eq!(last.decided_by.as_deref(), Some("agent"));
}

#[test]
fn stale_ids_never_close_a_newer_question() {
    let run = Run::new();
    let first = run.ask("k1");
    run.append("node.ask_timed_out", json!({"ask_id": first}))
        .unwrap();
    let second = run.ask("k2");
    run.append(
        "node.answered",
        json!({"ask_id": first, "answer": "late", "decided_by": "agent"}),
    )
    .unwrap();
    assert_eq!(run.node().open_ask.unwrap().ask_id, second);
    assert_eq!(run.node().last_ask.unwrap().outcome, AskOutcome::TimedOut);
}

#[test]
fn escalation_moves_the_deadline_only_later() {
    let run = Run::new();
    let id = run.ask("k1");
    let before = run.node().open_ask.unwrap().expires_at;
    run.append(
        "node.ask_escalated",
        json!({"ask_id": id, "extend_secs": 1}),
    )
    .unwrap();
    let open = run.node().open_ask.unwrap();
    assert_eq!(open.expires_at, before, "never earlier");
    assert!(open.escalated_at.is_some());
    run.append(
        "node.ask_escalated",
        json!({"ask_id": id, "extend_secs": 3600, "note": "n"}),
    )
    .unwrap();
    let open = run.node().open_ask.unwrap();
    assert!(open.expires_at > before);
    assert_eq!(open.escalation_note.as_deref(), Some("n"));
}

#[test]
fn node_ending_cancels_the_open_question() {
    for (kind, data) in [
        ("node.report", json!({"success": false, "summary": "s"})),
        ("node.retry", json!({"attempt": 1})),
        ("worker.exited", json!({"attempt": 0, "exit_code": 0})),
    ] {
        let run = Run::new();
        run.ask("k1");
        run.append(kind, data).unwrap();
        let node = run.node();
        assert!(node.open_ask.is_none(), "{kind}");
        let last = node.last_ask.unwrap();
        assert_eq!(last.outcome, AskOutcome::Cancelled, "{kind}");
        assert_eq!(last.cancel_reason.as_deref(), Some(kind));
    }
}

#[test]
fn malformed_events_are_rejected_before_entering_the_log() {
    let run = Run::new();
    for bad in [
        json!({"attempt": 0, "question": " ", "default": "a", "idempotency_key": "k", "timeout_secs": 60}),
        json!({"attempt": 0, "question": "q", "idempotency_key": "k", "timeout_secs": 60}),
        json!({"attempt": 0, "question": "q", "default": "a", "idempotency_key": "k", "timeout_secs": 0}),
        json!({"attempt": 0, "question": "q", "default": "a", "idempotency_key": "has space", "timeout_secs": 60}),
        json!({"attempt": 0, "question": "q", "default": "a", "idempotency_key": "k", "timeout_secs": 60, "options": ["x", "x"]}),
    ] {
        assert!(run.append("node.asked", bad).is_err());
    }
    let id = run.ask("k1");
    for bad in [
        json!({"ask_id": id, "decided_by": "agent"}),
        json!({"ask_id": id, "answer": "x", "choice": "a", "decided_by": "agent"}),
        json!({"ask_id": id, "answer": "x", "decided_by": "boss"}),
        json!({"ask_id": id, "choice": "zzz", "decided_by": "agent"}),
        json!({"ask_id": id, "answer": "keep", "choice": 123, "decided_by": "agent"}),
        json!({"ask_id": id, "answer": 7, "choice": "a", "decided_by": "agent"}),
    ] {
        assert!(run.append("node.answered", bad).is_err());
    }
    assert!(run.node().open_ask.is_some());
}
