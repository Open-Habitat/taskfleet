//! Integration tests for a worker's blocking question to its parent
//! (`node ask` / `run answer` / `run escalate`, ADR 0004).
//!
//! Runs are synthesized through the sanctioned write path (`run create
//! --skip-materialize` + `event create node.created`), so no supervisor is
//! spawned; the commands under test are plain processes, as they are for a
//! real worker in any harness. The complete spawned Claude round trip lives in
//! `e2e_spinoff.rs`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;

mod common;
use common::TestHome;

fn bin(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    c.env("TASKFLEET_HOME", home)
        .env("HOME", home)
        .env("TASKFLEET_TEST_SKIP_MATERIALIZE", "1");
    c
}

/// Run expecting exit `want`; return the stdout envelope (outcome) or, for an
/// error, the stderr envelope.
fn run_exit(cmd: &mut Command, want: i32) -> Value {
    let out = cmd.output().expect("spawn");
    let code = out.status.code().expect("exit code");
    assert_eq!(
        code,
        want,
        "want exit {want}, got {code}; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.stdout.is_empty() {
        let stderr = String::from_utf8(out.stderr).unwrap();
        serde_json::from_str(stderr.lines().last().unwrap()).expect("error envelope")
    } else {
        serde_json::from_slice(&out.stdout).expect("stdout JSON")
    }
}

fn pending_run(home: &Path) -> String {
    let v = run_exit(
        bin(home).args([
            "--output", "json", "run", "create", "--kind", "spinoff", "--title", "ask",
        ]),
        0,
    );
    let id = v["data"]["run_id"].as_str().unwrap().to_string();
    event(home, &id, "node.created", json!({"kind": "spinoff"}));
    id
}

fn event(home: &Path, run_id: &str, kind: &str, data: Value) {
    let keep = TempDir::new().unwrap();
    let f = keep.path().join("data.json");
    std::fs::write(&f, data.to_string()).unwrap();
    run_exit(
        bin(home).args([
            "--output",
            "json",
            "event",
            "create",
            run_id,
            "--kind",
            kind,
            "--node-id",
            "n-0001",
            "--from-file",
            f.to_str().unwrap(),
        ]),
        0,
    );
}

fn ask(home: &Path, run_id: &str, key: &str, extra: &[&str]) -> Command {
    let mut c = bin(home);
    c.args([
        "--output",
        "json",
        "node",
        "ask",
        run_id,
        "n-0001",
        "--question",
        "Drop the v1 API?",
        "--option",
        "keep",
        "--option",
        "drop",
        "--default",
        "keep",
        "--idempotency-key",
        key,
    ]);
    c.args(extra);
    c
}

