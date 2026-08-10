use crate::app::App;
use crate::config::{ACID, BAD, BORDER, FG, MUTED, PANEL_BG, POST_FG, TWEET_FG, WARN};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

pub const NUM_PANES: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneId {
    Gate,
    Treasury,
    Stake,
    Mcap,
    Announce,
    Signals,
    Activity,
    Market,
    Holders,
}

impl PaneId {
    pub fn all() -> [PaneId; NUM_PANES] {
        [
            PaneId::Gate,
            PaneId::Treasury,
            PaneId::Stake,
            PaneId::Mcap,
            PaneId::Announce,
            PaneId::Signals,
            PaneId::Activity,
            PaneId::Market,
            PaneId::Holders,
        ]
    }

    pub fn title(self) -> &'static str {
        match self {
            PaneId::Gate => "PRICE / FLOW",
            PaneId::Treasury => "PRIMARY LP",
            PaneId::Stake => "AUDIT / RISK",
            PaneId::Mcap => "ANSEM SUPPLY",
            PaneId::Announce => "ANNOUNCE FEED",
            PaneId::Signals => "SIGNALS",
            PaneId::Activity => "DEX + INFERENCE",
            PaneId::Market => "ANSEM MARKET",
            PaneId::Holders => "HOLDERS / DIST",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::all()[i % NUM_PANES]
    }

    /// Announce + activity wrap; KPI cards stay fixed-width.
    fn wraps(self) -> bool {
        matches!(self, PaneId::Announce | PaneId::Activity)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Focus(pub usize);

impl Focus {
    pub fn pane(self) -> PaneId {
        PaneId::from_index(self.0)
    }

    pub fn next(self) -> Self {
        Focus((self.0 + 1) % NUM_PANES)
    }

    pub fn prev(self) -> Self {
        Focus((self.0 + NUM_PANES - 1) % NUM_PANES)
    }

    pub fn from_index(i: usize) -> Self {
        Focus(i % NUM_PANES)
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Length(8), // top KPI row (hero + 4–5 support)
            Constraint::Min(10),   // mid
            Constraint::Length(11), // bottom market + holders
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    // header — acid brand bar
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                app.header_text(),
                Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(Style::default().bg(PANEL_BG)),
        root[0],
    );

    // top row: 4 equal KPI cards
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(root[1]);
    render_pane(f, app, PaneId::Gate, top[0]);
    render_pane(f, app, PaneId::Treasury, top[1]);
    render_pane(f, app, PaneId::Stake, top[2]);
    render_pane(f, app, PaneId::Mcap, top[3]);

    // mid: announce dominant | signals + activity
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(root[2]);
    render_pane_titled(f, app, PaneId::Announce, &app.announce_title(), mid[0]);

    let mid_right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(mid[1]);
    render_pane(f, app, PaneId::Signals, mid_right[0]);
    render_pane(f, app, PaneId::Activity, mid_right[1]);

    // bottom
    let bot = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(root[3]);
    render_pane(f, app, PaneId::Market, bot[0]);
    render_pane(f, app, PaneId::Holders, bot[1]);

    // footer — muted left, brand right
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(app.footer_text(), Style::default().fg(MUTED)),
        ])),
        root[4],
    );
}

fn render_pane(f: &mut Frame, app: &App, id: PaneId, area: Rect) {
    render_pane_titled(f, app, id, id.title(), area);
}

fn render_pane_titled(f: &mut Frame, app: &App, id: PaneId, title: &str, area: Rect) {
    let focused = app.focus.pane() == id;
    let border_style = if focused {
        Style::default().fg(ACID).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER)
    };
    let title_style = Style::default().fg(ACID).add_modifier(Modifier::BOLD);

    let scroll = app.scroll_of(id);
    let lines = app.lines_for(id);
    let content_h = area.height.saturating_sub(2);
    let max_scroll = lines.len().saturating_sub(content_h as usize) as u16;
    let scroll = scroll.min(max_scroll);

    let text_lines: Vec<Line> = lines.iter().map(|l| style_line(id, l)).collect();

    let title_text = if focused {
        format!(" {title} · focus ")
    } else {
        format!(" {title} ")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title_text, title_style))
        .style(Style::default().bg(PANEL_BG));

    let mut para = Paragraph::new(text_lines).block(block).scroll((scroll, 0));
    if id.wraps() {
        para = para.wrap(Wrap { trim: true });
    }

    f.render_widget(para, area);

    if focused && max_scroll > 0 {
        let mut state = ScrollbarState::new(max_scroll as usize).position(scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .thumb_symbol("█")
                .style(Style::default().fg(MUTED)),
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut state,
        );
    }
}

