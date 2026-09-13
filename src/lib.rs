//! Metis application runtime SDK.

use percent_encoding::percent_decode_str;
use reqwest::blocking::{Client as HttpClient, Request};
use reqwest::Method;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

#[derive(Debug)]
pub enum MetisError {
    MissingConfig(String),
    InvalidConfig(String),
    DependencyUnavailable(String),
    Upstream {
        status: u16,
        reason: String,
        message: String,
    },
    Transport(String),
}

impl fmt::Display for MetisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConfig(message) => write!(formatter, "metis: missing config: {message}"),
            Self::InvalidConfig(message) => write!(formatter, "metis: invalid config: {message}"),
            Self::DependencyUnavailable(message) => {
                write!(formatter, "metis: dependency unavailable: {message}")
            }
            Self::Upstream {
                status,
                reason,
                message,
            } => write!(
                formatter,
                "metis: runtime request failed: status={status} reason={reason} message={message}"
            ),
            Self::Transport(message) => write!(formatter, "metis: transport error: {message}"),
        }
    }
}

impl std::error::Error for MetisError {}

#[derive(Debug, Default)]
pub struct Config {
    pub platform_endpoint: Option<String>,
    pub app_id: Option<String>,
    pub app_token: Option<String>,
    pub environment: Option<HashMap<String, String>>,
    pub cache_ttl: Option<Duration>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub app_id: String,
    pub alias: String,
    pub required: bool,
    pub requested_version: String,
    pub resolved_version: String,
    pub package_sha256: String,
    pub direct: bool,
    pub resolution_error: String,
    pub app_type: String,
    pub available: bool,
    pub web_base_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceEndpoint {
    pub app_id: String,
    pub endpoint_name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub available: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestContext {
    pub tenant_id: String,
    pub user_id: String,
    pub role: String,
    pub source_app_id: String,
    pub actor_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub values: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectStorageConfig {
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub bucket: String,
    pub shared_buckets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationConfig {
    pub id: String,
    pub name: String,
    pub version: String,
    pub platform_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct DependenciesResponse {
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    expires: Instant,
    value: Value,
}

#[derive(Clone)]
pub struct Client {
    endpoint: Url,
    app_id: String,
    token: String,
    http: HttpClient,
    ttl: Duration,
    environment: Arc<HashMap<String, String>>,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl Client {
    /// Builds a client from the process environment.
    pub fn from_env() -> Result<Self, MetisError> {
        Self::new(Config::default())
    }

    /// Builds a client; explicit non-empty values override the supplied environment.
    pub fn new(config: Config) -> Result<Self, MetisError> {
        let environment = config.environment.unwrap_or_else(|| env::vars().collect());
        let value = |explicit: &Option<String>, name: &str| {
            explicit
                .as_deref()
                .filter(|item| !item.trim().is_empty())
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    environment
                        .get(name)
                        .map(|item| item.trim().to_owned())
                        .unwrap_or_default()
                })
        };
        let endpoint_text = value(&config.platform_endpoint, "METIS_PLATFORM_ENDPOINT");
        let app_id = value(&config.app_id, "METIS_APP_ID");
        let token = value(&config.app_token, "METIS_APP_TOKEN");
        if endpoint_text.is_empty() || app_id.is_empty() || token.is_empty() {
            return Err(MetisError::MissingConfig(
                "METIS_PLATFORM_ENDPOINT, METIS_APP_ID and METIS_APP_TOKEN are required".into(),
            ));
        }
        let endpoint = Url::parse(&endpoint_text).map_err(|_| {
            MetisError::InvalidConfig("METIS_PLATFORM_ENDPOINT must be an absolute HTTP URL".into())
        })?;
        if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none() {
            return Err(MetisError::InvalidConfig(
                "METIS_PLATFORM_ENDPOINT must be an absolute HTTP URL".into(),
            ));
        }
        let ttl = config.cache_ttl.unwrap_or(Duration::from_secs(30));
        let http = HttpClient::builder()
            .build()
            .map_err(|error| MetisError::Transport(error.to_string()))?;
        Ok(Self {
            endpoint,
            app_id,
            token,
            http,
            ttl,
            environment: Arc::new(environment),
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn list_dependencies(&self, refresh: bool) -> Result<Vec<Dependency>, MetisError> {
        Ok(self
            .get::<DependenciesResponse>(&self.runtime_path(&["dependencies"]), refresh)?
            .dependencies)
    }

    pub fn dependency(&self, selector: &str, refresh: bool) -> Result<Dependency, MetisError> {
        self.get(&self.runtime_path(&["dependencies", selector]), refresh)
    }

    pub fn web_url(&self, selector: &str, request_path: &str) -> Result<String, MetisError> {
        let dependency = self.dependency(selector, false)?;
        if !dependency.available || dependency.web_base_path.is_empty() {
            return Err(MetisError::DependencyUnavailable(
                "web dependency is unavailable".into(),
            ));
        }
        self.absolute_path(&dependency.web_base_path, request_path)
            .map(|url| url.to_string())
    }

    pub fn new_web_request(
        &self,
        selector: &str,
        method: &str,
        request_path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Request, MetisError> {
        let url = self.web_url(selector, request_path)?;
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| MetisError::InvalidConfig("HTTP method is invalid".into()))?;
        self.http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.token))
            .body(body.unwrap_or_default())
            .build()
            .map_err(|error| MetisError::Transport(error.to_string()))
    }

    pub fn service_endpoint(
        &self,
        selector: &str,
        endpoint_name: &str,
        refresh: bool,
    ) -> Result<ServiceEndpoint, MetisError> {
        self.get(
            &self.runtime_path(&["dependencies", selector, "endpoints", endpoint_name]),
            refresh,
        )
    }

    pub fn model(&self, slot: &str) -> Result<ModelConfig, MetisError> {
        let parts: Vec<&str> = slot.split('.').collect();
        if parts.len() != 2
            || !matches!(parts[0], "llm" | "embedding" | "rerank")
            || parts[1].is_empty()
            || !parts[1].chars().all(|character| character.is_ascii_digit())
        {
            return Err(MetisError::InvalidConfig(format!(
                "invalid model slot {slot}"
            )));
        }
        let prefix = format!("METIS_{}_{}_", parts[0].to_uppercase(), parts[1]);
        let values = self
            .environment
            .iter()
            .filter_map(|(name, value)| {
                name.strip_prefix(&prefix)
                    .map(|key| (key.to_owned(), value.clone()))
            })
            .collect::<HashMap<_, _>>();
        let endpoint = values.get("ENDPOINT").cloned().unwrap_or_default();
        let model = values.get("MODEL").cloned().unwrap_or_default();
        let api_key = values.get("API_KEY").cloned().unwrap_or_default();
        if endpoint.is_empty() || model.is_empty() || api_key.is_empty() {
            return Err(MetisError::MissingConfig(format!(
                "model slot {slot} requires ENDPOINT, MODEL and API_KEY"
            )));
        }
        Ok(ModelConfig {
            endpoint,
            model,
            api_key,
            values,
        })
    }

    pub fn object_storage(&self) -> Result<ObjectStorageConfig, MetisError> {
        let get = |name: &str| self.environment.get(name).cloned().unwrap_or_default();
        let endpoint = get("METIS_S3_ENDPOINT");
        let region = get("METIS_S3_REGION");
        let access_key = get("METIS_S3_ACCESS_KEY");
        let secret_key = get("METIS_S3_SECRET_KEY");
        let bucket = get("METIS_S3_BUCKET");
        if endpoint.is_empty()
            || access_key.is_empty()
            || secret_key.is_empty()
            || bucket.is_empty()
        {
            return Err(MetisError::MissingConfig(
                "object storage requires endpoint, credentials and bucket".into(),
            ));
        }
        let raw_shared = get("METIS_S3_SHARED_BUCKETS");
        let shared_buckets = if raw_shared.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<String>>(&raw_shared).map_err(|_| {
                MetisError::InvalidConfig(
                    "METIS_S3_SHARED_BUCKETS must be a JSON string array".into(),
                )
            })?
        };
        Ok(ObjectStorageConfig {
            endpoint,
            region,
            access_key,
            secret_key,
            bucket,
            shared_buckets,
        })
    }

    pub fn application(&self) -> ApplicationConfig {
        ApplicationConfig {
            id: self.app_id.clone(),
            name: self
                .environment
                .get("METIS_APP_NAME")
                .cloned()
                .unwrap_or_default(),
            version: self
                .environment
                .get("METIS_APP_VERSION")
                .cloned()
                .unwrap_or_default(),
            platform_endpoint: self.endpoint.to_string().trim_end_matches('/').to_owned(),
        }
    }

    pub fn setting(&self, key: &str) -> Result<String, MetisError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(MetisError::InvalidConfig("setting key is required".into()));
        }
        let name = format!(
            "METIS_SETTING_{}",
            key.to_uppercase().replace(['-', '.', ' '], "_")
        );
        self.environment
            .get(&name)
            .cloned()
            .ok_or_else(|| MetisError::MissingConfig(format!("setting {key:?} is not injected")))
    }

    pub fn entry_port(&self) -> Result<u16, MetisError> {
        self.port("METIS_ENTRY_PORT")
    }

    pub fn endpoint_port(&self, name: &str) -> Result<u16, MetisError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(MetisError::InvalidConfig(
                "endpoint name is required".into(),
            ));
        }
        self.port(&format!(
            "METIS_ENDPOINT_{}_PORT",
            name.to_uppercase().replace('-', "_")
        ))
    }

