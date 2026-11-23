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

pub(crate) fn print(port: Option<u16>, options: &Output) -> Result<()> {
    let mut output = io::stdout().lock();
    let lines = if let Some(port) = port {
        let http = format!("http://127.0.0.1:{port}");
        let socks = format!("socks5h://127.0.0.1:{port}");
        if options.applied {
            vec![
                "Proxy environment enabled".into(),
                format!("HTTP proxy  : {http}"),
                format!("SOCKS5 proxy: {socks}"),
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
        status::write_box(&mut output, lines.into_iter().map(|line| (line.clone(), line)).collect())
    }
}
