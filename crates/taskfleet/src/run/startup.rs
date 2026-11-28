//! Read-time detection of a live worker that never reached its first turn
//! (`startup_blocked`).
//!
//! A Taskfleet-launched Pi worker can stop at an interactive startup prompt
//! (a trust question, a missing-cwd selector, an authentication prompt) after
//! the PID handshake and before it ever runs a turn. Nothing told the parent:
//! attention otherwise comes only from an agent-emitted request or a worker
//! exit, and `--agent-startup-timeout` covers only the handshake before exec.
//! This module projects a computed hint for that shape so `run show`,
//! `run list`, and a default `run wait` make it visible and name `run attach`
//! as the way to see the prompt.
//!
//! Like [`crate::run::stalled`] it is a **computed** read-time hint, never a
//! persisted status: no event is appended and no projection is written, so
//! state-integrity invariants 1–3 are not in play and replay is unchanged. It
//! never changes the run's status, outcome, retry, merge, or teardown.
//!
//! ## What "reached its first turn" means here
//!
//! The only evidence used is the native Pi transcript Taskfleet itself wrote
//! for the attempt before the worker started (`Node::evidence`). Taskfleet
//! writes exactly one header line into that file; Pi appends its first entry
//! once it has opened the session, which happens only after its interactive
//! startup prompts are behind it. So "the transcript still holds nothing but
//! Taskfleet's header" after the grace is the blocked signature.
//!
//! Worker telemetry is deliberately not an input: its public contract
//! (`contracts/worker-telemetry-v1`) promises that telemetry never affects
//! wait settlement, and a last-told sample cannot say whether a turn ever
//! started. A worker without Taskfleet-written Pi evidence (Claude, a
//! caller-owned Pi, an interactive run) therefore never shows this hint: an
//! absent signal is not evidence of a blocked worker.
//!
//! A resumed resident round reuses a transcript that already has entries, so a
//! prompt during a resumed round reads as "reached" — a missed hint, never a
//! false one. Every unreadable or unexpected file state also reads as reached.
//!
//! ## Waking `run wait` once
//!
//! A default `run wait` settles on this hint at most once per invocation, and
//! never immediately on re-entry: it wakes when the run has been blocked for
//! the grace measured from the later of the attempt's start and the wait's own
//! start. The wait that is running when the grace expires wakes then; a
//! parent that re-waits after telling a person to attach is not woken again
//! for another grace, so a re-wait cannot spin. `--terminal` ignores the hint
//! like every other attention signal.

use std::io::Read;
use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use taskfleet_core::{AgentOwner, EvidenceStatus, Lifecycle, Manifest, Node, RunPaths, Status};

/// Default time a live worker may take to reach its first turn before the run
/// reads as startup-blocked. Pi opens its session within seconds of a clean
/// start; two minutes leaves a slow start well clear while still surfacing a
/// stuck prompt long before a parent's default six-hour wait.
pub const GRACE_SECS: i64 = 120;
/// Operational override for [`GRACE_SECS`]. Whole seconds; invalid or negative
/// values fall back to the default.
pub const GRACE_ENV: &str = "TASKFLEET_STARTUP_BLOCKED_GRACE_SECS";
/// Stable machine reason for the hint (`error` in `run wait`, `reason` in the
/// detail view).
pub const REASON: &str =
    "worker has not reached its first turn; it may be stopped at an interactive startup prompt";

/// Upper bound on the bytes read from the transcript. Taskfleet's header line
/// is a few hundred bytes; a first line longer than this is not the file
/// Taskfleet wrote and reads as reached.
const READ_LIMIT: u64 = 16 * 1024;

/// Effective startup grace.
#[must_use]
pub fn grace() -> SignedDuration {
    let secs = std::env::var(GRACE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(GRACE_SECS);
    SignedDuration::from_secs(secs)
}

/// Whether the attempt's native transcript shows Pi past its startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstTurn {
    /// Pi appended an entry after Taskfleet's header, or the evidence could
    /// not be read as the untouched header (fail closed: no hint).
    Reached,
    /// The transcript still holds only Taskfleet's header line.
    NotReached,
}

