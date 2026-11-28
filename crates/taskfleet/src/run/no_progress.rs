//! Read-time detection of a live worker that has stopped making observable
//! progress after its first turn (`no_progress`).
//!
//! A live autonomous Pi worker can sit behind a spinner for days: no event is
//! appended after `supervisor.started`, its supervisor and process are alive,
//! and nothing that settles `run wait` ever fires. Attention otherwise comes
//! only from an agent-emitted request, a worker exit, a dead supervisor, or a
//! worker that never started its first turn ([`crate::run::startup`]). This
//! module projects a computed hint for the remaining shape so `run show`,
//! `run list`, and a default `run wait` say since when no progress has been
//! seen instead of reporting a healthy pending run.
//!
//! Like [`crate::run::startup`] and [`crate::run::stalled`] it is a
//! **computed** read-time hint, never a persisted status: no event is appended
//! and no projection is written, so state-integrity invariants 1–3 are not in
//! play and replay is unchanged. It never kills the worker and never changes
//! the run's status, outcome, retry, merge, or teardown. It only informs.
//!
//! ## The signal
//!
//! The only evidence is the modification time of the native Pi transcript
//! Taskfleet itself wrote for the current attempt (`Node::evidence`). Pi
//! appends an entry for every message, tool call, and tool result, so a
//! transcript that has not changed for the interval means nothing observable
//! happened in that time. The hint applies only once the transcript holds
//! more than Taskfleet's header line (the worker passed its first turn);
//! before that, [`crate::run::startup`] owns the run.
//!
//! Claude workers never show the hint: Taskfleet records the Claude session
//! id but not the path of Claude's transcript, and guessing Claude's storage
//! layout would make an internal of another tool a contract here. A worker
//! without Taskfleet-written Pi evidence (Claude, a caller-owned Pi, an
//! interactive run) therefore has no signal, and an absent signal is not
//! evidence of a silent worker. Every unreadable or unexpected file state
//! also reads as "no signal". Worker telemetry is not an input, for the same
//! reason as in [`crate::run::startup`].
//!
//! ## Long tool calls
//!
//! Pi writes a tool call's result only when the call ends, so a multi-hour
//! build is indistinguishable from a hang by the transcript alone. A live
//! child process of the worker is not used to suppress the hint: the
//! recorded pid may be a wrapper rather than Pi, Pi may keep long-lived
//! helper children, observing children is platform-specific, and a hung tool
//! command (one shape of the silent worker this hint exists for) has a live
//! child too. The interval is therefore long by default ([`INTERVAL_SECS`])
//! and overridable ([`INTERVAL_ENV`]); a false hint on a long build costs one
//! look from the parent, never the work.
//!
//! ## Waking `run wait` once
//!
//! Exactly as [`crate::run::startup::wakes_wait`]: a default `run wait`
//! settles on this hint when the transcript has been silent for the interval
//! measured from the later of the last transcript write and the wait's own
//! start. The wait running when the interval expires wakes then; a re-wait is
//! not woken again until another full interval of silence, so it cannot
//! spin, and any new transcript write starts a new idle period. `--terminal`
//! ignores the hint like every other attention signal.

use std::io::Read;
use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use serde::Serialize;

use taskfleet_core::{AgentOwner, EvidenceStatus, Lifecycle, Manifest, Node, RunPaths, Status};

use crate::run::startup::FirstTurn;

/// Default time a live worker's transcript may stay unchanged before the run
/// reads as making no progress. Two hours stays clear of an ordinary long
/// build or test run while still surfacing a hung worker well inside a
/// parent's default six-hour wait.
pub const INTERVAL_SECS: i64 = 2 * 60 * 60;
/// Operational override for [`INTERVAL_SECS`]. Whole seconds; invalid or
/// negative values fall back to the default.
pub const INTERVAL_ENV: &str = "TASKFLEET_NO_PROGRESS_GRACE_SECS";
/// Stable machine reason for the hint (`error` in `run wait`, `reason` in the
/// detail view).
pub const REASON: &str =
    "live worker has written nothing to its transcript for the no-progress interval; it may be hung";

