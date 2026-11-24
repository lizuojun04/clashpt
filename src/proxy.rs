use crate::status;
use anyhow::Result;
use clap::Args;
use std::io::{self, Write};

const VARIABLES: &str = "HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy";

#[derive(Args)]
pub(crate) struct Output {
    /// Emit plain export or unset commands for shell integration
    #[arg(long)]
    shell: bool,
    #[arg(long, hide = true, conflicts_with = "shell")]
    applied: bool,
}

pub(crate) fn print(endpoint: Option<crate::api::Endpoint>, options: &Output) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let enabled = endpoint.is_some();
    let lines = if let Some(endpoint) = endpoint {
        let port = endpoint.mixed_port;
        anyhow::ensure!(endpoint.proxy_password.len() == 64 && endpoint.proxy_password.bytes().all(|b| b.is_ascii_hexdigit()), "invalid proxy credentials");
        let http = format!("http://clashpt:{}@127.0.0.1:{port}", endpoint.proxy_password);
        let socks = format!("socks5h://clashpt:{}@127.0.0.1:{port}", endpoint.proxy_password);
        if options.applied {
            vec![
                "Proxy environment enabled".into(),
                format!("HTTP proxy  : 127.0.0.1:{port}"),
                format!("SOCKS5 proxy: 127.0.0.1:{port}"),
            ]
        } else {
            [
                ("HTTP_PROXY", &http),
                ("HTTPS_PROXY", &http),
                ("ALL_PROXY", &socks),
                ("http_proxy", &http),
                ("https_proxy", &http),
                ("all_proxy", &socks),
            ].into_iter().map(|(name, address)| format!("export {name}='{address}'")).collect()
        }
    } else if options.applied {
        vec!["Proxy environment cleared".into()]
    } else {
        vec![format!("unset {VARIABLES}")]
    };

    if options.shell {
        for line in lines {
            writeln!(output, "{line}")?;
        }
        Ok(())
    } else {
        let rows = lines.into_iter().enumerate().map(|(index, line)| {
            if options.applied && index == 0 {
                status::operation_line(enabled, &line, &stdout)
            } else {
                (line.clone(), line)
            }
        }).collect();
        status::write_box(&mut output, rows)
    }
}