    fn port(&self, name: &str) -> Result<u16, MetisError> {
        self.environment
            .get(name)
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port > 0)
            .ok_or_else(|| MetisError::MissingConfig(format!("{name} is not a valid port")))
    }

    fn runtime_path(&self, parts: &[&str]) -> String {
        let mut url = self.endpoint.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .expect("HTTP endpoint cannot be a base URL");
            segments
                .push("api")
                .push("runtime")
                .push("v1")
                .push("apps")
                .push(&self.app_id);
            for part in parts {
                segments.push(part.trim());
            }
        }
        url.path().to_owned()
    }

    fn absolute_path(&self, base_path: &str, request_path: &str) -> Result<Url, MetisError> {
        if request_path.contains("://") || request_path.starts_with("//") {
            return Err(MetisError::InvalidConfig(
                "dependency path must be relative".into(),
            ));
        }
        let (raw_path, query) = request_path
            .split_once('?')
            .map_or((request_path, None), |(path, query)| (path, Some(query)));
        let decoded = percent_decode_str(raw_path)
            .decode_utf8()
            .map_err(|_| MetisError::InvalidConfig("dependency path encoding is invalid".into()))?;
        if decoded.contains('\\') || decoded.split('/').any(|part| part == "." || part == "..") {
            return Err(MetisError::InvalidConfig(
                "dependency path must stay within the application root".into(),
            ));
        }
        let mut url = self.endpoint.clone();
        let endpoint_base = self.endpoint.path().trim_end_matches('/');
        let path = format!(
            "{}{}{}{}",
            endpoint_base,
            if base_path.starts_with('/') { "" } else { "/" },
            base_path.trim_end_matches('/'),
            if raw_path.is_empty() {
                "".to_owned()
            } else {
                format!("/{}", raw_path.trim_start_matches('/'))
            }
        );
        url.set_path(&path);
        url.set_query(query);
        Ok(url)
    }

    fn get<T: DeserializeOwned>(&self, path: &str, refresh: bool) -> Result<T, MetisError> {
        if !refresh && !self.ttl.is_zero() {
            if let Some(entry) = self
                .cache
                .lock()
                .map_err(|_| MetisError::Transport("cache lock poisoned".into()))?
                .get(path)
                .cloned()
            {
                if entry.expires > Instant::now() {
                    return serde_json::from_value(entry.value).map_err(|_| MetisError::Upstream {
                        status: 502,
                        reason: "UPSTREAM_FAILURE".into(),
                        message: "runtime response is invalid".into(),
                    });
                }
            }
        }
        let mut url = self.endpoint.clone();
        url.set_path(path);
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .map_err(|error| MetisError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|error| MetisError::Transport(error.to_string()))?;
        let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if !status.is_success() {
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("UPSTREAM_FAILURE")
                .to_owned();
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("runtime request failed")
                .to_owned();
            return Err(MetisError::Upstream {
                status: status.as_u16(),
                reason,
                message,
            });
        }
        if self.ttl > Duration::ZERO {
            self.cache
                .lock()
                .map_err(|_| MetisError::Transport("cache lock poisoned".into()))?
                .insert(
                    path.to_owned(),
                    CacheEntry {
                        expires: Instant::now() + self.ttl,
                        value: payload.clone(),
                    },
                );
        }
        serde_json::from_value(payload).map_err(|_| MetisError::Upstream {
            status: 502,
            reason: "UPSTREAM_FAILURE".into(),
            message: "runtime response is invalid".into(),
        })
    }
}

pub fn context_from_headers(headers: &HashMap<String, String>) -> RequestContext {
    let get = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    RequestContext {
        tenant_id: get("X-Platform-Tenant-Id"),
        user_id: get("X-Platform-User-Id"),
        role: get("X-Platform-Role"),
        source_app_id: get("X-Platform-Source-App-Id"),
        actor_type: get("X-Platform-Actor-Type"),
    }
}
