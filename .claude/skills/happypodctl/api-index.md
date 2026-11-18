# API index

Every route of the Happy Pod API v1, the command that calls it, and what the
simulator does. Source of truth: happy-pod/mcp `docs/API.md` and `src/demo.ts`.
If a route is not here, it does not exist: do not guess one.

Base URL `https://api.happypod.io/v1` (demo: `https://demo.happypod.local/v1`).
Every request sends `Authorization: Bearer <key>` and `User-Agent: happypodctl/<version>`.
Success is `{"data": ...}`; failure is `{"error": {"code", "message", "details"?}}`.

## Account and catalogue

| Route | Command | MCP tool | Demo |
|---|---|---|---|
| `GET /me` | `whoami` (`me`) | `whoami` | `mode: "demo"`, $25 credit |
| `GET /regions` | `region list` | `list_regions` | us-east, eu-west, ap-southeast, ap-northeast (no gpu) |
| `GET /sizes` | `size list` | `list_sizes` | nano, small, medium, large, xlarge |
| `GET /gpu-types?region=` | `gpu list [--region]` | `list_gpu_types` | l4, rtx-4090, a100-80gb, h100-80gb |

## Projects

| Route | Command | MCP tool | Errors |
|---|---|---|---|
| `GET /projects` | `project list` | `list_projects` | |
| `POST /projects` `{name}` | `project create <name>` | `create_project` | `409 NAME_TAKEN`, `422 VALIDATION` |
| `DELETE /projects/:id` `{confirm}` | `project delete <id> --confirm <name>` | `delete_project` | `409 PROJECT_NOT_EMPTY`, `422 CONFIRM_MISMATCH` |
| (`GET /projects` to check) | `project use <id>` | | saves `project` in the config file; refused in demo |

## Pods: ship

| Route | Command | MCP tool | Notes |
|---|---|---|---|
| `GET /projects/:id/pods` | `pod list [-p]` | `list_pods` | |
| `GET /pods/:id` | `pod get <id>` | `get_pod` | |
| `POST /projects/:id/pods?dryRun=` | `pod create <name> --repo\|--image [...]` | `create_pod`, `create_gpu_pod` | `--gpu-type` makes `kind: gpu`. Returns `{pod, deployment, price}`. `409 NO_GPU_CAPACITY` with `details.tryRegions`, `409 NAME_TAKEN`, `422 UNKNOWN_SIZE/UNKNOWN_GPU_TYPE/UNKNOWN_REGION` |
| `POST /pods/:id/deployments` | `pod deploy <id> [--wait]` | `deploy_pod` | `{id, status, source}` |
| `GET /pods/:id/deployments?limit=` | `deployment list <pod-id>` | `list_deployments` | newest first, `live` flag |
| `GET /deployments/:id` | `deployment get <id>`, `deployment wait <id>`, `--wait` | `get_deployment` | demo advances one step per read |
| `GET /deployments/:id/logs?tail=` | `deployment logs <id>` | `get_deployment_logs` | `{lines: [string]}` |

## Pods: operate

| Route | Command | MCP tool | Notes |
|---|---|---|---|
| `GET /pods/:id/logs?since=&level=&tail=` | `pod logs <id>` | `get_pod_logs` | `{lines: [{at, level, replica, message}]}` |
| `GET /pods/:id/metrics?window=` | `pod metrics <id>` | `get_pod_metrics` | `cpuPercent, memoryPercent, requestsPerSecond, errorRatePercent, p95Ms` |
| `POST /pods/:id/diagnose` | `pod diagnose <id>` | `diagnose_pod` | `{healthy, findings: [{severity, title, evidence, fix}], metrics}` |
| `POST /pods/:id/restart` | `pod restart <id>` | `restart_pod` | `409 NOT_RUNNING` |
| `POST /pods/:id/stop` | `pod stop <id>` | `stop_pod` | |
| `POST /pods/:id/start` | `pod start <id>` | `start_pod` | |
| `PATCH /pods/:id/scale?dryRun=` `{size?, replicas?}` | `pod scale <id> [--size] [--replicas] [--dry-run]` | `scale_pod` | dry run returns `current` and `new`; `422 GPU_POD` for a gpu size change |
| `POST /pods/:id/rollback` `{deploymentId}` | `pod rollback <id> <deployment-id>` | `rollback_pod` | `409 NOT_ROLLBACKABLE` |
| `DELETE /pods/:id` `{confirm}` | `pod delete <id> --confirm <name>` | `delete_pod` | `422 CONFIRM_MISMATCH` |

## Variables and domains

| Route | Command | MCP tool | Notes |
|---|---|---|---|
| `GET /pods/:id/env` | `env list <pod-id>` | `list_env` | secrets masked |
| `PUT /pods/:id/env` `{env, secrets}` | `env set <pod-id> KEY=VALUE... [--secret KEY=VALUE]...` | `set_env` | applies on the next deploy; names are `UPPER_SNAKE_CASE` (checked client-side) |
| `DELETE /pods/:id/env/:key` | `env unset <pod-id> <key>` | `unset_env` | |
| `GET /pods/:id/domains` | `domain list <pod-id>` | `list_domains` | `pending_dns`, `issuing`, `active` |
| `POST /pods/:id/domains` `{hostname}` | `domain add <pod-id> <hostname>` | `add_domain` | returns the DNS record: CNAME for a subdomain, A for an apex |

## Databases

| Route | Command | MCP tool | Notes |
|---|---|---|---|
| `GET /projects/:id/databases` | `db list [-p]` | `list_databases` | |
| `POST /projects/:id/databases?dryRun=` | `db create <name> --engine postgres\|redis [--plan] [--region]` | `create_database` | hobby is free; standard $0.03/h, pro $0.11/h |
| `GET /databases/:id/connection?reveal=` | `db connection <id> [--reveal]` | `get_database_connection` | password masked unless `reveal=true` |
| `DELETE /databases/:id` `{confirm}` | `db delete <id> --confirm <name>` | `delete_database` | |

## CLI only (no API route)

| Command | What it does |
|---|---|
| `doctor [--api-key-stdin]` | reports key, url, reachability and default project; prompts for a key (hidden) when none is set and stdin is a terminal; checks a new key with `GET /me` before saving it |
| `demo reset` | deletes `$HAPPYPOD_HOME/demo.json` |

## Demo seed

Project `prj_demo` (`my-saas`) with app pod `pod_web` (`web`, repo
`https://github.com/acme/my-saas`, small, running, live deployment `dep_seed`)
and hobby Postgres `db_main` (`main`). In demo mode the default project is
`prj_demo`.

Failure triggers: a repo containing `needs-db` fails without `DATABASE_URL`; an
image containing `broken` fails to start; a pod name containing `leaky` reports
93% memory.
