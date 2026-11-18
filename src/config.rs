//! Settings: flag, then environment, then `~/.happypod/config.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct File {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Default project for commands that need one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// `$HAPPYPOD_HOME`, or `~/.happypod`.
pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("HAPPYPOD_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    let user = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| ".".into());
    PathBuf::from(user).join(".happypod")
}

pub fn path() -> PathBuf {
    home().join("config.json")
}

pub fn demo_state_path() -> PathBuf {
    home().join("demo.json")
}

pub fn load() -> Result<File, String> {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| format!("{} is not valid json: {e}", path().display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(e) => Err(format!("could not read {}: {e}", path().display())),
    }
}

/// Writes the file readable by the owner only: it holds the API key.
pub fn save(f: &File) -> Result<PathBuf, String> {
    let p = path();
    let dir = home();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let body = serde_json::to_string_pretty(f).map_err(|e| e.to_string())? + "\n";
    write_private(&p, body.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", p.display()))?;
    Ok(p)
}

#[cfg(unix)]
fn write_private(p: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_private(p: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(p, data)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Where each setting came from, for `doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Flag,
    Env,
    File,
    Default,
    Demo,
    None,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub demo: bool,
    pub api_key: String,
    pub api_key_source: Source,
    pub api_url: String,
    pub api_url_source: Source,
    pub project: Option<String>,
    pub project_source: Source,
}

pub fn resolve(
    file: &File,
    demo_flag: bool,
    url_flag: Option<String>,
    project_flag: Option<String>,
) -> Resolved {
    let demo = demo_flag || env("HAPPYPOD_MODE").is_some_and(|m| m.eq_ignore_ascii_case("demo"));
    let pick = |flag: Option<String>, var: &str, file: Option<String>| match (flag, env(var), file)
    {
        (Some(v), _, _) => (Some(v), Source::Flag),
        (_, Some(v), _) => (Some(v), Source::Env),
        (_, _, Some(v)) => (Some(v), Source::File),
        _ => (None, Source::None),
    };
    let (project, project_source) = pick(project_flag, "HAPPYPOD_PROJECT", file.project.clone());
    if demo {
        return Resolved {
            demo,
            api_key: crate::demo::KEY.into(),
            api_key_source: Source::Demo,
            api_url: crate::demo::URL.into(),
            api_url_source: Source::Demo,
            project: project.or_else(|| Some("prj_demo".into())),
            project_source: if project_source == Source::None {
                Source::Demo
            } else {
                project_source
            },
        };
    }
    let (key, key_source) = pick(None, "HAPPYPOD_API_KEY", file.api_key.clone());
    let (url, url_source) = match pick(url_flag, "HAPPYPOD_API_URL", file.api_url.clone()) {
        (Some(u), s) => (u, s),
        (None, _) => (crate::api::DEFAULT_URL.to_string(), Source::Default),
    };
    Resolved {
        demo,
        api_key: key.unwrap_or_default(),
        api_key_source: key_source,
        api_url: url,
        api_url_source: url_source,
        project,
        project_source,
    }
}

/// `hp_live_abcd…wxyz`: enough to tell keys apart, never the whole key.
pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 10 {
        return "•".repeat(n.max(4));
    }
    let head: String = key.chars().take(6).collect();
    let tail: String = key.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_never_shows_the_middle() {
        assert_eq!(mask("hp_live_0123456789abcdef"), "hp_liv…cdef");
        assert_eq!(mask("short"), "•••••");
    }
}
