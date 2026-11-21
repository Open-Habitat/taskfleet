//! `node ask` — a worker's blocking question to its parent (ADR 0004).
//!
//! The worker states the question, the default it will take when nobody
//! answers, and an idempotency key. The first call appends `node.asked` under
//! the run's exclusive lock; the deadline is fixed then, from that event's
//! timestamp plus `--timeout`, and nothing the worker does later moves it. The
//! command then polls the node projection for at most `--wait` (harnesses cap
//! how long one shell command may run) and exits with the outcome:
//!
//! | exit | outcome     | meaning                                                   |
//! |------|-------------|-----------------------------------------------------------|
//! | 0    | `answered`  | the parent's answer or chosen option, and who decided     |
//! | 4    | `pending`   | `--wait` elapsed; re-invoke with the same key             |
//! | 2    | `timed_out` | the deadline passed; take the default, which is repeated  |
//! | 3    | `cancelled` | the run was cancelled or the node/attempt ended           |
//! | 1    | (error)     | usage error, unknown run, or a refused second question    |
//!
//! Exit 2 is also the generic system-error code; a real outcome always carries
//! its success envelope on stdout, an error carries an error envelope on
//! stderr. Re-invoking with the same key re-attaches to the same question
//! (found by its `node.asked` envelope key, so even a question answered while
//! the command was killed reports its answer). A different question while one
//! is open is refused and names the open one.
//!
//! The command that first observes, under the exclusive lock, an open question
//! past its deadline appends `node.ask_timed_out`, so a racing `run answer`
//! either lands first or is refused; exactly one closing event exists.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};

use taskfleet_core::reducer::{
    valid_ask_key, valid_ask_option, valid_ask_text, MAX_ASK_DEFAULT_BYTES, MAX_ASK_OPTIONS,
    MAX_ASK_QUESTION_BYTES, MAX_ASK_TIMEOUT_SECS,
};
use taskfleet_core::{
    append_and_apply_unlocked, find_prior_with_key, read_all_events, read_manifest_opt,
    read_node_opt, AskOutcome, Manifest, Node, NodeId, RunLock, RunPaths,
};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_node_id, run_paths_from_cli_arg};

/// Longest single `--wait`.
const MAX_WAIT: Duration = Duration::from_secs(24 * 3600);
const POLL_START: Duration = Duration::from_millis(100);
const POLL_CAP: Duration = Duration::from_secs(1);

pub struct Args<'a> {
    pub current: bool,
    pub run_id: Option<String>,
    pub node_id: Option<String>,
    pub question: Option<String>,
    pub question_file: Option<PathBuf>,
    pub default: String,
    pub options: Vec<String>,
    pub timeout: Duration,
    pub wait: Duration,
    pub idempotency_key: String,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct Payload {
    run_id: String,
    node_id: String,
    /// Absent only for `cancelled` on a node that had already ended before
    /// anything was asked.
    ask_id: Option<u64>,
    /// `answered | pending | timed_out | cancelled`.
    outcome: &'static str,
    /// True when this call re-attached to a question asked earlier with the
    /// same key rather than asking it now.
    attached: bool,
    question: String,
    options: Vec<String>,
    default: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<chrono::DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    choice: Option<String>,
    /// `agent` or `person`, as the answering parent stated it. Unverified.
    #[serde(skip_serializing_if = "Option::is_none")]
    decided_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cancel_reason: Option<String>,
    /// What to act on: the answer, the chosen option, or (timed out) the
    /// default. Absent for `pending` and `cancelled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    take: Option<String>,
    /// What the worker does next.
    next: String,
}

fn read_question(text: Option<String>, file: Option<&Path>) -> Result<String, CliError> {
    let q = match (text, file) {
        (Some(t), None) => t,
        (None, Some(path)) => {
            let f = std::fs::File::open(path).map_err(|e| {
                CliError::user(
                    "invalid_question_file",
                    format!("open {}: {e}", path.display()),
                )
                .with_invalid_value(path.display().to_string())
            })?;
            let mut bytes = Vec::new();
            f.take(MAX_ASK_QUESTION_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| {
                    CliError::user(
                        "invalid_question_file",
                        format!("read {}: {e}", path.display()),
                    )
                })?;
            String::from_utf8(bytes).map_err(|e| {
                CliError::user(
                    "invalid_question_file",
                    format!("{} is not UTF-8: {e}", path.display()),
                )
            })?
        }
        _ => {
            return Err(CliError::user(
                "invalid_question",
                "provide --question <text> or --question-file <file>",
            ))
        }
    };
    if !valid_ask_text(&q, MAX_ASK_QUESTION_BYTES) {
        return Err(CliError::user(
            "invalid_question",
            format!(
                "question must be nonblank UTF-8 of at most {MAX_ASK_QUESTION_BYTES} bytes without NUL"
            ),
        ));
    }
    Ok(q)
}

