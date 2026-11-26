//! Pi project trust for Taskfleet-launched Pi workers.
//!
//! Pi asks for project trust when its cwd carries trust-requiring project
//! resources (`.pi/settings.json`, `.pi/skills`, `.agents/skills` in the cwd
//! or an ancestor, …). A worker's worktree is a fresh path that Pi's trust
//! store (`<agent dir>/trust.json`) does not name unless an ancestor does, so
//! a plain `pi` stops at an interactive prompt nobody answers in an unattended
//! run.
//!
//! The launcher therefore passes Pi's own one-run override, `--approve` or
//! `--no-approve`, carrying the decision the person already made for the run's
//! source checkout: Pi's nearest trust-store entry for that path, read with
//! Pi's semantics (`ProjectTrustStore.get`). Nothing is written to the store,
//! so removed worktrees leave no entries behind, and the worker is never
//! trusted beyond what the person gave the same repository. A source with no
//! decision, an unreadable or invalid store, or an unknown source resolves to
//! `--no-approve` for an autonomous worker (nobody can answer the prompt) and
//! to no flag for an interactive one, whose person answers Pi's prompt
//! themselves. A trust flag already in the profile argv is the operator's
//! explicit choice and stands.

use std::path::{Component, Path, PathBuf};

/// Pi's trust-override words (Pi's `cli/args.js`).
const TRUST_FLAGS: &[&str] = &["--approve", "-a", "--no-approve", "-na"];

/// The largest trust store this reader parses; Pi's store is a small map.
const MAX_STORE_BYTES: u64 = 1 << 20;

/// What the launcher appends to a Pi worker's argv, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiTrust {
    /// The source checkout is trusted: `--approve`.
    Approve,
    /// The source checkout is not trusted, or the decision cannot be made for
    /// an unattended worker: `--no-approve`.
    NoApprove(NoApproveReason),
    /// No flag: the profile argv already chooses, or an interactive worker's
    /// person answers Pi's own prompt.
    Unchanged,
}

/// Why a worker gets `--no-approve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoApproveReason {
    /// The person saved "do not trust" for the source or an ancestor.
    Distrusted,
    /// No entry decides the source; an autonomous worker cannot be asked.
    Undecided,
    /// The trust store or the source could not be judged.
    Unjudgeable,
}

impl PiTrust {
    /// The argv word to append, if any.
    #[must_use]
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Self::Approve => Some("--approve"),
            Self::NoApprove(_) => Some("--no-approve"),
            Self::Unchanged => None,
        }
    }

    /// A short, path-free description for the generated launcher's comment.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Approve => "approve: the source checkout is trusted in Pi's trust store",
            Self::NoApprove(NoApproveReason::Distrusted) => {
                "no-approve: the source checkout is not trusted in Pi's trust store"
            }
            Self::NoApprove(NoApproveReason::Undecided) => {
                "no-approve: no trust decision for the source checkout; unattended worker"
            }
            Self::NoApprove(NoApproveReason::Unjudgeable) => {
                "no-approve: Pi's trust store or the source checkout cannot be judged"
            }
            Self::Unchanged => "unchanged",
        }
    }
}

/// Resolve the trust flag for a Pi worker from the process environment.
///
/// `command` is the selected candidate argv (its first word is the program),
/// `source` the run's recorded source checkout, `autonomous` whether nobody
/// is expected to answer the worker's terminal.
#[must_use]
pub fn resolve(command: &[String], source: Option<&Path>, autonomous: bool) -> PiTrust {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let agent_dir = std::env::var_os("PI_CODING_AGENT_DIR").map(PathBuf::from);
    resolve_with(
        command,
        source,
        autonomous,
        home.as_deref(),
        agent_dir.as_deref(),
    )
}

