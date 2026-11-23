use super::{App, Overview, Page, Rate, help, theme::Theme};
use crate::{settings::Settings, status};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Padding, Paragraph, Row, Table, Tabs, Wrap},
};

pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>, settings: &Settings) {
    let theme = &settings.theme;
    let area = frame.area();
    frame.buffer_mut().set_style(area, theme.surface());
    let [tabs, body, status] = Layout::vertical([
        Constraint::Length(3), Constraint::Fill(1), Constraint::Length(1),
    ]).areas(area);
    let page = app.page;
    let labels = Page::NAMES.iter().enumerate().map(|(i, title)| format!("{} {title}", i + 1));
    frame.render_widget(Tabs::new(labels)
        .select(page.index())
        .style(theme.secondary())
        .highlight_style(theme.active_tab())
        .block(panel(theme, " ClashPT ", false).title(Line::from(" ? Keys ").right_aligned())), tabs);

    match page {
        Page::Overview => overview(frame, body, &app.overview, theme),
        Page::Proxies => proxies(frame, body, app, theme, &settings.indicators.active_node),
        Page::Rules => rules(frame, body, app, theme),
        Page::Conns => connections(frame, body, app, theme),
        Page::Logs => logs(frame, body, app, theme),
        Page::Profiles => profiles(frame, body, app, settings),
    }

    let (message, success) = app.status();
    let mut notice = Vec::with_capacity(2);
    if success && !settings.indicators.running.is_empty() {
        notice.push(Span::styled(format!("{} ", settings.indicators.running), Style::new().fg(theme.service_indicator(true))));
    }
    notice.push(Span::styled(clean(message.as_ref()), theme.status()));
    frame.render_widget(Paragraph::new(Line::from(notice)), status);
    if let Some(offset) = &mut app.help_offset {
        help::draw(frame, page, offset, theme);
    }
    if let Some(name) = &app.delete_confirmation {
        confirm_delete(frame, name, theme);
    }
}

fn confirm_delete(frame: &mut Frame<'_>, name: &str, theme: &Theme) {
    let area = frame.area();
    let prompt = Line::from(format!("Delete {:?}?", clean(name)));
    let keys = Line::from("y Delete   n / Esc Cancel");
    let width = (prompt.width().max(keys.width()) + 6).min(usize::from(area.width)) as u16;
    let height = area.height.min(6);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);

    frame.render_widget(Clear, popup);
    let block = panel(theme, " Delete profile ", true).style(theme.popup()).border_style(theme.popup_border());
    frame.render_widget(Paragraph::new(vec![prompt, Line::default(), keys])
        .alignment(Alignment::Center).wrap(Wrap { trim: true }).block(block), popup);
}

fn panel(theme: &Theme, title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    Block::default().title(title).borders(Borders::ALL).border_type(BorderType::Rounded)
        .style(theme.text()).border_style(theme.border(focused))
}

fn overview(frame: &mut Frame<'_>, area: Rect, view: &Overview, theme: &Theme) {
    let [info, graph] = if area.width >= 76 {
        Layout::horizontal([Constraint::Length(31), Constraint::Fill(1)]).areas(area)
    } else {
        Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area)
    };
    let current = view.history.back().copied();
    let peak = view.history.iter().fold(Rate::default(), |max, rate| Rate {
        up: max.up.max(rate.up), down: max.down.max(rate.down),
    });
    let version = if view.version.is_empty() { "—".into() } else { clean(&view.version) };
    let upload = Style::new().fg(theme.graph_upload);
    let download = Style::new().fg(theme.graph_download);
    let mut details = vec![
        metric(theme, "Upload", current.map_or_else(|| "—".into(), |v| rate(v.up)), upload),
        metric(theme, "Download", current.map_or_else(|| "—".into(), |v| rate(v.down)), download),
        metric(theme, "Uploaded", bytes(view.snapshot.upload_total), upload),
        metric(theme, "Downloaded", bytes(view.snapshot.download_total), download),
        metric(theme, "Active", view.snapshot.connections.len().to_string(), theme.secondary()),
        metric(theme, "Memory", bytes(view.snapshot.memory), theme.secondary()),
        metric(theme, "Mihomo", version, theme.accent()),
    ];
    if area.width >= 76 {
        details.insert(2, Line::default());
        details.insert(5, Line::default());
    }
    frame.render_widget(Paragraph::new(details).block(panel(theme, " Traffic summary ", true)), info);

    let title = if graph.width >= 57 { " Live traffic · last 60 samples " } else { " Live traffic " };
    let block = panel(theme, title, false);
    let inner = block.inner(graph);
    frame.render_widget(block, graph);
    let [legend, plot] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(format!(" ▲ up peak {} ", rate(peak.up)), upload),
        Span::styled(format!(" ▼ down peak {} ", rate(peak.down)), download),
    ])), legend);
    if plot.height < 3 || plot.width == 0 { return; }
    let upper = (plot.height - 1) / 2;
    let lower = plot.height - upper - 1;
    let axis = plot.y + upper;
    let scale = peak.up.max(peak.down).max(1);
    let buffer = frame.buffer_mut();
    for column in 0..plot.width {
        buffer[(plot.x + column, axis)].set_symbol("─").set_fg(theme.graph_axis);
    }
    let window = usize::from(plot.width).min(60);
    let recent = view.history.len().min(window);
    let width = usize::from(plot.width);
    for column in 0..width {
        let slot = column * window / width;
        let Some(offset) = slot.checked_sub(window - recent) else { continue };
        let sample = view.history[view.history.len() - recent + offset];
        let x = plot.x + column as u16;
        let up = degrees(sample.up, scale, upper);
        let down = degrees(sample.down, scale, lower);
        let bottom = [" ", "⣀", "⣤", "⣶", "⣿"];
        let top = [" ", "⠉", "⠛", "⠿", "⣿"];
        for row in 0..up.div_ceil(4) {
            let marks = (up - row * 4).min(4) as usize;
            buffer[(x, axis - 1 - row)].set_symbol(bottom[marks]).set_fg(theme.graph_upload);
        }
        for row in 0..down.div_ceil(4) {
            let marks = (down - row * 4).min(4) as usize;
            buffer[(x, axis + 1 + row)].set_symbol(top[marks]).set_fg(theme.graph_download);
        }
    }
}