/// Upper bound on the bytes read to tell a header-only transcript from one
/// past its first turn (same bound as [`crate::run::startup`]).
const READ_LIMIT: u64 = 16 * 1024;

/// Effective no-progress interval.
#[must_use]
pub fn interval() -> SignedDuration {
    let secs = std::env::var(INTERVAL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(INTERVAL_SECS);
    SignedDuration::from_secs(secs)
}

/// When the current attempt's native Pi transcript was last written, provided
/// it shows the worker past its first turn. `None` when there is no
/// Taskfleet-written evidence for the current attempt, the file cannot be read
/// as expected, or it still holds only Taskfleet's header.
#[must_use]
pub fn last_progress(paths: &RunPaths, node: &Node) -> Option<Timestamp> {
    let evidence = node.evidence.as_ref()?;
    if evidence.attempt != node.retry_attempts || evidence.status != EvidenceStatus::Pending {
        return None;
    }
    let path =
        crate::supervise::evidence::checked_live_path(paths, &evidence.live_session_path).ok()?;
    read_transcript(&path)
}

fn read_transcript(path: &Path) -> Option<Timestamp> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    taskfleet_core::nofollow(&mut options);
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = Timestamp::try_from(metadata.modified().ok()?).ok()?;
    let mut bytes = Vec::new();
    file.take(READ_LIMIT).read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() || crate::run::startup::classify_bytes(&bytes) == FirstTurn::NotReached {
        return None;
    }
    Some(modified)
}

/// The instant from which the run counts as making no progress, or `None`
/// when it does not (yet) at `now`.
///
/// Every condition must hold: an autonomous, Taskfleet-launched,
/// single-worker run that is not terminal; a node still working whose current
/// attempt has completed the PID handshake and has no told exit; no open
/// request, question, or resident round; no higher-precedence verdict
/// (`other_attention`: attention-required, a stall, a supervisor not presumed
/// working, startup-blocked); a transcript last written at `last_progress`;
/// and the interval since then has strictly elapsed.
#[must_use]
pub fn silent_since(
    manifest: &Manifest,
    node: &Node,
    last_progress: Option<Timestamp>,
    other_attention: bool,
    now: Timestamp,
) -> Option<Timestamp> {
    let eligible = manifest.lifecycle == Lifecycle::Autonomous
        && manifest.agent_owner == AgentOwner::Taskfleet
        && manifest.node_count == 1
        && !manifest.status.is_terminal()
        && matches!(node.status, Status::Pending | Status::Running)
        && node.started_at.is_some()
        && node.agent_pid.is_some()
        && node.worker_exit.is_none()
        && node.awaiting_input.is_none()
        && node.open_ask.is_none()
        && node
            .resident
            .as_deref()
            .and_then(taskfleet_core::ResidentState::open_round)
            .is_none()
        && !other_attention;
    if !eligible {
        return None;
    }
    let since = last_progress?.checked_add(interval()).ok()?;
    (now > since).then_some(since)
}

/// Whether a default `run wait` begun at `wait_started_at` should settle on a
/// run silent since `since`: the interval is measured from the later of the
/// last transcript write and the wait start, so each wait wakes at most once
/// and never immediately on re-entry.
#[must_use]
pub fn wakes_wait(since: Timestamp, wait_started_at: Timestamp, now: Timestamp) -> bool {
    let last = since.checked_sub(interval()).unwrap_or(since);
    now.duration_since(last.max(wait_started_at)) > interval()
}

