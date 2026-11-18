//! The built-in simulator behind `--demo` and `HAPPYPOD_MODE=demo`.
//!
//! It speaks the same v1 contract as the real API and mirrors happy-pod/mcp `src/demo.ts`:
//! a seeded project, deployments that move queued → building → deploying → success|failed
//! one step per poll, synthetic logs and metrics, and example prices. The MCP server keeps
//! this state in memory; the CLI is a new process per command, so it lives in a JSON file
//! between runs. Nothing leaves the machine and nothing is billed.

use crate::api::{ApiError, Request, Response, Transport};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const URL: &str = "https://demo.happypod.local/v1";
pub const KEY: &str = "hp_demo_key";

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Port {
    port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    public: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct Pod {
    id: String,
    project_id: String,
    name: String,
    kind: String,
    repo: Option<String>,
    branch: Option<String>,
    root_directory: Option<String>,
    image: Option<String>,
    size: Option<String>,
    replicas: u32,
    region: String,
    ports: Vec<Port>,
    env: BTreeMap<String, String>,
    secrets: BTreeMap<String, String>,
    gpu_type: Option<String>,
    gpu_count: Option<u32>,
    volume_gb: Option<u32>,
    status: String,
    domains: Vec<Value>,
    created_at: String,
    live_deployment_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Deployment {
    id: String,
    pod_id: String,
    status: String,
    source: String,
    started_at: String,
    finished_at: Option<String>,
    log: Vec<String>,
    will_fail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Database {
    id: String,
    project_id: String,
    name: String,
    engine: String,
    plan: String,
    region: String,
    status: String,
    password: String,
    created_at: String,
}

/// Everything the simulator remembers between commands. Vectors keep creation order.
#[derive(Serialize, Deserialize, Default, Debug)]
struct State {
    seq: u64,
    projects: Vec<Project>,
    pods: Vec<Pod>,
    deployments: Vec<Deployment>,
    databases: Vec<Database>,
    idempotency: BTreeMap<String, Value>,
}

const REGIONS: &[(&str, &str, bool)] = &[
    ("us-east", "Ashburn, United States", true),
    ("eu-west", "Amsterdam, Netherlands", true),
    ("ap-southeast", "Singapore", true),
    ("ap-northeast", "Tokyo, Japan", false),
];
const SIZES: &[(&str, f64, f64, f64)] = &[
    ("nano", 0.25, 0.5, 0.004),
    ("small", 1.0, 1.0, 0.012),
    ("medium", 2.0, 4.0, 0.04),
    ("large", 4.0, 8.0, 0.08),
    ("xlarge", 8.0, 16.0, 0.16),
];
const GPUS: &[(&str, u32, f64, &[&str])] = &[
    ("l4", 24, 0.44, &["us-east", "eu-west", "ap-southeast"]),
    ("rtx-4090", 24, 0.69, &["us-east", "eu-west"]),
    ("a100-80gb", 80, 1.89, &["us-east"]),
    ("h100-80gb", 80, 2.99, &["us-east", "ap-southeast"]),
];
const DB_PLANS: &[(&str, f64)] = &[("hobby", 0.0), ("standard", 0.03), ("pro", 0.11)];
const HOURS_PER_MONTH: f64 = 730.0;
/// Path segments that are names, not ids, when a request path is matched to a route.
const FIXED: &[&str] = &[
    "pods",
    "databases",
    "deployments",
    "env",
    "domains",
    "logs",
    "metrics",
    "connection",
    "diagnose",
    "restart",
    "stop",
    "start",
    "scale",
    "rollback",
];

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}
fn money(hourly: f64) -> Value {
    json!({ "hourlyUsd": round(hourly, 4), "monthlyUsd": round(hourly * HOURS_PER_MONTH, 2) })
}
fn err(status: u16, code: &str, msg: impl Into<String>) -> ApiError {
    ApiError::new(status, code, msg)
}
fn not_found(what: &str, id: &str) -> ApiError {
    err(404, "NOT_FOUND", format!("{what} {id} not found"))
}

/// Random characters from `alphabet`, drawn from a v4 UUID.
fn random_chars(n: usize, alphabet: &[u8]) -> String {
    let mut out = String::new();
    while out.len() < n {
        for b in uuid::Uuid::new_v4().as_bytes() {
            if out.len() == n {
                break;
            }
            out.push(alphabet[*b as usize % alphabet.len()] as char);
        }
    }
    out
}
const ALNUM: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const HEX: &[u8] = b"0123456789abcdef";

fn base36(mut n: u64) -> String {
    let mut s = Vec::new();
    loop {
        s.push(ALNUM[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

impl State {
    fn nid(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}_{}{}", base36(self.seq), random_chars(4, ALNUM))
    }
    fn project(&self, id: &str) -> Result<&Project, ApiError> {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| not_found("Project", id))
    }
    fn pod_idx(&self, id: &str) -> Result<usize, ApiError> {
        self.pods
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| not_found("Pod", id))
    }
    fn dep_idx(&self, id: &str) -> Result<usize, ApiError> {
        self.deployments
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| not_found("Deployment", id))
    }
    fn db_idx(&self, id: &str) -> Result<usize, ApiError> {
        self.databases
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| not_found("Database", id))
    }
    fn project_name(&self, id: &str) -> String {
        self.projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }
    fn url(&self, p: &Pod) -> Value {
        if p.ports.iter().any(|x| x.public != Some(false)) {
            json!(format!(
                "https://{}-{}.demo.happypod.local",
                p.name,
                self.project_name(&p.project_id)
            ))
        } else {
            Value::Null
        }
    }
    fn last_deployment(&self, pod_id: &str) -> Option<&Deployment> {
        self.deployments.iter().rev().find(|d| d.pod_id == pod_id)
    }
    fn pod_view(&self, p: &Pod) -> Value {
        let source = match &p.repo {
            Some(repo) => {
                let mut s =
                    json!({ "repo": repo, "branch": p.branch.as_deref().unwrap_or("main") });
                if let Some(rd) = &p.root_directory {
                    s["rootDirectory"] = json!(rd);
                }
                s
            }
            None => json!({ "image": p.image }),
        };
        let mut v = json!({
            "id": p.id, "projectId": p.project_id, "name": p.name, "kind": p.kind, "status": p.status, "source": source,
        });
        let o = v.as_object_mut().expect("object");
        if let Some(s) = &p.size {
            o.insert("size".into(), json!(s));
        }
        if let Some(g) = &p.gpu_type {
            o.insert("gpuType".into(), json!(g));
            o.insert("gpuCount".into(), json!(p.gpu_count));
        }
        o.insert("replicas".into(), json!(p.replicas));
        o.insert("region".into(), json!(p.region));
        o.insert("ports".into(), json!(p.ports));
        o.insert("url".into(), self.url(p));
        o.insert("domains".into(), json!(p.domains));
        o.insert("price".into(), price_of(p).unwrap_or(Value::Null));
        o.insert(
            "lastDeployment".into(),
            self.last_deployment(&p.id).map_or(
                Value::Null,
                |d| json!({ "id": d.id, "status": d.status, "source": d.source }),
            ),
        );
        o.insert("createdAt".into(), json!(p.created_at));
        v
    }
}

struct PriceIn<'a> {
    kind: &'a str,
    size: Option<&'a str>,
    replicas: u32,
    gpu_type: Option<&'a str>,
    gpu_count: u32,
    volume_gb: u32,
}