fn degrees(value: u64, scale: u64, height: u16) -> u16 {
    (u128::from(value) * u128::from(height) * 4).div_ceil(u128::from(scale)) as u16
}

fn metric(theme: &Theme, label: &str, value: String, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<12}"), theme.text()),
        Span::styled(value, style.add_modifier(Modifier::BOLD)),
    ])
}

fn proxies(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme, symbol: &str) {
    let [left, right] = if area.width >= 44 {
        Layout::horizontal([Constraint::Percentage(32), Constraint::Fill(1)]).areas(area)
    } else {
        Layout::vertical([Constraint::Percentage(35), Constraint::Fill(1)]).areas(area)
    };
    let groups = std::iter::once(ListItem::new(format!("All nodes  ({})", app.catalog.nodes.len())))
        .chain(app.catalog.groups.iter().map(|group| ListItem::new(format!("{}  ({})", clean(&group.name), group.all.len()))))
        .collect::<Vec<_>>();
    let marker = if symbol.is_empty() { String::new() } else { format!("{symbol} ") };
    let blank_marker = " ".repeat(Line::raw(marker.as_str()).width());
    let (title, rows) = if let Some(group) = app.current_group() {
        let title = format!(" {} · {} ", clean(&group.name), clean(&group.kind));
        let rows = group.all.iter().map(|name| {
            let target = app.catalog.by_name(name);
            let node = target.and_then(|target| app.catalog.nodes.get(target));
            let measured = target.and_then(|target| app.measured.get(target)).copied();
            let delay = measured.or_else(|| node.and_then(|n| n.delay))
                .or_else(|| app.catalog.groups_delay.get(name).copied());
            let kind = node.map(|n| n.kind.as_str()).unwrap_or("Group");
            let indicator = if group.now == *name { &marker } else { &blank_marker };
            proxy_row(theme, name, kind, delay, indicator)
        }).collect::<Vec<_>>();
        (title, rows)
    } else {
        let rows = app.catalog.nodes.iter().map(|(key, node)| {
            let kind = key.provider.as_ref().map_or_else(|| node.kind.clone(), |name| format!("{} · {name}", node.kind));
            let delay = app.measured.get(key).copied().or(node.delay);
            proxy_row(theme, &key.name, &kind, delay, &blank_marker)
        }).collect::<Vec<_>>();
        (" All nodes ".to_string(), rows)
    };
    frame.render_stateful_widget(List::new(groups).block(panel(theme, " Strategy groups ", !app.node_focus).padding(Padding::new(1, 0, 0, 0)))
        .highlight_style(theme.selection(!app.node_focus)), left, &mut app.group);
    frame.render_stateful_widget(List::new(rows).block(panel(theme, title, app.node_focus).padding(Padding::new(1, 0, 0, 0)))
        .highlight_style(theme.selection(app.node_focus)), right, &mut app.node);
}

fn proxy_row(theme: &Theme, name: &str, kind: &str, delay: Option<u64>, indicator: &str) -> ListItem<'static> {
    let value = match delay {
        None => "—".to_owned(),
        Some(0) => "failed".to_owned(),
        Some(ms) => format!("{ms} ms"),
    };
    ListItem::new(Line::from(vec![
        Span::styled(indicator.to_owned(), theme.active_node_indicator()),
        Span::styled(format!("{value:>8}  "), theme.latency(delay)),
        Span::raw(clean(name)),
        Span::styled(format!("  {}", clean(kind)), theme.secondary()),
    ]))
}