/// [`silent_since`] with the transcript read only when every cheaper
/// condition already holds, so a healthy or finished run costs no file read.
#[must_use]
pub fn detect(
    paths: &RunPaths,
    manifest: &Manifest,
    node: &Node,
    other_attention: bool,
    now: Timestamp,
) -> Option<Timestamp> {
    silent_since(manifest, node, Some(Timestamp::MIN), other_attention, now)?;
    silent_since(
        manifest,
        node,
        last_progress(paths, node),
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
) -> Option<NoProgressView> {
    detect(paths, manifest, node, other_attention, now)
        .map(|since| NoProgressView::build(manifest, node, since, now))
}

/// Context `run show`, `run list`, and `run wait` attach to a run whose live
/// worker has made no observable progress (`no_progress_detail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoProgressView {
    /// Stable machine reason ([`REASON`]).
    pub reason: &'static str,
    /// When the worker last wrote to its native transcript: the last progress
    /// seen.
    pub last_progress_at: Timestamp,
    /// Seconds since `last_progress_at`, clamped at 0.
    pub silent_secs: i64,
    /// The effective interval in seconds.
    pub interval_secs: i64,
    /// Last-observed worker pid.
    pub worker_pid: Option<i32>,
    /// One-line remedy: look, steer, or cancel.
    pub resume_hint: String,
}

impl NoProgressView {
    #[must_use]
    pub fn build(manifest: &Manifest, node: &Node, since: Timestamp, now: Timestamp) -> Self {
        let last_progress_at = since.checked_sub(interval()).unwrap_or(since);
        Self {
            reason: REASON,
            last_progress_at,
            silent_secs: now.duration_since(last_progress_at).as_secs().max(0),
            interval_secs: interval().as_secs(),
            worker_pid: node.agent_pid,
            resume_hint: resume_hint(manifest.run_id.as_str(), manifest.resident.is_some()),
        }
    }
}

/// The copyable remedy: look with `run attach`, steer with `run message`
/// (a resident can also be stopped at a round boundary with
/// `run interrupt`), or give up with `run cancel`. The run id is
/// single-quoted for safe pasting.
#[must_use]
pub fn resume_hint(run_id: &str, resident: bool) -> String {
    let q = format!("'{}'", run_id.replace('\'', "'\\''"));
    let interrupt = if resident {
        format!(", `run interrupt {q}` to end the round")
    } else {
        String::new()
    };
    format!(
        "worker is alive but has shown no progress; look with `taskfleet run attach {q}`, \
         steer it with `run message {q} <text>`{interrupt}, or `run cancel {q}` if it is hung \
         (a long tool call also looks like this)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn wait_wakes_once_from_the_later_anchor() {
        // Tests do not set the process-global override, so interval() is default.
        let last = ts("2026-11-01T12:00:00Z");
        let since = last + SignedDuration::from_secs(INTERVAL_SECS);
        let after = |s| since + SignedDuration::from_secs(s);
        let early_wait = last - SignedDuration::from_secs(5);
        assert!(wakes_wait(since, early_wait, after(1)));
        assert!(!wakes_wait(since, early_wait, since));
        let rewait = after(10);
        assert!(!wakes_wait(since, rewait, after(11)));
        assert!(!wakes_wait(since, rewait, after(10 + INTERVAL_SECS)));
        assert!(wakes_wait(since, rewait, after(11 + INTERVAL_SECS)));
    }

    #[test]
    fn header_only_or_empty_transcript_has_no_signal() {
        let dir = crate::run::private_fixture::private_home();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, b"{\"type\":\"session\"}\n").unwrap();
        assert_eq!(read_transcript(&path), None);
        std::fs::write(&path, b"").unwrap();
        assert_eq!(read_transcript(&path), None);
        std::fs::write(&path, b"{\"type\":\"session\"}\n{\"type\":\"message\"}\n").unwrap();
        assert!(read_transcript(&path).is_some());
        assert_eq!(read_transcript(&dir.path().join("missing")), None);
    }

    #[test]
    fn resume_hint_names_the_remedies_and_quotes_the_id() {
        let hint = resume_hint("01run", false);
        assert!(hint.contains("taskfleet run attach '01run'"), "{hint}");
        assert!(hint.contains("run message '01run'"), "{hint}");
        assert!(hint.contains("run cancel '01run'"), "{hint}");
        assert!(!hint.contains("run interrupt"), "{hint}");
        assert!(resume_hint("01run", true).contains("run interrupt '01run'"));
        assert!(resume_hint("a'b", false).contains("'a'\\''b'"));
    }
}
