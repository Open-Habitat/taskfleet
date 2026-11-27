# Result

The release gate no longer depends on the caller's umask. `tempfile`,
`create_dir_all` and `fs::write` honour the umask, so fixtures that
production later checks for privacy came out group-writable under 0002.
Running the release nextest suite under umask 0002 found 26 such tests, not
just the reported one: the `linux_lifetime` unit test, the `caller_create`
integration tests (state root, workspace, Pi session store and files),
`tests/linux_lifetime.rs` (runtime dir), and the native Pi history fixtures in
`run::caller_discard` and `run::merge`. Each fixture now sets 0700/0600
explicitly via `tests/common` helpers (`private_tempdir`,
`create_private_dir_all`, `write_private_file`) or the `cfg(test)`
`run::private_fixture` module. No production code changed.

## Verified

- `scripts/validate-local-release.sh` passed on commit 525952c8 under
  `umask 0002` and again under `umask 077` (Linux arm64).

## Not verified

- macOS: not run there. The changes only set modes on test fixtures, so the
  outcome should be the same.
- Umasks looser than 0002 (for example 0000) were not run through the full
  gate.
