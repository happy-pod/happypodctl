//! End-to-end runs of the built binary against the demo simulator. Each test gets its own
//! HAPPYPOD_HOME, so the simulator state and config never leak between tests or into ~/.happypod.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("happypodctl-test-{}", uuid_like()));
        std::fs::create_dir_all(&p).unwrap();
        Home(p)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_happypodctl"))
            .args(args)
            .env("HAPPYPOD_HOME", &self.0)
            .env_remove("HAPPYPOD_API_KEY")
            .env_remove("HAPPYPOD_API_URL")
            .env_remove("HAPPYPOD_MODE")
            .env_remove("HAPPYPOD_PROJECT")
            .output()
            .unwrap()
    }

    /// Runs in demo mode and expects success; returns stdout as JSON.
    fn ok(&self, args: &[&str]) -> Value {
        let mut all = vec!["--demo"];
        all.extend_from_slice(args);
        let out = self.run(&all);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "{args:?} stdout is not json ({e}): {}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }

    /// Runs in demo mode and expects failure; returns (exit code, stderr JSON).
    fn fail(&self, args: &[&str]) -> (i32, Value) {
        let mut all = vec!["--demo"];
        all.extend_from_slice(args);
        let out = self.run(&all);
        assert!(!out.status.success(), "{args:?} should fail");
        (
            out.status.code().unwrap(),
            serde_json::from_slice(&out.stderr).unwrap_or(Value::Null),
        )
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uuid_like() -> String {
    format!("{}-{:?}", std::process::id(), std::thread::current().id())
        .replace(['(', ')'], "")
        .to_lowercase()
        + &std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
}

#[test]
fn whoami_says_demo() {
    let h = Home::new();
    assert_eq!(h.ok(&["whoami"])["mode"], "demo");
}

#[test]
fn ship_a_repo_and_wait_until_live() {
    let h = Home::new();
    let dry = h.ok(&[
        "pod",
        "create",
        "api",
        "--repo",
        "https://github.com/acme/api",
        "--size",
        "medium",
        "--replicas",
        "2",
        "--dry-run",
    ]);
    assert_eq!(dry["dryRun"], true);
    assert_eq!(dry["price"]["monthlyUsd"], 58.4);
    assert_eq!(
        h.ok(&["pod", "list"]).as_array().unwrap().len(),
        1,
        "a dry run creates nothing"
    );

    let v = h.ok(&[
        "pod",
        "create",
        "api",
        "--repo",
        "https://github.com/acme/api",
        "--env",
        "NODE_ENV=production",
        "--wait",
    ]);
    assert_eq!(v["deployment"]["status"], "success");
    assert_eq!(v["pod"]["status"], "running");
    assert_eq!(v["pod"]["url"], "https://api-my-saas.demo.happypod.local");
    assert_eq!(h.ok(&["pod", "list"]).as_array().unwrap().len(), 2);
}

#[test]
fn failed_build_exits_6_with_logs_and_ids() {
    let h = Home::new();
    let (code, err) = h.fail(&[
        "pod",
        "create",
        "shop",
        "--repo",
        "https://github.com/acme/needs-db",
        "--wait",
    ]);
    assert_eq!(code, 6);
    assert_eq!(err["code"], "DEPLOYMENT_FAILED");
    assert!(
        err["details"]["podId"]
            .as_str()
            .unwrap()
            .starts_with("pod_"),
        "the billed pod's id must be in the error"
    );
    assert!(
        err["details"]["logs"]
            .to_string()
            .contains("DATABASE_URL is not set")
    );

    // Fix it the way the logs say, then deploy again.
    let pod = err["details"]["podId"].as_str().unwrap().to_string();
    let conn = h.ok(&["db", "connection", "db_main", "--reveal"]);
    let secret = format!("DATABASE_URL={}", conn["url"].as_str().unwrap());
    h.ok(&["env", "set", &pod, "--secret", &secret]);
    assert_eq!(
        h.ok(&["pod", "deploy", &pod, "--wait"])["status"],
        "success"
    );
    assert_eq!(
        h.ok(&["env", "list", &pod])["secrets"]["DATABASE_URL"],
        "••••••"
    );
}

#[test]
fn deletes_need_the_exact_name() {
    let h = Home::new();
    let (code, err) = h.fail(&["db", "delete", "db_main", "--confirm", "wrong"]);
    assert_eq!((code, err["code"].as_str()), (1, Some("CONFIRM_MISMATCH")));
    assert_eq!(
        h.ok(&["db", "delete", "db_main", "--confirm", "main"])["deleted"],
        "db_main"
    );
    let (code, _) = h.fail(&["db", "connection", "db_main"]);
    assert_eq!(code, 4);
}

#[test]
fn gpu_pod_without_capacity_exits_5() {
    let h = Home::new();
    let (code, err) = h.fail(&[
        "pod",
        "create",
        "llm",
        "--image",
        "vllm/vllm-openai:latest",
        "--gpu-type",
        "a100-80gb",
        "--region",
        "eu-west",
    ]);
    assert_eq!(code, 5);
    assert_eq!(err["details"]["tryRegions"][0], "us-east");
}

#[test]
fn usage_errors_exit_2() {
    let h = Home::new();
    let (code, err) = h.fail(&["pod", "create", "x"]);
    assert_eq!((code, err["code"].as_str()), (2, Some("USAGE")));
    let (code, _) = h.fail(&["pod", "delete", "pod_web"]);
    assert_eq!(code, 2, "clap rejects a delete without --confirm");
    let (code, _) = h.fail(&["env", "set", "pod_web", "lower=case"]);
    assert_eq!(code, 2);
}

#[test]
fn rollback_and_scale() {
    let h = Home::new();
    h.ok(&["pod", "deploy", "pod_web", "--wait"]);
    let deps = h.ok(&["deployment", "list", "pod_web"]);
    assert_eq!(deps[0]["live"], true);
    assert_eq!(
        h.ok(&["pod", "rollback", "pod_web", "dep_seed"])["live"],
        "dep_seed"
    );
    let s = h.ok(&["pod", "scale", "pod_web", "--replicas", "3", "--dry-run"]);
    assert_eq!(
        (
            s["current"]["replicas"].as_u64(),
            s["new"]["replicas"].as_u64()
        ),
        (Some(1), Some(3))
    );
    assert_eq!(
        h.ok(&["pod", "get", "pod_web"])["replicas"],
        1,
        "a dry run changes nothing"
    );
}

#[test]
fn table_output_is_for_people() {
    let h = Home::new();
    let out = h.run(&["--demo", "-o", "table", "pod", "list"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("id "), "{text}");
    assert!(text.contains("pod_web"));
}

#[test]
fn demo_reset_forgets_changes() {
    let h = Home::new();
    h.ok(&["project", "create", "side-project"]);
    assert_eq!(h.ok(&["project", "list"]).as_array().unwrap().len(), 2);
    h.ok(&["demo", "reset"]);
    assert_eq!(h.ok(&["project", "list"]).as_array().unwrap().len(), 1);
}

#[test]
fn live_mode_without_a_key_exits_3_and_doctor_explains() {
    let h = Home::new();
    let out = h.run(&["whoami"]);
    assert_eq!(out.status.code(), Some(3));
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["code"], "NO_API_KEY");

    let out = h.run(&["doctor"]);
    assert_eq!(out.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["healthy"], false);
    assert_eq!(report["checks"][0]["check"], "api key");
}

#[test]
fn doctor_in_demo_mode_is_healthy() {
    let h = Home::new();
    let r = h.ok(&["doctor"]);
    assert_eq!(
        (r["healthy"].as_bool(), r["mode"].as_str()),
        (Some(true), Some("demo"))
    );
}

#[test]
fn stop_then_start() {
    let h = Home::new();
    assert_eq!(h.ok(&["pod", "stop", "pod_web"])["status"], "stopped");
    assert_eq!(h.ok(&["pod", "start", "pod_web"])["status"], "running");
}

#[test]
fn restart_a_stopped_pod_is_a_conflict() {
    let h = Home::new();
    h.ok(&["pod", "stop", "pod_web"]);
    let (code, err) = h.fail(&["pod", "restart", "pod_web"]);
    assert_eq!((code, err["code"].as_str()), (5, Some("NOT_RUNNING")));
}