fn style_line(id: PaneId, raw: &str) -> Line<'static> {
    match id {
        PaneId::Announce => style_announce(raw),
        PaneId::Signals => style_signal(raw),
        PaneId::Activity => style_activity(raw),
        PaneId::Gate | PaneId::Mcap | PaneId::Market => style_kpi(raw, true),
        _ => style_kpi(raw, false),
    }
}

fn style_announce(raw: &str) -> Line<'static> {
    // "MM-DD HH:MM POST  text…"
    if let Some(idx) = raw.find(" POST ") {
        let when = raw[..idx].to_string();
        let rest = raw[idx + 6..].to_string();
        return Line::from(vec![
            Span::styled(when, Style::default().fg(MUTED)),
            Span::styled(" POST ", Style::default().fg(POST_FG).add_modifier(Modifier::BOLD)),
            Span::styled(rest, Style::default().fg(TWEET_FG)),
        ]);
    }
    Line::from(Span::styled(
        raw.to_string(),
        Style::default().fg(TWEET_FG),
    ))
}

fn style_signal(raw: &str) -> Line<'static> {
    let (mark, rest) = if let Some(r) = raw.strip_prefix("● ") {
        ("● ", r)
    } else if let Some(r) = raw.strip_prefix("◐ ") {
        ("◐ ", r)
    } else if let Some(r) = raw.strip_prefix("○ ") {
        ("○ ", r)
    } else {
        return Line::from(Span::styled(raw.to_string(), Style::default().fg(FG)));
    };
    let mark_style = match mark.chars().next().unwrap_or('·') {
        '●' => Style::default().fg(ACID),
        '◐' => Style::default().fg(WARN),
        _ => Style::default().fg(BAD),
    };
    // "LABEL   detail"
    let mut parts = rest.splitn(2, "  ");
    let label = parts.next().unwrap_or("").to_string();
    let detail = parts.next().unwrap_or("").to_string();
    Line::from(vec![
        Span::styled(mark.to_string(), mark_style),
        Span::styled(
            format!("{label:<12}"),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            detail,
            if mark.starts_with('○') {
                Style::default().fg(BAD)
            } else if mark.starts_with('◐') {
                Style::default().fg(WARN)
            } else {
                Style::default().fg(ACID)
            },
        ),
    ])
}

fn style_activity(raw: &str) -> Line<'static> {
    if raw.starts_with('─') || raw.starts_with("--") {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(MUTED),
        ));
    }
    // highlight B/S legs
    if raw.contains(" B ") || raw.contains("B $") || raw.starts_with("5m") || raw.starts_with("1h")
        || raw.starts_with("6h") || raw.starts_with("24h")
    {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(ACID),
        ));
    }
    if raw.contains("inf") || raw.contains("ANSEM") {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(TWEET_FG),
        ));
    }
    Line::from(Span::styled(raw.to_string(), Style::default().fg(FG)))
}

fn style_kpi(raw: &str, hero_first: bool) -> Line<'static> {
    // Hero lines: start with ANSEM / big price / HOLDERS count
    if hero_first
        && (raw.starts_with("ANSEM ")
            || raw.starts_with("$")
            || raw.starts_with("holders ")
            || raw.starts_with("HOLDERS"))
    {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(ACID).add_modifier(Modifier::BOLD),
        ));
    }
    if raw.contains('▲') {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(ACID),
        ));
    }
    if raw.contains('▼') {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(BAD),
        ));
    }
    if raw.starts_with("──") || raw.starts_with('[') {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(MUTED),
        ));
    }
    // label/value split on 2+ spaces
    if let Some(idx) = raw.find("  ") {
        let label = raw[..idx].to_string();
        let value = raw[idx..].to_string();
        return Line::from(vec![
            Span::styled(label, Style::default().fg(MUTED)),
            Span::styled(value, Style::default().fg(FG)),
        ]);
    }
    Line::from(Span::styled(raw.to_string(), Style::default().fg(FG)))
}