fn events(home: &Path, run_id: &str) -> Vec<Value> {
    std::fs::read_to_string(home.join("runs").join(run_id).join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn count(home: &Path, run_id: &str, kind: &str) -> usize {
    events(home, run_id)
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

fn wait_for_open_ask(home: &Path, run_id: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let shown = run_exit(
            bin(home).args(["--output", "json", "run", "show", run_id]),
            0,
        );
        if let Some(id) = shown["data"]["open_ask"]["ask_id"].as_u64() {
            return id;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no open ask appeared");
}

fn finish(child: Child, want: i32) -> Value {
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(want),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A worker asks and blocks; the parent's default `run wait` returns at once
/// on the question, the parent answers, and the blocked command exits 0 with
/// the answer. A re-invocation with the same key (a killed command) re-attaches
/// to the same, already answered, question.
#[test]
fn worker_asks_parent_waits_answers_and_worker_continues() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let worker = ask(home.path(), &id, "api-v1", &["--wait", "30s"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let ask_id = wait_for_open_ask(home.path(), &id);

    let start = Instant::now();
    let waited = run_exit(
        bin(home.path()).args(["--output", "json", "run", "wait", &id, "--timeout", "20s"]),
        0,
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "no grace for a question"
    );
    let run = &waited["data"]["runs"][0];
    assert_eq!(run["asked"], true);
    assert_eq!(run["open_ask"]["ask_id"], ask_id);
    assert_eq!(run["open_ask"]["state"], "open");
    assert_eq!(run["open_ask"]["default"], "keep");
    assert_eq!(
        run["awaiting_input"], false,
        "a question is not awaiting_input"
    );
    // --terminal ignores questions.
    run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "wait",
            &id,
            "--terminal",
            "--timeout",
            "1s",
        ]),
        2,
    );
    // An option that does not exist is refused without closing the question.
    let bad = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--choice",
            "maybe",
        ]),
        1,
    );
    assert_eq!(bad["error"]["code"], "invalid_choice");

    let answered = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--choice",
            "drop",
            "--decided-by",
            "person",
        ]),
        0,
    );
    assert_eq!(answered["data"]["outcome"], "answered");
    assert_eq!(answered["data"]["replayed"], false);

    let out = finish(worker, 0);
    assert_eq!(out["data"]["outcome"], "answered");
    assert_eq!(out["data"]["take"], "drop");
    assert_eq!(out["data"]["choice"], "drop");
    assert_eq!(out["data"]["decided_by"], "person");
    assert_eq!(out["data"]["ask_id"], ask_id);

    // Killed-and-re-run: same key re-attaches and reports the same answer.
    let again = run_exit(&mut ask(home.path(), &id, "api-v1", &["--wait", "1s"]), 0);
    assert_eq!(again["data"]["attached"], true);
    assert_eq!(again["data"]["ask_id"], ask_id);
    assert_eq!(again["data"]["take"], "drop");
    // An identical repeat answer is a replay; a different one is refused.
    let replay = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--choice",
            "drop",
            "--decided-by",
            "person",
        ]),
        0,
    );
    assert_eq!(replay["data"]["replayed"], true);
    let late = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--choice",
            "keep",
        ]),
        1,
    );
    assert_eq!(late["error"]["code"], "ask_closed");
    assert_eq!(count(home.path(), &id, "node.answered"), 1);
    assert_eq!(count(home.path(), &id, "node.asked"), 1);

    // The question no longer settles the wait; run show keeps last_ask.
    let shown = run_exit(
        bin(home.path()).args(["--output", "json", "run", "show", &id]),
        0,
    );
    assert!(shown["data"]["open_ask"].is_null());
    assert_eq!(shown["data"]["last_ask"]["outcome"], "answered");
    assert_eq!(shown["data"]["last_ask"]["decided_by"], "person");
    let listed = run_exit(
        bin(home.path()).args(["--output", "json", "run", "list"]),
        0,
    );
    assert_eq!(listed["data"]["runs"][0]["last_ask"]["ask_id"], ask_id);
}

/// `--wait` bounds one invocation (exit 4, pending); re-running with the same
/// key attaches without moving the deadline. A different question while one
/// is open is refused, as is reusing the key for a different question.
#[test]
fn pending_reattach_keeps_deadline_and_one_question_per_node() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let first = run_exit(
        &mut ask(home.path(), &id, "k1", &["--wait", "0s", "--timeout", "1h"]),
        4,
    );
    assert_eq!(first["data"]["outcome"], "pending");
    assert_eq!(first["data"]["attached"], false);
    let expires = first["data"]["expires_at"].clone();
    std::thread::sleep(Duration::from_millis(1100));
    let second = run_exit(
        &mut ask(home.path(), &id, "k1", &["--wait", "0s", "--timeout", "7d"]),
        4,
    );
    assert_eq!(second["data"]["attached"], true);
    assert_eq!(second["data"]["ask_id"], first["data"]["ask_id"]);
    assert_eq!(
        second["data"]["expires_at"], expires,
        "deadline never moves"
    );

    let other = run_exit(&mut ask(home.path(), &id, "k2", &["--wait", "0s"]), 1);
    assert_eq!(other["error"]["code"], "ask_already_open");
    assert_eq!(other["error"]["details"]["ask_id"], first["data"]["ask_id"]);

    let mut conflict = bin(home.path());
    conflict.args([
        "--output",
        "json",
        "node",
        "ask",
        &id,
        "n-0001",
        "--question",
        "Something else?",
        "--default",
        "stop",
        "--idempotency-key",
        "k1",
        "--wait",
        "0s",
    ]);
    let conflict = run_exit(&mut conflict, 1);
    assert_eq!(conflict["error"]["code"], "idempotency_conflict");
    assert_eq!(count(home.path(), &id, "node.asked"), 1);

    // --default is required.
    let mut missing = bin(home.path());
    missing.args([
        "node",
        "ask",
        &id,
        "n-0001",
        "--question",
        "q",
        "--idempotency-key",
        "k3",
    ]);
    let out = missing.output().unwrap();
    assert!(!out.status.success());
}

