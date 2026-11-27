---
created: 2026-11-26
updated: 2026-11-27
type: bug
reporter: agent
status: fixed
priority: normal
provenance: agent
source_ref: stint:2026-11-26:release-0.21.1-umask
lane: release
closed: 2026-11-27
closed_by: agent:taskfleet-worker
---

# Release gate test depends on caller umask (linux_lifetime runtime validation)

## Description

`linux_lifetime::tests::runtime_validation_rejects_wrong_owner_symlink_missing_and_open_permissions` asserts `validate_dir(tempdir, uid).is_ok()`, but `tempfile::tempdir()` creates the directory with mode `0o777 & !umask`. Under a common umask 0002 the directory is 0775 and the first assertion fails (`linux_lifetime.rs:303`); under umask 077 it passes. Seen on 2026-11-26: the 0.21.1 release cut's local gate failed from an interactive ssh shell (umask 0002) after the worker's gate had passed in a umask-077 environment; resumed with `umask 077` and published.

Expected: the test sets 0700 on its tempdir before the positive assertion, so the release gate does not depend on the caller's umask. Consider whether other tests make the same assumption.
