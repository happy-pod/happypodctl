//! Command line shape. Help text is lowercase and short, like the rest of the output.

use crate::output::Format;
use clap::{Args, Parser, Subcommand};

const ABOUT: &str =
    "happypodctl - ship, scale, watch and fix apps, gpu pods and databases on happy pod.";
const AFTER: &str = "getting started:
  1. create an api key at https://happypod.io (settings > api keys)
  2. run: happypodctl doctor   (asks for the key and saves it)
     or:  export HAPPYPOD_API_KEY=hp_...
  try it with no account: happypodctl --demo pod list

output is json unless you pass -o table. errors are one json object on stderr.
exit codes: 0 ok, 1 api error, 2 usage, 3 auth, 4 not found, 5 conflict, 6 deployment failed or wait timed out.
docs: https://docs.happypod.io";

#[derive(Parser, Debug)]
#[command(name = "happypodctl", version, about = ABOUT, after_help = AFTER, propagate_version = true)]
pub struct Cli {
    /// output format
    #[arg(short, long, global = true, value_enum, default_value_t = Format::Json)]
    pub output: Format,
    /// use the built-in simulator: nothing is real, nothing is billed (or HAPPYPOD_MODE=demo)
    #[arg(long, global = true)]
    pub demo: bool,
    /// api base url (or HAPPYPOD_API_URL)
    #[arg(long, global = true, value_name = "URL")]
    pub api_url: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// show the account behind the api key: plan, credit and spend
    #[command(visible_alias = "me")]
    Whoami,
    /// list regions where pods and databases run
    #[command(subcommand)]
    Region(ListOnly),
    /// list cpu pod sizes and prices
    #[command(subcommand)]
    Size(ListOnly),
    /// list gpu types, prices and regions with capacity
    #[command(subcommand)]
    Gpu(GpuCmd),
    /// manage projects
    #[command(subcommand, visible_alias = "projects")]
    Project(ProjectCmd),
    /// ship and operate app pods and gpu pods
    #[command(subcommand, visible_alias = "pods")]
    Pod(PodCmd),
    /// inspect deployments of a pod
    #[command(subcommand, visible_aliases = ["deployments", "dep"])]
    Deployment(DeploymentCmd),
    /// manage a pod's environment variables and secrets
    #[command(subcommand)]
    Env(EnvCmd),
    /// manage custom domains
    #[command(subcommand, visible_alias = "domains")]
    Domain(DomainCmd),
    /// manage postgres and redis databases
    #[command(subcommand, visible_aliases = ["database", "databases"])]
    Db(DbCmd),
    /// check the setup and save an api key
    Doctor(DoctorArgs),
    /// manage the built-in simulator
    #[command(subcommand)]
    Demo(DemoCmd),
}

#[derive(Subcommand, Debug)]
pub enum ListOnly {
    /// list them
    #[command(visible_alias = "ls")]
    List,
}

#[derive(Subcommand, Debug)]
pub enum GpuCmd {
    /// list gpu types
    #[command(visible_alias = "ls")]
    List {
        /// only gpus with capacity in this region
        #[arg(long)]
        region: Option<String>,
    },
}