/// [`resolve`] with the environment passed explicitly (for tests).
#[must_use]
pub fn resolve_with(
    command: &[String],
    source: Option<&Path>,
    autonomous: bool,
    home: Option<&Path>,
    agent_dir_env: Option<&Path>,
) -> PiTrust {
    if command
        .iter()
        .skip(1)
        .any(|arg| TRUST_FLAGS.contains(&arg.as_str()))
    {
        return PiTrust::Unchanged;
    }
    let decision = source.ok_or(()).and_then(|source| {
        let store = trust_store_path(home, agent_dir_env).ok_or(())?;
        nearest_decision(&store, source)
    });
    match decision {
        Ok(Some(true)) => PiTrust::Approve,
        Ok(Some(false)) => PiTrust::NoApprove(NoApproveReason::Distrusted),
        Ok(None) if autonomous => PiTrust::NoApprove(NoApproveReason::Undecided),
        Err(()) if autonomous => PiTrust::NoApprove(NoApproveReason::Unjudgeable),
        Ok(None) | Err(()) => PiTrust::Unchanged,
    }
}

/// Pi's trust store: `PI_CODING_AGENT_DIR` (with Pi's `~`, `~/` and
/// `file://` forms) or `~/.pi/agent`, plus `trust.json`. Pi resolves a
/// relative agent directory against its own cwd, the worker's worktree, which
/// is not known here, so a relative one is not judged.
fn trust_store_path(home: Option<&Path>, agent_dir_env: Option<&Path>) -> Option<PathBuf> {
    let home = home.filter(|h| h.is_absolute());
    let dir = match agent_dir_env.filter(|d| !d.as_os_str().is_empty()) {
        None => home?.join(".pi").join("agent"),
        Some(dir) => {
            let text = dir.to_str()?;
            if text == "~" {
                home?.to_path_buf()
            } else if let Some(rest) = text.strip_prefix("~/") {
                home?.join(rest)
            } else if let Some(rest) = text.strip_prefix("file://") {
                // Only the local, unescaped form; anything else is not judged.
                if rest.starts_with('/') && !rest.contains('%') {
                    PathBuf::from(rest)
                } else {
                    return None;
                }
            } else {
                dir.to_path_buf()
            }
        }
    };
    dir.is_absolute().then(|| dir.join("trust.json"))
}

/// Pi's nearest trust entry for `source`: `Ok(Some(decision))`, `Ok(None)`
/// when no entry decides it (including a missing store), and `Err` when the
/// store or the path cannot be judged. Pi compares canonical paths against
/// its keys resolved as absolute paths; a `null` value means "no decision" and
/// the walk continues to the parent.
fn nearest_decision(store: &Path, source: &Path) -> Result<Option<bool>, ()> {
    let canonical = std::fs::canonicalize(source).map_err(|_| ())?;
    let bytes = match std::fs::metadata(store) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
        Ok(meta) if !meta.is_file() || meta.len() > MAX_STORE_BYTES => return Err(()),
        Ok(_) => std::fs::read(store).map_err(|_| ())?,
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| ())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let serde_json::Value::Object(map) = serde_json::from_str(text).map_err(|_| ())? else {
        return Err(());
    };
    let mut entries = Vec::with_capacity(map.len());
    for (key, value) in &map {
        let decision = match value {
            serde_json::Value::Bool(b) => Some(*b),
            serde_json::Value::Null => None,
            _ => return Err(()),
        };
        // Pi resolves a relative key against its cwd; such a key cannot be
        // compared here and could only name a path relative to the worktree.
        if let (Some(decision), Some(key)) = (decision, lexical_absolute(Path::new(key))) {
            entries.push((key, decision));
        }
    }
    let mut dir = Some(canonical.as_path());
    while let Some(current) = dir {
        if let Some((_, decision)) = entries.iter().find(|(key, _)| key == current) {
            return Ok(Some(*decision));
        }
        dir = current.parent();
    }
    Ok(None)
}

