//! `node` subcommand — read (`list`, `show`) + agent self-report (`report`).
//!
//! Mirrors the noun-module pattern set by `run/`: one file per verb,
//! shared types here, single `dispatch` entry point called from
//! `cli.rs`. The `report` verb is a domain verb (design.md §2.0, §2.2)
//! — it's the agent's structured-report sink and accepts the §7.3
//! payload schema directly rather than going through `event create`.

pub mod ask;
pub mod dto;
pub mod list;
pub mod report;
pub mod round_report;
pub mod show;
pub mod telemetry;

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;

use crate::error::CliError;
use crate::output::OutputSpec;

#[derive(Subcommand, Debug)]
pub enum NodeAction {
    /// Ask the parent agent a blocking question and wait for its answer
    /// (ADR 0004). Works from any harness that can run a shell command.
    ///
    /// The first call records the question and fixes its deadline
    /// (`--timeout`); `--wait` bounds this one invocation. Re-run with the same
    /// `--idempotency-key` to re-attach; the deadline never moves. Exit 0
    /// `answered` (act on `data.take`), 4 `pending` (re-run with the same key),
    /// 2 `timed_out` (continue on your default, repeated in `data.take`, and
    /// name that in your final report), 3 `cancelled` (stop; do not merge),
    /// 1 usage error or a second question while one is open.
    /// Example: taskfleet node ask --current --question "Drop the v1 API?"
    ///   --option keep --option drop --default keep --idempotency-key api-v1 --json
    Ask {
        /// Exact run id (or pass --current).
        #[arg(required_unless_present = "current", requires = "node_id")]
        run_id: Option<String>,
        /// Node id, e.g. n-0001 (or pass --current).
        node_id: Option<String>,
        /// Resolve the run from the current worktree, as `run show --current`
        /// does; the node defaults to n-0001.
        #[arg(long, conflicts_with = "run_id")]
        current: bool,
        /// The question (at most 8 KiB).
        #[arg(
            long,
            required_unless_present = "question_file",
            conflicts_with = "question_file"
        )]
        question: Option<String>,
        /// UTF-8 file holding the question.
        #[arg(long)]
        question_file: Option<PathBuf>,
        /// What you will do if nobody answers. Must be something your brief
        /// already allows, which may be "stop and report the decision".
        #[arg(long)]
        default: String,
        /// A named choice the parent may pick with `run answer --choice`.
        /// Repeat for several (at most 16).
        #[arg(long = "option")]
        options: Vec<String>,
        /// Deadline, set once when the question is first asked (e.g. 30m, 2h).
        #[arg(long, value_parser = crate::run::wait::parse_duration, default_value = "30m")]
        timeout: Duration,
        /// How long this invocation blocks before returning `pending` (exit 4).
        /// Keep it under your harness's command time limit.
        #[arg(long, value_parser = crate::run::wait::parse_duration, default_value = "8m")]
        wait: Duration,
        /// Stable key for this question; re-asking with it re-attaches.
        #[arg(long)]
        idempotency_key: String,
    },
    /// List nodes belonging to a run.
    List {
        run_id: String,
        /// Filter by status (e.g. `running`, `done`).
        #[arg(long)]
        status: Option<String>,
    },
    /// Print one node's JSON projection.
    Show { run_id: String, node_id: String },
    /// Update one bounded, advisory last-told worker telemetry sample.
    Telemetry {
        #[command(subcommand)]
        action: TelemetryAction,
    },
    /// Agent self-submission of a structured terminal report (§7.3).
    Report {
        run_id: String,
        node_id: String,
        /// JSON file containing the §7.3 report payload.
        #[arg(long)]
        from_file: PathBuf,
        /// Dedup token — a repeat call with the same key returns the
        /// existing event's `seq` instead of appending again.
        #[arg(long)]
        idempotency_key: Option<String>,
        /// Validate the payload and exit 0 without writing anything
        /// to the run's events.jsonl or projection files.
        #[arg(long)]
        dry_run: bool,
    },
    /// End a resident worker's round (ADR 0003): record the round summary as a
    /// told `node.round_reported`, then stop. The exit that follows is the end
    /// of the round, `run wait` settles at once, and the next `run message`
    /// resumes the same session in the same worktree. Resident runs only.
    /// Example: taskfleet node round-report <run-id> --summary-file /tmp/round.md
    RoundReport {
        /// Exact run id from the worker's run context.
        run_id: String,
        /// Reporting node id.
        #[arg(long, default_value = "n-0001")]
        node: String,
        /// UTF-8 file with the round summary (at most 16 KiB): what was done,
        /// what is recommended, what is needed next.
        #[arg(long, conflicts_with = "summary")]
        summary_file: Option<PathBuf>,
        /// Inline round summary (alternative to --summary-file).
        #[arg(long)]
        summary: Option<String>,
        /// Validate and report the round number without appending anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum TelemetryAction {
    /// Replace the current attempt's advisory telemetry sample.
    Update {
        /// Exact run id. Required unless --input-file is used.
        #[arg(long, required_unless_present = "input_file")]
        run_id: Option<String>,
        /// Exact node id. Required unless --input-file is used.
        #[arg(long, required_unless_present = "input_file")]
        node_id: Option<String>,
        /// Absolute current attempt. Required unless --input-file is used.
        #[arg(long, required_unless_present = "input_file")]
        attempt: Option<u32>,
        /// Last-told activity. Required unless --input-file is used.
        #[arg(long, value_enum, required_unless_present = "input_file")]
        state: Option<telemetry::StateArg>,
        /// Bounded active tool count (1-32), only for `tool_running`.
        #[arg(long, conflicts_with = "input_file")]
        active_tool_count: Option<u8>,
        /// Sanitized tool name, only when active-tool-count is 1.
        #[arg(long, conflicts_with = "input_file")]
        tool_name: Option<String>,
        /// Strict v1 JSON request path, or '-' for stdin. Maximum raw size: 4096 bytes.
        #[arg(
            long,
            value_name = "PATH|-",
            conflicts_with_all = ["run_id", "node_id", "attempt", "state", "active_tool_count", "tool_name"]
        )]
        input_file: Option<PathBuf>,
    },
}