fn validate(args: &Args<'_>) -> Result<(), CliError> {
    if !valid_ask_text(&args.default, MAX_ASK_DEFAULT_BYTES) {
        return Err(CliError::user(
            "invalid_default",
            format!(
                "--default must be nonblank, at most {MAX_ASK_DEFAULT_BYTES} bytes, without NUL: it is what you do if nobody answers"
            ),
        ));
    }
    if args.options.len() > MAX_ASK_OPTIONS {
        return Err(CliError::user(
            "invalid_option",
            format!("at most {MAX_ASK_OPTIONS} --option values"),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for o in &args.options {
        if !valid_ask_option(o) || !seen.insert(o.as_str()) {
            return Err(CliError::user(
                "invalid_option",
                "each --option must be distinct, nonblank, at most 256 characters, without control characters",
            )
            .with_invalid_value(o.clone()));
        }
    }
    if !valid_ask_key(&args.idempotency_key) {
        return Err(CliError::user(
            "invalid_idempotency_key",
            "--idempotency-key must be 1..=128 visible ASCII characters",
        )
        .with_invalid_value(args.idempotency_key.clone()));
    }
    let t = args.timeout.as_secs();
    if args.timeout.subsec_nanos() != 0 || !(1..=MAX_ASK_TIMEOUT_SECS).contains(&t) {
        return Err(CliError::user(
            "invalid_timeout",
            format!("--timeout must be whole seconds in 1s..={MAX_ASK_TIMEOUT_SECS}s"),
        ));
    }
    if args.wait > MAX_WAIT {
        return Err(CliError::user("invalid_wait", "--wait must be at most 24h"));
    }
    Ok(())
}

fn resolve(args: &Args<'_>) -> Result<(RunPaths, NodeId), CliError> {
    let root = crate::home::root_dir()?;
    let (run_id, node) = if args.current {
        let cwd = std::env::current_dir()
            .map_err(|e| CliError::system("io_error", format!("read current directory: {e}")))?;
        let id = crate::run::ownership::resolve_current(&root, &cwd)?.to_string();
        (id, args.node_id.clone().unwrap_or_else(|| "n-0001".into()))
    } else {
        let run = args.run_id.clone().ok_or_else(|| {
            CliError::user("invalid_target", "pass --current or <RUN_ID> <NODE_ID>")
        })?;
        let node = args.node_id.clone().ok_or_else(|| {
            CliError::user("invalid_target", "pass --current or <RUN_ID> <NODE_ID>")
        })?;
        (run, node)
    };
    let node_id = parse_node_id(&node)?;
    let paths = run_paths_from_cli_arg(&root, &run_id)?;
    Ok((paths, node_id))
}

fn load(paths: &RunPaths, node_id: &NodeId) -> Result<(Manifest, Node), CliError> {
    let manifest = read_manifest_opt(paths)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user("run_not_found", format!("run {} not found", paths.run_id))
        })?;
    let node = read_node_opt(paths, node_id)
        .map_err(from_core)?
        .ok_or_else(|| {
            CliError::user("node_not_found", format!("node {node_id} not found"))
                .with_invalid_value(node_id.as_str())
        })?;
    Ok((manifest, node))
}

/// The envelope key: node-scoped, so two nodes may reuse a worker key.
fn envelope_key(node_id: &NodeId, key: &str) -> String {
    format!("ask:{node_id}:{key}")
}

/// The question being waited on, fixed after the ask/attach step.
struct Asked {
    ask_id: u64,
    question: String,
    options: Vec<String>,
    default: String,
    attached: bool,
}

enum Start {
    Asked(Asked),
    /// The node had ended before anything was asked.
    Ended(String),
}

fn ask_or_attach(
    args: &Args<'_>,
    question: &str,
    paths: &RunPaths,
    node_id: &NodeId,
) -> Result<Start, CliError> {
    let guard = RunLock::acquire_existing(&paths.lock()).map_err(from_core)?;
    let witness = guard.witness();
    let (manifest, node) = load(paths, node_id)?;
    let key = envelope_key(node_id, &args.idempotency_key);
    if let Some(prior) =
        find_prior_with_key(&witness, paths, "node.asked", &key).map_err(from_core)?
    {
        let pq = prior.data.get("question").and_then(Value::as_str);
        if prior.node_id.as_deref() != Some(node_id.as_str()) || pq != Some(question) {
            return Err(CliError::user(
                "idempotency_conflict",
                format!(
                    "idempotency key {:?} already asked a different question (ask {}); use a new key for a new question",
                    args.idempotency_key, prior.seq
                ),
            )
            .with_invalid_value(args.idempotency_key.clone())
            .with_details(json!({"ask_id": prior.seq, "question": pq})));
        }
        let strings = |k: &str| -> Vec<String> {
            prior
                .data
                .get(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        return Ok(Start::Asked(Asked {
            ask_id: prior.seq,
            question: question.to_string(),
            options: strings("options"),
            default: prior
                .data
                .get("default")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            attached: true,
        }));
    }
    if manifest.status.is_terminal() || node.status.is_terminal() {
        return Ok(Start::Ended(format!(
            "node {node_id} of run {} has ended",
            paths.run_id
        )));
    }
    if node.worker_exit.is_some() {
        return Ok(Start::Ended(format!(
            "the current worker attempt of node {node_id} has already exited"
        )));
    }
    if let Some(open) = node.open_ask.as_deref() {
        return Err(CliError::user(
            "ask_already_open",
            format!(
                "node {node_id} already has open question {} (key {:?}); wait for it with that key, or ask again after it closes",
                open.ask_id, open.idempotency_key
            ),
        )
        .with_details(json!({
            "ask_id": open.ask_id,
            "idempotency_key": open.idempotency_key,
            "question": open.question,
            "expires_at": open.expires_at,
        })));
    }
    let seq = append_and_apply_unlocked(
        &witness,
        paths,
        "node.asked",
        Some(node_id),
        Some(&key),
        json!({
            "attempt": node.retry_attempts,
            "question": question,
            "options": args.options,
            "default": args.default,
            "idempotency_key": args.idempotency_key,
            "timeout_secs": args.timeout.as_secs(),
        }),
    )
    .map_err(from_core)?;
    drop(guard);
    Ok(Start::Asked(Asked {
        ask_id: seq,
        question: question.to_string(),
        options: args.options.clone(),
        default: args.default.clone(),
        attached: false,
    }))
}

/// One observation of where the question stands.
enum Seen {
    Open {
        expired: bool,
        expires_at: chrono::DateTime<Utc>,
    },
    Closed(Closure),
}

struct Closure {
    outcome: AskOutcome,
    answer: Option<String>,
    choice: Option<String>,
    decided_by: Option<String>,
    cancel_reason: Option<String>,
}

fn observe(paths: &RunPaths, node_id: &NodeId, ask_id: u64) -> Result<Seen, CliError> {
    let (manifest, node) = RunLock::with_shared_lock(&paths.lock(), || {
        Ok((read_manifest_opt(paths)?, read_node_opt(paths, node_id)?))
    })
    .map_err(from_core)?;
    let manifest = manifest
        .ok_or_else(|| CliError::system("io_error", "run manifest disappeared while waiting"))?;
    let node =
        node.ok_or_else(|| CliError::system("io_error", "node disappeared while waiting"))?;
    if let Some(open) = node.open_ask.as_deref().filter(|o| o.ask_id == ask_id) {
        if manifest.status.is_terminal() || node.status.is_terminal() {
            return Ok(Seen::Closed(Closure {
                outcome: AskOutcome::Cancelled,
                answer: None,
                choice: None,
                decided_by: None,
                cancel_reason: Some(format!("run {}", crate::run::status_kebab(manifest.status))),
            }));
        }
        return Ok(Seen::Open {
            expired: Utc::now() >= open.expires_at,
            expires_at: open.expires_at,
        });
    }
    if let Some(last) = node.last_ask.as_deref().filter(|l| l.ask.ask_id == ask_id) {
        return Ok(Seen::Closed(Closure {
            outcome: last.outcome,
            answer: last.answer.clone(),
            choice: last.choice.clone(),
            decided_by: last.decided_by.clone(),
            cancel_reason: last.cancel_reason.clone(),
        }));
    }
    // A newer question has closed since: find how this one ended in the log.
    Ok(Seen::Closed(closure_from_log(paths, node_id, ask_id)?))
}

fn closure_from_log(paths: &RunPaths, node_id: &NodeId, ask_id: u64) -> Result<Closure, CliError> {
    let events = RunLock::with_shared_lock(&paths.lock(), || read_all_events(&paths.events()))
        .map_err(from_core)?;
    for ev in events.iter().filter(|e| {
        e.node_id.as_ref() == Some(node_id)
            && e.data.get("ask_id").and_then(Value::as_u64) == Some(ask_id)
    }) {
        let s = |k: &str| ev.data.get(k).and_then(Value::as_str).map(str::to_string);
        match ev.kind.as_str() {
            "node.answered" => {
                return Ok(Closure {
                    outcome: AskOutcome::Answered,
                    answer: s("answer"),
                    choice: s("choice"),
                    decided_by: s("decided_by"),
                    cancel_reason: None,
                })
            }
            "node.ask_timed_out" => {
                return Ok(Closure {
                    outcome: AskOutcome::TimedOut,
                    answer: None,
                    choice: None,
                    decided_by: None,
                    cancel_reason: None,
                })
            }
            _ => {}
        }
    }
    Ok(Closure {
        outcome: AskOutcome::Cancelled,
        answer: None,
        choice: None,
        decided_by: None,
        cancel_reason: Some("node or attempt ended".into()),
    })
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    validate(&args)?;
    let question = read_question(args.question.clone(), args.question_file.as_deref())?;
    let (paths, node_id) = resolve(&args)?;
    let asked = match ask_or_attach(&args, &question, &paths, &node_id)? {
        Start::Asked(a) => a,
        Start::Ended(reason) => {
            let payload = Payload {
                run_id: paths.run_id.to_string(),
                node_id: node_id.to_string(),
                ask_id: None,
                outcome: "cancelled",
                attached: false,
                question,
                options: args.options.clone(),
                default: args.default.clone(),
                expires_at: None,
                answer: None,
                choice: None,
                decided_by: None,
                cancel_reason: Some(reason),
                take: None,
                next: NEXT_CANCELLED.into(),
            };
            return finish(&payload, 3, args.spec, args.warnings);
        }
    };
    let start = Instant::now();
    let mut backoff = POLL_START;
    let mut expires_at = None;
    let closure = loop {
        match observe(&paths, &node_id, asked.ask_id)? {
            Seen::Closed(c) => break c,
            Seen::Open { expired: true, .. } => {
                let guard = RunLock::acquire_existing(&paths.lock()).map_err(from_core)?;
                crate::run::ask::time_out_if_expired(
                    &guard.witness(),
                    &paths,
                    &node_id,
                    asked.ask_id,
                    Utc::now(),
                )?;
                drop(guard);
            }
            Seen::Open {
                expired: false,
                expires_at: at,
            } => {
                expires_at = Some(at);
                let elapsed = start.elapsed();
                if elapsed >= args.wait {
                    let payload = Payload {
                        run_id: paths.run_id.to_string(),
                        node_id: node_id.to_string(),
                        ask_id: Some(asked.ask_id),
                        outcome: "pending",
                        attached: asked.attached,
                        question: asked.question,
                        options: asked.options,
                        default: asked.default,
                        expires_at,
                        answer: None,
                        choice: None,
                        decided_by: None,
                        cancel_reason: None,
                        take: None,
                        next: format!(
                            "still open: re-run the same `node ask` with --idempotency-key {} to keep waiting (the deadline does not move)",
                            args.idempotency_key
                        ),
                    };
                    return finish(&payload, 4, args.spec, args.warnings);
                }
                let until_deadline = (at - Utc::now()).to_std().unwrap_or_default();
                let nap = backoff
                    .min(args.wait - elapsed)
                    .min(until_deadline)
                    .max(Duration::from_millis(10));
                std::thread::sleep(nap);
                backoff = (backoff * 2).min(POLL_CAP);
            }
        }
    };
    let (code, outcome, take, next) = match closure.outcome {
        AskOutcome::Answered => (
            0,
            "answered",
            closure.choice.clone().or_else(|| closure.answer.clone()),
            "continue on the answer; name in your final report that your parent decided this".to_string(),
        ),
        AskOutcome::TimedOut => (
            2,
            "timed_out",
            Some(asked.default.clone()),
            "nobody answered before the deadline: continue on your default and name this decision-by-default in your final report".to_string(),
        ),
        AskOutcome::Cancelled => (3, "cancelled", None, NEXT_CANCELLED.to_string()),
    };
    let payload = Payload {
        run_id: paths.run_id.to_string(),
        node_id: node_id.to_string(),
        ask_id: Some(asked.ask_id),
        outcome,
        attached: asked.attached,
        question: asked.question,
        options: asked.options,
        default: asked.default,
        expires_at,
        answer: closure.answer,
        choice: closure.choice,
        decided_by: closure.decided_by,
        cancel_reason: closure.cancel_reason,
        take,
        next,
    };
    finish(&payload, code, args.spec, args.warnings)
}

const NEXT_CANCELLED: &str =
    "the run was cancelled or this node ended: stop, preserve your work, do not merge";

fn finish(p: &Payload, code: i32, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Text => {
            let id = p.ask_id.map_or_else(|| "-".to_string(), |i| i.to_string());
            println!("ask {id}: {}", p.outcome);
            if let Some(take) = &p.take {
                println!("take: {take}");
            }
            if let Some(d) = &p.decided_by {
                println!("decided_by: {d}");
            }
            println!("next: {}", p.next);
            output::emit_text_warnings(warnings);
        }
        _ => output::emit_envelope(p, spec, warnings)?,
    }
    if code == 0 {
        return Ok(());
    }
    crate::cli::flush_logs();
    std::process::exit(code);
}
