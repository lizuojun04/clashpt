//! Translates configured terminal colors into widget styles.

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Theme {
    background: Color,
    text: Color,
    secondary_text: Color,
    accent: Color,
    active_tab_text: Color,
    active_node_indicator: Color,
    active_profile_indicator: Option<Color>,
    running_indicator: Color,
    stopped_indicator: Color,
    status_text: Color,
    border: Color,
    focused_border: Color,
    selection_background: Color,
    unfocused_selection_background: Color,
    popup_text: Color,
    popup_key: Color,
    popup_border: Color,
    popup_background: Option<Color>,
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

impl Theme {
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

    pub(super) fn active_tab(&self) -> Style {
        Style::new().fg(self.active_tab_text).add_modifier(Modifier::BOLD)
    }

    pub(super) fn active_node_indicator(&self) -> Style {
        Style::new().fg(self.active_node_indicator)
    }

    pub(crate) fn profile_indicator(&self) -> Color {
        self.active_profile_indicator.unwrap_or(self.running_indicator)
    }

    pub(crate) fn service_indicator(&self, running: bool) -> Color {
        if running { self.running_indicator } else { self.stopped_indicator }
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
        let style = Style::new().fg(self.popup_text);
        match self.popup_background {
            Some(color) => style.bg(color),
            None => style,
        }
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
