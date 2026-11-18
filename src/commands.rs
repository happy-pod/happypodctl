//! What each command does: one or two API calls, then print.

use crate::api::{ApiError, Client, Query, new_idempotency_key};
use crate::cli::*;
use crate::config::{self, Resolved, Source};
use crate::output::{self, Columns, Format};
use serde_json::{Map, Value, json};
use std::io::{IsTerminal, Read};
use std::time::{Duration, Instant};

pub struct Ctx {
    pub client: Client,
    pub cfg: Resolved,
    pub format: Format,
}

type Out = Result<(), ApiError>;

const PROJECTS: Columns = &[
    ("id", "id"),
    ("name", "name"),
    ("pods", "pods"),
    ("databases", "databases"),
];
const PODS: Columns = &[
    ("id", "id"),
    ("name", "name"),
    ("kind", "kind"),
    ("status", "status"),
    ("size", "size"),
    ("gpu", "gpuType"),
    ("replicas", "replicas"),
    ("region", "region"),
    ("usd/mo", "price.monthlyUsd"),
    ("url", "url"),
];
const DEPLOYMENTS: Columns = &[
    ("id", "id"),
    ("status", "status"),
    ("source", "source"),
    ("live", "live"),
    ("started", "startedAt"),
];
const DATABASES: Columns = &[
    ("id", "id"),
    ("name", "name"),
    ("engine", "engine"),
    ("plan", "plan"),
    ("status", "status"),
    ("region", "region"),
];
const REGIONS: Columns = &[("id", "id"), ("city", "city"), ("gpu", "gpu")];
const SIZES: Columns = &[
    ("id", "id"),
    ("vcpu", "vcpu"),
    ("memory gb", "memoryGb"),
    ("usd/h", "hourlyUsd"),
    ("usd/mo", "monthlyUsd"),
];
const GPUS: Columns = &[
    ("id", "id"),
    ("vram gb", "vramGb"),
    ("usd/h", "hourlyUsd"),
    ("usd/mo", "monthlyUsd"),
    ("regions", "regions"),
];
const DOMAINS: Columns = &[
    ("hostname", "hostname"),
    ("status", "status"),
    ("dns type", "dns.type"),
    ("dns value", "dns.value"),
];
const NONE: Columns = &[];

pub fn usage(msg: impl Into<String>) -> ApiError {
    ApiError::new(0, "USAGE", msg)
}

/// Exit code for an error. Documented in `--help` and in the skill: agents branch on it.
pub fn exit_code(e: &ApiError) -> i32 {
    match (e.status, e.code.as_str()) {
        (_, "USAGE") => 2,
        (401 | 403, _) | (_, "NO_API_KEY") => 3,
        (404, _) => 4,
        (409, _) => 5,
        (_, "DEPLOYMENT_FAILED" | "WAIT_TIMEOUT") => 6,
        _ => 1,
    }
}

fn enc(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn kv_list(items: &[String], what: &str) -> Result<Map<String, Value>, ApiError> {
    let mut m = Map::new();
    for it in items {
        let (k, v) = it
            .split_once('=')
            .ok_or_else(|| usage(format!("{what} must be KEY=VALUE, got \"{it}\"")))?;
        let valid = !k.is_empty()
            && k.len() <= 128
            && k.chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
            && k.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(usage(format!(
                "{what} name \"{k}\" must be UPPER_SNAKE_CASE"
            )));
        }
        m.insert(k.into(), json!(v));
    }
    Ok(m)
}

fn ports(items: &[String]) -> Result<Vec<Value>, ApiError> {
    items
        .iter()
        .map(|p| {
            let (num, private) = match p.split_once(':') {
                Some((n, "private")) => (n, true),
                Some(_) => {
                    return Err(usage(format!(
                        "--port must be PORT or PORT:private, got \"{p}\""
                    )));
                }
                None => (p.as_str(), false),
            };
            let port: u16 = num
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| usage(format!("--port {num} is not a port number")))?;
            Ok(json!({ "port": port, "public": !private }))
        })
        .collect()
}

