use crate::{api::{Client, Endpoint, Target}, config};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, num::NonZeroUsize, path::Path, sync::{Arc, Mutex, mpsc}, thread, time::Duration};
use tiny_http::{Header, Method, Request, Response, StatusCode};

pub(crate) const DEFAULT_WORKERS: NonZeroUsize = NonZeroUsize::new(8).unwrap();

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Settings {
    #[serde(flatten)]
    pub(crate) endpoint: Endpoint,
    #[serde(default = "default_workers")]
    pub(crate) workers: NonZeroUsize,
}

fn default_workers() -> NonZeroUsize {
    DEFAULT_WORKERS
}

pub fn run(config_path: &Path, socket: &Path) -> Result<()> {
    let settings: Settings = config::read_json(config_path)?;
    let server = tiny_http::Server::http_unix(&settings.endpoint.socket)
        .map_err(|error| anyhow::anyhow!("{error}")).context("bind local control API")?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&settings.endpoint.socket, std::fs::Permissions::from_mode(0o666))?;
    let client = Arc::new(Client::unix(socket)?);
    let expected = format!("Bearer {}", settings.endpoint.secret);
    let (send, receive) = mpsc::sync_channel::<Request>(16);
    let receive = Arc::new(Mutex::new(receive));
    for _ in 0..settings.workers.get() {
        let (client, receive) = (Arc::clone(&client), Arc::clone(&receive));
        thread::Builder::new().spawn(move || loop {
            let request = receive.lock().unwrap().recv();
            let Ok(mut request) = request else { break };
            let (path, _) = request.url().split_once('?').unwrap_or((request.url(), ""));
            if request.method() == &Method::Get && path == "/logs" {
                match client.logs(Duration::from_secs(2)) {
                    Ok(entries) => {
                        let mut body = Vec::new();
                        for entry in entries {
                            if serde_json::to_writer(&mut body, &entry).is_err() { break; }
                            body.push(b'\n');
                        }
                        let response = Response::from_data(body).with_header(content_type("application/x-ndjson"));
                        let _ = request.respond(response);
                    }
                    Err(_) => respond(request, 503, json!({ "message": "Mihomo logs are unavailable" })),
                }
            } else {
                let (status, body) = dispatch(&mut request, &client);
                respond(request, status, body);
            }
        }).context("start control API worker")?;
    }
    for request in server.incoming_requests() {
        let authenticated = request.headers().iter()
            .any(|header| header.field.equiv("Authorization") && same_secret(header.value.as_str(), &expected));
        if !authenticated {
            respond(request, 401, json!({ "message": "unauthorized" }));
        } else if let Err(mpsc::TrySendError::Full(request)) = send.try_send(request) {
            respond(request, 503, json!({ "message": "control API is busy" }));
        }
    }
    Ok(())
}

fn content_type(value: &str) -> Header {
    Header::from_bytes("Content-Type", value).unwrap()
}

fn respond(request: Request, status: u16, body: Value) {
    let response = Response::from_string(body.to_string()).with_status_code(StatusCode(status))
        .with_header(content_type("application/json"));
    let _ = request.respond(response);
}

fn json_result(result: Result<impl Serialize>) -> (u16, Value) {
    match result.and_then(|value| Ok(serde_json::to_value(value)?)) {
        Ok(value) => (200, value),
        Err(_) => (503, json!({ "message": "Mihomo data is unavailable" })),
    }
}

fn segment(value: &str) -> Option<String> {
    if value.is_empty() { return None; }
    let value = percent_encoding::percent_decode_str(value).decode_utf8().ok()?;
    (!matches!(value.as_ref(), "." | "..")).then(|| value.into_owned())
}

fn dispatch(request: &mut Request, client: &Client) -> (u16, Value) {
    let (path, _) = request.url().split_once('?').unwrap_or((request.url(), ""));
    let parts = path.split('/').collect::<Vec<_>>();
    match (request.method(), parts.as_slice()) {
        (&Method::Get, ["", "version"]) => json_result(client.version()),
        (&Method::Get, ["", "proxies"]) => json_result(client.proxies()),
        (&Method::Get, ["", "providers", "proxies"]) => json_result(client.providers()),
        (&Method::Get, ["", "rules"]) => json_result(client.rules()),
        (&Method::Get, ["", "connections"]) => json_result(client.connections()),
        (&Method::Get, ["", "proxies", name, "delay"]) => {
            match segment(name) {
                Some(name) => json_result(client.delay(&Target { name, provider: None })),
                None => (400, json!({ "message": "invalid proxy name" })),
            }
        }
        (&Method::Get, ["", "providers", "proxies", provider, name, "healthcheck"]) => {
            match (segment(provider), segment(name)) {
                (Some(provider), Some(name)) => json_result(client.delay(&Target { name, provider: Some(provider) })),
                _ => (400, json!({ "message": "invalid provider or proxy name" })),
            }
        }
        (&Method::Put, ["", "proxies", group]) => {
            let Some(group) = segment(group) else {
                return (400, json!({ "message": "invalid group name" }));
            };
            let mut body = Vec::new();
            if request.as_reader().take(16385).read_to_end(&mut body).is_err() || body.len() > 16384 {
                return (400, json!({ "message": "invalid selection body" }));
            }
            let Ok(body) = serde_json::from_slice::<Value>(&body) else {
                return (400, json!({ "message": "invalid selection body" }));
            };
            let Some(node) = body.get("name").and_then(Value::as_str) else {
                return (400, json!({ "message": "missing node name" }));
            };
            match client.select(&group, node) {
                Ok(()) => (200, json!({})),
                Err(error) => (400, json!({ "message": error.to_string() })),
            }
        }
        _ => (404, json!({ "message": "unknown control endpoint" })),
    }
}

fn same_secret(value: &str, expected: &str) -> bool {
    value.len() == expected.len() && value.bytes().zip(expected.bytes())
        .fold(0, |different, (left, right)| different | (left ^ right)) == 0
}