/// With no answer the worker gets `timed_out` (exit 2) with its default at the
/// deadline; a later answer is refused and names the default taken. Exactly
/// one closing event exists.
#[test]
fn unanswered_question_times_out_to_default_and_late_answer_is_refused() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let out = run_exit(
        &mut ask(
            home.path(),
            &id,
            "t1",
            &["--timeout", "1s", "--wait", "20s"],
        ),
        2,
    );
    assert_eq!(out["data"]["outcome"], "timed_out");
    assert_eq!(out["data"]["take"], "keep");
    let ask_id = out["data"]["ask_id"].as_u64().unwrap();
    let late = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--text",
            "too late",
        ]),
        1,
    );
    assert_eq!(late["error"]["code"], "ask_closed");
    assert!(late["error"]["message"]
        .as_str()
        .unwrap()
        .contains("default: keep"));
    assert_eq!(late["error"]["details"]["took_default"], true);
    assert_eq!(count(home.path(), &id, "node.ask_timed_out"), 1);
    assert_eq!(count(home.path(), &id, "node.answered"), 0);
    // The worker may ask again after the first question closed.
    let next = run_exit(&mut ask(home.path(), &id, "t2", &["--wait", "0s"]), 4);
    assert_ne!(next["data"]["ask_id"], ask_id);
}

/// If the worker's command is gone when the deadline passes, readers derive
/// `expired`, the wait no longer returns for it, and the parent's late answer
/// itself records the single timeout and is refused.
#[test]
fn expired_question_without_worker_is_closed_by_the_late_answer() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let first = run_exit(
        &mut ask(
            home.path(),
            &id,
            "gone",
            &["--timeout", "1s", "--wait", "0s"],
        ),
        4,
    );
    let ask_id = first["data"]["ask_id"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let shown = run_exit(
        bin(home.path()).args(["--output", "json", "run", "show", &id]),
        0,
    );
    assert_eq!(shown["data"]["open_ask"]["state"], "expired");
    run_exit(
        bin(home.path()).args(["--output", "json", "run", "wait", &id, "--timeout", "1s"]),
        2,
    );
    let late = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id.to_string(),
            "--choice",
            "drop",
        ]),
        1,
    );
    assert_eq!(late["error"]["code"], "ask_closed");
    assert_eq!(count(home.path(), &id, "node.ask_timed_out"), 1);
    // A re-attaching worker learns the timeout.
    let again = run_exit(&mut ask(home.path(), &id, "gone", &["--wait", "5s"]), 2);
    assert_eq!(again["data"]["take"], "keep");
}

