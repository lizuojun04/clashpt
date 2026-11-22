use super::{App, Overview, Page, Rate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Row, Table, Tabs},
};

const ACCENT: Color = Color::Cyan;
const UP: Color = Color::Green;
const DOWN: Color = Color::LightBlue;
const MUTED: Color = Color::DarkGray;
const CURSOR: Style = Style::new().add_modifier(Modifier::REVERSED);

pub(super) fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let [tabs, body, status, keys] = Layout::vertical([
        Constraint::Length(3), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1),
    ]).areas(frame.area());
    let page = app.page;
    let labels = Page::NAMES.iter().enumerate().map(|(i, title)| format!("{} {title}", i + 1));
    frame.render_widget(Tabs::new(labels)
        .select(page.index())
        .style(Style::default().fg(Color::Gray))
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .block(panel(" ClashPT ", false)), tabs);

    let help = match page {
        Page::Overview => { overview(frame, body, &app.overview); "1–5 Pages   r Refresh   q Quit" }
        Page::Proxies => {
            proxies(frame, body, app);
            "Tab Focus   ↑↓ Move   Enter Select   d Test node   t Test all   r Refresh   q Quit"
        }
        Page::Rules => { rules(frame, body, app); "↑↓ Scroll   PgUp/PgDn Page   r Refresh   q Quit" }
        Page::Conns => { connections(frame, body, app); "↑↓ Scroll   PgUp/PgDn Page   r Refresh   q Quit" }
        Page::Logs => { logs(frame, body, app); "↑↓ Scroll   End Follow   q Quit" }
    };

    frame.render_widget(Paragraph::new(clean(&app.messages[page.index()])).style(Style::default().fg(Color::Yellow)), status);
    frame.render_widget(Paragraph::new(help).style(Style::default().fg(Color::Gray)), keys);
}

fn panel(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    let color = if focused { ACCENT } else { MUTED };
    Block::default().title(title).borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
}

fn overview(frame: &mut Frame<'_>, area: Rect, view: &Overview) {
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
    let mut details = vec![
        metric("Upload", current.map_or_else(|| "—".into(), |v| rate(v.up)), UP),
        metric("Download", current.map_or_else(|| "—".into(), |v| rate(v.down)), DOWN),
        metric("Uploaded", bytes(view.snapshot.upload_total), UP),
        metric("Downloaded", bytes(view.snapshot.download_total), DOWN),
        metric("Active", view.snapshot.connections.len().to_string(), Color::White),
        metric("Memory", bytes(view.snapshot.memory), Color::White),
        metric("Mihomo", version, ACCENT),
    ];
    if area.width >= 76 {
        details.insert(2, Line::default());
        details.insert(5, Line::default());
    }
    frame.render_widget(Paragraph::new(details).block(panel(" Traffic summary ", true)), info);

    let title = if graph.width >= 57 { " Live traffic · last 60 samples " } else { " Live traffic " };
    let block = panel(title, false);
    let inner = block.inner(graph);
    frame.render_widget(block, graph);
    let [legend, plot] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(format!(" ▲ up peak {} ", rate(peak.up)), Style::default().fg(UP)),
        Span::styled(format!(" ▼ down peak {} ", rate(peak.down)), Style::default().fg(DOWN)),
    ])), legend);
    if plot.height < 3 || plot.width == 0 { return; }
    let upper = (plot.height - 1) / 2;
    let lower = plot.height - upper - 1;
    let axis = plot.y + upper;
    let scale = peak.up.max(peak.down).max(1);
    let buffer = frame.buffer_mut();
    for column in 0..plot.width {
        buffer[(plot.x + column, axis)].set_symbol("─").set_fg(MUTED);
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
            buffer[(x, axis - 1 - row)].set_symbol(bottom[marks]).set_fg(UP);
        }
        for row in 0..down.div_ceil(4) {
            let marks = (down - row * 4).min(4) as usize;
            buffer[(x, axis + 1 + row)].set_symbol(top[marks]).set_fg(DOWN);
        }
    }
}

fn degrees(value: u64, scale: u64, height: u16) -> u16 {
    (u128::from(value) * u128::from(height) * 4).div_ceil(u128::from(scale)) as u16
}

fn metric(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {label:<12}"), Style::default().fg(Color::Gray)),
        Span::styled(value, Style::default().fg(color).add_modifier(Modifier::BOLD)),
    ])
}