fn pod_price(p: &PriceIn) -> Result<Value, ApiError> {
    if p.kind == "gpu" {
        let gt = p.gpu_type.unwrap_or_default();
        let g = GPUS.iter().find(|g| g.0 == gt).ok_or_else(|| {
            err(
                422,
                "UNKNOWN_GPU_TYPE",
                format!("Unknown gpuType \"{gt}\". See list_gpu_types."),
            )
        })?;
        return Ok(money(
            g.2 * f64::from(p.gpu_count.max(1)) + f64::from(p.volume_gb) * 0.0001,
        ));
    }
    let name = p.size.unwrap_or("small");
    let s = SIZES.iter().find(|s| s.0 == name).ok_or_else(|| {
        err(
            422,
            "UNKNOWN_SIZE",
            format!("Unknown size \"{name}\". See list_sizes."),
        )
    })?;
    Ok(money(s.3 * f64::from(p.replicas.max(1))))
}

fn price_of(p: &Pod) -> Result<Value, ApiError> {
    pod_price(&PriceIn {
        kind: &p.kind,
        size: p.size.as_deref(),
        replicas: p.replicas,
        gpu_type: p.gpu_type.as_deref(),
        gpu_count: p.gpu_count.unwrap_or(1),
        volume_gb: p.volume_gb.unwrap_or(20),
    })
}

fn first_port(p: &Pod) -> u16 {
    p.ports.first().map_or(3000, |x| x.port)
}

fn build_log(p: &Pod, ok: bool, fail: Option<&str>) -> Vec<String> {
    let src = match &p.repo {
        Some(r) => format!("{r}#{}", p.branch.as_deref().unwrap_or("main")),
        None => p.image.clone().unwrap_or_default(),
    };
    let mut lines = vec![format!("==> Fetching {src}")];
    if p.repo.is_some() {
        lines.extend(
            [
                "==> Detected Node.js 22 (package.json), using npm",
                "$ npm ci",
                "added 412 packages in 9s",
                "$ npm run build",
                "✓ Compiled successfully",
            ]
            .map(String::from),
        );
    } else {
        lines.push(format!(
            "==> Pulling {}",
            p.image.as_deref().unwrap_or_default()
        ));
        lines.push("image pulled (1.2 GB)".into());
    }
    let on = p
        .size
        .as_deref()
        .or(p.gpu_type.as_deref())
        .unwrap_or_default();
    lines.push(format!("==> Starting {} replica(s) on {on}", p.replicas));
    if ok {
        lines.push(format!("==> Listening on port {}", first_port(p)));
        lines.push("==> Health check passed".into());
        lines.push("==> Live".into());
    } else {
        lines.push(fail.unwrap_or("Error: process exited with code 1").into());
        lines.push(format!(
            "==> Health check failed: no response on port {}",
            first_port(p)
        ));
        lines.push("==> Deployment failed, previous version kept live".into());
    }
    lines
}

fn metrics(p: &Pod) -> Map<String, Value> {
    let mut h: u32 = 0;
    for c in p.id.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    let mem = if p.name.contains("leaky") {
        93
    } else {
        35 + h % 40
    };
    let rps = if p.status == "running" {
        round(2.0 + f64::from(h % 90) / 3.0, 1)
    } else {
        0.0
    };
    let mut m = Map::new();
    m.insert("cpuPercent".into(), json!(18 + h % 50));
    m.insert("memoryPercent".into(), json!(mem));
    m.insert("requestsPerSecond".into(), json!(rps));
    m.insert(
        "errorRatePercent".into(),
        json!(round(f64::from((h >> 3) % 30) / 10.0, 1)),
    );
    m.insert("p95Ms".into(), json!(80 + h % 400));
    m
}