/// Inspect the current attempt's native Pi transcript. Returns `None` when the
/// node has no Taskfleet-written evidence for its current attempt, so no
/// first-turn signal exists at all.
#[must_use]
pub fn first_turn(paths: &RunPaths, node: &Node) -> Option<FirstTurn> {
    let evidence = node.evidence.as_ref()?;
    if evidence.attempt != node.retry_attempts || evidence.status != EvidenceStatus::Pending {
        return None;
    }
    let Ok(path) =
        crate::supervise::evidence::checked_live_path(paths, &evidence.live_session_path)
    else {
        return Some(FirstTurn::Reached);
    };
    Some(classify_transcript(&path))
}

fn classify_transcript(path: &Path) -> FirstTurn {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    taskfleet_core::nofollow(&mut options);
    let Ok(file) = options.open(path) else {
        return FirstTurn::Reached;
    };
    let mut bytes = Vec::new();
    if file.take(READ_LIMIT).read_to_end(&mut bytes).is_err() {
        return FirstTurn::Reached;
    }
    classify_bytes(&bytes)
}

/// Pure classification of the transcript's leading bytes: exactly one
/// complete JSON line whose `type` is `session`, followed by nothing but
/// whitespace, is the untouched header.
pub(crate) fn classify_bytes(bytes: &[u8]) -> FirstTurn {
    let Some(newline) = bytes.iter().position(|b| *b == b'\n') else {
        return FirstTurn::Reached;
    };
    let (header, rest) = bytes.split_at(newline);
    let is_header = serde_json::from_slice::<serde_json::Value>(header)
        .is_ok_and(|v| v.get("type").and_then(serde_json::Value::as_str) == Some("session"));
    if is_header && rest.iter().all(u8::is_ascii_whitespace) {
        FirstTurn::NotReached
    } else {
        FirstTurn::Reached
    }
}

/// The instant from which the run counts as startup-blocked, or `None` when it
/// is not (yet) blocked at `now`.
///
/// Every condition must hold: an autonomous, Taskfleet-launched, single-worker
/// run that is not terminal; a node still working whose current attempt has
/// completed the PID handshake (`agent_pid` recorded) and has no told exit; no
/// higher-precedence verdict (`other_attention`: attention-required, a stall,
/// an open request or question); the first-turn signal says not reached; and
/// the grace since the attempt started has strictly elapsed.
#[must_use]
pub fn blocked_since(
    manifest: &Manifest,
    node: &Node,
    first_turn: Option<FirstTurn>,
    other_attention: bool,
    now: Timestamp,
) -> Option<Timestamp> {
    let started_at = node.started_at?;
    let eligible = manifest.lifecycle == Lifecycle::Autonomous
        && manifest.agent_owner == AgentOwner::Taskfleet
        && manifest.node_count == 1
        && !manifest.status.is_terminal()
        && matches!(node.status, Status::Pending | Status::Running)
        && node.agent_pid.is_some()
        && node.worker_exit.is_none()
        && node.awaiting_input.is_none()
        && node.open_ask.is_none()
        && !other_attention
        && first_turn == Some(FirstTurn::NotReached);
    let since = started_at.checked_add(grace()).ok()?;
    (eligible && now > since).then_some(since)
}

/// Whether a default `run wait` begun at `wait_started_at` should settle on a
/// run blocked since `since` (see the module docs): the grace is measured from
/// the later of the attempt start and the wait start, so each wait wakes at
/// most once and never immediately on re-entry.
#[must_use]
pub fn wakes_wait(since: Timestamp, wait_started_at: Timestamp, now: Timestamp) -> bool {
    let attempt_started = since.checked_sub(grace()).unwrap_or(since);
    let anchor = attempt_started.max(wait_started_at);
    now.duration_since(anchor) > grace()
}

/// [`blocked_since`] with the transcript read only when every cheaper
/// condition already holds, so a healthy or finished run costs no file read.
#[must_use]
pub fn detect(
    paths: &RunPaths,
    manifest: &Manifest,
    node: &Node,
    other_attention: bool,
    now: Timestamp,
) -> Option<Timestamp> {
    blocked_since(
        manifest,
        node,
        Some(FirstTurn::NotReached),
        other_attention,
        now,
    )?;
    blocked_since(
        manifest,
        node,
        first_turn(paths, node),
        other_attention,
        now,
    )
}

