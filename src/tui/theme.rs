//! Loads the terminal palette and translates it into widget styles.

use anyhow::{Context, Result};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::{fs, io, path::Path};

const DEFAULT_FILE: &str = "clashpt.yaml";

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Config {
    theme: Theme,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Theme {
    background: Color,
    text: Color,
    secondary_text: Color,
    accent: Color,
    status_text: Color,
    border: Color,
    focused_border: Color,
    selection_background: Color,
    unfocused_selection_background: Color,
    popup_text: Color,
    popup_key: Color,
    popup_border: Color,
    popup_background: Color,
    pub(super) graph_upload: Color,
    pub(super) graph_download: Color,
    pub(super) graph_axis: Color,
    latency_unmeasured: Color,
    latency_fast: Color,
    latency_medium: Color,
    latency_slow: Color,
    latency_bad: Color,
    latency_failed: Color,
    log_info: Color,
    log_warning: Color,
    log_error: Color,
    log_debug: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Reset,
            text: Color::Rgb(180, 180, 180),
            secondary_text: Color::Gray,
            accent: Color::Cyan,
            status_text: Color::Yellow,
            border: Color::DarkGray,
            focused_border: Color::Cyan,
            selection_background: Color::Rgb(43, 58, 82),
            unfocused_selection_background: Color::Rgb(30, 39, 54),
            popup_text: Color::Rgb(180, 180, 180),
            popup_key: Color::Cyan,
            popup_border: Color::LightBlue,
            popup_background: Color::Rgb(19, 27, 40),
            graph_upload: Color::Green,
            graph_download: Color::LightBlue,
            graph_axis: Color::DarkGray,
            latency_unmeasured: Color::DarkGray,
            latency_fast: Color::Green,
            latency_medium: Color::Yellow,
            latency_slow: Color::Rgb(255, 161, 72),
            latency_bad: Color::Red,
            latency_failed: Color::Red,
            log_info: Color::Cyan,
            log_warning: Color::Yellow,
            log_error: Color::Red,
            log_debug: Color::DarkGray,
        }
    }
}

impl Theme {
    pub(super) fn load(path: Option<&Path>) -> Result<Self> {
        let file = path.unwrap_or_else(|| Path::new(DEFAULT_FILE));
        let content = match fs::read_to_string(file) {
            Ok(content) => content,
            Err(error) if path.is_none() && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
        };
        let config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("parse {}", file.display()))?;
        Ok(config.theme)
    }

    pub(super) fn surface(&self) -> Style {
        self.text().bg(self.background)
    }

    pub(super) fn text(&self) -> Style {
        Style::new().fg(self.text)
    }

    pub(super) fn secondary(&self) -> Style {
        Style::new().fg(self.secondary_text)
    }

    pub(super) fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub(super) fn emphasis(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    pub(super) fn status(&self) -> Style {
        Style::new().fg(self.status_text)
    }

    pub(super) fn selection(&self, focused: bool) -> Style {
        let color = if focused { self.selection_background } else { self.unfocused_selection_background };
        Style::new().bg(color)
    }

    pub(super) fn border(&self, focused: bool) -> Style {
        Style::new().fg(if focused { self.focused_border } else { self.border })
    }

    pub(super) fn popup(&self) -> Style {
        Style::new().fg(self.popup_text).bg(self.popup_background)
    }

    pub(super) fn popup_border(&self) -> Style {
        Style::new().fg(self.popup_border)
    }

    pub(super) fn popup_key(&self) -> Style {
        Style::new().fg(self.popup_key).add_modifier(Modifier::BOLD)
    }

    pub(super) fn latency(&self, delay: Option<u64>) -> Style {
        let color = match delay {
            None => self.latency_unmeasured,
            Some(0) => self.latency_failed,
            Some(1..=249) => self.latency_fast,
            Some(250..=499) => self.latency_medium,
            Some(500..=999) => self.latency_slow,
            Some(_) => self.latency_bad,
        };
        Style::new().fg(color)
    }

    pub(super) fn log_level(&self, level: &str) -> Style {
        let color = match level {
            "error" => self.log_error,
            "warn" | "warning" => self.log_warning,
            "debug" => self.log_debug,
            _ => self.log_info,
        };
        Style::new().fg(color)
    }
}
