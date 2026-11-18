# Recipe: add or change a command

For a route that is already in `api-index.md`. For a new route, use
`endpoint.md`.

## 1. Flags in `cli.rs`

Add a variant to the right subcommand enum. Help strings are lowercase, short,
and say the consequence when there is one ("irreversible", "billing resumes").

```rust
/// stop a pod. compute billing stops, volumes are kept
Stop { id: String },
```

Reuse the shared pieces instead of re-declaring flags:

| Need | Use |
|---|---|
| a project | `#[command(flatten)] project: ProjectArg` |
| a paid call | `#[command(flatten)] paid: Paid` (`--dry-run`, `--idempotency-key`) |
| waiting for a deployment | `#[command(flatten)] wait: Wait` |
| a delete | `#[arg(long, value_name = "NAME", required = true)] confirm: String` |
| a fixed set of values | `#[arg(long, value_parser = ["a", "b"])]` |

Never add a flag that takes an API key or a secret value by itself. Secrets come
in as `--secret KEY=VALUE` to `env set` and `pod create`, which the API stores
write-only.

## 2. Behaviour in `commands.rs`

Match the variant in the group function and keep it to the call and the print:

```rust
PodCmd::Stop { id } => ctx.print(&ctx.post(pod_path(&id, "/stop"))?, NONE),
```

- Build paths with `enc()` around every user-supplied segment.
- Query: `Query::new().opt("tail", tail).flag("dryRun", dry_run)`. Unset values
  are dropped.
- Body: `json!` for small bodies; a `Map` when fields are optional (see
  `pod_create`).
- Paid create: `let key = if dry { None } else { Some(paid.idempotency_key.unwrap_or_else(new_idempotency_key)) };`
- A list worth a table: add a `Columns` const with dotted paths
  (`("usd/mo", "price.monthlyUsd")`) and pass it to `ctx.print`.
- A usage problem the parser cannot catch: `return Err(usage("..."))`.

## 3. Test it

An end-to-end test in `tests/cli.rs` (see `testing.md`): the happy path, plus the
error path with its exit code.

## 4. Keep in step

`README.md` command table, `api-index.md` command column.
