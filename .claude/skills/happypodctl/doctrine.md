# The happypodctl doctrine

What good code looks like in this repo. Read it before writing any.

---

## 1. The agent is the main user

An agent runs `happypodctl ...`, reads stdout, and decides what to do next from
the exit code and the error `code`. So:

- **stdout is data.** JSON by default (`serde_json::to_string_pretty`), the API's
  own field names (camelCase), nothing else on stdout. `-o table` is opt-in.
- **stderr is one JSON object** on failure: `{"error", "code", "status", "details"?}`.
  Progress or prompts go to stderr too, never stdout.
- **codes are stable.** The API's `UPPER_SNAKE` codes pass through unchanged.
  CLI-side codes are few and fixed: `USAGE`, `NO_API_KEY`, `NETWORK_ERROR`,
  `BAD_RESPONSE`, `CONFIG`, `DEMO_STATE`, `DEPLOYMENT_FAILED`, `WAIT_TIMEOUT`,
  `DOCTOR`, `HTTP_<status>` (a non-JSON error body).
- **text is lowercase and short.** Help strings, CLI messages and hints. API
  messages pass through as the API wrote them.
- **say what to do next.** A good error names the fix: "no project given. pass
  --project <id>, set HAPPYPOD_PROJECT, or run `happypodctl project use <id>`",
  and puts the choices in `details` (here, the project list).

## 2. Money and data: the safety rules

These mirror the MCP server's tool annotations. Do not weaken them.

| Rule | How it shows up |
|---|---|
| Price before paying | every paid create and `pod scale` takes `--dry-run`; it sends `?dryRun=true` and **no** Idempotency-Key |
| A retry never buys twice | paid creates send an `Idempotency-Key`: the caller's `--idempotency-key`, else a fresh UUID |
| Deletes are deliberate | `--confirm <exact name>` is a required clap arg; the API checks the name (`422 CONFIRM_MISMATCH`) |
| Secrets stay hidden | `db connection` masks the password unless `--reveal`; `env list` shows secrets masked; `doctor` prints the key masked (`config::mask`) |
| The key never goes on the command line | there is no `--api-key` flag (shell history, `ps`). `doctor` prompts with hidden input or reads `--api-key-stdin` |

## 3. Waiting is not retrying

`--wait` polls `GET /deployments/:id` until `success` or `failed`. The pod
already exists and bills while we wait, so:

- a poll error is **state unknown**, not failure: keep polling.
- only these end the wait early: `401`/`403`/`NO_API_KEY` (retrying cannot
  help), and `404` on **two consecutive** reads (gone, or a typo).
- every error from a wait carries `details.deploymentId` and, when known,
  `details.podId`, so the caller can find the billed resource instead of
  creating another.
- `failed` returns `DEPLOYMENT_FAILED` (exit 6) with the last 30 log lines in
  `details.logs`: the agent's next step is to read them.
- in demo mode the poll interval is 20 ms; against the API it is 3 s.

## 4. Demo mode is a first-class backend

`src/demo.rs` is a `Transport`, so commands do not know which one they talk to.
It ports happy-pod/mcp `src/demo.ts` route for route, with one difference: the
CLI is a new process per command, so state persists in
`$HAPPYPOD_HOME/demo.json` (written atomically through a `.tmp` file).

- Reads do not save state, **except** `GET /deployments/:id`, which advances a
  build one step (queued → building → deploying → success|failed). That is how
  `--wait` sees progress without a clock.
- The failure triggers are the MCP's: a repo URL containing `needs-db` fails
  without `DATABASE_URL`; an image containing `broken` fails to start. Docs and
  tests rely on them.
- When `demo.ts` changes, port the change. Keep messages, codes and numbers the
  same: the docs show demo output.

## 5. Thin client, no framework

- **One call per command where possible.** `pod create --wait` is the largest
  flow: create, poll, re-read the pod for its URL.
- **`serde_json::Value` for API data.** The CLI does not own the schema, the API
  does. Typed structs would drop fields the API adds later. Typed structs are
  for *our* state (`demo.rs`, `config.rs`).
- **No async.** `reqwest::blocking` is enough for one request at a time. Do not
  add tokio to `main`.
- **Few dependencies.** clap, serde, serde_json, reqwest (rustls, like the
  happy-pod API crate), chrono, uuid, rpassword. Ask before adding one.

## 6. Smells, ranked

1. A route in `commands.rs` that `demo.rs` does not answer (`NO_ROUTE` in tests).
2. A secret in stdout without `--reveal`, or a key accepted as a flag.
3. A paid call without `--dry-run`, or a dry run that sends an Idempotency-Key.
4. Anything printed to stdout besides the result (it breaks `| jq`).
5. `unwrap()`/`expect()` on API data: use `.as_str().unwrap_or_default()` and
   friends, the API may omit fields.
6. A new exit code, or a changed one, without updating help, `SKILL.md` and README.
7. A path segment built from user input without `enc()`.
8. Mixed-case or long help text.
