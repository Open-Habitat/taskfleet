//! Folds for a worker's blocking question to its parent (ADR 0004).
//!
//! Four told events, all node-scoped:
//!
//! - `node.asked` opens [`Node::open_ask`] with the question, the optional
//!   options, the worker's default, its idempotency key, and a deadline fixed
//!   once from the event timestamp plus `timeout_secs`;
//! - `node.ask_escalated` marks the open question as passed on to a person and
//!   can move the deadline later, never earlier;
//! - `node.answered` and `node.ask_timed_out` close it into
//!   [`Node::last_ask`].
//!
//! Every closing or escalating event names its question by `ask_id` (the seq of
//! its `node.asked`), and only the open question with that id is affected, so
//! the first closing event wins and a stale answer can never close a newer
//! question. Whether an answer arrived before the deadline is decided by the
//! command that appends it, under the run's exclusive lock; the fold itself is
//! a pure function of the log. Payloads are validated before any
//! state-dependent no-op so a malformed event never enters the durable log.
//!
//! A question also ends when its node does: a terminal status or report, a
//! retry, the told worker exit, a round report, or a resumed round each close
//! it as `cancelled` via [`cancel_open_ask`].

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use serde_json::Value;

use super::{require_envelope_node_id, ProjectionOp};
use crate::error::{Error, Result};
use crate::paths::RunPaths;
use crate::projections::read_node_opt;
use crate::schema::{AskOutcome, ClosedAsk, Event, Node, OpenAsk};

/// Largest accepted question, in bytes.
pub const MAX_ASK_QUESTION_BYTES: usize = 8 * 1024;
/// Largest accepted default, in bytes.
pub const MAX_ASK_DEFAULT_BYTES: usize = 4 * 1024;
/// Largest accepted free-text answer, in bytes.
pub const MAX_ASK_ANSWER_BYTES: usize = 16 * 1024;
/// Largest accepted escalation note, in bytes.
pub const MAX_ASK_NOTE_BYTES: usize = 2 * 1024;
/// Most options one question may carry.
pub const MAX_ASK_OPTIONS: usize = 16;
/// Longest option, in characters.
pub const MAX_ASK_OPTION_CHARS: usize = 256;
/// Longest idempotency key, in characters.
pub const MAX_ASK_KEY_CHARS: usize = 128;
/// Longest deadline a question may set or an escalation may extend by.
pub const MAX_ASK_TIMEOUT_SECS: u64 = 7 * 24 * 3600;

fn corrupt(events_path: &Path, ev: &Event, what: &str) -> Error {
    Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason: format!("event seq={} kind={} {what}", ev.seq, ev.kind),
    }
}

/// True for nonblank text without NUL of at most `max` bytes.
#[must_use]
pub fn valid_ask_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.contains('\0')
}

/// True for a 1..=128 character key of visible ASCII.
#[must_use]
pub fn valid_ask_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_ASK_KEY_CHARS && s.bytes().all(|b| b.is_ascii_graphic())
}

/// True for a nonblank option without control characters, at most
/// [`MAX_ASK_OPTION_CHARS`] long.
#[must_use]
pub fn valid_ask_option(s: &str) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= MAX_ASK_OPTION_CHARS
        && !s.chars().any(char::is_control)
}

fn str_field<'a>(ev: &'a Event, key: &str) -> Option<&'a str> {
    ev.data.get(key).and_then(Value::as_str)
}

fn ask_id(events_path: &Path, ev: &Event) -> Result<u64> {
    ev.data
        .get("ask_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| corrupt(events_path, ev, "requires unsigned `ask_id`"))
}

fn secs(events_path: &Path, ev: &Event, key: &str) -> Result<Option<u64>> {
    match ev.data.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|s| (1..=MAX_ASK_TIMEOUT_SECS).contains(s))
            .map(Some)
            .ok_or_else(|| {
                corrupt(
                    events_path,
                    ev,
                    &format!("requires `{key}` in 1..={MAX_ASK_TIMEOUT_SECS}"),
                )
            }),
    }
}

