use super::{Page, theme::Theme};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph},
};

type Shortcut = (&'static str, &'static str);

const COMMON: &[Shortcut] = &[
    ("1–6", "Switch tabs"),
    ("r", "Refresh this page"),
    ("q", "Quit ClashPT"),
    ("Enter / Esc / ?", "Close shortcuts"),
];

fn page_keys(page: Page) -> &'static [Shortcut] {
    match page {
        Page::Overview => &[],
        Page::Proxies => &[
            ("Tab / ← → / h l", "Switch between groups and nodes"),
            ("↑ ↓ / k j", "Move selection"),
            ("PgUp / PgDn", "Move 12 rows"),
            ("Enter", "Open group or select node"),
            ("d", "Test current node"),
            ("g", "Test nodes in current group"),
            ("t", "Test all nodes"),
        ],
        Page::Rules => &[
            ("↑ ↓ / k j", "Scroll rules"),
            ("PgUp / PgDn", "Move 12 rows"),
        ],
        Page::Conns => &[
            ("↑ ↓ / k j", "Scroll active connections"),
            ("PgUp / PgDn", "Move 12 rows"),
        ],
        Page::Profiles => &[
            ("↑ ↓ / k j", "Highlight profile"),
            ("PgUp / PgDn", "Move 12 rows"),
            ("Enter", "Activate profile"),
            ("Delete / x", "Delete profile"),
        ],
        Page::Logs => &[
            ("↑ ↓ / k j", "Scroll logs"),
            ("PgUp / PgDn", "Move 12 rows"),
            ("End", "Follow the latest logs"),
        ],
    }
}

pub(super) fn draw(frame: &mut Frame<'_>, page: Page, offset: &mut usize, theme: &Theme) {
    let area = frame.area();
    if area.width < 5 || area.height < 3 {
        return;
    }

    let rows = page_keys(page).iter().chain(COMMON).copied().collect::<Vec<_>>();
    let column = rows.iter().flat_map(|(key, action)| [key, action])
        .map(|text| Line::from(*text).width()).max().unwrap_or(0) as u16;
    let max_odd_width = if area.width % 2 == 0 { area.width - 1 } else { area.width };
    let width = (2 * column + 5).min(max_odd_width);
    let height = (rows.len() as u16 + 4).min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = Block::default()
        .title(Line::from(format!(" {} shortcuts ", Page::NAMES[page.index()])).centered())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.popup_border())
        .style(theme.popup())
        .padding(Padding::new(u16::from(width >= 7), u16::from(width >= 7),
            u16::from(height >= 5), u16::from(height >= 5)));

    let content = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    *offset = (*offset).min(rows.len().saturating_sub(content.height as usize));
    let half = (content.width - 1) / 2;
    let keys = Rect::new(content.x, content.y, half, content.height);
    let actions = Rect::new(content.x + half + 1, content.y, half, content.height);
    let visible_rows = &rows[*offset..];
    let keys_text = visible_rows.iter().map(|(key, _)| Line::from(*key)).collect::<Vec<_>>();
    let actions_text = visible_rows.iter().map(|(_, action)| Line::from(*action)).collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(keys_text).alignment(Alignment::Right).style(theme.popup_key()), keys);
    frame.render_widget(Paragraph::new(actions_text).alignment(Alignment::Left), actions);
}
