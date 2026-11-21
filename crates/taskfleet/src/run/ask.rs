//! A worker's blocking question to its parent (ADR 0004): the read views every
//! surface shares, and the parent-side verbs `run answer` and `run escalate`.
//!
//! The worker asks with `node ask` (`crate::node::ask`), which appends
//! `node.asked` and blocks. The parent's default `run wait` returns at once on
//! an open question that is neither escalated nor past its deadline. The
//! parent then answers (`node.answered`) or says it passed the question on to
//! a person (`node.ask_escalated`), which stops the question from returning
//! `run wait` and may move the deadline later.
//!
//! Who closes a question first is decided under the run's exclusive lock: a
//! command that finds the open question past its deadline appends
//! `node.ask_timed_out` instead of its own event, so an answer racing the
//! deadline leaves exactly one closing event and both sides report it. A
//! refused late answer names the default the worker took.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;

use taskfleet_core::{
    append_and_apply_unlocked, read_manifest_opt, read_node_opt, AskOutcome, ClosedAsk, LockedRun,
    Node, NodeId, OpenAsk, RunLock, RunPaths,
};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, run_paths_from_cli_arg};

/// The open question as read surfaces show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAskView {
    pub ask_id: u64,
    pub node_id: String,
    pub attempt: u32,
    pub question: String,
    pub options: Vec<String>,
    pub default: String,
    pub asked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Seconds until the deadline, never negative.
    pub remaining_secs: i64,
    /// `open` (waiting for the parent), `escalated` (the parent passed it on to
    /// a person), or `expired` (past its deadline with no closing event yet:
    /// the worker takes its default, and no answer is accepted).
    pub state: &'static str,
    pub escalated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_note: Option<String>,
    /// Copyable parent command.
    pub answer_hint: String,
}

impl OpenAskView {
    #[must_use]
    pub fn build(run_id: &str, node_id: &str, open: &OpenAsk, now: DateTime<Utc>) -> Self {
        let expired = now >= open.expires_at;
        let state = if expired {
            "expired"
        } else if open.escalated_at.is_some() {
            "escalated"
        } else {
            "open"
        };
        let node_flag = if node_id == "n-0001" {
            String::new()
        } else {
            format!(" --node {node_id}")
        };
        let answer_hint = if open.options.is_empty() {
            format!(
                "taskfleet run answer {run_id}{node_flag} --ask {} --text '<answer>'",
                open.ask_id
            )
        } else {
            format!(
                "taskfleet run answer {run_id}{node_flag} --ask {} --choice '<option>' (or --text '<answer>')",
                open.ask_id
            )
        };
        Self {
            ask_id: open.ask_id,
            node_id: node_id.to_string(),
            attempt: open.attempt,
            question: open.question.clone(),
            options: open.options.clone(),
            default: open.default.clone(),
            asked_at: open.asked_at,
            expires_at: open.expires_at,
            remaining_secs: open
                .expires_at
                .signed_duration_since(now)
                .num_seconds()
                .max(0),
            state,
            escalated: open.escalated_at.is_some(),
            escalated_at: open.escalated_at,
            escalation_note: open.escalation_note.clone(),
            answer_hint,
        }
    }

    /// True when this question returns the parent's default `run wait`.
    #[must_use]
    pub fn settles_wait(&self) -> bool {
        self.state == "open"
    }
}

/// How the most recent question ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastAskView {
    pub ask_id: u64,
    pub question: String,
    pub default: String,
    /// `answered`, `timed_out`, or `cancelled`.
    pub outcome: &'static str,
    pub closed_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
    /// True when the worker continued on its default (`timed_out`).
    pub took_default: bool,
}

impl LastAskView {
    #[must_use]
    pub fn build(closed: &ClosedAsk) -> Self {
        Self {
            ask_id: closed.ask.ask_id,
            question: closed.ask.question.clone(),
            default: closed.ask.default.clone(),
            outcome: closed.outcome.wire(),
            closed_at: closed.closed_at,
            answer: closed.answer.clone(),
            choice: closed.choice.clone(),
            decided_by: closed.decided_by.clone(),
            cancel_reason: closed.cancel_reason.clone(),
            took_default: closed.outcome == AskOutcome::TimedOut,
        }
    }
}