/// `at + secs`, saturating at the end of the representable range. Validated
/// inputs are bounded by `MAX_ASK_TIMEOUT_SECS`; saturation only keeps a
/// corrupt event value from panicking the reducer.
fn deadline(at: Timestamp, secs: u64) -> Timestamp {
    let secs = i64::try_from(secs).unwrap_or(i64::MAX);
    at.checked_add(SignedDuration::from_secs(secs))
        .unwrap_or(Timestamp::MAX)
}

/// Close the open question, if any, as `cancelled` because its node ended.
/// Called next to every place the reducer ends a node or its worker attempt.
pub(super) fn cancel_open_ask(n: &mut Node, ev: &Event) {
    if let Some(open) = n.open_ask.take() {
        n.last_ask = Some(Box::new(ClosedAsk {
            ask: *open,
            outcome: AskOutcome::Cancelled,
            closed_at: ev.ts,
            closed_seq: ev.seq,
            answer: None,
            choice: None,
            decided_by: None,
            cancel_reason: Some(ev.kind.clone()),
        }));
    }
}

pub(super) fn reduce_node_asked(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let attempt = ev
        .data
        .get("attempt")
        .and_then(Value::as_u64)
        .and_then(|a| u32::try_from(a).ok())
        .ok_or_else(|| corrupt(&events_path, ev, "requires unsigned `attempt`"))?;
    let question = str_field(ev, "question")
        .filter(|q| valid_ask_text(q, MAX_ASK_QUESTION_BYTES))
        .ok_or_else(|| {
            corrupt(
                &events_path,
                ev,
                &format!("requires nonblank `question` of at most {MAX_ASK_QUESTION_BYTES} bytes"),
            )
        })?;
    let default = str_field(ev, "default")
        .filter(|d| valid_ask_text(d, MAX_ASK_DEFAULT_BYTES))
        .ok_or_else(|| {
            corrupt(
                &events_path,
                ev,
                &format!("requires nonblank `default` of at most {MAX_ASK_DEFAULT_BYTES} bytes"),
            )
        })?;
    let key = str_field(ev, "idempotency_key")
        .filter(|k| valid_ask_key(k))
        .ok_or_else(|| {
            corrupt(
                &events_path,
                ev,
                "requires an `idempotency_key` of 1..=128 visible ASCII characters",
            )
        })?;
    let timeout = secs(&events_path, ev, "timeout_secs")?
        .ok_or_else(|| corrupt(&events_path, ev, "requires `timeout_secs`"))?;
    let options: Vec<String> = match ev.data.get("options") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => {
            let opts: Option<Vec<String>> = values
                .iter()
                .map(|v| {
                    v.as_str()
                        .filter(|s| valid_ask_option(s))
                        .map(str::to_string)
                })
                .collect();
            let opts = opts
                .filter(|o| o.len() <= MAX_ASK_OPTIONS)
                .filter(|o| {
                    let mut seen = std::collections::HashSet::new();
                    o.iter().all(|s| seen.insert(s.as_str()))
                })
                .ok_or_else(|| {
                    corrupt(
                        &events_path,
                        ev,
                        &format!(
                            "requires at most {MAX_ASK_OPTIONS} distinct nonblank `options` of at most {MAX_ASK_OPTION_CHARS} characters"
                        ),
                    )
                })?;
            opts
        }
        Some(_) => return Err(corrupt(&events_path, ev, "`options` must be an array")),
    };

    let Some(mut n) = read_node_opt(paths, &node_id)? else {
        return Ok(vec![]);
    };
    // One open question per node, from the live current attempt only. The
    // command refuses these cases before appending; the fold stays first-wins.
    if n.status.is_terminal()
        || n.worker_exit.is_some()
        || n.retry_attempts != attempt
        || n.open_ask.is_some()
    {
        return Ok(vec![]);
    }
    n.open_ask = Some(Box::new(OpenAsk {
        ask_id: ev.seq,
        attempt,
        question: question.to_string(),
        options,
        default: default.to_string(),
        idempotency_key: key.to_string(),
        asked_at: ev.ts,
        expires_at: deadline(ev.ts, timeout),
        escalated_at: None,
        escalation_note: None,
    }));
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