/// [`detect`] rendered as the read-surface view.
#[must_use]
pub fn view(
    paths: &RunPaths,
    manifest: &Manifest,
    node: &Node,
    other_attention: bool,
    now: Timestamp,
) -> Option<StartupBlockedView> {
    detect(paths, manifest, node, other_attention, now)
        .map(|since| StartupBlockedView::build(manifest.run_id.as_str(), node, since, now))
}

/// Context `run show`, `run list`, and `run wait` attach to a startup-blocked
/// run (`startup_blocked_detail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupBlockedView {
    /// Stable machine reason ([`REASON`]).
    pub reason: &'static str,
    /// When the current worker attempt started (after its PID handshake).
    pub started_at: Timestamp,
    /// Seconds since the attempt started, clamped at 0.
    pub pending_age_secs: i64,
    /// The effective grace in seconds.
    pub grace_secs: i64,
    /// Last-observed worker pid.
    pub worker_pid: Option<i32>,
    /// One-line remedy naming `run attach`.
    pub resume_hint: String,
}

impl StartupBlockedView {
    #[must_use]
    pub fn build(run_id: &str, node: &Node, since: Timestamp, now: Timestamp) -> Self {
        let started_at = node
            .started_at
            .unwrap_or_else(|| since.checked_sub(grace()).unwrap_or(since));
        Self {
            reason: REASON,
            started_at,
            pending_age_secs: now.duration_since(started_at).as_secs().max(0),
            grace_secs: grace().as_secs(),
            worker_pid: node.agent_pid,
            resume_hint: resume_hint(run_id),
        }
    }
}