/// Both views for one node, for `run show`, `run list`, and `run wait`.
#[must_use]
pub fn views(
    run_id: &str,
    node: &Node,
    now: DateTime<Utc>,
) -> (Option<OpenAskView>, Option<LastAskView>) {
    let open = (!node.status.is_terminal())
        .then_some(node.open_ask.as_deref())
        .flatten()
        .map(|o| OpenAskView::build(run_id, node.node_id.as_str(), o, now));
    let last = node.last_ask.as_deref().map(LastAskView::build);
    (open, last)
}

/// Close the open question as timed out if, under the held lock, it is past
/// its deadline. Returns true when this call appended `node.ask_timed_out`.
pub(crate) fn time_out_if_expired(
    witness: &LockedRun<'_>,
    paths: &RunPaths,
    node_id: &NodeId,
    ask_id: u64,
    now: DateTime<Utc>,
) -> Result<bool, CliError> {
    let node = read_node_opt(paths, node_id).map_err(from_core)?;
    let expired = node
        .as_ref()
        .and_then(|n| n.open_ask.as_deref())
        .is_some_and(|o| o.ask_id == ask_id && now >= o.expires_at);
    if expired {
        append_and_apply_unlocked(
            witness,
            paths,
            "node.ask_timed_out",
            Some(node_id),
            None,
            json!({"ask_id": ask_id}),
        )
        .map_err(from_core)?;
    }
    Ok(expired)
}

/// What `run answer` delivers.
pub enum Reply {
    Text(String),
    Choice(String),
}

pub struct AnswerArgs<'a> {
    pub run_id: String,
    pub node_id: String,
    pub ask_id: u64,
    pub reply: Reply,
    pub decided_by: &'static str,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct AnswerPayload {
    run_id: String,
    node_id: String,
    ask_id: u64,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    choice: Option<String>,
    decided_by: String,
    /// The event that closed the question. On a replay it is the earlier one.
    event_seq: u64,
    /// True when an identical answer had already closed this question.
    replayed: bool,
}

/// Locked state shared by `run answer` and `run escalate`.
struct Target {
    paths: RunPaths,
    node_id: NodeId,
}

fn target(run_id: &str, node_id: &str) -> Result<Target, CliError> {
    let node_id = parse_node_id(node_id)?;
    let root = crate::home::root_dir()?;
    let paths = run_paths_from_cli_arg(&root, run_id)?;
    Ok(Target { paths, node_id })
}

fn load(t: &Target) -> Result<Node, CliError> {
    read_manifest_opt(&t.paths)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user("run_not_found", format!("run {} not found", t.paths.run_id))
                .with_invalid_value(t.paths.run_id.to_string())
        })?;
    read_node_opt(&t.paths, &t.node_id)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user("node_not_found", format!("node {} not found", t.node_id))
                .with_invalid_value(t.node_id.as_str())
        })
}

/// The refusal for a question that is not open: how it ended and what the
/// worker took, or which question is open instead.
fn not_open(t: &Target, node: &Node, ask_id: u64) -> CliError {
    if let Some(last) = node.last_ask.as_deref().filter(|l| l.ask.ask_id == ask_id) {
        let view = LastAskView::build(last);
        let took = match last.outcome {
            AskOutcome::TimedOut => format!("the worker took its default: {}", last.ask.default),
            AskOutcome::Answered => "it was already answered".to_string(),
            AskOutcome::Cancelled => "its node ended before an answer".to_string(),
        };
        return CliError::user(
            "ask_closed",
            format!(
                "ask {ask_id} of run {} is closed ({}); {took}",
                t.paths.run_id,
                last.outcome.wire()
            ),
        )
        .with_invalid_value(ask_id.to_string())
        .with_details(serde_json::to_value(view).unwrap_or_default());
    }
    if let Some(open) = node.open_ask.as_deref() {
        return CliError::user(
            "ask_not_open",
            format!(
                "ask {ask_id} is not the open question of node {}; the open one is ask {}",
                t.node_id, open.ask_id
            ),
        )
        .with_invalid_value(ask_id.to_string())
        .with_expected(json!([open.ask_id]));
    }
    CliError::user(
        "ask_not_found",
        format!(
            "node {} of run {} has no question {ask_id} open or recently closed",
            t.node_id, t.paths.run_id
        ),
    )
    .with_invalid_value(ask_id.to_string())
}