/// "90s", "10m", "1h" or plain seconds.
pub fn duration(s: &str) -> Result<Duration, ApiError> {
    let s = s.trim();
    let (num, unit) = s
        .find(|c: char| !c.is_ascii_digit())
        .map_or((s, "s"), |i| s.split_at(i));
    let n: u64 = num
        .parse()
        .map_err(|_| usage(format!("bad duration \"{s}\": use 90s, 10m or 1h")))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(usage(format!("bad duration \"{s}\": use 90s, 10m or 1h"))),
    };
    Ok(Duration::from_secs(secs))
}

impl Ctx {
    fn print(&self, v: &Value, cols: Columns) {
        output::print(v, self.format, cols);
    }

    fn get(&self, path: impl Into<String>, q: Query) -> Result<Value, ApiError> {
        self.client.get(path, q)
    }

    fn post(&self, path: impl Into<String>) -> Result<Value, ApiError> {
        self.client.send("POST", path, Query::new(), None, None)
    }

    /// The project from the flag, the environment or the saved default. When none is set,
    /// the error lists the projects so the caller can pick one.
    fn project(&self, flag: &ProjectArg) -> Result<String, ApiError> {
        if let Some(p) = flag.project.clone().or_else(|| self.cfg.project.clone()) {
            return Ok(p);
        }
        let list = self.get("/projects", Query::new()).unwrap_or(Value::Null);
        let projects: Vec<Value> = list
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| json!({ "id": p["id"], "name": p["name"] }))
                    .collect()
            })
            .unwrap_or_default();
        Err(ApiError::new(0, "USAGE", "no project given. pass --project <id>, set HAPPYPOD_PROJECT, or run `happypodctl project use <id>`")
            .with_details(json!({ "projects": projects })))
    }

    /// Polls a deployment until it ends. Poll errors count as "state unknown": the pod already
    /// exists and bills, so a transport error must not make the caller create it again.
    /// Auth errors end the wait, and so does a deployment that is missing twice in a row.
    fn wait(
        &self,
        deployment_id: &str,
        timeout: Duration,
        pod_id: Option<&str>,
    ) -> Result<Value, ApiError> {
        let interval = if self.cfg.demo {
            Duration::from_millis(20)
        } else {
            Duration::from_secs(3)
        };
        let start = Instant::now();
        let mut misses = 0;
        let mut last = Value::Null;
        let with_ids = |e: ApiError| {
            let mut d = json!({ "deploymentId": deployment_id });
            if let Some(p) = pod_id {
                d["podId"] = json!(p);
            }
            e.with_details(d)
        };
        loop {
            match self.get(format!("/deployments/{}", enc(deployment_id)), Query::new()) {
                Ok(d) => {
                    misses = 0;
                    match d["status"].as_str() {
                        Some("success") => return Ok(d),
                        Some("failed") => {
                            let logs = self
                                .get(
                                    format!("/deployments/{}/logs", enc(deployment_id)),
                                    Query::new().opt("tail", Some(30)),
                                )
                                .ok();
                            let mut details = json!({ "deploymentId": deployment_id, "logs": logs.map(|l| l["lines"].clone()).unwrap_or(Value::Null) });
                            if let Some(p) = pod_id {
                                details["podId"] = json!(p);
                            }
                            return Err(ApiError::new(0, "DEPLOYMENT_FAILED", format!("deployment {deployment_id} failed. read the logs in details, fix, then deploy again"))
                                .with_details(details));
                        }
                        _ => last = d,
                    }
                }
                Err(e) if matches!(e.status, 401 | 403) || e.code == "NO_API_KEY" => return Err(e),
                Err(e) if e.status == 404 => {
                    misses += 1;
                    if misses >= 2 {
                        return Err(with_ids(e));
                    }
                }
                Err(_) => {}
            }
            if start.elapsed() >= timeout {
                let status = last["status"].as_str().unwrap_or("unknown").to_string();
                return Err(with_ids(ApiError::new(
                    0,
                    "WAIT_TIMEOUT",
                    format!(
                        "stopped waiting after {}s; deployment is still {status}",
                        timeout.as_secs()
                    ),
                )));
            }
            std::thread::sleep(interval);
        }
    }
}