/// Node's `path.resolve` for an absolute path: drop `.` and empty components,
/// apply `..` lexically. `None` for a relative path.
fn lexical_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pi() -> Vec<String> {
        vec!["pi".into()]
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
    }

    fn fixture(store: Option<&str>) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap().join("home");
        let repo = home.join("src").join("project");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
        if let Some(store) = store {
            let body = store.replace("$HOME", home.to_str().unwrap());
            std::fs::write(home.join(".pi/agent/trust.json"), body).unwrap();
        }
        Fixture {
            _tmp: tmp,
            home,
            repo,
        }
    }

    fn run(f: &Fixture, command: &[String], autonomous: bool) -> PiTrust {
        resolve_with(command, Some(&f.repo), autonomous, Some(&f.home), None)
    }

    #[test]
    fn trusted_source_approves() {
        let f = fixture(Some(r#"{"$HOME/src/project": true}"#));
        assert_eq!(run(&f, &pi(), true), PiTrust::Approve);
        assert_eq!(run(&f, &pi(), false), PiTrust::Approve);
    }

    #[test]
    fn nearest_ancestor_decides() {
        let f = fixture(Some(r#"{"$HOME/src": true, "$HOME/src/project/": false}"#));
        assert_eq!(
            run(&f, &pi(), true),
            PiTrust::NoApprove(NoApproveReason::Distrusted)
        );
        let f = fixture(Some(r#"{"$HOME/src": true, "$HOME/src/project": null}"#));
        assert_eq!(run(&f, &pi(), true), PiTrust::Approve);
        let f = fixture(Some(r#"{"$HOME": false, "$HOME/src/./x/..": true}"#));
        assert_eq!(run(&f, &pi(), true), PiTrust::Approve);
    }

    #[test]
    fn a_decision_below_the_source_does_not_count() {
        let f = fixture(Some(r#"{"$HOME/src/project/sub": true}"#));
        assert_eq!(
            run(&f, &pi(), true),
            PiTrust::NoApprove(NoApproveReason::Undecided)
        );
    }

    #[test]
    fn undecided_source_is_no_approve_only_when_autonomous() {
        for store in [None, Some("{}"), Some("\u{feff}{\"relative/key\": true}")] {
            let f = fixture(store);
            assert_eq!(
                run(&f, &pi(), true),
                PiTrust::NoApprove(NoApproveReason::Undecided),
                "{store:?}"
            );
            assert_eq!(run(&f, &pi(), false), PiTrust::Unchanged, "{store:?}");
        }
    }

    #[test]
    fn unjudgeable_store_never_approves() {
        for store in ["not json", "[]", r#"{"/x": "yes"}"#] {
            let f = fixture(Some(store));
            assert_eq!(
                run(&f, &pi(), true),
                PiTrust::NoApprove(NoApproveReason::Unjudgeable),
                "{store}"
            );
            assert_eq!(run(&f, &pi(), false), PiTrust::Unchanged, "{store}");
        }
        let f = fixture(Some(r#"{"$HOME/src": true}"#));
        assert_eq!(
            resolve_with(&pi(), None, true, Some(&f.home), None),
            PiTrust::NoApprove(NoApproveReason::Unjudgeable)
        );
        assert_eq!(
            resolve_with(&pi(), Some(&f.repo), true, None, None),
            PiTrust::NoApprove(NoApproveReason::Unjudgeable)
        );
        assert_eq!(
            resolve_with(
                &pi(),
                Some(&f.repo),
                true,
                Some(&f.home),
                Some(Path::new("rel"))
            ),
            PiTrust::NoApprove(NoApproveReason::Unjudgeable)
        );
    }

    #[test]
    fn agent_dir_env_forms() {
        let f = fixture(None);
        let alt = f.home.join("alt");
        std::fs::create_dir_all(&alt).unwrap();
        std::fs::write(
            alt.join("trust.json"),
            format!(r#"{{"{}": true}}"#, f.repo.display()),
        )
        .unwrap();
        for env in [
            alt.display().to_string(),
            "~/alt".to_string(),
            format!("file://{}", alt.display()),
        ] {
            assert_eq!(
                resolve_with(
                    &pi(),
                    Some(&f.repo),
                    true,
                    Some(&f.home),
                    Some(Path::new(&env))
                ),
                PiTrust::Approve,
                "{env}"
            );
        }
    }

    #[test]
    fn profile_trust_flag_stands() {
        let f = fixture(Some(r#"{"$HOME/src/project": true}"#));
        for flag in TRUST_FLAGS {
            let command = vec!["pi".to_string(), (*flag).to_string()];
            assert_eq!(run(&f, &command, true), PiTrust::Unchanged, "{flag}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_source_is_judged_by_its_canonical_path() {
        let f = fixture(Some(r#"{"$HOME/src/project": true}"#));
        let link = f.home.join("link");
        std::os::unix::fs::symlink(&f.repo, &link).unwrap();
        assert_eq!(
            resolve_with(&pi(), Some(&link), true, Some(&f.home), None),
            PiTrust::Approve
        );
    }
}