/// Request body fields the simulator reads.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Body {
    name: Option<String>,
    confirm: Option<String>,
    kind: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
    root_directory: Option<String>,
    image: Option<String>,
    size: Option<String>,
    replicas: Option<u32>,
    region: Option<String>,
    ports: Option<Vec<Port>>,
    env: Option<BTreeMap<String, String>>,
    secrets: Option<BTreeMap<String, String>>,
    gpu_type: Option<String>,
    gpu_count: Option<u32>,
    volume_gb: Option<u32>,
    deployment_id: Option<String>,
    hostname: Option<String>,
    engine: Option<String>,
    plan: Option<String>,
}

/// One simulator bound to a state file.
pub struct Sim {
    pub path: PathBuf,
    pub now: fn() -> DateTime<Utc>,
}

impl Sim {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            now: Utc::now,
        }
    }

    /// Deletes the saved state; the next call starts from the seeded project again.
    pub fn reset(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn ts(&self) -> String {
        (self.now)().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    fn load(&self) -> Result<State, ApiError> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| {
                err(
                    500,
                    "DEMO_STATE",
                    format!(
                        "demo state {} is unreadable, run `happypodctl demo reset`: {e}",
                        self.path.display()
                    ),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(self.seed()),
            Err(e) => Err(err(500, "DEMO_STATE", e.to_string())),
        }
    }

    fn save(&self, st: &State) -> Result<(), ApiError> {
        let io = |e: std::io::Error| err(500, "DEMO_STATE", e.to_string());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(st).unwrap_or_default()).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    /// The starting state: a project with a running web pod and a hobby Postgres.
    fn seed(&self) -> State {
        let now = self.ts();
        let web = Pod {
            id: "pod_web".into(),
            project_id: "prj_demo".into(),
            name: "web".into(),
            kind: "app".into(),
            repo: Some("https://github.com/acme/my-saas".into()),
            branch: Some("main".into()),
            size: Some("small".into()),
            replicas: 1,
            region: "us-east".into(),
            ports: vec![Port {
                port: 3000,
                public: Some(true),
            }],
            env: BTreeMap::from([("NODE_ENV".into(), "production".into())]),
            secrets: BTreeMap::from([(
                "DATABASE_URL".into(),
                "postgres://app:s3cret@db.internal:5432/main".into(),
            )]),
            status: "running".into(),
            created_at: now.clone(),
            live_deployment_id: Some("dep_seed".into()),
            ..Default::default()
        };
        let dep = Deployment {
            id: "dep_seed".into(),
            pod_id: web.id.clone(),
            status: "success".into(),
            source: "main@4f2a9c1".into(),
            started_at: now.clone(),
            finished_at: Some(now.clone()),
            log: build_log(&web, true, None),
            will_fail: None,
        };
        State {
            seq: 100,
            projects: vec![Project {
                id: "prj_demo".into(),
                name: "my-saas".into(),
                created_at: now.clone(),
            }],
            pods: vec![web],
            deployments: vec![dep],
            databases: vec![Database {
                id: "db_main".into(),
                project_id: "prj_demo".into(),
                name: "main".into(),
                engine: "postgres".into(),
                plan: "hobby".into(),
                region: "us-east".into(),
                status: "available".into(),
                password: "s3cret".into(),
                created_at: now,
            }],
            idempotency: BTreeMap::new(),
        }
    }

    fn start_deployment(&self, st: &mut State, pod: usize) -> Deployment {
        let p = &st.pods[pod];
        let repo = p.repo.as_deref().unwrap_or_default();
        let image = p.image.as_deref().unwrap_or_default();
        let has_db = p.secrets.contains_key("DATABASE_URL") || p.env.contains_key("DATABASE_URL");
        let will_fail = if repo.contains("needs-db") && !has_db {
            Some("Error: DATABASE_URL is not set (src/db.ts:4)".to_string())
        } else if image.contains("broken") {
            Some("Error: exec /app/start.sh: no such file or directory".to_string())
        } else {
            None
        };
        let source = if p.repo.is_some() {
            format!(
                "{}@{}",
                p.branch.as_deref().unwrap_or("main"),
                random_chars(7, HEX)
            )
        } else {
            image.to_string()
        };
        let pod_id = p.id.clone();
        let d = Deployment {
            id: st.nid("dep"),
            pod_id,
            status: "queued".into(),
            source,
            started_at: self.ts(),
            finished_at: None,
            log: vec![],
            will_fail,
        };
        st.deployments.push(d.clone());
        st.pods[pod].status = "deploying".into();
        d
    }

    /// Moves a deployment one step per read, the way a real build progresses between polls.
    fn advance(&self, st: &mut State, di: usize) {
        let next = match st.deployments[di].status.as_str() {
            "queued" => "building",
            "building" => "deploying",
            "deploying" => "",
            _ => return,
        };
        if !next.is_empty() {
            st.deployments[di].status = next.into();
            return;
        }
        let Ok(pi) = st.pod_idx(&st.deployments[di].pod_id.clone()) else {
            return;
        };
        let now = self.ts();
        let d = &mut st.deployments[di];
        let p = &mut st.pods[pi];
        d.finished_at = Some(now);
        if let Some(f) = d.will_fail.clone() {
            d.status = "failed".into();
            d.log = build_log(p, false, Some(&f));
            p.status = if p.live_deployment_id.is_some() {
                "running"
            } else {
                "failed"
            }
            .into();
        } else {
            d.status = "success".into();
            d.log = build_log(p, true, None);
            p.status = "running".into();
            p.live_deployment_id = Some(d.id.clone());
        }
    }

    fn handle(
        &self,
        st: &mut State,
        method: &str,
        path: &str,
        q: &[(String, String)],
        b: Body,
        key: Option<&str>,
    ) -> Result<(u16, Value), ApiError> {
        let get = |k: &str| qget(q, k);
        let dry = get("dryRun") == Some("true");
        let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let shape: Vec<&str> = seg
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i % 2 == 1 && !FIXED.contains(s) {
                    ":id"
                } else {
                    s
                }
            })
            .collect();
        let route = format!("{method} /{}", shape.join("/"));
        let idem = key.map(|k| format!("{route}|{k}"));
        if let Some(prev) = idem.as_ref().and_then(|k| st.idempotency.get(k)) {
            return Ok((200, prev.clone()));
        }
        let id = |i: usize| seg.get(i).copied().unwrap_or_default().to_string();
        let ok = |v: Value| Ok((200, v));
        let created = |st: &mut State, v: Value| {
            if let Some(k) = &idem {
                st.idempotency.insert(k.clone(), v.clone());
            }
            Ok((201, v))
        };

        match route.as_str() {
            "GET /me" => ok(json!({
                "mode": "demo", "account": "Demo account", "plan": "starter", "creditUsd": 25, "spendThisMonthUsd": 3.42, "keyScope": "all projects",
                "note": "Built-in simulator: nothing here is real and nothing is billed."
            })),
            "GET /regions" => ok(REGIONS.iter().map(|r| json!({ "id": r.0, "city": r.1, "gpu": r.2 })).collect()),
            "GET /sizes" => ok(SIZES
                .iter()
                .map(|s| {
                    let m = money(s.3);
                    json!({ "id": s.0, "vcpu": s.1, "memoryGb": s.2, "hourlyUsd": m["hourlyUsd"], "monthlyUsd": m["monthlyUsd"] })
                })
                .collect()),
            "GET /gpu-types" => {
                let r = get("region");
                ok(GPUS
                    .iter()
                    .filter(|g| r.is_none_or(|r| g.3.contains(&r)))
                    .map(|g| {
                        let m = money(g.2);
                        json!({ "id": g.0, "vramGb": g.1, "hourlyUsd": m["hourlyUsd"], "monthlyUsd": m["monthlyUsd"], "regions": g.3 })
                    })
                    .collect())
            }

            "GET /projects" => ok(st
                .projects
                .iter()
                .map(|p| {
                    let pods = st.pods.iter().filter(|x| x.project_id == p.id).count();
                    let dbs = st.databases.iter().filter(|x| x.project_id == p.id).count();
                    json!({ "id": p.id, "name": p.name, "createdAt": p.created_at, "pods": pods, "databases": dbs })
                })
                .collect()),
            "POST /projects" => {
                let name = b.name.filter(|n| !n.is_empty()).ok_or_else(|| err(422, "VALIDATION", "name is required"))?;
                if st.projects.iter().any(|p| p.name == name) {
                    return Err(err(409, "NAME_TAKEN", format!("A project named {name} already exists")));
                }
                let p = Project { id: st.nid("prj"), name, created_at: self.ts() };
                st.projects.push(p.clone());
                created(st, json!(p))
            }
            "DELETE /projects/:id" => {
                let p = st.project(&id(1))?.clone();
                if b.confirm.as_deref() != Some(&p.name) {
                    return Err(err(422, "CONFIRM_MISMATCH", format!("confirm must be the project name \"{}\"", p.name)));
                }
                if st.pods.iter().any(|x| x.project_id == p.id) || st.databases.iter().any(|x| x.project_id == p.id) {
                    return Err(err(409, "PROJECT_NOT_EMPTY", "Delete its pods and databases first"));
                }
                st.projects.retain(|x| x.id != p.id);
                ok(json!({ "deleted": p.id }))
            }

            "GET /projects/:id/pods" => {
                st.project(&id(1))?;
                ok(st.pods.iter().filter(|p| p.project_id == id(1)).map(|p| st.pod_view(p)).collect())
            }
            "POST /projects/:id/pods" => {
                st.project(&id(1))?;
                let kind = if b.kind.as_deref() == Some("gpu") { "gpu" } else { "app" };
                let region = b.region.clone().unwrap_or_else(|| "us-east".into());
                if !REGIONS.iter().any(|r| r.0 == region) {
                    return Err(err(422, "UNKNOWN_REGION", format!("Unknown region \"{region}\". See list_regions.")));
                }
                if kind == "gpu" {
                    let gt = b.gpu_type.as_deref().unwrap_or_default();
                    let g = GPUS.iter().find(|g| g.0 == gt);
                    if !g.is_some_and(|g| g.3.contains(&region.as_str())) {
                        let try_regions: Vec<&str> = g.map(|g| g.3.to_vec()).unwrap_or_default();
                        return Err(err(409, "NO_GPU_CAPACITY", format!("No {gt} capacity in {region} right now")).with_details(json!({ "tryRegions": try_regions })));
                    }
                }
                let price = pod_price(&PriceIn {
                    kind,
                    size: b.size.as_deref(),
                    replicas: b.replicas.unwrap_or(1),
                    gpu_type: b.gpu_type.as_deref(),
                    gpu_count: b.gpu_count.unwrap_or(1),
                    volume_gb: b.volume_gb.unwrap_or(20),
                })?;
                let name = b.name.clone().unwrap_or_default();
                if st.pods.iter().any(|p| p.project_id == id(1) && p.name == name) {
                    return Err(err(409, "NAME_TAKEN", format!("A pod named {name} already exists in this project")));
                }
                if dry {
                    let note = if kind == "gpu" { "Billed per hour while running. Stop it when the job is done." } else { "Billed per hour while running." };
                    return ok(json!({ "dryRun": true, "kind": kind, "price": price, "note": note }));
                }
                let gpu = kind == "gpu";
                let pod = Pod {
                    id: st.nid("pod"),
                    project_id: id(1),
                    name,
                    kind: kind.into(),
                    repo: b.repo,
                    branch: b.branch,
                    root_directory: b.root_directory,
                    image: b.image,
                    size: if gpu { None } else { Some(b.size.unwrap_or_else(|| "small".into())) },
                    replicas: b.replicas.unwrap_or(1),
                    region,
                    ports: b.ports.filter(|p| !p.is_empty()).unwrap_or_else(|| vec![Port { port: if gpu { 8000 } else { 3000 }, public: Some(true) }]),
                    env: b.env.unwrap_or_default(),
                    secrets: b.secrets.unwrap_or_default(),
                    gpu_type: b.gpu_type,
                    gpu_count: gpu.then(|| b.gpu_count.unwrap_or(1)),
                    volume_gb: gpu.then(|| b.volume_gb.unwrap_or(20)),
                    status: "deploying".into(),
                    domains: vec![],
                    created_at: self.ts(),
                    live_deployment_id: None,
                };
                st.pods.push(pod);
                let pi = st.pods.len() - 1;
                let d = self.start_deployment(st, pi);
                let view = st.pod_view(&st.pods[pi]);
                created(st, json!({ "pod": view, "deployment": { "id": d.id, "status": d.status }, "price": price }))
            }

            r if r.starts_with("GET /pods/:id")
                || r.starts_with("POST /pods/:id")
                || r.starts_with("PUT /pods/:id")
                || r.starts_with("PATCH /pods/:id")
                || r.starts_with("DELETE /pods/:id") =>
            {
                let pi = st.pod_idx(&id(1))?;
                let res = self.pod_route(st, r, pi, &seg, b, q, dry)?;
                if res.0 == 201 { created(st, res.1) } else { Ok(res) }
            }

            "GET /deployments/:id" => {
                let di = st.dep_idx(&id(1))?;
                self.advance(st, di);
                let d = &st.deployments[di];
                ok(json!({ "id": d.id, "podId": d.pod_id, "status": d.status, "source": d.source, "startedAt": d.started_at, "finishedAt": d.finished_at }))
            }
            "GET /deployments/:id/logs" => {
                let d = &st.deployments[st.dep_idx(&id(1))?];
                let tail = get("tail").and_then(|t| t.parse::<usize>().ok()).unwrap_or(200);
                let lines = if d.log.is_empty() { vec![format!("==> {}…", d.status)] } else { d.log.clone() };
                let from = lines.len().saturating_sub(tail);
                ok(json!({ "deploymentId": d.id, "status": d.status, "lines": lines[from..] }))
            }

            "GET /projects/:id/databases" => {
                st.project(&id(1))?;
                ok(st.databases.iter().filter(|d| d.project_id == id(1)).map(db_view).collect())
            }
            "POST /projects/:id/databases" => {
                st.project(&id(1))?;
                let plan = b.plan.clone().unwrap_or_else(|| "hobby".into());
                let hourly = DB_PLANS.iter().find(|p| p.0 == plan).map(|p| p.1).ok_or_else(|| err(422, "UNKNOWN_PLAN", format!("Unknown plan {plan}")))?;
                let price = money(hourly);
                if dry {
                    return ok(json!({ "dryRun": true, "engine": b.engine, "plan": plan, "price": price }));
                }
                let d = Database {
                    id: st.nid("db"),
                    project_id: id(1),
                    name: b.name.unwrap_or_default(),
                    engine: b.engine.unwrap_or_default(),
                    plan,
                    region: b.region.unwrap_or_else(|| "us-east".into()),
                    status: "available".into(),
                    password: random_chars(12, ALNUM),
                    created_at: self.ts(),
                };
                let mut v = db_view(&d);
                v["price"] = price;
                st.databases.push(d);
                created(st, v)
            }
            "GET /databases/:id/connection" => {
                let d = &st.databases[st.db_idx(&id(1))?];
                let reveal = get("reveal") == Some("true");
                let pw = if reveal { d.password.as_str() } else { "••••••" };
                let host = format!("{}.{}.db.happypod.local", d.name, d.region);
                if d.engine == "postgres" {
                    ok(json!({ "host": host, "port": 5432, "user": "app", "database": d.name, "url": format!("postgres://app:{pw}@{host}:5432/{}", d.name), "revealed": reveal }))
                } else {
                    ok(json!({ "host": host, "port": 6379, "user": "default", "database": null, "url": format!("redis://default:{pw}@{host}:6379"), "revealed": reveal }))
                }
            }
            "DELETE /databases/:id" => {
                let d = st.databases[st.db_idx(&id(1))?].clone();
                if b.confirm.as_deref() != Some(&d.name) {
                    return Err(err(422, "CONFIRM_MISMATCH", format!("confirm must be the database name \"{}\"", d.name)));
                }
                st.databases.retain(|x| x.id != d.id);
                ok(json!({ "deleted": d.id }))
            }
            _ => Err(err(404, "NO_ROUTE", format!("{method} {path} is not part of the Happy Pod API"))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pod_route(
        &self,
        st: &mut State,
        route: &str,
        pi: usize,
        seg: &[&str],
        b: Body,
        q: &[(String, String)],
        dry: bool,
    ) -> Result<(u16, Value), ApiError> {
        let get = |k: &str| qget(q, k);
        let ok = |v: Value| Ok((200, v));
        let pod_id = st.pods[pi].id.clone();
        match route {
            "GET /pods/:id" => ok(st.pod_view(&st.pods[pi])),
            "DELETE /pods/:id" => {
                let name = st.pods[pi].name.clone();
                if b.confirm.as_deref() != Some(&name) {
                    return Err(err(
                        422,
                        "CONFIRM_MISMATCH",
                        format!("confirm must be the pod name \"{name}\""),
                    ));
                }
                st.pods.remove(pi);
                st.deployments.retain(|d| d.pod_id != pod_id);
                ok(json!({ "deleted": pod_id }))
            }
            "POST /pods/:id/deployments" => {
                let d = self.start_deployment(st, pi);
                Ok((
                    201,
                    json!({ "id": d.id, "status": d.status, "source": d.source }),
                ))
            }
            "GET /pods/:id/deployments" => {
                let limit = get("limit")
                    .and_then(|l| l.parse::<usize>().ok())
                    .unwrap_or(20);
                let live = st.pods[pi].live_deployment_id.clone();
                ok(st
                    .deployments
                    .iter()
                    .rev()
                    .filter(|d| d.pod_id == pod_id)
                    .take(limit)
                    .map(|d| json!({ "id": d.id, "status": d.status, "source": d.source, "startedAt": d.started_at, "finishedAt": d.finished_at, "live": live.as_deref() == Some(&d.id) }))
                    .collect())
            }
            "POST /pods/:id/rollback" => {
                let did = b.deployment_id.unwrap_or_default();
                let d = st.deployments[st.dep_idx(&did)?].clone();
                if d.pod_id != pod_id || d.status != "success" {
                    return Err(err(
                        409,
                        "NOT_ROLLBACKABLE",
                        "Only an earlier successful deployment of this pod can be rolled back to",
                    ));
                }
                let p = &mut st.pods[pi];
                p.live_deployment_id = Some(d.id.clone());
                p.status = "running".into();
                ok(json!({ "live": d.id, "source": d.source }))
            }
            "POST /pods/:id/restart" => {
                let p = &st.pods[pi];
                if p.status != "running" {
                    return Err(err(409, "NOT_RUNNING", format!("Pod is {}", p.status)));
                }
                ok(json!({ "restarted": p.id, "replicas": p.replicas }))
            }
            "POST /pods/:id/stop" => {
                st.pods[pi].status = "stopped".into();
                ok(json!({ "id": pod_id, "status": "stopped" }))
            }
            "POST /pods/:id/start" => {
                let p = &mut st.pods[pi];
                if p.status == "stopped" {
                    p.status = "running".into();
                }
                ok(json!({ "id": pod_id, "status": p.status }))
            }
            "PATCH /pods/:id/scale" => {
                let p = &st.pods[pi];
                if p.kind == "gpu" && b.size.is_some() {
                    return Err(err(
                        422,
                        "GPU_POD",
                        "GPU pods are resized by creating a new pod with another gpuType",
                    ));
                }
                let size = b.size.or_else(|| p.size.clone());
                let replicas = b.replicas.unwrap_or(p.replicas);
                let cur = price_of(p)?;
                let new = pod_price(&PriceIn {
                    kind: &p.kind,
                    size: size.as_deref(),
                    replicas,
                    gpu_type: p.gpu_type.as_deref(),
                    gpu_count: p.gpu_count.unwrap_or(1),
                    volume_gb: p.volume_gb.unwrap_or(20),
                })?;
                if dry {
                    return ok(json!({
                        "dryRun": true,
                        "current": { "size": p.size, "replicas": p.replicas, "hourlyUsd": cur["hourlyUsd"], "monthlyUsd": cur["monthlyUsd"] },
                        "new": { "size": size, "replicas": replicas, "hourlyUsd": new["hourlyUsd"], "monthlyUsd": new["monthlyUsd"] }
                    }));
                }
                let p = &mut st.pods[pi];
                p.size = size;
                p.replicas = replicas;
                ok(json!({ "id": pod_id, "size": p.size, "replicas": p.replicas, "price": new }))
            }
            "GET /pods/:id/logs" => {
                let p = &st.pods[pi];
                let level = get("level").unwrap_or("all");
                let base = [
                    ("info", "GET / 200 12ms"),
                    ("info", "GET /api/items 200 48ms"),
                    (
                        "warn",
                        "slow query: SELECT * FROM orders WHERE user_id = $1 (1840ms)",
                    ),
                    (
                        "error",
                        "TypeError: Cannot read properties of undefined (reading 'email') at /app/src/routes/profile.ts:27",
                    ),
                    ("info", "POST /api/checkout 200 1912ms"),
                ];
                let now = (self.now)();
                let lines: Vec<Value> = base
                    .iter()
                    .enumerate()
                    .filter(|(_, (l, _))| level == "all" || *l == level || (level == "warn" && *l == "error"))
                    .map(|(i, (l, m))| {
                        let at = now - chrono::Duration::minutes(5 - i as i64);
                        json!({ "at": at.to_rfc3339_opts(SecondsFormat::Millis, true), "level": l, "replica": format!("{}-0", p.name), "message": m })
                    })
                    .collect();
                ok(json!({ "podId": p.id, "since": get("since").unwrap_or("1h"), "lines": lines }))
            }
            "GET /pods/:id/metrics" => {
                let p = &st.pods[pi];
                let mut m = Map::new();
                m.insert("podId".into(), json!(p.id));
                m.insert("window".into(), json!(get("window").unwrap_or("1h")));
                m.insert("replicas".into(), json!(p.replicas));
                m.extend(metrics(p));
                ok(Value::Object(m))
            }
            "POST /pods/:id/diagnose" => {
                let p = &st.pods[pi];
                let m = metrics(p);
                let mut findings = vec![];
                if let Some(d) = st.last_deployment(&p.id).filter(|d| d.status == "failed") {
                    let evidence = d
                        .log
                        .iter()
                        .find(|l| l.starts_with("Error"))
                        .cloned()
                        .unwrap_or_else(|| "see get_deployment_logs".into());
                    let fix = if d.log.join(",").contains("DATABASE_URL") {
                        "Create or reuse a database, pass its URL with set_env secrets.DATABASE_URL, then deploy_pod"
                    } else {
                        "Read get_deployment_logs, fix the start command or Dockerfile, deploy_pod again"
                    };
                    findings.push(json!({ "severity": "critical", "title": "Last deployment failed", "evidence": evidence, "fix": fix, "deploymentId": d.id }));
                }
                let num = |k: &str| m[k].as_f64().unwrap_or_default();
                if num("memoryPercent") > 85.0 {
                    findings.push(json!({ "severity": "high", "title": "Memory nearly full", "evidence": format!("memory {}% of {}", m["memoryPercent"], p.size.as_deref().unwrap_or_default()), "fix": "scale_pod to the next size (dryRun first), then look for a memory leak" }));
                }
                if num("p95Ms") > 400.0 {
                    findings.push(json!({ "severity": "medium", "title": "Slow responses", "evidence": format!("p95 {} ms; slow query on orders.user_id in the logs", m["p95Ms"]), "fix": "Add an index on orders.user_id in the app's migrations, then deploy_pod" }));
                }
                if num("errorRatePercent") > 2.0 {
                    findings.push(json!({ "severity": "medium", "title": "Elevated error rate", "evidence": format!("{}% of requests fail; TypeError in profile.ts:27", m["errorRatePercent"]), "fix": "Guard the undefined user in src/routes/profile.ts, then deploy_pod" }));
                }
                ok(
                    json!({ "podId": p.id, "healthy": findings.is_empty(), "findings": findings, "metrics": m }),
                )
            }
            "GET /pods/:id/env" => {
                let p = &st.pods[pi];
                let masked: BTreeMap<&String, &str> =
                    p.secrets.keys().map(|k| (k, "••••••")).collect();
                ok(json!({ "env": p.env, "secrets": masked }))
            }
            "PUT /pods/:id/env" => {
                let p = &mut st.pods[pi];
                p.env.extend(b.env.unwrap_or_default());
                for (k, v) in b.secrets.unwrap_or_default() {
                    p.env.remove(&k);
                    p.secrets.insert(k, v);
                }
                ok(
                    json!({ "env": p.env.keys().collect::<Vec<_>>(), "secrets": p.secrets.keys().collect::<Vec<_>>(), "note": "Applies on the next deploy: call deploy_pod." }),
                )
            }
            "DELETE /pods/:id/env/:id" => {
                let key = seg.get(3).copied().unwrap_or_default().to_string();
                let p = &mut st.pods[pi];
                p.env.remove(&key);
                p.secrets.remove(&key);
                ok(json!({ "removed": key }))
            }
            "GET /pods/:id/domains" => ok(json!(st.pods[pi].domains)),
            "POST /pods/:id/domains" => {
                let host = b.hostname.unwrap_or_default().to_lowercase();
                let target = format!(
                    "{}-{}.demo.happypod.local",
                    st.pods[pi].name,
                    st.project_name(&st.pods[pi].project_id)
                );
                let dns = if host.matches('.').count() == 1 {
                    json!({ "type": "A", "name": host, "value": "203.0.113.10" })
                } else {
                    json!({ "type": "CNAME", "name": host, "value": target })
                };
                let dom = json!({ "hostname": host, "status": "pending_dns", "dns": dns });
                st.pods[pi].domains.push(dom.clone());
                Ok((201, dom))
            }
            _ => Err(err(
                404,
                "NO_ROUTE",
                format!("{route} is not part of the Happy Pod API"),
            )),
        }
    }
}

fn qget<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
    q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn db_view(d: &Database) -> Value {
    json!({ "id": d.id, "projectId": d.project_id, "name": d.name, "engine": d.engine, "plan": d.plan, "region": d.region, "status": d.status, "createdAt": d.created_at })
}

impl Transport for Sim {
    fn send(&self, _base: &str, _key: &str, _ua: &str, req: &Request) -> Result<Response, String> {
        let reply = |status: u16, v: Value| {
            Ok(Response {
                status,
                body: v.to_string(),
            })
        };
        let body: Body = match &req.body {
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(b) => b,
                Err(e) => {
                    return reply(
                        400,
                        json!({ "error": { "code": "BAD_JSON", "message": e.to_string() } }),
                    );
                }
            },
            None => Body::default(),
        };
        let mut st = match self.load() {
            Ok(s) => s,
            Err(e) => {
                return reply(
                    e.status,
                    json!({ "error": { "code": e.code, "message": e.message } }),
                );
            }
        };
        let method = req.method.to_uppercase();
        let path = req.path.strip_prefix("/v1").unwrap_or(&req.path);
        match self.handle(
            &mut st,
            &method,
            path,
            &req.query,
            body,
            req.idempotency_key.as_deref(),
        ) {
            Ok((status, data)) => {
                // Reads change nothing, except a deployment read, which moves the build along.
                if method != "GET" || path.starts_with("/deployments/") {
                    if let Err(e) = self.save(&st) {
                        return reply(
                            e.status,
                            json!({ "error": { "code": e.code, "message": e.message } }),
                        );
                    }
                }
                reply(status, json!({ "data": data }))
            }
            Err(e) => {
                let mut o = json!({ "code": e.code, "message": e.message });
                if let Some(d) = e.details {
                    o["details"] = *d;
                }
                reply(e.status, json!({ "error": o }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Client, Query};

    fn client() -> (Client, PathBuf) {
        let dir = std::env::temp_dir().join(format!("happypodctl-demo-{}", uuid::Uuid::new_v4()));
        let path = dir.join("demo.json");
        (Client::new(URL, KEY, Box::new(Sim::new(path.clone()))), dir)
    }

    #[test]
    fn seeded_project_is_there() {
        let (c, dir) = client();
        let v = c.get("/projects", Query::new()).unwrap();
        assert_eq!(v[0]["id"], "prj_demo");
        assert_eq!(v[0]["pods"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deployment_moves_one_step_per_read_and_state_persists() {
        let (c, dir) = client();
        let body = json!({ "kind": "app", "name": "api", "repo": "https://github.com/acme/api" });
        let r = c
            .send(
                "POST",
                "/projects/prj_demo/pods",
                Query::new(),
                Some(body),
                None,
            )
            .unwrap();
        let dep = r["deployment"]["id"].as_str().unwrap().to_string();
        let seen: Vec<String> = (0..4)
            .map(|_| {
                c.get(format!("/deployments/{dep}"), Query::new()).unwrap()["status"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(seen, ["building", "deploying", "success", "success"]);
        let pod = r["pod"]["id"].as_str().unwrap();
        assert_eq!(
            c.get(format!("/pods/{pod}"), Query::new()).unwrap()["status"],
            "running"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_database_fails_the_build() {
        let (c, dir) = client();
        let body =
            json!({ "kind": "app", "name": "shop", "repo": "https://github.com/acme/needs-db" });
        let r = c
            .send(
                "POST",
                "/projects/prj_demo/pods",
                Query::new(),
                Some(body),
                None,
            )
            .unwrap();
        let dep = r["deployment"]["id"].as_str().unwrap().to_string();
        for _ in 0..3 {
            c.get(format!("/deployments/{dep}"), Query::new()).unwrap();
        }
        let logs = c
            .get(format!("/deployments/{dep}/logs"), Query::new())
            .unwrap();
        assert_eq!(logs["status"], "failed");
        assert!(
            logs["lines"]
                .to_string()
                .contains("DATABASE_URL is not set")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dry_run_changes_nothing_and_idempotency_key_dedupes() {
        let (c, dir) = client();
        let body = json!({ "name": "cache", "engine": "redis", "plan": "standard" });
        let dry = c
            .send(
                "POST",
                "/projects/prj_demo/databases",
                Query::new().flag("dryRun", true),
                Some(body.clone()),
                None,
            )
            .unwrap();
        assert_eq!(dry["price"]["monthlyUsd"], 21.9);
        let a = c
            .send(
                "POST",
                "/projects/prj_demo/databases",
                Query::new(),
                Some(body.clone()),
                Some("k1".into()),
            )
            .unwrap();
        let b = c
            .send(
                "POST",
                "/projects/prj_demo/databases",
                Query::new(),
                Some(body),
                Some("k1".into()),
            )
            .unwrap();
        assert_eq!(a["id"], b["id"]);
        assert_eq!(
            c.get("/projects/prj_demo/databases", Query::new())
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gpu_without_capacity_suggests_regions() {
        let (c, dir) = client();
        let body = json!({ "kind": "gpu", "name": "llm", "image": "vllm/vllm-openai", "gpuType": "a100-80gb", "region": "eu-west" });
        let e = c
            .send(
                "POST",
                "/projects/prj_demo/pods",
                Query::new(),
                Some(body),
                None,
            )
            .unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (409, "NO_GPU_CAPACITY"));
        assert_eq!(e.details.unwrap()["tryRegions"], json!(["us-east"]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_needs_the_exact_name() {
        let (c, dir) = client();
        let e = c
            .send(
                "DELETE",
                "/databases/db_main",
                Query::new(),
                Some(json!({ "confirm": "nope" })),
                None,
            )
            .unwrap_err();
        assert_eq!(e.code, "CONFIRM_MISMATCH");
        c.send(
            "DELETE",
            "/databases/db_main",
            Query::new(),
            Some(json!({ "confirm": "main" })),
            None,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
