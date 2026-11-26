# Result

The tmux server's identity is recorded as the kernel start identity
(`tmux_server_pid_start_identity` on the spawn/retry event,
`server_pid_start_identity` on `TmuxIdentity` and `RetainedDisplay`) and
compared through `watchdog::RecordedStart`. Seconds decide only for records
without it (legacy runs, with `RecordedStart`'s 1 s tolerance instead of the
previous exact match). A start that cannot be read while the server PID is
alive is `Unsafe` (retention retries, maintenance preserves) rather than
`Gone`. The completed-display count bound groups by the identity, so drifting
seconds no longer split one server's displays. Persistent-retention spawns now
also require the identity to be readable.

No state-schema bump: losing the optional field to an older writer only falls
back to the seconds comparison.

## Verified

- Full local release gate passed on eec62d41 (1349 tests); the closure stamp then rewrote only its message (d3bd08f8, identical tree).
- Unit test `server_generation_decides_from_kernel_identity_and_never_reads_unreadable_as_gone`.
- Real private tmux expiry test now records the identity with seconds stale by
  5 s and still expires the owned display.
- `run attach` integration: identity mismatch with matching seconds is
  stale; on Linux a matching identity with seconds stale by 5 s validates.
- Count-bound unit test with one display's seconds drifted keeps one group.

## Not verified

- An actual NTP step on a live host; drift was simulated by recording stale seconds.
- macOS (identity path is the existing `pbi_start` reader; not run here).
