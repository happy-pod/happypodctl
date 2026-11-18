# Agent guide for happypodctl

`happypodctl` is the Happy Pod command line: a thin Rust client for the Happy Pod
API v1, plus a built-in simulator (`--demo`). Its first user is an AI agent
running shell commands, its second is a person at a terminal. Every design rule
here follows from that order.

## Read this first

A skill for this codebase ships at **`.claude/skills/happypodctl/`**:

| File | What it gives you |
|---|---|
| `.claude/skills/happypodctl/SKILL.md` | start here: layout, the request path, the rules |
| `.claude/skills/happypodctl/doctrine.md` | what good code looks like here; read before writing any |
| `.claude/skills/happypodctl/api-index.md` | every API route, its command, and its demo behaviour. **Check here before adding or calling a route** |
| `.claude/skills/happypodctl/workflow.md` | the change loop and how to verify it |
| `.claude/skills/happypodctl/errors.md` | symptom → cause for the compiler, clippy and the tests |
| `.claude/skills/happypodctl/recipes/` | add a command, add an API route, write tests |

If your tool supports Agent Skills, it will load `SKILL.md` automatically. If
not, read it directly: it is a normal markdown file.

## The three rules that prevent most mistakes

1. **The contract is the MCP server's.** Routes, bodies and error codes match
   happy-pod/mcp `docs/API.md` and `src/demo.ts`. A route the MCP server does
   not have does not exist yet: add it there first, then here.
2. **Every route works in demo mode.** A new call goes into `src/demo.rs` in the
   same change, with a test. `--demo` is how agents, the docs and our own tests
   try the CLI without an account.
3. **Output is for machines first.** JSON on stdout by default, one JSON error
   object on stderr, a documented exit code. Never print a secret the user did
   not ask to reveal.

## Before you call it done

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

## More

- Docs: <https://docs.happypod.io>
- API contract: <https://github.com/happy-pod/mcp/blob/main/docs/API.md>
