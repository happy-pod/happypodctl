---
name: happypodctl
description: Use when writing, reviewing, or debugging happypodctl, the Happy Pod CLI in Rust: anything involving its clap commands, the API client, the --demo simulator, JSON output and exit codes, --wait, or its tests. Carries the codebase doctrine, the full API route index with the command and demo behaviour for each, and recipes, so you can change the CLI without breaking what agents rely on.
---

# Working on happypodctl

`happypodctl` is a thin client. Each command is one or two calls to the Happy
Pod API v1, printed as JSON. The value is in what it does **not** do: no hidden
retries that buy a second pod, no secrets on screen, no output an agent has to
scrape.

## The three rules that prevent most mistakes

1. **The contract is the MCP server's.** happy-pod/mcp `docs/API.md` is the
   source of truth, and its `src/demo.ts` is the reference behaviour. This repo
   follows it; it never leads it. See `api-index.md`.
2. **Every route works in demo mode.** `src/demo.rs` implements the whole
   contract. A route without demo support cannot be tested here and cannot be
   tried by a user without an account.
3. **Output is for machines first.** JSON on stdout unless `-o table`. Errors
   are one JSON object on stderr with a stable `code` and a documented exit
   code. Text is lowercase and short.

## How a command runs

```
main.rs   Cli::parse()  →  config::resolve (flag > env > ~/.happypod/config.json)
          → make_client: demo::Sim or api::Http behind the Transport trait
commands.rs  run(ctx, command)  →  ctx.get / ctx.client.send  →  output::print
          error  →  ApiError  →  stderr JSON  →  exit_code()
```

## Which file does what

```
src/
  main.rs       entry: parse, pick the transport, print the error, exit
  cli.rs        every command, flag and help string (clap derive)
  commands.rs   what each command does; project resolution; --wait; doctor; exit codes
  api.rs        Client, ApiError, Request/Response, the Transport trait, Http transport
  demo.rs       the simulator: the whole v1 contract, state in ~/.happypod/demo.json
  config.rs     settings and their source; the config file (0600, holds the key)
  output.rs     json and table printing
tests/cli.rs    the built binary against the simulator, one temp HAPPYPOD_HOME per test
```

## Read next: pick by what you are doing

| Situation | File |
|---|---|
| **Adding or changing a command** | `recipes/command.md` |
| **The API gained a route** | `recipes/endpoint.md`: client, demo, command and test in one change |
| **Writing tests** | `recipes/testing.md` |
| **The compiler or clippy is angry** | `errors.md` |
| Anything else, before writing code | `doctrine.md` |
| Which route a command calls, and what the demo returns | `api-index.md` |

## Exit codes (stable: agents branch on them)

| Code | Meaning | From |
|---|---|---|
| 0 | ok | |
| 1 | api or runtime error | 422, 5xx, network, config |
| 2 | usage | clap errors, `USAGE` |
| 3 | auth | 401, 403, `NO_API_KEY` |
| 4 | not found | 404 |
| 5 | conflict | 409 |
| 6 | deployment failed or wait timed out | `DEPLOYMENT_FAILED`, `WAIT_TIMEOUT` |

Changing a code is a breaking change. Adding one needs `cli.rs` help text, this
table and `README.md` updated together.

## Settings

| Setting | Flag | Environment | File key |
|---|---|---|---|
| api key | (none: never on the command line) | `HAPPYPOD_API_KEY` | `apiKey` |
| api url | `--api-url` | `HAPPYPOD_API_URL` | `apiUrl` |
| project | `--project` / `-p` | `HAPPYPOD_PROJECT` | `project` |
| demo | `--demo` | `HAPPYPOD_MODE=demo` | |
| home dir | | `HAPPYPOD_HOME` (default `~/.happypod`) | |

`doctor` is the only command that writes the key; `project use` is the only one
that writes the default project.

## Before you call it done

- `cargo fmt --all`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- New route: in `api-index.md`, in `demo.rs`, covered by `tests/cli.rs`.
- No secret printed unmasked unless the user passed `--reveal`; no
  `std::process::exit` outside `main.rs`; no `unwrap()` on API data.
