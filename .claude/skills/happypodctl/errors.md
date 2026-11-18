# When the compiler, clippy or a test is angry

Symptom → cause. Look here before changing code.

---

## The stop rule

If three attempts at the same error have not moved it, the cause is not what you
think. Re-read the full error including the `help:` line, then check the entry
below that matches.

---

## 1. clippy `result_large_err`: "the Err-variant is at least 128 bytes"

`ApiError` is returned everywhere, so its size matters. `details` is
`Option<Box<Value>>` for this reason. Keep new fields small; box anything large.
Do not silence the lint.

## 2. "lifetime may not live long enough" around a query closure

```rust
let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
```

Closure lifetime elision ties the returned `&str` to `k`, not `q`. Use the named
helper `qget(q, k)` in `demo.rs`, which has the explicit `'a`.

## 3. A clap flag "cannot be used with" another, or is ignored

`conflicts_with` and `requires` take the **field** name (`gpu_type`), not the
flag (`--gpu-type`). A typo there is not a compile error; it is a panic in
debug builds when the command is parsed. `cargo test` runs the commands, so it
shows up as a failed end-to-end test.

## 4. End-to-end test: stdout "is not json"

Something printed to stdout besides the result: a `println!` for progress, or
table output in a JSON path. Progress goes to `eprintln!`. Check `ctx.format`
before printing extra lines (see `pod diagnose`).

## 5. End-to-end test passes alone, fails with others

Shared state. Every test must use its own `Home` (temp `HAPPYPOD_HOME`), and
must not rely on `HAPPYPOD_*` from your shell: `Home::run` removes them. A test
that reads `~/.happypod` is a bug.

## 6. Demo: `NO_ROUTE` "is not part of the Happy Pod API"

The command calls a path `demo.rs` does not match. Routes are matched on a
shape where every odd segment that is not in `FIXED` becomes `:id`. A new
fixed word (`/pods/:id/something`) must be added to `FIXED`, or it is read as an
id and the route never matches.

## 7. Demo: a state change does not stick between commands

`Sim::send` saves after every non-GET request and after `GET /deployments/:id`.
A GET that changes state must be added to that condition, or the change is lost
when the process exits.

## 8. Demo: "demo state ... is unreadable"

The state file is from an older shape. `happypodctl demo reset`. When you change
a state struct, make new fields `#[serde(default)]` so old files still load.

## 9. `--wait` hangs until the timeout in a test

The deployment never reaches `success`/`failed`: each `GET /deployments/:id`
must advance it (doctrine §4). Check the route still calls `self.advance`.

## 10. Exit code is 1 where you expected 2

Clap errors exit 2 on their own. A usage problem found in `commands.rs` must be
returned with `usage(...)` (code `USAGE`), not `ApiError::new(422, ...)`.