pub fn dispatch(
    action: NodeAction,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match action {
        NodeAction::Ask {
            run_id,
            node_id,
            current,
            question,
            question_file,
            default,
            options,
            timeout,
            wait,
            idempotency_key,
        } => ask::run(ask::Args {
            current,
            run_id,
            node_id,
            question,
            question_file,
            default,
            options,
            timeout,
            wait,
            idempotency_key,
            spec,
            warnings,
        }),
        NodeAction::List { run_id, status } => list::run(list::Args {
            run_id,
            status,
            spec,
            warnings,
        }),
        NodeAction::Show { run_id, node_id } => show::run(&run_id, &node_id, spec, warnings),
        NodeAction::Telemetry { action } => match action {
            TelemetryAction::Update {
                run_id,
                node_id,
                attempt,
                state,
                active_tool_count,
                tool_name,
                input_file,
            } => telemetry::update(telemetry::Args {
                run_id,
                node_id,
                attempt,
                state,
                active_tool_count,
                tool_name,
                input_file,
                spec,
                warnings,
            }),
        },
        NodeAction::Report {
            run_id,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
        } => report::run(report::Args {
            run_id,
            node_id,
            from_file,
            idempotency_key,
            dry_run,
            spec,
            warnings,
        }),
        NodeAction::RoundReport {
            run_id,
            node,
            summary_file,
            summary,
            dry_run,
        } => round_report::run(round_report::Args {
            run_id,
            node_id: node,
            summary,
            summary_file,
            dry_run,
            spec,
            warnings,
        }),
    }
}