/// `run escalate` keeps the question open, moves the deadline later, and stops
/// the question from returning the parent's `run wait`, so the parent can wait
/// again; the escalated question can still be answered.
#[test]
fn escalated_question_stops_returning_wait_and_can_be_answered_later() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let first = run_exit(
        &mut ask(
            home.path(),
            &id,
            "e1",
            &["--timeout", "10m", "--wait", "0s"],
        ),
        4,
    );
    let ask_id = first["data"]["ask_id"].as_u64().unwrap().to_string();
    run_exit(
        bin(home.path()).args(["--output", "json", "run", "wait", &id, "--timeout", "5s"]),
        0,
    );
    let esc = run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "escalate",
            &id,
            "--ask",
            &ask_id,
            "--extend",
            "4h",
            "--note",
            "asked the person",
        ]),
        0,
    );
    assert_eq!(esc["data"]["open_ask"]["state"], "escalated");
    assert!(esc["data"]["open_ask"]["remaining_secs"].as_i64().unwrap() > 3 * 3600);
    let waited = run_exit(
        bin(home.path()).args(["--output", "json", "run", "wait", &id, "--timeout", "1s"]),
        2,
    );
    assert_eq!(waited["data"]["runs"][0]["open_ask"]["escalated"], true);
    assert!(waited["data"]["runs"][0]["asked"].is_null());
    run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "run",
            "answer",
            &id,
            "--ask",
            &ask_id,
            "--text",
            "Keep it one more release.",
        ]),
        0,
    );
    let got = run_exit(&mut ask(home.path(), &id, "e1", &["--wait", "5s"]), 0);
    assert_eq!(got["data"]["take"], "Keep it one more release.");
    assert_eq!(got["data"]["decided_by"], "agent");
}

/// The node ending while the worker waits closes the question as cancelled,
/// and the worker's command exits 3.
#[test]
fn node_ending_cancels_the_open_question() {
    let home = TestHome::new();
    let id = pending_run(home.path());
    let worker = ask(home.path(), &id, "c1", &["--wait", "30s"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_open_ask(home.path(), &id);
    let keep = TempDir::new().unwrap();
    let report = keep.path().join("r.json");
    std::fs::write(&report, r#"{"success":false,"summary":"stopped"}"#).unwrap();
    run_exit(
        bin(home.path()).args([
            "--output",
            "json",
            "node",
            "report",
            &id,
            "n-0001",
            "--from-file",
            report.to_str().unwrap(),
        ]),
        0,
    );
    let out = finish(worker, 3);
    assert_eq!(out["data"]["outcome"], "cancelled");
    assert_eq!(out["data"]["cancel_reason"], "node.report");
    // Asking on an ended node does not open a question.
    let late = run_exit(&mut ask(home.path(), &id, "c2", &["--wait", "0s"]), 3);
    assert!(late["data"]["ask_id"].is_null());
    assert_eq!(count(home.path(), &id, "node.asked"), 1);
}

/// An answer racing the deadline leaves exactly one closing event, and both
/// commands report that one.
#[test]
fn answer_racing_the_deadline_leaves_one_closing_event() {
    let home = TestHome::new();
    for round in 0..3 {
        let id = pending_run(home.path());
        let worker = ask(
            home.path(),
            &id,
            "race",
            &["--timeout", "1s", "--wait", "20s"],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        let ask_id = wait_for_open_ask(home.path(), &id).to_string();
        // Aim at the deadline.
        std::thread::sleep(Duration::from_millis(900 + round * 50));
        let answer = bin(home.path())
            .args([
                "--output", "json", "run", "answer", &id, "--ask", &ask_id, "--choice", "drop",
            ])
            .output()
            .unwrap();
        let out = worker.wait_with_output().unwrap();
        let worker_out: Value = serde_json::from_slice(&out.stdout).unwrap();
        let closes = count(home.path(), &id, "node.answered")
            + count(home.path(), &id, "node.ask_timed_out");
        assert_eq!(closes, 1, "exactly one closing event");
        if answer.status.success() {
            assert_eq!(out.status.code(), Some(0));
            assert_eq!(worker_out["data"]["outcome"], "answered");
        } else {
            assert_eq!(out.status.code(), Some(2));
            assert_eq!(worker_out["data"]["outcome"], "timed_out");
            let err: Value = serde_json::from_str(
                String::from_utf8_lossy(&answer.stderr)
                    .lines()
                    .last()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(err["error"]["code"], "ask_closed");
        }
    }
}
