use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::{collections::BTreeMap, io::{BufRead, BufReader, Read}, path::Path, time::Duration};

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct Endpoint {
    #[serde(default)]
    pub socket: std::path::PathBuf,
    #[serde(default)]
    pub proxy_password: String,
    pub mixed_port: u16,
    pub secret: String,
}

#[derive(Deserialize, Serialize)]
pub struct Group {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Version {
    pub version: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Proxy {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub now: String,
    pub all: Option<Vec<String>>,
    #[serde(default)]
    pub history: Vec<Delay>,
    #[serde(rename = "provider-name", default)]
    pub provider: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Delay {
    pub delay: u64,
}

#[derive(Deserialize, Serialize)]
pub struct Proxies {
    pub proxies: BTreeMap<String, Proxy>,
}

#[derive(Deserialize, Serialize)]
pub struct Provider {
    pub proxies: Vec<Proxy>,
}

#[derive(Deserialize, Serialize)]
pub struct Providers {
    pub providers: BTreeMap<String, Provider>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Target {
    pub name: String,
    pub provider: Option<String>,
}

impl Target {
    pub fn path(&self) -> String {
        let name = encode_segment(&self.name);
        match &self.provider {
            Some(provider) => format!("/providers/proxies/{}/{name}/healthcheck", encode_segment(provider)),
            None => format!("/proxies/{name}/delay"),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct Rule {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub payload: String,
    pub proxy: String,
}

#[derive(Deserialize, Serialize)]
pub struct Rules {
    pub rules: Vec<Rule>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Connections {
    pub upload_total: u64,
    pub download_total: u64,
    pub memory: u64,
    #[serde(deserialize_with = "nullable_connections")]
    pub connections: Vec<Connection>,
}

fn nullable_connections<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Connection>, D::Error> {
    Ok(Option::<Vec<Connection>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub metadata: Metadata,
    pub upload: u64,
    pub download: u64,
    pub chains: Vec<String>,
    pub rule: String,
    pub rule_payload: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Metadata {
    pub network: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "sourceIP")]
    pub source_ip: String,
    #[serde(rename = "destinationIP")]
    pub destination_ip: String,
    pub destination_port: String,
    pub host: String,
}

#[derive(Deserialize, Serialize)]
pub struct LogEntry {
    #[serde(rename = "type")]
    pub level: String,
    pub payload: String,
}

pub struct Client {
    agent: reqwest::blocking::Client,
    base: String,
    secret: Option<String>,
}

pub(crate) fn private_socket_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    anyhow::ensure!(path.is_absolute(), "expected an absolute socket directory");
    for (index, parent) in path.ancestors().enumerate() {
        let meta = std::fs::symlink_metadata(parent)?;
        let trusted = if index == 0 {
            meta.uid() == uid && meta.mode() & 0o077 == 0
        } else {
            (meta.uid() == uid || meta.uid() == 0) &&
                (meta.mode() & 0o022 == 0 || meta.mode() & 0o1000 != 0)
        };
        anyhow::ensure!(meta.is_dir() && trusted, "untrusted control socket directory: {}", parent.display());
    }
    Ok(())
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        anyhow::ensure!(endpoint.socket.is_absolute(), "re-import this profile or reapply the system deployment to enable secure control");
        use std::os::unix::fs::MetadataExt;
        let parent = endpoint.socket.parent().context("missing socket directory")?;
        let system = endpoint.socket == Path::new(crate::system::GATEWAY_SOCKET);
        if system {
            let run = std::fs::symlink_metadata("/run")?;
            anyhow::ensure!(run.is_dir() && run.uid() == 0 && run.mode() & 0o022 == 0,
                "insecure system runtime directory");
            match std::fs::symlink_metadata(parent) {
                Ok(meta) => anyhow::ensure!(meta.is_dir() && meta.mode() & 0o022 == 0, "insecure gateway directory"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(e.into()),
            }
        } else {
            private_socket_directory(parent)?;
        }
        Ok(Self {
            agent: Self::builder().unix_socket(endpoint.socket).build()?,
            base: "http://localhost".into(),
            secret: system.then_some(endpoint.secret),
        })
    }

    pub fn unix(socket: &Path) -> Result<Self> {
        Ok(Self { agent: Self::builder().unix_socket(socket).build()?,
            base: "http://localhost".into(), secret: None })
    }

    fn builder() -> reqwest::blocking::ClientBuilder {
        reqwest::blocking::Client::builder().no_proxy()
            .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(2))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let request = self.agent.request(method, format!("{}{path}", self.base));
        if let Some(secret) = &self.secret {
            request.bearer_auth(secret)
        } else {
            request
        }
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.request(reqwest::Method::GET, path).send()
            .map_err(|error| connection_context(error, path))?.error_for_status()?;
        serde_json::from_reader(response).with_context(|| format!("decode {path}"))
    }

    pub fn ready(&self) -> bool {
        self.version().is_ok()
    }

    pub fn version(&self) -> Result<Version> {
        self.get("/version")
    }

    pub fn proxies(&self) -> Result<Proxies> {
        self.get("/proxies")
    }

    pub fn providers(&self) -> Result<Providers> {
        self.get("/providers/proxies")
    }

    pub fn groups(&self) -> Result<Vec<Group>> {
        Ok(self.proxies()?.proxies.into_iter().filter_map(|(name, proxy)| {
            proxy.all.map(|all| Group { name, kind: proxy.kind, now: proxy.now, all })
        }).collect())
    }

    pub fn rules(&self) -> Result<Rules> {
        self.get("/rules")
    }

    pub fn connections(&self) -> Result<Connections> {
        self.get("/connections")
    }

    pub fn delay(&self, target: &Target) -> Result<Delay> {
        let response = self.request(reqwest::Method::GET, &target.path())
            .query(&[("url", "https://www.gstatic.com/generate_204"), ("timeout", "4000")])
            .timeout(Duration::from_secs(12))
            .send().context("test node delay")?.error_for_status()?;
        serde_json::from_reader(response).context("decode node delay")
    }

    pub fn logs(&self, window: Duration) -> Result<Vec<LogEntry>> {
        let response = self.request(reqwest::Method::GET, "/logs").query(&[("level", "info")])
            .timeout(window).send().map_err(|error| connection_context(error, "read Mihomo logs"))?
            .error_for_status()?;
        let mut reader = BufReader::new(response);
        let mut entries = std::collections::VecDeque::with_capacity(160);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.by_ref().take(32 * 1024).read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) if !line.ends_with(b"\n") => break,
                Err(error) if !matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                    if entries.is_empty() { return Err(error).context("read Mihomo logs"); }
                    break;
                }
                Err(_) => break,
                Ok(_) => {
                    if let Ok(entry) = serde_json::from_slice(&line) {
                        if entries.len() == 160 { entries.pop_front(); }
                        entries.push_back(entry);
                    }
                }
            }
        }
        Ok(entries.into())
    }

    pub fn select(&self, group: &str, node: &str) -> Result<()> {
        let encoded = encode_segment(group);
        let body = json!({ "name": node }).to_string();
        let response = self.request(reqwest::Method::PUT, &format!("/proxies/{encoded}"))
            .header("Content-Type", "application/json").body(body).send()
            .map_err(|error| connection_context(error, "switch Mihomo node"))?;
        let code = response.status();
        if !code.is_success() {
            let message = serde_json::from_reader::<_, serde_json::Value>(response)
                .ok().and_then(|v| v.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| "request rejected".into());
            bail!("Mihomo returned HTTP {code}: {message}");
        }
        Ok(())
    }
}

fn encode_segment(name: &str) -> String {
    percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn connection_context(error: reqwest::Error, operation: &str) -> anyhow::Error {
    if error.is_connect() {
        anyhow::Error::new(error).context(format!(
            "{operation}: Mihomo control API is unreachable. Check clashpt status and start Mihomo with clashpt start (or clashpt --system start)"
        ))
    } else {
        anyhow::Error::new(error).context(operation.to_owned())
    }
}
