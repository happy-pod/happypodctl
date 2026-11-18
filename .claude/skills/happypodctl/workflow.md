# From a request to working code

The order of operations. `doctrine.md` says what good looks like; this says how
to get there without wasted turns.

---

## Every change has the same shape

```
contract  →  demo  →  command  →  test  →  verify
```

1. **Contract.** Find the route in `api-index.md`. Not there? It is not in the
   API. Stop and say so: the route goes into happy-pod/mcp `docs/API.md` and
   `src/demo.ts` first.
2. **Demo.** Make `src/demo.rs` answer it exactly as `demo.ts` does.
3. **Command.** Flags in `cli.rs`, behaviour in `commands.rs`.
4. **Test.** A unit test in the module for logic, an end-to-end test in
   `tests/cli.rs` for the command.
5. **Verify.** The three gates below, then run the command by hand in demo mode.

## Decomposing a request

| The user says | You need |
|---|---|
| "add a command for X" and X is in `api-index.md` | `recipes/command.md` |
| "support the new /v1/... route" | `recipes/endpoint.md` |
| "make it wait for X" | the `Ctx::wait` pattern in `commands.rs`; read doctrine §3 |
| "show it as a table" | a `Columns` const in `commands.rs`; JSON stays the default |
| "make X configurable" | `config.rs`: flag > env > file. A secret never gets a flag |
| "it breaks in CI / for agents" | check stdout has only the result, and the exit code |

## Verify

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Then by hand, with a throwaway home so your real config is untouched:

```sh
export HAPPYPOD_HOME=$(mktemp -d)
cargo run -q -- --demo <your command>
cargo run -q -- --demo -o table <your command>
cargo run -q -- --demo <your command> 2>&1 >/dev/null; echo "exit $?"   # the error path
```

Against the real API: only with the owner's go-ahead and key, read-only
commands first, `--dry-run` before anything paid. Never create a paid resource to
test a change.

## Keep in step

| You changed | Also update |
|---|---|
| a command or flag | `README.md` command table, `api-index.md` |
| an exit code or CLI error code | `cli.rs` `AFTER` text, `SKILL.md`, `README.md` |
| the demo seed or a failure trigger | `api-index.md` "Demo seed"; tell the docs (docs.happypod.io shows demo output) |
| a dependency | `doctrine.md` §5 list |