pub fn run(ctx: &Ctx, cmd: Command) -> Out {
    match cmd {
        Command::Whoami => ctx.print(&ctx.get("/me", Query::new())?, NONE),
        Command::Region(ListOnly::List) => ctx.print(&ctx.get("/regions", Query::new())?, REGIONS),
        Command::Size(ListOnly::List) => ctx.print(&ctx.get("/sizes", Query::new())?, SIZES),
        Command::Gpu(GpuCmd::List { region }) => ctx.print(
            &ctx.get("/gpu-types", Query::new().opt("region", region))?,
            GPUS,
        ),
        Command::Project(c) => project(ctx, c)?,
        Command::Pod(c) => pod(ctx, c)?,
        Command::Deployment(c) => deployment(ctx, c)?,
        Command::Env(c) => env(ctx, c)?,
        Command::Domain(DomainCmd::List { pod_id }) => ctx.print(
            &ctx.get(format!("/pods/{}/domains", enc(&pod_id)), Query::new())?,
            DOMAINS,
        ),
        Command::Domain(DomainCmd::Add { pod_id, hostname }) => {
            let v = ctx.client.send(
                "POST",
                format!("/pods/{}/domains", enc(&pod_id)),
                Query::new(),
                Some(json!({ "hostname": hostname })),
                None,
            )?;
            ctx.print(&v, NONE)
        }
        Command::Db(c) => db(ctx, c)?,
        Command::Doctor(_) | Command::Demo(_) => unreachable!("handled before a client exists"),
    }
    Ok(())
}

fn project(ctx: &Ctx, c: ProjectCmd) -> Out {
    match c {
        ProjectCmd::List => ctx.print(&ctx.get("/projects", Query::new())?, PROJECTS),
        ProjectCmd::Create {
            name,
            idempotency_key,
        } => {
            let key = idempotency_key.unwrap_or_else(new_idempotency_key);
            ctx.print(
                &ctx.client.send(
                    "POST",
                    "/projects",
                    Query::new(),
                    Some(json!({ "name": name })),
                    Some(key),
                )?,
                NONE,
            )
        }
        ProjectCmd::Delete { id, confirm } => ctx.print(
            &ctx.client.send(
                "DELETE",
                format!("/projects/{}", enc(&id)),
                Query::new(),
                Some(json!({ "confirm": confirm })),
                None,
            )?,
            NONE,
        ),
        ProjectCmd::Use { id } => {
            // Check it exists first, so a typo is not saved as the default.
            let list = ctx.get("/projects", Query::new())?;
            let found = list
                .as_array()
                .and_then(|a| a.iter().find(|p| p["id"] == id.as_str()))
                .cloned();
            let Some(p) = found else {
                return Err(ApiError::new(
                    404,
                    "NOT_FOUND",
                    format!("project {id} not found"),
                ));
            };
            if ctx.cfg.demo {
                return Err(usage(
                    "demo mode does not save a default project. pass --project or set HAPPYPOD_PROJECT",
                ));
            }
            let mut file = config::load().map_err(|e| ApiError::new(0, "CONFIG", e))?;
            file.project = Some(id);
            let path = config::save(&file).map_err(|e| ApiError::new(0, "CONFIG", e))?;
            ctx.print(
                &json!({ "project": p, "savedTo": path.display().to_string() }),
                NONE,
            )
        }
    }
    Ok(())
}