/// The copyable remedy: look at the prompt with `run attach`, or give up with
/// `run cancel`. The run id is single-quoted for safe pasting.
#[must_use]
pub fn resume_hint(run_id: &str) -> String {
    let q = format!("'{}'", run_id.replace('\'', "'\\''"));
    format!(
        "worker has not started its first turn; see what it is waiting for with \
         `taskfleet run attach {q}`, or `run cancel {q}` to lay it to rest"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn header_only_transcript_is_not_reached() {
        let header = br#"{"type":"session","version":3,"id":"x","cwd":"/w"}"#;
        let mut bytes = header.to_vec();
        bytes.push(b'\n');
        assert_eq!(classify_bytes(&bytes), FirstTurn::NotReached);
    }

    #[test]
    fn any_entry_after_the_header_is_reached() {
        let bytes = b"{\"type\":\"session\"}\n{\"type\":\"model_change\"}\n";
        assert_eq!(classify_bytes(bytes), FirstTurn::Reached);
    }

    #[test]
    fn unexpected_transcripts_fail_closed_to_reached() {
        // Empty, partial, non-header, or non-JSON first lines are not the file
        // Taskfleet wrote, so they never raise the hint.
        for bytes in [
            &b""[..],
            b"{\"type\":\"session\"}",
            b"{\"type\":\"message\"}\n",
            b"not json\n",
        ] {
            assert_eq!(classify_bytes(bytes), FirstTurn::Reached, "{bytes:?}");
        }
    }

    #[test]
    fn wait_wakes_once_from_the_later_anchor() {
        // Tests do not set the process-global override, so grace() is default.
        let started = ts("2026-11-01T12:00:00Z");
        let since = started + SignedDuration::from_secs(GRACE_SECS);
        let after = |s| since + SignedDuration::from_secs(s);
        // A wait running since before the attempt wakes as soon as it is blocked.
        let early_wait = started - SignedDuration::from_secs(5);
        assert!(wakes_wait(since, early_wait, after(1)));
        assert!(!wakes_wait(since, early_wait, since));
        // A re-wait begun after the wake is not woken again until a full grace
        // of its own has passed.
        let rewait = after(10);
        assert!(!wakes_wait(since, rewait, after(11)));
        assert!(!wakes_wait(since, rewait, after(10 + GRACE_SECS)));
        assert!(wakes_wait(since, rewait, after(11 + GRACE_SECS)));
    }

    fn manifest() -> Manifest {
        Manifest {
            schema_version: 1,
            applied_seq: 1,
            run_id: taskfleet_core::RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            kind: taskfleet_core::Kind::Spinoff,
            lifecycle: Lifecycle::Autonomous,
            caller_settlement_intent: None,
            agent_owner: AgentOwner::Taskfleet,
            title: "t".into(),
            status: Status::Pending,
            created_at: ts("2026-11-01T11:59:00Z"),
            updated_at: ts("2026-11-01T12:00:00Z"),
            source_repo: None,
            source_branch: None,
            worktree_root: None,
            managed_tmux_session: None,
            tmux_retention: None,
            notify_cmd: None,
            harness: Some("pi".into()),
            agent_selection: None,
            node_count: 1,
            created_by_run_id: None,
            created_by_session: None,
            parent_run_id: None,
            parent_node_id: None,
            resident: None,
        }
    }

    fn node() -> Node {
        Node {
            schema_version: 1,
            node_id: taskfleet_core::NodeId::parse_str("n-0001").unwrap(),
            run_id: taskfleet_core::RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            parent_node_id: None,
            kind: taskfleet_core::Kind::Spinoff,
            status: Status::Pending,
            task: None,
            worktree_path: None,
            branch: None,
            base_sha: None,
            tmux_window: None,
            tmux_identity: None,
            evidence: None,
            caller_pi_session: None,
            caller_pi_lifecycle: None,
            retained_display: None,
            retention_unavailable: None,
            agent_pid: Some(42),
            agent_pid_start_time: None,
            agent_pid_start_identity: None,
            supervisor_pid: None,
            children: vec![],
            started_at: Some(ts("2026-11-01T12:00:00Z")),
            updated_at: ts("2026-11-01T12:00:00Z"),
            last_report: None,
            last_processed_report_seq_by_child: serde_json::Map::new(),
            retry_attempts: 0,
            worker_exit: None,
            pending_merge: None,
            first_death_at: None,
            awaiting_input: None,
            open_ask: None,
            last_ask: None,
            claude_session_id: None,
            resident: None,
        }
    }

    const BLOCKED: Option<FirstTurn> = Some(FirstTurn::NotReached);

    fn late() -> Timestamp {
        ts("2026-11-01T12:00:00Z") + SignedDuration::from_secs(GRACE_SECS + 1)
    }

    #[test]
    fn blocked_only_after_the_strict_grace() {
        let since = ts("2026-11-01T12:00:00Z") + SignedDuration::from_secs(GRACE_SECS);
        assert_eq!(
            blocked_since(&manifest(), &node(), BLOCKED, false, late()),
            Some(since)
        );
        assert_eq!(
            blocked_since(&manifest(), &node(), BLOCKED, false, since),
            None
        );
    }

    #[test]
    fn no_signal_or_a_reached_turn_is_never_blocked() {
        for signal in [None, Some(FirstTurn::Reached)] {
            assert_eq!(
                blocked_since(&manifest(), &node(), signal, false, late()),
                None
            );
        }
    }

    #[test]
    fn ineligible_runs_and_nodes_are_never_blocked() {
        let m = manifest();
        let n = node();
        let cases: Vec<(Manifest, Node, bool)> = vec![
            (
                Manifest {
                    lifecycle: Lifecycle::Interactive,
                    ..m.clone()
                },
                n.clone(),
                false,
            ),
            (
                Manifest {
                    agent_owner: AgentOwner::Caller,
                    ..m.clone()
                },
                n.clone(),
                false,
            ),
            (
                Manifest {
                    node_count: 2,
                    ..m.clone()
                },
                n.clone(),
                false,
            ),
            (
                Manifest {
                    status: Status::Done,
                    ..m.clone()
                },
                n.clone(),
                false,
            ),
            (
                m.clone(),
                Node {
                    agent_pid: None,
                    ..n.clone()
                },
                false,
            ),
            (
                m.clone(),
                Node {
                    worker_exit: Some(taskfleet_core::WorkerExit {
                        code: Some(1),
                        signal: None,
                        at: late(),
                    }),
                    ..n.clone()
                },
                false,
            ),
            (
                m.clone(),
                Node {
                    status: Status::Failed,
                    ..n.clone()
                },
                false,
            ),
            (m.clone(), n.clone(), true),
        ];
        for (m, n, other) in cases {
            assert_eq!(blocked_since(&m, &n, BLOCKED, other, late()), None);
        }
    }

    #[test]
    fn resume_hint_names_attach_and_quotes_the_id() {
        let hint = resume_hint("01run");
        assert!(hint.contains("taskfleet run attach '01run'"), "{hint}");
        assert!(hint.contains("run cancel '01run'"), "{hint}");
        assert!(resume_hint("a'b").contains("'a'\\''b'"));
    }
}
