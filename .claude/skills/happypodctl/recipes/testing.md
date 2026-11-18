# Recipe: tests

Two layers. Both run with `cargo test`, offline, in under a second.

## Unit tests: next to the code

`#[cfg(test)] mod tests` at the bottom of the module. For parsing and pure
logic: `duration`, `kv_list`, `ports`, `exit_code`, `enc` in `commands.rs`;
table layout in `output.rs`; the error envelope in `api.rs` (with a `Canned`
transport); simulator behaviour in `demo.rs` (a `Client` on a `Sim` in a temp
dir).

## End-to-end tests: `tests/cli.rs`

Run the built binary (`env!("CARGO_BIN_EXE_happypodctl")`) against the
simulator. Each test makes its own `Home`, a temp `HAPPYPOD_HOME` removed on
drop, and `Home::run` strips every `HAPPYPOD_*` variable from your shell.

```rust
#[test]
fn stop_then_start() {
    let h = Home::new();
    assert_eq!(h.ok(&["pod", "stop", "pod_web"])["status"], "stopped");
    assert_eq!(h.ok(&["pod", "start", "pod_web"])["status"], "running");
}

#[test]
fn restart_a_stopped_pod_is_a_conflict() {
    let h = Home::new();
    h.ok(&["pod", "stop", "pod_web"]);
    let (code, err) = h.fail(&["pod", "restart", "pod_web"]);
    assert_eq!((code, err["code"].as_str()), (5, Some("NOT_RUNNING")));
}
```

- `h.ok(args)` adds `--demo`, asserts success and parses stdout as JSON: this
  also proves nothing else was printed.
- `h.fail(args)` adds `--demo`, asserts failure and returns the exit code and the
  stderr JSON.
- `h.run(args)` is raw: use it for live mode (no `--demo`) and table output.

## What to cover for a command

1. The happy path, asserting the fields an agent reads next (ids, `status`, `url`).
2. The most likely failure, asserting the exit code and `code`.
3. For paid calls: the dry run returns a price **and** changes nothing.
4. For waits: success, `DEPLOYMENT_FAILED` with `details.podId` and logs.

## Never

- Call the real API from a test. Tests stay offline and free.
- Depend on test order or on `~/.happypod`.
- Assert on random ids; assert on prefixes (`pod_`, `dep_`) or on seed ids.