fn pod(ctx: &Ctx, c: PodCmd) -> Out {
    let pod_path = |id: &str, rest: &str| format!("/pods/{}{rest}", enc(id));
    match c {
        PodCmd::List(p) => {
            let project = ctx.project(&p)?;
            ctx.print(
                &ctx.get(format!("/projects/{}/pods", enc(&project)), Query::new())?,
                PODS,
            )
        }
        PodCmd::Get { id } => ctx.print(&ctx.get(pod_path(&id, ""), Query::new())?, NONE),
        PodCmd::Create(a) => pod_create(ctx, *a)?,
        PodCmd::Deploy {
            id,
            idempotency_key,
            wait,
        } => {
            let timeout = duration(&wait.wait_timeout)?;
            let key = idempotency_key.unwrap_or_else(new_idempotency_key);
            let d = ctx.client.send(
                "POST",
                pod_path(&id, "/deployments"),
                Query::new(),
                None,
                Some(key),
            )?;
            if wait.wait {
                let dep = d["id"].as_str().unwrap_or_default().to_string();
                let done = ctx.wait(&dep, timeout, Some(&id))?;
                ctx.print(&done, NONE)
            } else {
                ctx.print(&d, NONE)
            }
        }
        PodCmd::Start { id } => ctx.print(&ctx.post(pod_path(&id, "/start"))?, NONE),
        PodCmd::Stop { id } => ctx.print(&ctx.post(pod_path(&id, "/stop"))?, NONE),
        PodCmd::Restart { id } => ctx.print(&ctx.post(pod_path(&id, "/restart"))?, NONE),
        PodCmd::Scale {
            id,
            size,
            replicas,
            dry_run,
        } => {
            if size.is_none() && replicas.is_none() {
                return Err(usage("give --size, --replicas or both"));
            }
            let mut body = Map::new();
            if let Some(s) = size {
                body.insert("size".into(), json!(s));
            }
            if let Some(r) = replicas {
                body.insert("replicas".into(), json!(r));
            }
            let v = ctx.client.send(
                "PATCH",
                pod_path(&id, "/scale"),
                Query::new().flag("dryRun", dry_run),
                Some(Value::Object(body)),
                None,
            )?;
            ctx.print(&v, NONE)
        }
        PodCmd::Rollback { id, deployment_id } => {
            let v = ctx.client.send(
                "POST",
                pod_path(&id, "/rollback"),
                Query::new(),
                Some(json!({ "deploymentId": deployment_id })),
                None,
            )?;
            ctx.print(&v, NONE)
        }
        PodCmd::Delete { id, confirm } => ctx.print(
            &ctx.client.send(
                "DELETE",
                pod_path(&id, ""),
                Query::new(),
                Some(json!({ "confirm": confirm })),
                None,
            )?,
            NONE,
        ),
        PodCmd::Logs {
            id,
            since,
            level,
            tail,
        } => {
            let v = ctx.get(
                pod_path(&id, "/logs"),
                Query::new()
                    .opt("since", since)
                    .opt("level", level)
                    .opt("tail", tail),
            )?;
            match ctx.format {
                Format::Table => ctx.print(
                    &v["lines"],
                    &[
                        ("at", "at"),
                        ("level", "level"),
                        ("replica", "replica"),
                        ("message", "message"),
                    ],
                ),
                Format::Json => ctx.print(&v, NONE),
            }
        }
        PodCmd::Metrics { id, window } => ctx.print(
            &ctx.get(
                pod_path(&id, "/metrics"),
                Query::new().opt("window", window),
            )?,
            NONE,
        ),
        PodCmd::Diagnose { id } => {
            let v = ctx.post(pod_path(&id, "/diagnose"))?;
            match ctx.format {
                Format::Table => {
                    println!("healthy: {}", v["healthy"]);
                    ctx.print(
                        &v["findings"],
                        &[
                            ("severity", "severity"),
                            ("title", "title"),
                            ("evidence", "evidence"),
                            ("fix", "fix"),
                        ],
                    )
                }
                Format::Json => ctx.print(&v, NONE),
            }
        }
    }
    Ok(())
}