pub fn answer(args: AnswerArgs<'_>) -> Result<(), CliError> {
    let t = target(&args.run_id, &args.node_id)?;
    let (answer, choice) = match args.reply {
        Reply::Text(text) => (Some(text), None),
        Reply::Choice(c) => (None, Some(c)),
    };
    if let Some(text) = &answer {
        if !taskfleet_core::reducer::valid_ask_text(
            text,
            taskfleet_core::reducer::MAX_ASK_ANSWER_BYTES,
        ) {
            return Err(CliError::user(
                "invalid_answer",
                format!(
                    "answer must be nonblank UTF-8 of at most {} bytes without NUL",
                    taskfleet_core::reducer::MAX_ASK_ANSWER_BYTES
                ),
            ));
        }
    }
    let guard = RunLock::acquire_existing(&t.paths.lock()).map_err(from_core)?;
    let witness = guard.witness();
    let node = load(&t)?;
    let now = Utc::now();
    let is_open = node
        .open_ask
        .as_deref()
        .is_some_and(|o| o.ask_id == args.ask_id)
        && !node.status.is_terminal();
    if !is_open {
        // An identical repeat of the answer that closed it is a replay.
        if let Some(last) = node
            .last_ask
            .as_deref()
            .filter(|l| l.ask.ask_id == args.ask_id && l.outcome == AskOutcome::Answered)
        {
            if last.answer == answer
                && last.choice == choice
                && last.decided_by.as_deref() == Some(args.decided_by)
            {
                let payload = AnswerPayload {
                    run_id: t.paths.run_id.to_string(),
                    node_id: t.node_id.to_string(),
                    ask_id: args.ask_id,
                    outcome: "answered",
                    answer,
                    choice,
                    decided_by: args.decided_by.to_string(),
                    event_seq: last.closed_seq,
                    replayed: true,
                };
                drop(guard);
                return emit_answer(&payload, args.spec, args.warnings);
            }
        }
        return Err(not_open(&t, &node, args.ask_id));
    }
    if time_out_if_expired(&witness, &t.paths, &t.node_id, args.ask_id, now)? {
        let node = load(&t)?;
        return Err(not_open(&t, &node, args.ask_id));
    }
    let open = node.open_ask.as_deref().expect("checked open above");
    if let Some(c) = &choice {
        if !open.options.iter().any(|o| o == c) {
            let err = if open.options.is_empty() {
                CliError::user(
                    "invalid_choice",
                    format!("ask {} has no options; answer with --text", args.ask_id),
                )
            } else {
                CliError::user(
                    "invalid_choice",
                    format!("{c:?} is not an option of ask {}", args.ask_id),
                )
                .with_expected(json!(open.options))
            };
            return Err(err.with_invalid_value(c.clone()));
        }
    }
    let mut data = json!({"ask_id": args.ask_id, "decided_by": args.decided_by});
    if let Some(a) = &answer {
        data["answer"] = json!(a);
    }
    if let Some(c) = &choice {
        data["choice"] = json!(c);
    }
    let seq = append_and_apply_unlocked(
        &witness,
        &t.paths,
        "node.answered",
        Some(&t.node_id),
        None,
        data,
    )
    .map_err(from_core)?;
    drop(guard);
    let payload = AnswerPayload {
        run_id: t.paths.run_id.to_string(),
        node_id: t.node_id.to_string(),
        ask_id: args.ask_id,
        outcome: "answered",
        answer,
        choice,
        decided_by: args.decided_by.to_string(),
        event_seq: seq,
        replayed: false,
    };
    emit_answer(&payload, args.spec, args.warnings)
}

fn emit_answer(p: &AnswerPayload, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Text => {
            println!(
                "answered ask {} of {} ({}){}; the worker continues on it",
                p.ask_id,
                p.run_id,
                p.decided_by,
                if p.replayed { ", already recorded" } else { "" }
            );
            output::emit_text_warnings(warnings);
            Ok(())
        }
        _ => output::emit_envelope(p, spec, warnings),
    }
}

pub struct EscalateArgs<'a> {
    pub run_id: String,
    pub node_id: String,
    pub ask_id: u64,
    pub extend: Option<std::time::Duration>,
    pub note: Option<String>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct EscalatePayload {
    run_id: String,
    node_id: String,
    ask_id: u64,
    event_seq: u64,
    open_ask: OpenAskView,
}