fn proxies(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let [left, right] = if area.width >= 44 {
        Layout::horizontal([Constraint::Percentage(32), Constraint::Fill(1)]).areas(area)
    } else {
        Layout::vertical([Constraint::Percentage(35), Constraint::Fill(1)]).areas(area)
    };
    let groups = std::iter::once(ListItem::new(format!("All nodes  ({})", app.catalog.nodes.len())))
        .chain(app.catalog.groups.iter().map(|group| ListItem::new(format!("{}  ({})", clean(&group.name), group.all.len()))))
        .collect::<Vec<_>>();
    let (title, rows) = if let Some(group) = app.current_group() {
        let title = format!(" {} · {} ", clean(&group.name), clean(&group.kind));
        let rows = group.all.iter().map(|name| {
            let target = app.catalog.by_name(name);
            let node = target.and_then(|target| app.catalog.nodes.get(target));
            let measured = target.and_then(|target| app.measured.get(target)).copied();
            let delay = measured.or_else(|| node.and_then(|n| n.delay))
                .or_else(|| app.catalog.groups_delay.get(name).copied());
            let kind = node.map(|n| n.kind.as_str()).unwrap_or("Group");
            proxy_row(name, kind, delay, group.now == *name)
        }).collect::<Vec<_>>();
        (title, rows)
    } else {
        let rows = app.catalog.nodes.iter().map(|(key, node)| {
            let kind = key.provider.as_ref().map_or_else(|| node.kind.clone(), |name| format!("{} · {name}", node.kind));
            let delay = app.measured.get(key).copied().or(node.delay);
            proxy_row(&key.name, &kind, delay, false)
        }).collect::<Vec<_>>();
        (" All nodes ".to_string(), rows)
    };
    frame.render_stateful_widget(List::new(groups).block(panel(" Strategy groups ", !app.node_focus))
        .highlight_style(if app.node_focus { Style::default().fg(ACCENT) } else { CURSOR }), left, &mut app.group);
    frame.render_stateful_widget(List::new(rows).block(panel(title, app.node_focus))
        .highlight_style(if app.node_focus { CURSOR } else { Style::default() }), right, &mut app.node);
}

fn proxy_row(name: &str, kind: &str, delay: Option<u64>, selected: bool) -> ListItem<'static> {
    let (value, color) = match delay {
        None => ("—".to_owned(), MUTED),
        Some(0) => ("failed".to_owned(), Color::Red),
        Some(ms) => (format!("{ms} ms"), match ms {
            ..=249 => UP,
            250..=499 => Color::Yellow,
            500..=999 => Color::Rgb(255, 161, 72),
            _ => Color::Red,
        }),
    };
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {value:>8}  "), Style::default().fg(color)),
        Span::styled(if selected { "● " } else { "  " }, Style::default().fg(ACCENT)),
        Span::raw(clean(name)),
        Span::styled(format!("  {}", clean(kind)), Style::default().fg(Color::Gray)),
    ]))
}

fn rules(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let rows = app.rules.iter().enumerate().map(|(i, rule)| Row::new([
        (i + 1).to_string(), clean(&rule.kind), clean(&rule.payload), clean(&rule.proxy),
    ]));
    let table = Table::new(rows, [Constraint::Length(5), Constraint::Percentage(22), Constraint::Fill(2), Constraint::Fill(1)])
        .header(header(["#", "TYPE", "MATCH", "POLICY"])).row_highlight_style(CURSOR)
        .block(panel(format!(" Rules · {} ", app.rules.len()), true));
    frame.render_stateful_widget(table, area, &mut app.rules_cursor);
}

fn connections(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
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
    ]).header(header(["TYPE", "SOURCE", "DESTINATION", "RULE", "PROXY CHAIN", "DOWN", "UP"]))
        .row_highlight_style(CURSOR)
        .block(panel(format!(" Active connections · {} ", app.connections.connections.len()), true));
    frame.render_stateful_widget(table, area, &mut app.conn_cursor);
    if app.connections.connections.is_empty() {
        empty(frame, area, "No active connections");
    }
}

fn logs(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let rows = app.logs.iter().map(|entry| {
        let color = match entry.level.as_str() {
            "error" => Color::Red,
            "warn" | "warning" => Color::Yellow,
            "debug" => MUTED,
            _ => ACCENT,
        };
        ListItem::new(Line::from(vec![
            Span::styled(format!(" {:<8} ", clean(&entry.level).to_uppercase()), Style::default().fg(color)),
            Span::raw(clean(&entry.payload)),
        ]))
    });
    let title = if app.follow_logs { " Logs · following " } else { " Logs · paused scrolling " };
    frame.render_stateful_widget(List::new(rows).block(panel(title, true))
        .highlight_style(if app.follow_logs { Style::default() } else { CURSOR }),
        area, &mut app.log_cursor);
    if app.logs.is_empty() {
        empty(frame, area, "Waiting for Mihomo logs");
    }
}

fn header<const N: usize>(labels: [&'static str; N]) -> Row<'static> {
    Row::new(labels).style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
}

fn empty(frame: &mut Frame<'_>, area: Rect, label: &str) {
    if area.height > 5 && area.width > 2 {
        let area = Rect::new(area.x + 1, area.y + area.height / 2, area.width - 2, 1);
        frame.render_widget(Paragraph::new(label).alignment(Alignment::Center)
            .style(Style::default().fg(Color::Gray)), area);
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