fn pod_create(ctx: &Ctx, a: PodCreate) -> Out {
    if a.repo.is_none() == a.image.is_none() {
        return Err(usage("give exactly one source: --repo or --image"));
    }
    let timeout = duration(&a.wait.wait_timeout)?;
    let project = ctx.project(&a.project)?;
    let gpu = a.gpu_type.is_some();
    let mut body = Map::new();
    body.insert("kind".into(), json!(if gpu { "gpu" } else { "app" }));
    body.insert("name".into(), json!(a.name));
    let mut put = |k: &str, v: Option<Value>| {
        if let Some(v) = v {
            body.insert(k.into(), v);
        }
    };
    put("repo", a.repo.map(Value::from));
    put("branch", a.branch.map(Value::from));
    put("rootDirectory", a.root_dir.map(Value::from));
    put("image", a.image.map(Value::from));
    put("size", a.size.map(Value::from));
    put("replicas", a.replicas.map(Value::from));
    put("gpuType", a.gpu_type.map(Value::from));
    put("gpuCount", a.gpu_count.map(Value::from));
    put("volumeGb", a.volume_gb.map(Value::from));
    put("region", a.region.map(Value::from));
    put(
        "ports",
        (!a.ports.is_empty())
            .then(|| ports(&a.ports).map(Value::from))
            .transpose()?,
    );
    put(
        "env",
        (!a.env.is_empty())
            .then(|| kv_list(&a.env, "--env").map(Value::from))
            .transpose()?,
    );
    put(
        "secrets",
        (!a.secrets.is_empty())
            .then(|| kv_list(&a.secrets, "--secret").map(Value::from))
            .transpose()?,
    );

    let dry = a.paid.dry_run;
    let key = if dry {
        None
    } else {
        Some(a.paid.idempotency_key.unwrap_or_else(new_idempotency_key))
    };
    let v = ctx.client.send(
        "POST",
        format!("/projects/{}/pods", enc(&project)),
        Query::new().flag("dryRun", dry),
        Some(Value::Object(body)),
        key,
    )?;
    if dry || !a.wait.wait {
        ctx.print(&v, NONE);
        return Ok(());
    }
    let pod_id = v["pod"]["id"].as_str().unwrap_or_default().to_string();
    let dep = v["deployment"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let done = ctx.wait(&dep, timeout, Some(&pod_id))?;
    let pod = ctx
        .get(format!("/pods/{}", enc(&pod_id)), Query::new())
        .unwrap_or_else(|_| v["pod"].clone());
    ctx.print(
        &json!({ "pod": pod, "deployment": done, "price": v["price"] }),
        NONE,
    );
    Ok(())
}

fn deployment(ctx: &Ctx, c: DeploymentCmd) -> Out {
    match c {
        DeploymentCmd::List { pod_id, limit } => ctx.print(
            &ctx.get(
                format!("/pods/{}/deployments", enc(&pod_id)),
                Query::new().opt("limit", limit),
            )?,
            DEPLOYMENTS,
        ),
        DeploymentCmd::Get { id } => ctx.print(
            &ctx.get(format!("/deployments/{}", enc(&id)), Query::new())?,
            NONE,
        ),
        DeploymentCmd::Logs { id, tail } => {
            let v = ctx.get(
                format!("/deployments/{}/logs", enc(&id)),
                Query::new().opt("tail", tail),
            )?;
            match ctx.format {
                Format::Table => v["lines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .for_each(|l| println!("{}", l.as_str().unwrap_or_default())),
                Format::Json => ctx.print(&v, NONE),
            }
        }
        DeploymentCmd::Wait { id, timeout } => {
            let done = ctx.wait(&id, duration(&timeout)?, None)?;
            ctx.print(&done, NONE)
        }
    }
    Ok(())
}

fn env(ctx: &Ctx, c: EnvCmd) -> Out {
    match c {
        EnvCmd::List { pod_id } => ctx.print(
            &ctx.get(format!("/pods/{}/env", enc(&pod_id)), Query::new())?,
            NONE,
        ),
        EnvCmd::Set {
            pod_id,
            vars,
            secrets,
        } => {
            if vars.is_empty() && secrets.is_empty() {
                return Err(usage("give at least one KEY=VALUE or --secret KEY=VALUE"));
            }
            let body = json!({ "env": kv_list(&vars, "variable")?, "secrets": kv_list(&secrets, "--secret")? });
            ctx.print(
                &ctx.client.send(
                    "PUT",
                    format!("/pods/{}/env", enc(&pod_id)),
                    Query::new(),
                    Some(body),
                    None,
                )?,
                NONE,
            )
        }
        EnvCmd::Unset { pod_id, key } => ctx.print(
            &ctx.client.send(
                "DELETE",
                format!("/pods/{}/env/{}", enc(&pod_id), enc(&key)),
                Query::new(),
                None,
                None,
            )?,
            NONE,
        ),
    }
    Ok(())
}

fn db(ctx: &Ctx, c: DbCmd) -> Out {
    match c {
        DbCmd::List(p) => {
            let project = ctx.project(&p)?;
            ctx.print(
                &ctx.get(
                    format!("/projects/{}/databases", enc(&project)),
                    Query::new(),
                )?,
                DATABASES,
            )
        }
        DbCmd::Create {
            name,
            project,
            engine,
            plan,
            region,
            paid,
        } => {
            let project = ctx.project(&project)?;
            let mut body = json!({ "name": name, "engine": engine });
            if let Some(p) = plan {
                body["plan"] = json!(p);
            }
            if let Some(r) = region {
                body["region"] = json!(r);
            }
            let key = if paid.dry_run {
                None
            } else {
                Some(paid.idempotency_key.unwrap_or_else(new_idempotency_key))
            };
            let v = ctx.client.send(
                "POST",
                format!("/projects/{}/databases", enc(&project)),
                Query::new().flag("dryRun", paid.dry_run),
                Some(body),
                key,
            )?;
            ctx.print(&v, NONE)
        }
        DbCmd::Connection { id, reveal } => ctx.print(
            &ctx.get(
                format!("/databases/{}/connection", enc(&id)),
                Query::new().flag("reveal", reveal),
            )?,
            NONE,
        ),
        DbCmd::Delete { id, confirm } => ctx.print(
            &ctx.client.send(
                "DELETE",
                format!("/databases/{}", enc(&id)),
                Query::new(),
                Some(json!({ "confirm": confirm })),
                None,
            )?,
            NONE,
        ),
    }
    Ok(())
}

/// `doctor`: report the setup, optionally save a new key, and check it against the API.
/// The only command that writes the api key to disk.
pub fn doctor(
    args: &DoctorArgs,
    demo: bool,
    url_flag: Option<String>,
    format: Format,
    make_client: impl Fn(&Resolved) -> Result<Client, ApiError>,
) -> Out {
    let mut file = config::load().map_err(|e| ApiError::new(0, "CONFIG", e))?;
    let mut saved = None;
    if !demo {
        let new_key = if args.api_key_stdin {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| usage(format!("could not read stdin: {e}")))?;
            Some(s.trim().to_string()).filter(|s| !s.is_empty())
        } else if std::io::stdin().is_terminal()
            && config::resolve(&file, false, url_flag.clone(), None)
                .api_key
                .is_empty()
        {
            eprintln!("no api key yet. create one at https://happypod.io (settings > api keys).");
            let k = rpassword::prompt_password("paste it here (input is hidden): ")
                .map_err(|e| usage(e.to_string()))?;
            Some(k.trim().to_string()).filter(|s| !s.is_empty())
        } else {
            None
        };
        if let Some(k) = new_key {
            let probe = config::resolve(
                &config::File {
                    api_key: Some(k.clone()),
                    ..file.clone()
                },
                false,
                url_flag.clone(),
                None,
            );
            make_client(&Resolved {
                api_key: k.clone(),
                ..probe
            })?
            .get("/me", Query::new())?;
            file.api_key = Some(k);
            saved = Some(
                config::save(&file)
                    .map_err(|e| ApiError::new(0, "CONFIG", e))?
                    .display()
                    .to_string(),
            );
        }
    }
    let cfg = config::resolve(&file, demo, url_flag, None);
    let mut checks = vec![];
    let key_ok = !cfg.api_key.is_empty();
    checks.push(json!({
        "check": "api key",
        "ok": key_ok,
        "source": cfg.api_key_source,
        "value": if key_ok { json!(config::mask(&cfg.api_key)) } else { Value::Null },
        "hint": if key_ok { Value::Null } else { json!("run `happypodctl doctor` in a terminal, or pipe a key into `happypodctl doctor --api-key-stdin`") },
    }));
    checks.push(json!({ "check": "api url", "ok": true, "source": cfg.api_url_source, "value": cfg.api_url }));
    let mut account = Value::Null;
    if key_ok {
        match make_client(&cfg)?.get("/me", Query::new()) {
            Ok(me) => {
                checks.push(json!({ "check": "api reachable and key accepted", "ok": true }));
                account = me;
            }
            Err(e) => checks.push(json!({ "check": "api reachable and key accepted", "ok": false, "error": e.to_json() })),
        }
    }
    if cfg.project_source != Source::None {
        checks.push(json!({ "check": "default project", "ok": true, "source": cfg.project_source, "value": cfg.project }));
    }
    let healthy = checks.iter().all(|c| c["ok"] == true);
    let report = json!({ "healthy": healthy, "mode": if demo { "demo" } else { "live" }, "configFile": config::path().display().to_string(), "savedKeyTo": saved, "account": account, "checks": checks });
    match format {
        Format::Table => output::print(
            &report["checks"],
            format,
            &[
                ("check", "check"),
                ("ok", "ok"),
                ("source", "source"),
                ("value", "value"),
            ],
        ),
        Format::Json => output::print(&report, format, NONE),
    }
    if healthy {
        Ok(())
    } else {
        let failed: Vec<Value> = checks.into_iter().filter(|c| c["ok"] != true).collect();
        let first = failed.first().cloned().unwrap_or(Value::Null);
        let status = first["error"]["status"].as_u64().unwrap_or(401) as u16;
        Err(
            ApiError::new(status, "DOCTOR", "setup is not complete; see checks")
                .with_details(json!({ "failed": failed })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(duration("2").unwrap(), Duration::from_secs(2));
        assert!(duration("ten").is_err());
    }

    #[test]
    fn env_names_must_be_upper_snake() {
        assert!(kv_list(&["DATABASE_URL=postgres://x".into()], "--secret").is_ok());
        assert_eq!(
            kv_list(&["db_url=x".into()], "--secret").unwrap_err().code,
            "USAGE"
        );
        assert_eq!(
            kv_list(&["NOVALUE".into()], "--env").unwrap_err().code,
            "USAGE"
        );
        assert_eq!(kv_list(&["A=b=c".into()], "--env").unwrap()["A"], "b=c");
    }

    #[test]
    fn ports_take_a_private_suffix() {
        assert_eq!(
            ports(&["3000".into(), "9000:private".into()]).unwrap(),
            vec![
                json!({"port":3000,"public":true}),
                json!({"port":9000,"public":false})
            ]
        );
        assert!(ports(&["0".into()]).is_err());
        assert!(ports(&["80:public".into()]).is_err());
    }

    #[test]
    fn exit_codes_by_kind() {
        let code = |s: u16, c: &str| exit_code(&ApiError::new(s, c, ""));
        assert_eq!(
            (
                code(0, "USAGE"),
                code(401, "X"),
                code(404, "NOT_FOUND"),
                code(409, "NAME_TAKEN"),
                code(0, "WAIT_TIMEOUT"),
                code(500, "X")
            ),
            (2, 3, 4, 5, 6, 1)
        );
    }

    #[test]
    fn path_segments_are_escaped() {
        assert_eq!(enc("pod_1"), "pod_1");
        assert_eq!(enc("../me"), "..%2Fme");
    }
}
