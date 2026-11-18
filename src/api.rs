//! A small client for the Happy Pod API v1.
//!
//! The contract is happy-pod/mcp `docs/API.md`: success is `{"data": ...}`, failure is
//! `{"error": {"code", "message", "details"}}` with a matching HTTP status. Requests go
//! through a [`Transport`] so the built-in simulator ([`crate::demo`]) can stand in for
//! the network.

use serde_json::{Map, Value, json};
use std::fmt;
use std::time::Duration;

pub const DEFAULT_URL: &str = "https://api.happypod.io/v1";

/// A failed call. `status` 0 means there was no HTTP response.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Option<Box<Value>>,
}

impl ApiError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(Box::new(details));
        self
    }

    /// The JSON object printed on stderr.
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("error".into(), json!(self.message));
        o.insert("code".into(), json!(self.code));
        o.insert("status".into(), json!(self.status));
        if let Some(d) = &self.details {
            o.insert("details".into(), (**d).clone());
        }
        Value::Object(o)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.to_lowercase(), self.message)
    }
}

impl std::error::Error for ApiError {}

/// One request, relative to the API base URL.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    /// Starts with `/`, without the `/v1` base.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
    pub idempotency_key: Option<String>,
}

/// A raw HTTP answer.
pub struct Response {
    pub status: u16,
    pub body: String,
}

pub trait Transport {
    fn send(
        &self,
        base_url: &str,
        api_key: &str,
        user_agent: &str,
        req: &Request,
    ) -> Result<Response, String>;
}

/// The real network.
pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

impl Transport for Http {
    fn send(
        &self,
        base_url: &str,
        api_key: &str,
        user_agent: &str,
        req: &Request,
    ) -> Result<Response, String> {
        let method =
            reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut rb = self
            .client
            .request(method, format!("{base_url}{}", req.path))
            .query(&req.query)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .header("User-Agent", user_agent);
        if let Some(b) = &req.body {
            rb = rb.json(b);
        }
        if let Some(k) = &req.idempotency_key {
            rb = rb.header("Idempotency-Key", k);
        }
        let res = rb.send().map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        let body = res.text().map_err(|e| e.to_string())?;
        Ok(Response { status, body })
    }
}

pub struct Client {
    pub base_url: String,
    pub api_key: String,
    pub user_agent: String,
    pub transport: Box<dyn Transport>,
}

impl Client {
    pub fn new(base_url: &str, api_key: &str, transport: Box<dyn Transport>) -> Self {
        let base = if base_url.is_empty() {
            DEFAULT_URL
        } else {
            base_url
        };
        Self {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            user_agent: format!("happypodctl/{}", env!("CARGO_PKG_VERSION")),
            transport,
        }
    }

    /// Sends one request and returns its `data` member.
    pub fn call(&self, req: Request) -> Result<Value, ApiError> {
        if self.api_key.is_empty() {
            return Err(ApiError::new(
                401,
                "NO_API_KEY",
                "no api key. run `happypodctl doctor`, set HAPPYPOD_API_KEY, or pass --demo to use the built-in simulator",
            ));
        }
        let res = self
            .transport
            .send(&self.base_url, &self.api_key, &self.user_agent, &req)
            .map_err(|e| {
                ApiError::new(
                    0,
                    "NETWORK_ERROR",
                    format!("could not reach {}: {e}", self.base_url),
                )
            })?;
        let parsed: Option<Value> = serde_json::from_str(&res.body).ok();
        let api_err = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .filter(|e| e.is_object());
        if res.status >= 400 || api_err.is_some() {
            let mut e = ApiError::new(res.status, "", "");
            if let Some(err) = api_err {
                e.code = err
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                e.message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                e.details = err
                    .get("details")
                    .cloned()
                    .filter(|d| !d.is_null())
                    .map(Box::new);
            }
            if e.code.is_empty() {
                e.code = format!("HTTP_{}", res.status);
            }
            if e.message.is_empty() {
                let text: String = res.body.trim().chars().take(300).collect();
                e.message = if text.is_empty() {
                    format!("http {}", res.status)
                } else {
                    text
                };
            }
            return Err(e);
        }
        match parsed {
            Some(Value::Object(mut o)) if o.contains_key("data") => {
                Ok(o.remove("data").unwrap_or(Value::Null))
            }
            Some(v) => Ok(v),
            None if res.body.trim().is_empty() => Ok(Value::Null),
            None => Err(ApiError::new(
                res.status,
                "BAD_RESPONSE",
                "the api did not return json",
            )),
        }
    }

