use crate::{api::Endpoint, service::ProfileInfo, settings::Settings};
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
    let mut lines = vec![marked_line(&settings, running, state_name(running), use_color(&stdout))];
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

pub(crate) fn print_profiles(profiles: &[ProfileInfo]) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if profiles.is_empty() {
        let text = String::from("No saved profiles");
        return write_box(&mut output, vec![(text.clone(), text)]);
    }

    let settings = Settings::load(None)?;
    let symbol = settings.indicators.active_profile();
    let color = settings.theme.profile_indicator();
    let colored = use_color(&stdout);
    let lines = profile_rows(profiles, symbol).into_iter().map(|(active, marker, row)| {
        let plain = format!("{marker}{row}");
        let styled = if active && colored && !symbol.is_empty() {
            format!("{} {row}", symbol.with(color.into()))
        } else {
            plain.clone()
        };
        (plain, styled)
    }).collect();
    write_box(&mut output, lines)
}

pub(crate) fn profile_rows(profiles: &[ProfileInfo], symbol: &str) -> Vec<(bool, String, String)> {
    let prefix = if symbol.is_empty() { String::new() } else { format!("{symbol} ") };
    let indent = " ".repeat(Line::raw(prefix.as_str()).width());
    let name_width = profiles.iter().map(|profile| Line::raw(profile.name.as_str()).width()).max().unwrap_or(0);
    let type_width = profiles.iter().map(|profile| profile.source.len()).max().unwrap_or(0);
    profiles.iter().map(|profile| {
        let name_padding = " ".repeat(name_width - Line::raw(profile.name.as_str()).width());
        let row = format!("{}{name_padding}  {:<type_width$}  {}", profile.name, profile.source, profile.mixed_port);
        let marker = if profile.active { prefix.clone() } else { indent.clone() };
        (profile.active, marker, row)
    }).collect()
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
    print_marked(running, state_name(running))
}

pub(crate) fn print_success(message: &str) -> Result<()> {
    print_marked(true, message)
}

pub(crate) fn print_failure(message: &str) -> Result<()> {
    let stderr = io::stderr();
    let (_, line) = operation_line(false, message, &stderr);
    writeln!(stderr.lock(), "{line}")?;
    Ok(())
}

fn print_marked(running: bool, message: &str) -> Result<()> {
    let stdout = io::stdout();
    let (_, line) = operation_line(running, message, &stdout);
    writeln!(stdout.lock(), "{line}")?;
    Ok(())
}

pub(crate) fn operation_line(running: bool, message: &str, stream: &impl IsTerminal) -> (String, String) {
    // Presentation settings must not prevent service or profile operations.
    let settings = Settings::load(None).unwrap_or_default();
    marked_line(&settings, running, message, use_color(stream))
}

fn state_name(running: bool) -> &'static str {
    if running { "running" } else { "stopped" }
}

fn marked_line(settings: &Settings, running: bool, message: &str, use_color: bool) -> (String, String) {
    let symbol = if running { &settings.indicators.running } else { &settings.indicators.stopped };
    let plain_state = if symbol.is_empty() { message.to_owned() } else { format!("{symbol} {message}") };
    let styled_state = if use_color && !symbol.is_empty() {
        let color = settings.theme.service_indicator(running);
        format!("{} {message}", symbol.as_str().with(color.into()))
    } else {
        plain_state.clone()
    };

    (plain_state, styled_state)
}

fn use_color(stream: &impl IsTerminal) -> bool {
    stream.is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}