#[derive(Args, Debug, Clone, Default)]
pub struct ProjectArg {
    /// project id (or HAPPYPOD_PROJECT, or the saved default)
    #[arg(short, long, value_name = "ID")]
    pub project: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct Paid {
    /// return the price only and change nothing
    #[arg(long)]
    pub dry_run: bool,
    /// a retry with the same key returns the first result instead of doing it twice
    #[arg(long, value_name = "KEY")]
    pub idempotency_key: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct Wait {
    /// wait until the deployment succeeds or fails
    #[arg(long)]
    pub wait: bool,
    /// give up waiting after this long (e.g. 90s, 10m)
    #[arg(long, value_name = "DURATION", default_value = "15m")]
    pub wait_timeout: String,
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// list projects with their pod and database counts
    #[command(visible_alias = "ls")]
    List,
    /// create an empty project (free)
    Create {
        name: String,
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,
    },
    /// delete an empty project. irreversible
    #[command(visible_alias = "rm")]
    Delete {
        id: String,
        /// the project's exact name
        #[arg(long, value_name = "NAME", required = true)]
        confirm: String,
    },
    /// save a default project for later commands
    Use { id: String },
}

#[derive(Subcommand, Debug)]
pub enum PodCmd {
    /// list pods in a project
    #[command(visible_alias = "ls")]
    List(ProjectArg),
    /// show one pod
    Get { id: String },
    /// create an app pod (--repo or --image) or a gpu pod (--gpu-type) and start its first deployment
    Create(Box<PodCreate>),
    /// build and roll out the pod again from its source
    Deploy {
        id: String,
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,
        #[command(flatten)]
        wait: Wait,
    },
    /// start a stopped pod. billing resumes
    Start { id: String },
    /// stop a pod. compute billing stops, volumes are kept
    Stop { id: String },
    /// restart the running replicas without rebuilding
    Restart { id: String },
    /// change size and/or replica count
    Scale {
        id: String,
        #[arg(long)]
        size: Option<String>,
        #[arg(long)]
        replicas: Option<u32>,
        /// return the current and new price only
        #[arg(long)]
        dry_run: bool,
    },
    /// put an earlier successful deployment back live (no rebuild)
    Rollback { id: String, deployment_id: String },
    /// delete a pod with its deployments, domains and volumes. irreversible
    #[command(visible_alias = "rm")]
    Delete {
        id: String,
        /// the pod's exact name
        #[arg(long, value_name = "NAME", required = true)]
        confirm: String,
    },
    /// runtime logs across replicas
    Logs {
        id: String,
        /// e.g. 15m, 1h, 2d
        #[arg(long)]
        since: Option<String>,
        #[arg(long, value_parser = ["error", "warn", "info", "all"])]
        level: Option<String>,
        #[arg(long)]
        tail: Option<u32>,
    },
    /// cpu, memory, requests, error rate and p95 latency
    Metrics {
        id: String,
        #[arg(long, value_parser = ["15m", "1h", "6h", "24h", "7d"])]
        window: Option<String>,
    },
    /// health check with findings, evidence and a suggested fix for each
    Diagnose { id: String },
}

#[derive(Args, Debug)]
pub struct PodCreate {
    /// lowercase name, used in the default url
    pub name: String,
    #[command(flatten)]
    pub project: ProjectArg,
    /// git repository url (https)
    #[arg(long, conflicts_with = "image")]
    pub repo: Option<String>,
    /// branch to build (default: the repository's default branch)
    #[arg(long, requires = "repo")]
    pub branch: Option<String>,
    /// sub-folder for monorepos
    #[arg(long, requires = "repo", value_name = "DIR")]
    pub root_dir: Option<String>,
    /// container image, e.g. ghcr.io/acme/api:1.4
    #[arg(long)]
    pub image: Option<String>,
    /// cpu size from `happypodctl size list` (default small)
    #[arg(long, conflicts_with = "gpu_type")]
    pub size: Option<String>,
    #[arg(long, conflicts_with = "gpu_type")]
    pub replicas: Option<u32>,
    /// makes it a gpu pod; a type from `happypodctl gpu list`. needs --image
    #[arg(long, requires = "image")]
    pub gpu_type: Option<String>,
    #[arg(long, requires = "gpu_type")]
    pub gpu_count: Option<u32>,
    /// persistent volume at /workspace in gb (default 20)
    #[arg(long, requires = "gpu_type", value_name = "GB")]
    pub volume_gb: Option<u32>,
    #[arg(long)]
    pub region: Option<String>,
    /// port the app listens on; add :private to skip the https url. repeatable
    #[arg(long = "port", value_name = "PORT[:private]")]
    pub ports: Vec<String>,
    /// plain variable. repeatable
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,
    /// secret variable, write-only. repeatable
    #[arg(long = "secret", value_name = "KEY=VALUE")]
    pub secrets: Vec<String>,
    #[command(flatten)]
    pub paid: Paid,
    #[command(flatten)]
    pub wait: Wait,
}

#[derive(Subcommand, Debug)]
pub enum DeploymentCmd {
    /// a pod's deployments, newest first
    #[command(visible_alias = "ls")]
    List {
        pod_id: String,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// one deployment's status
    Get { id: String },
    /// build and start-up log
    Logs {
        id: String,
        #[arg(long)]
        tail: Option<u32>,
    },
    /// wait until a deployment succeeds or fails
    Wait {
        id: String,
        #[arg(long, value_name = "DURATION", default_value = "15m")]
        timeout: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum EnvCmd {
    /// list variables; secrets come back masked
    #[command(visible_alias = "ls")]
    List { pod_id: String },
    /// create or update variables. applies on the next deploy
    Set {
        pod_id: String,
        /// plain variables
        #[arg(value_name = "KEY=VALUE")]
        vars: Vec<String>,
        /// secret variable, write-only. repeatable
        #[arg(long = "secret", value_name = "KEY=VALUE")]
        secrets: Vec<String>,
    },
    /// remove one variable. applies on the next deploy
    Unset { pod_id: String, key: String },
}

#[derive(Subcommand, Debug)]
pub enum DomainCmd {
    /// a pod's domains with dns and certificate status
    #[command(visible_alias = "ls")]
    List { pod_id: String },
    /// attach a custom domain; prints the dns record to create
    Add { pod_id: String, hostname: String },
}

#[derive(Subcommand, Debug)]
pub enum DbCmd {
    /// databases in a project
    #[command(visible_alias = "ls")]
    List(ProjectArg),
    /// create a managed postgres or redis database
    Create {
        name: String,
        #[command(flatten)]
        project: ProjectArg,
        #[arg(long, value_parser = ["postgres", "redis"])]
        engine: String,
        /// hobby is free
        #[arg(long, value_parser = ["hobby", "standard", "pro"])]
        plan: Option<String>,
        #[arg(long)]
        region: Option<String>,
        #[command(flatten)]
        paid: Paid,
    },
    /// host, port, user and connection url (password masked unless --reveal)
    Connection {
        id: String,
        /// include the real password. pass it on with `env set --secret`, do not print it
        #[arg(long)]
        reveal: bool,
    },
    /// delete a database with all its data and backups. irreversible
    #[command(visible_alias = "rm")]
    Delete {
        id: String,
        /// the database's exact name
        #[arg(long, value_name = "NAME", required = true)]
        confirm: String,
    },
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// read an api key from stdin and save it (instead of the prompt)
    #[arg(long)]
    pub api_key_stdin: bool,
}

#[derive(Subcommand, Debug)]
pub enum DemoCmd {
    /// forget everything done in demo mode and start from the sample project again
    Reset,
}
