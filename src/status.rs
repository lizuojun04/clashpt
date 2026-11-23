use crate::{api::Endpoint, settings::Settings};
use anyhow::Result;
use crossterm::style::Stylize;
use ratatui::text::Line;
use std::{io::{self, IsTerminal, Write}, path::Path};

pub(crate) fn print(endpoint: Option<Endpoint>, running: bool, json: bool, config: Option<&Path>) -> Result<()> {
    let mixed_address = endpoint.as_ref().map(|e| format!("127.0.0.1:{}", e.mixed_port));
    let proxy = mixed_address.as_ref().map(|address| format!("http://{address}"));
    let controller = endpoint.map(|e| format!("http://127.0.0.1:{}", e.controller_port));
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if json {
        writeln!(output, "{}", serde_json::json!({ "running": running, "proxy": proxy, "controller": controller }))?;
        return Ok(());
    }

    let settings = Settings::load(config)?;
    let mut lines = vec![state_line(&settings, running, use_color(&stdout))];
    let socks = mixed_address.map(|address| format!("socks5h://{address}"));
    let addresses = [
        ("HTTP proxy", proxy),
        ("SOCKS5 proxy", socks),
        ("Control API", controller),
    ];
    let label_width = addresses.iter().filter(|(_, address)| address.is_some())
        .map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, address) in addresses {
        if let Some(address) = address {
            let line = format!("{label:<label_width$}: {address}");
            lines.push((line.clone(), line));
        }
    }

    write_box(&mut output, lines)
}

pub(crate) fn write_box(output: &mut impl Write, lines: Vec<(String, String)>) -> Result<()> {
    let widths = lines.iter().map(|(plain, _)| Line::raw(plain).width()).collect::<Vec<_>>();
    let width = widths.iter().copied().max().unwrap_or(0);
    let rule = "─".repeat(width + 2);
    writeln!(output, "╭{rule}╮")?;
    for ((_, styled), line_width) in lines.into_iter().zip(widths) {
        writeln!(output, "│ {styled}{} │", " ".repeat(width - line_width))?;
    }
    writeln!(output, "╰{rule}╯")?;
    Ok(())
}

pub(crate) fn print_state(running: bool) -> Result<()> {
    // Presentation settings must not prevent starting or stopping the service.
    let settings = Settings::load(None).unwrap_or_default();
    let stdout = io::stdout();
    let (_, line) = state_line(&settings, running, use_color(&stdout));
    writeln!(stdout.lock(), "{line}")?;
    Ok(())
}

fn state_line(settings: &Settings, running: bool, use_color: bool) -> (String, String) {
    let (symbol, state) = if running {
        (&settings.indicators.running, "running")
    } else {
        (&settings.indicators.stopped, "stopped")
    };
    let plain_state = if symbol.is_empty() { state.to_owned() } else { format!("{symbol} {state}") };
    let styled_state = if use_color && !symbol.is_empty() {
        let color = settings.theme.service_indicator(running);
        format!("{} {state}", symbol.as_str().with(color.into()))
    } else {
        plain_state.clone()
    };

    (plain_state, styled_state)
}

fn use_color(stdout: &io::Stdout) -> bool {
    stdout.is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}
