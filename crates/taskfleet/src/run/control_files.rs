//! Per-run control files a native Pi launch keeps directly under
//! `<state root>/.creating`, outside the staging tree that publication
//! renames, so that they survive publication:
//!
//! - `message-<run>.ts`: the bundled message adapter Pi loads at start;
//! - `message-<run>-<attempt>.sock`: that attempt's message socket;
//! - `round-<run>-<attempt>.reported`: a resident's stop marker (ADR 0003).
//!
//! They exist only for a live attempt's input channel. Once a run is terminal
//! nothing reads them, so terminal teardown removes every one of them by name,
//! and `run upgrade-lock` attributes each to its run instead of treating it as
//! unknown creation state. The name is parsed exactly (a canonical run id and
//! a decimal attempt), so one run's sweep can never match another run's file.

use std::path::Path;

use taskfleet_core::RunId;

/// The run a `.creating` control file belongs to, or `None` when `name` is
/// not exactly one of the shapes above.
pub(crate) fn run_of(name: &str) -> Option<&str> {
    let id = if let Some(rest) = name.strip_prefix("message-") {
        match rest.strip_suffix(".ts") {
            Some(id) => id,
            None => attempt_suffixed(rest.strip_suffix(".sock")?)?,
        }
    } else {
        attempt_suffixed(name.strip_prefix("round-")?.strip_suffix(".reported")?)?
    };
    RunId::parse_str(id).ok().map(|_| id)
}

/// `<id>-<attempt>` → `<id>`, requiring a non-empty decimal attempt.
fn attempt_suffixed(stem: &str) -> Option<&str> {
    let (id, attempt) = stem.rsplit_once('-')?;
    (!attempt.is_empty() && attempt.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// Remove every control file of `run_id` under `<state_root>/.creating`.
///
/// Only for a run whose status is terminal: a live attempt still needs its
/// adapter, socket and round marker. Best-effort, because these are tiny
/// leftovers and must never veto teardown; a directory or anything else that
/// is not a plain file or socket is left alone. Returns the names removed.
pub(crate) fn remove_for_terminal_run(state_root: &Path, run_id: &str) -> Vec<String> {
    let dir = state_root.join(".creating");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if run_of(&name) != Some(run_id) {
            continue;
        }
        // `file_type` does not follow symlinks; removing a symlink removes
        // only the link.
        if entry.file_type().is_ok_and(|t| !t.is_dir())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed.push(name);
        }
    }
    removed.sort();
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01m5sx7xmqpmmp5dv2dm4hes7t";
    const OTHER: &str = "01m5sx7xmqpmmp5dv2dm4hes7v";

    #[test]
    fn names_are_parsed_exactly() {
        for name in [
            format!("message-{ID}.ts"),
            format!("message-{ID}-12.sock"),
            format!("round-{ID}-0.reported"),
        ] {
            assert_eq!(run_of(&name), Some(ID), "{name}");
        }
        for name in [
            format!("message-{ID}-.sock"),
            format!("message-{ID}.tsx"),
            format!("message-{ID}-1.ts"),
            format!("round-{ID}.reported"),
            format!("round-{ID}-x.reported"),
            format!("round-{ID}-1.reported.tmp"),
            "message-not-a-run.ts".to_string(),
            "round-not-a-run-1.reported".to_string(),
            "orphan".to_string(),
        ] {
            assert_eq!(run_of(&name), None, "{name}");
        }
    }

    #[test]
    fn sweep_removes_only_this_runs_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join(".creating");
        std::fs::create_dir_all(dir.join("pi-sessions").join(ID)).unwrap();
        let mine = [
            format!("message-{ID}.ts"),
            format!("message-{ID}-0.sock"),
            format!("message-{ID}-3.sock"),
            format!("round-{ID}-0.reported"),
            format!("round-{ID}-3.reported"),
        ];
        let theirs = [
            format!("message-{OTHER}.ts"),
            format!("round-{OTHER}-0.reported"),
            format!("message-{ID}.ts.bak"),
        ];
        for name in mine.iter().chain(&theirs) {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        // A directory with a control-file name is not ours to remove.
        std::fs::create_dir(dir.join(format!("round-{ID}-9.reported"))).unwrap();
        let mut expected = mine.to_vec();
        expected.sort();
        assert_eq!(remove_for_terminal_run(tmp.path(), ID), expected);
        for name in &mine {
            assert!(!dir.join(name).exists(), "{name}");
        }
        for name in &theirs {
            assert!(dir.join(name).exists(), "{name}");
        }
        assert!(dir.join(format!("round-{ID}-9.reported")).is_dir());
        assert!(dir.join("pi-sessions").join(ID).is_dir());
        assert_eq!(
            remove_for_terminal_run(&tmp.path().join("absent"), ID),
            Vec::<String>::new()
        );
    }
}