pub(super) fn reduce_node_ask_escalated(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let id = ask_id(&events_path, ev)?;
    let extend = secs(&events_path, ev, "extend_secs")?;
    let note = match ev.data.get("note") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .filter(|s| valid_ask_text(s, MAX_ASK_NOTE_BYTES))
                .ok_or_else(|| {
                    corrupt(
                        &events_path,
                        ev,
                        &format!(
                            "`note` must be nonblank text of at most {MAX_ASK_NOTE_BYTES} bytes"
                        ),
                    )
                })?
                .to_string(),
        ),
    };
    let Some(mut n) = read_node_opt(paths, &node_id)? else {
        return Ok(vec![]);
    };
    let Some(open) = n.open_ask.as_mut().filter(|o| o.ask_id == id) else {
        return Ok(vec![]);
    };
    open.escalated_at.get_or_insert(ev.ts);
    if note.is_some() {
        open.escalation_note = note;
    }
    if let Some(extend) = extend {
        open.expires_at = open.expires_at.max(deadline(ev.ts, extend));
    }
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

pub(super) fn reduce_node_answered(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let id = ask_id(&events_path, ev)?;
    // Absent and wrong-typed are different: a present non-string field is
    // malformed, never "not given".
    let typed = |k: &str| match ev.data.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.as_str())),
        Some(_) => Err(corrupt(
            &events_path,
            ev,
            &format!("`{k}` must be a string"),
        )),
    };
    let answer = typed("answer")?;
    let choice = typed("choice")?;
    let shape_ok = match (answer, choice) {
        (Some(a), None) => valid_ask_text(a, MAX_ASK_ANSWER_BYTES),
        (None, Some(c)) => valid_ask_option(c),
        _ => false,
    };
    if !shape_ok {
        return Err(corrupt(
            &events_path,
            ev,
            &format!(
                "requires exactly one of nonblank `answer` (at most {MAX_ASK_ANSWER_BYTES} bytes) or `choice`"
            ),
        ));
    }
    let decided_by = str_field(ev, "decided_by")
        .filter(|d| matches!(*d, "agent" | "person"))
        .ok_or_else(|| corrupt(&events_path, ev, "requires `decided_by` agent|person"))?;
    let Some(mut n) = read_node_opt(paths, &node_id)? else {
        return Ok(vec![]);
    };
    let Some(open) = n.open_ask.take_if(|o| o.ask_id == id) else {
        return Ok(vec![]);
    };
    if let Some(c) = choice {
        if !open.options.iter().any(|o| o == c) {
            return Err(corrupt(
                &events_path,
                ev,
                &format!("`choice` {c:?} is not an option of ask {id}"),
            ));
        }
    }
    n.last_ask = Some(Box::new(ClosedAsk {
        ask: *open,
        outcome: AskOutcome::Answered,
        closed_at: ev.ts,
        closed_seq: ev.seq,
        answer: answer.map(str::to_string),
        choice: choice.map(str::to_string),
        decided_by: Some(decided_by.to_string()),
        cancel_reason: None,
    }));
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}

pub(super) fn reduce_node_ask_timed_out(paths: &RunPaths, ev: &Event) -> Result<Vec<ProjectionOp>> {
    let events_path = paths.events();
    let node_id = require_envelope_node_id(&events_path, ev)?;
    let id = ask_id(&events_path, ev)?;
    let Some(mut n) = read_node_opt(paths, &node_id)? else {
        return Ok(vec![]);
    };
    let Some(open) = n.open_ask.take_if(|o| o.ask_id == id) else {
        return Ok(vec![]);
    };
    n.last_ask = Some(Box::new(ClosedAsk {
        ask: *open,
        outcome: AskOutcome::TimedOut,
        closed_at: ev.ts,
        closed_seq: ev.seq,
        answer: None,
        choice: None,
        decided_by: None,
        cancel_reason: None,
    }));
    n.updated_at = ev.ts;
    Ok(vec![ProjectionOp::Node(n)])
}
