<p align="center">
  <img src="https://raw.githubusercontent.com/happy-pod/.github/main/profile/happypod-banner.png" alt="Happy Pod: the DevOps AI agent for vibe coders" width="100%">
</p>

# happypodctl

The command line for [Happy Pod](https://happypod.io): ship an app from a repo, add a database, start a GPU pod, find out why production is slow and fix it, from your terminal, your scripts, your CI or your AI agent.

- **Built for agents first.** JSON on stdout, one JSON error on stderr, a stable exit code for every kind of failure.
- **Price before paying.** Every paid command takes `--dry-run`, and retries never create twice.
- **Try it without an account.** `--demo` runs every command against a built-in simulator.

## Install

From source, with [Rust](https://rustup.rs) 1.85 or newer:

```bash
cargo install --git https://github.com/happy-pod/happypodctl
```

## Get started

```bash
happypodctl doctor            # asks for your API key (happypod.io → Settings → API keys) and checks it
happypodctl project list
happypodctl project use prj_...   # default project for later commands
```

Or set the key in the environment: `export HAPPYPOD_API_KEY=hp_...`.

### Try it with no account

```bash
happypodctl --demo pod list
happypodctl --demo pod create api --repo https://github.com/acme/api --dry-run
happypodctl --demo pod create api --repo https://github.com/acme/api --wait
happypodctl --demo -o table pod diagnose pod_web
happypodctl demo reset        # start over from the sample project
```

The simulator has a sample project with a running web pod and a Postgres database. Builds go from queued to live, logs and metrics look real, and prices are example figures. A repo whose URL contains `needs-db` fails until it has a `DATABASE_URL`, so you can practise a fix. Nothing leaves your machine and nothing is billed. Its state is kept in `~/.happypod/demo.json`.

## Ship an app

```bash
# What will it cost?
happypodctl pod create web --repo https://github.com/acme/vibe-app --size small --dry-run

# Ship it and wait until it is live
happypodctl pod create web --repo https://github.com/acme/vibe-app --wait

# Add Postgres and pass its URL to the app as a secret
happypodctl db create main --engine postgres
happypodctl env set pod_... --secret DATABASE_URL="$(happypodctl db connection db_... --reveal | jq -r .url)"
happypodctl pod deploy pod_... --wait
```

A GPU pod is the same command with `--gpu-type` and an image:

```bash
happypodctl gpu list --region us-east
happypodctl pod create llm --image vllm/vllm-openai:latest --gpu-type l4 --dry-run
```

## Commands

| Group | Commands |
| --- | --- |
| Account | `whoami`, `region list`, `size list`, `gpu list` |
| Projects | `project list`, `project create`, `project delete`, `project use` |
| Ship | `pod list`, `pod get`, `pod create`, `pod deploy`, `deployment list`, `deployment get`, `deployment logs`, `deployment wait` |
| Operate | `pod logs`, `pod metrics`, `pod diagnose`, `pod restart`, `pod stop`, `pod start`, `pod scale`, `pod rollback`, `pod delete` |
| Variables and domains | `env list`, `env set`, `env unset`, `domain list`, `domain add` |
| Databases | `db list`, `db create`, `db connection`, `db delete` |
| Setup | `doctor`, `demo reset` |

`happypodctl <command> --help` shows every flag. The commands match the tools of the [Happy Pod MCP server](https://github.com/happy-pod/mcp) one to one.

## Output and errors

Output is JSON unless you pass `-o table`. A failure prints one JSON object on stderr:

```json
{"error":"No a100-80gb capacity in eu-west right now","code":"NO_GPU_CAPACITY","status":409,"details":{"tryRegions":["us-east"]}}
```

| Exit code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | API or runtime error |
| 2 | usage error |
| 3 | missing or rejected API key |
| 4 | not found |
| 5 | conflict (name taken, no capacity, not running) |
| 6 | deployment failed, or `--wait` timed out |

When `--wait` ends with an error, `details` carries the `podId` and `deploymentId`, and for a failed build the last log lines.

## Safety

- Paid commands (`pod create`, `pod scale`, `db create`) take `--dry-run`: you get the price and nothing changes.
- Creates send an `Idempotency-Key`, so a retry returns the first result instead of buying twice. Pass your own with `--idempotency-key`.
- Deletes need the resource's exact name: `pod delete pod_... --confirm web`.
- Secrets stay masked: `env list` and `db connection` hide them unless you pass `--reveal`.
- The API key is never a command-line flag. `doctor` asks for it with hidden input (or reads `--api-key-stdin`) and saves it to `~/.happypod/config.json`, readable only by you.

## Configuration

| Setting | Flag | Environment | Config file key |
| --- | --- | --- | --- |
| API key | | `HAPPYPOD_API_KEY` | `apiKey` |
| API URL | `--api-url` | `HAPPYPOD_API_URL` | `apiUrl` |
| Project | `--project`, `-p` | `HAPPYPOD_PROJECT` | `project` |
| Demo mode | `--demo` | `HAPPYPOD_MODE=demo` | |
| Config folder | | `HAPPYPOD_HOME` (default `~/.happypod`) | |

A flag wins over the environment, and the environment wins over the file.

## Development

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests run the built binary against the simulator and never touch the real API. Working on this repo with an AI agent? Start with [AGENTS.md](AGENTS.md); the skill in `.claude/skills/happypodctl/` carries the rules, the full API index and recipes.

## License

[MIT](LICENSE)