    pub fn get(&self, path: impl Into<String>, query: Query) -> Result<Value, ApiError> {
        self.call(Request {
            method: "GET".into(),
            path: path.into(),
            query: query.0,
            ..Default::default()
        })
    }

    pub fn send(
        &self,
        method: &str,
        path: impl Into<String>,
        query: Query,
        body: Option<Value>,
        idempotency_key: Option<String>,
    ) -> Result<Value, ApiError> {
        self.call(Request {
            method: method.into(),
            path: path.into(),
            query: query.0,
            body,
            idempotency_key,
        })
    }
}

/// Query string builder that drops unset values.
#[derive(Default)]
pub struct Query(pub Vec<(String, String)>);

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn opt<T: ToString>(mut self, key: &str, v: Option<T>) -> Self {
        if let Some(v) = v {
            self.0.push((key.into(), v.to_string()));
        }
        self
    }

    /// Adds `key=true` only when `on`.
    pub fn flag(self, key: &str, on: bool) -> Self {
        self.opt(key, on.then_some(true))
    }
}

/// A fresh Idempotency-Key for a create or paid call.
pub fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Canned(u16, &'static str, RefCell<Option<Request>>);
    impl Transport for Canned {
        fn send(&self, _: &str, _: &str, _: &str, req: &Request) -> Result<Response, String> {
            *self.2.borrow_mut() = Some(req.clone());
            Ok(Response {
                status: self.0,
                body: self.1.into(),
            })
        }
    }

    fn client(status: u16, body: &'static str) -> Client {
        Client::new(
            "",
            "hp_test",
            Box::new(Canned(status, body, RefCell::new(None))),
        )
    }

    #[test]
    fn unwraps_data() {
        let v = client(200, r#"{"data":{"id":"pod_1"}}"#)
            .get("/pods/pod_1", Query::new())
            .unwrap();
        assert_eq!(v["id"], "pod_1");
    }

    #[test]
    fn maps_error_envelope() {
        let e = client(409, r#"{"error":{"code":"NO_GPU_CAPACITY","message":"none","details":{"tryRegions":["us-east"]}}}"#)
            .get("/x", Query::new())
            .unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (409, "NO_GPU_CAPACITY"));
        assert_eq!(e.details.unwrap()["tryRegions"][0], "us-east");
    }

    #[test]
    fn non_json_error_keeps_status_and_text() {
        let e = client(502, "bad gateway")
            .get("/x", Query::new())
            .unwrap_err();
        assert_eq!(
            (e.status, e.code.as_str(), e.message.as_str()),
            (502, "HTTP_502", "bad gateway")
        );
    }

    #[test]
    fn missing_key_fails_before_sending() {
        let c = Client::new("", "", Box::new(Canned(200, "{}", RefCell::new(None))));
        assert_eq!(c.get("/me", Query::new()).unwrap_err().code, "NO_API_KEY");
    }

    #[test]
    fn query_drops_unset_values() {
        let q = Query::new()
            .opt("tail", Some(50))
            .opt::<u32>("limit", None)
            .flag("dryRun", false)
            .flag("reveal", true);
        assert_eq!(
            q.0,
            vec![
                ("tail".to_string(), "50".to_string()),
                ("reveal".to_string(), "true".to_string())
            ]
        );
    }
}