pub fn escalate(args: EscalateArgs<'_>) -> Result<(), CliError> {
    use taskfleet_core::reducer::{valid_ask_text, MAX_ASK_NOTE_BYTES, MAX_ASK_TIMEOUT_SECS};
    let extend_secs = match args.extend {
        None => None,
        Some(d) if d.as_secs() >= 1 && d.as_secs() <= MAX_ASK_TIMEOUT_SECS => Some(d.as_secs()),
        Some(d) => {
            return Err(CliError::user(
                "invalid_extend",
                format!("--extend must be 1s..={MAX_ASK_TIMEOUT_SECS}s"),
            )
            .with_invalid_value(format!("{}s", d.as_secs())))
        }
    };
    if let Some(note) = &args.note {
        if !valid_ask_text(note, MAX_ASK_NOTE_BYTES) {
            return Err(CliError::user(
                "invalid_note",
                format!("--note must be nonblank, at most {MAX_ASK_NOTE_BYTES} bytes, without NUL"),
            ));
        }
    }
    let t = target(&args.run_id, &args.node_id)?;
    let guard = RunLock::acquire_existing(&t.paths.lock()).map_err(from_core)?;
    let witness = guard.witness();
    let node = load(&t)?;
    let now = Utc::now();
    let is_open = node
        .open_ask
        .as_deref()
        .is_some_and(|o| o.ask_id == args.ask_id)
        && !node.status.is_terminal();
    if !is_open {
        return Err(not_open(&t, &node, args.ask_id));
    }
    if time_out_if_expired(&witness, &t.paths, &t.node_id, args.ask_id, now)? {
        let node = load(&t)?;
        return Err(not_open(&t, &node, args.ask_id));
    }
    let mut data = json!({"ask_id": args.ask_id});
    if let Some(s) = extend_secs {
        data["extend_secs"] = json!(s);
    }
    if let Some(n) = &args.note {
        data["note"] = json!(n);
    }
    let seq = append_and_apply_unlocked(
        &witness,
        &t.paths,
        "node.ask_escalated",
        Some(&t.node_id),
        None,
        data,
    )
    .map_err(from_core)?;
    let node = load(&t)?;
    drop(guard);
    let open = node
        .open_ask
        .as_deref()
        .map(|o| OpenAskView::build(t.paths.run_id.as_str(), t.node_id.as_str(), o, Utc::now()))
        .ok_or_else(|| CliError::system("io_error", "escalated question vanished"))?;
    let payload = EscalatePayload {
        run_id: t.paths.run_id.to_string(),
        node_id: t.node_id.to_string(),
        ask_id: args.ask_id,
        event_seq: seq,
        open_ask: open,
    };
    match args.spec.format {
        OutputFormat::Text => {
            println!(
                "escalated ask {} of {}; deadline {}; run wait no longer returns for it",
                payload.ask_id, payload.run_id, payload.open_ask.expires_at
            );
            output::emit_text_warnings(args.warnings);
            Ok(())
        }
        _ => output::emit_envelope(&payload, args.spec, args.warnings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(escalated: bool) -> OpenAsk {
        let at: DateTime<Utc> = "2026-11-21T12:00:00Z".parse().unwrap();
        OpenAsk {
            ask_id: 9,
            attempt: 0,
            question: "Which API?".into(),
            options: vec!["small".into(), "large".into()],
            default: "small".into(),
            idempotency_key: "k".into(),
            asked_at: at,
            expires_at: at + chrono::Duration::seconds(60),
            escalated_at: escalated.then_some(at),
            escalation_note: None,
        }
    }

    #[test]
    fn open_question_settles_wait_until_escalated_or_expired() {
        let o = open(false);
        let v = OpenAskView::build("R", "n-0001", &o, o.asked_at);
        assert_eq!(v.state, "open");
        assert!(v.settles_wait());
        assert_eq!(v.remaining_secs, 60);
        assert!(v.answer_hint.contains("--ask 9 --choice"));
        let v = OpenAskView::build("R", "n-0001", &open(true), o.asked_at);
        assert_eq!(v.state, "escalated");
        assert!(!v.settles_wait());
        let v = OpenAskView::build("R", "n-0002", &o, o.expires_at);
        assert_eq!(v.state, "expired");
        assert_eq!(v.remaining_secs, 0);
        assert!(!v.settles_wait());
        assert!(v.answer_hint.contains("--node n-0002"));
    }
}