fn rules(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = app.rules.iter().enumerate().map(|(i, rule)| Row::new([
        (i + 1).to_string(), clean(&rule.kind), clean(&rule.payload), clean(&rule.proxy),
    ]));
    let table = Table::new(rows, [Constraint::Length(5), Constraint::Percentage(22), Constraint::Fill(2), Constraint::Fill(1)])
        .header(header(theme, ["#", "TYPE", "MATCH", "POLICY"])).row_highlight_style(theme.selection(true))
        .block(panel(theme, format!(" Rules · {} ", app.rules.len()), true).padding(Padding::new(1, 0, 0, 0)));
    frame.render_stateful_widget(table, area, &mut app.rules_cursor);
}

fn connections(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = app.connections.connections.iter().map(|conn| {
        let meta = &conn.metadata;
        let host = if meta.host.is_empty() { &meta.destination_ip } else { &meta.host };
        let host = if host.contains(':') { format!("[{host}]") } else { host.to_owned() };
        let destination = if meta.destination_port.is_empty() { host } else { format!("{host}:{}", meta.destination_port) };
        let transport = if meta.kind.is_empty() { &meta.network } else { &meta.kind };
        let chain = conn.chains.iter().rev().map(|part| clean(part)).collect::<Vec<_>>().join(" → ");
        Row::new([
            clean(transport), clean(&meta.source_ip), clean(&destination),
            clean(&format!("{} {}", conn.rule, conn.rule_payload)).trim_end().to_owned(),
            chain, bytes(conn.download), bytes(conn.upload),
        ])
    });
    let table = Table::new(rows, [
        Constraint::Fill(1), Constraint::Fill(2), Constraint::Fill(3), Constraint::Fill(2),
        Constraint::Fill(3), Constraint::Length(10), Constraint::Length(10),
    ]).header(header(theme, ["TYPE", "SOURCE", "DESTINATION", "RULE", "PROXY CHAIN", "DOWN", "UP"]))
        .row_highlight_style(theme.selection(true))
        .block(panel(theme, format!(" Active connections · {} ", app.connections.connections.len()), true).padding(Padding::new(1, 0, 0, 0)));
    frame.render_stateful_widget(table, area, &mut app.conn_cursor);
    if app.connections.connections.is_empty() {
        empty(frame, area, theme, "No active connections");
    }
}

fn profiles(frame: &mut Frame<'_>, area: Rect, app: &mut App, settings: &Settings) {
    let theme = &settings.theme;
    let color = Style::new().fg(theme.profile_indicator());
    let rows = status::profile_rows(&app.profiles, settings.indicators.active_profile())
        .into_iter().map(|(_, marker, text)| ListItem::new(Line::from(vec![
            Span::styled(marker, color),
            Span::raw(clean(&text)),
        ])));
    let list = List::new(rows).block(panel(theme, " Saved profiles ", true).padding(Padding::new(1, 0, 0, 0)))
        .highlight_style(theme.selection(true));
    frame.render_stateful_widget(list, area, &mut app.profile_cursor);
    if app.profiles.is_empty() {
        let label = if app.system_mode {
            "System profiles are applied with sudo clashpt system apply"
        } else {
            "Save a profile with clashpt profile import or profile subscribe"
        };
        empty(frame, area, theme, label);
    }
}

fn logs(frame: &mut Frame<'_>, area: Rect, app: &mut App, theme: &Theme) {
    let rows = app.logs.iter().map(|entry| {
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<8} ", clean(&entry.level).to_uppercase()), theme.log_level(&entry.level)),
            Span::raw(clean(&entry.payload)),
        ]))
    });
    let title = if app.follow_logs { " Logs · following " } else { " Logs · paused scrolling " };
    frame.render_stateful_widget(List::new(rows).block(panel(theme, title, true))
        .highlight_style(if app.follow_logs { Style::default() } else { theme.selection(true) }),
        area, &mut app.log_cursor);
    if app.logs.is_empty() {
        empty(frame, area, theme, "Waiting for Mihomo logs");
    }
}

fn header<const N: usize>(theme: &Theme, labels: [&'static str; N]) -> Row<'static> {
    Row::new(labels).style(theme.emphasis())
}

fn empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme, label: &str) {
    if area.height > 5 && area.width > 2 {
        let area = Rect::new(area.x + 1, area.y + area.height / 2, area.width - 2, 1);
        frame.render_widget(Paragraph::new(label).alignment(Alignment::Center)
            .style(theme.secondary()), area);
    }
}

fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut scaled = value as f64;
    let mut index = 0;
    while scaled >= 1024.0 && index + 1 < UNITS.len() {
        scaled /= 1024.0;
        index += 1;
    }
    if index == 0 { format!("{value} B") } else { format!("{scaled:.1} {}", UNITS[index]) }
}

fn rate(value: u64) -> String {
    format!("{}/s", bytes(value))
}

fn clean(text: &str) -> String {
    text.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
}
