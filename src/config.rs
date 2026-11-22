use crate::api::Endpoint;
use anyhow::{Context, Result, bail, ensure};
use serde::{Serialize, de::DeserializeOwned};
use serde_yaml::{Mapping, Value};
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::DirBuilderExt,
    path::Path,
    time::Duration,
};

const MAX_CONFIG_SIZE: usize = 16 * 1024 * 1024;

pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
        .with_context(|| format!("create {}", path.display()))
}

pub fn write(path: &Path, content: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("missing parent directory")?)?;
    temp.write_all(content)?;
    temp.as_file().sync_all()?;
    temp.persist(path).with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("decode {}", path.display()))
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write(path, &serde_json::to_vec(value)?)
}

fn read_config(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take((MAX_CONFIG_SIZE + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_CONFIG_SIZE, "configuration exceeds 16 MiB");
    Ok(bytes)
}

pub fn local(path: &Path) -> Result<Vec<u8>> {
    read_config(File::open(path).with_context(|| format!("open {}", path.display()))?)
}

pub fn download(raw_url: &str) -> Result<Vec<u8>> {
    let url = url::Url::parse(raw_url).map_err(|_| anyhow::anyhow!("invalid subscription URL"))?;
    ensure!(matches!(url.scheme(), "https" | "http"), "subscription requires an HTTP or HTTPS URL");
    let agent = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let response = agent.get(url)
        .header("User-Agent", "clash.meta")
        .send()
        .map_err(|error| error.without_url()).context("subscription request failed")?;
    ensure!(response.status().is_success(), "subscription server returned HTTP {}", response.status());
    read_config(response).context("read subscription response")
}

pub fn secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn allocate_port(exclude: Option<u16>) -> Result<u16> {
    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        if Some(port) != exclude {
            return Ok(port);
        }
    }
}

fn endpoints(map: &Mapping, previous: Option<&Endpoint>) -> Result<Endpoint> {
    let mixed = match map.get("mixed-port") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let port = value.as_u64().and_then(|value| u16::try_from(value).ok())
                .context("mixed-port must be an integer between 0 and 65535")?;
            (port != 0).then_some(port)
        }
    };
    let controller = match map.get("external-controller") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let address = value.as_str().context("external-controller must be a host:port string")?.trim();
            if address.is_empty() {
                None
            } else {
                let (_, port) = address.rsplit_once(':').context("external-controller must include a port")?;
                let port = port.parse::<u16>().context("external-controller has an invalid port")?;
                (port != 0).then_some(port)
            }
        }
    };
    ensure!(mixed.is_none() || mixed != controller, "mixed-port and external-controller must use different ports");

    let controller = controller.or_else(|| previous.map(|e| e.controller_port).filter(|port| Some(*port) != mixed));
    let mixed = mixed.or_else(|| previous.map(|e| e.mixed_port).filter(|port| Some(*port) != controller));
    let controller_port = controller.map_or_else(|| allocate_port(mixed), Ok)?;
    let mixed_port = mixed.map_or_else(|| allocate_port(Some(controller_port)), Ok)?;
    let secret = previous.map(|e| e.secret.clone()).map_or_else(secret, Ok)?;
    Ok(Endpoint { controller_port, mixed_port, secret })
}

fn set(map: &mut Mapping, key: &str, value: impl Into<Value>) {
    map.insert(Value::from(key), value.into());
}

pub fn runtime(raw: &[u8], previous: Option<&Endpoint>, tun_enabled: bool, source_dir: Option<&Path>) -> Result<(Endpoint, Vec<u8>)> {
    let mut document: Value = serde_yaml::from_slice(raw)
        .map_err(|error| match error.location() {
            Some(location) => anyhow::anyhow!("invalid YAML at line {}, column {}", location.line(), location.column()),
            None => anyhow::anyhow!("invalid YAML"),
        })?;
    document.apply_merge().map_err(|_| anyhow::anyhow!("invalid YAML merge"))?;
    let map = document.as_mapping_mut().context("expected a complete Mihomo YAML configuration")?;
    ensure!(["proxy-groups", "rules"].iter().any(|key| map.contains_key(*key)),
        "expected a complete Mihomo configuration with proxy-groups or rules");

    let endpoint = endpoints(map, previous)?;

    for key in ["port", "socks-port", "redir-port", "tproxy-port", "listeners",
        "external-controller-tls", "external-controller-unix", "external-controller-pipe",
        "external-controller-cors", "external-doh-server", "external-ui", "external-ui-url",
        "external-controller-routing-mark"] {
        map.remove(key);
    }

    set(map, "allow-lan", false);
    set(map, "bind-address", "127.0.0.1");
    set(map, "mixed-port", endpoint.mixed_port);
    if tun_enabled {
        map.remove("external-controller");
        map.remove("secret");
        set(map, "external-controller-unix", crate::system::CORE_SOCKET);
    } else {
        set(map, "external-controller", format!("127.0.0.1:{}", endpoint.controller_port));
        set(map, "secret", endpoint.secret.as_str());
    }

    if let Some(dns) = map.get_mut("dns").and_then(Value::as_mapping_mut) {
        dns.remove("listen");
    }

    let mut tun = if tun_enabled {
        map.remove("tun").and_then(|v| v.as_mapping().cloned()).unwrap_or_default()
    } else {
        Mapping::new()
    };
    set(&mut tun, "enable", tun_enabled);
    if tun_enabled {
        set(&mut tun, "auto-route", true);
        set(&mut tun, "device", "clashpt-tun");
        if !tun.contains_key("stack") {
            set(&mut tun, "stack", "mixed");
        }
    }
    set(map, "tun", Value::Mapping(tun));

    let profile = map.entry(Value::from("profile")).or_insert_with(|| Value::Mapping(Mapping::new()));
    let profile = profile.as_mapping_mut().context("profile must be a YAML mapping")?;
    set(profile, "store-selected", true);

    for kind in ["proxy-providers", "rule-providers"] {
        if let Some(providers) = map.get_mut(kind).and_then(Value::as_mapping_mut) {
            for provider in providers.values_mut().filter_map(Value::as_mapping_mut) {
                if provider.get("type").and_then(Value::as_str) != Some("file") {
                    continue;
                }
                if let Some(path) = provider.get("path").and_then(Value::as_str) {
                    let path = Path::new(path);
                    if let Some(base) = source_dir {
                        let path = base.join(path).canonicalize().context("resolve local provider file")?;
                        set(provider, "path", path.to_str().context("provider path must be UTF-8")?);
                    } else if !tun_enabled {
                        bail!("URL subscriptions cannot reference local provider files");
                    }
                }
            }
        }
    }

    Ok((endpoint, serde_yaml::to_string(&document)?.into_bytes()))
}
