use crate::app::App;
use crate::config::{ACID, BAD, BORDER, MUTED, PANEL_BG, TWEET_FG, WARN};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
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
            PaneId::Gate => "BULL GATE",
            PaneId::Treasury => "TREASURY / MINT",
            PaneId::Stake => "STAKE / FEES",
            PaneId::Mcap => "ANSEM MCAP",
            PaneId::Announce => "ANNOUNCE FEED",
            PaneId::Signals => "SIGNALS",
            PaneId::Activity => "INFERENCE ACTIVITY",
            PaneId::Market => "ANSEM MARKET",
            PaneId::Holders => "ANSEM HOLDERS",
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::all()[i % NUM_PANES]
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
            Constraint::Length(7), // top row
            Constraint::Min(8),    // mid
            Constraint::Length(9), // bottom
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    // header
    f.render_widget(
        Paragraph::new(app.header_text()).style(
            Style::default()
                .fg(ACID)
                .add_modifier(Modifier::BOLD),
        ),
        root[0],
    );

    // top row: 4 panes
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

    // mid: announce | (signals / activity)
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(root[2]);
    render_pane_titled(f, app, PaneId::Announce, &app.announce_title(), mid[0]);

    let mid_right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
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

    // footer
    f.render_widget(
        Paragraph::new(app.footer_text()).style(Style::default().fg(MUTED)),
        root[4],
    );
}

fn render_pane(f: &mut Frame, app: &App, id: PaneId, area: Rect) {
    render_pane_titled(f, app, id, id.title(), area);
}

fn render_pane_titled(f: &mut Frame, app: &App, id: PaneId, title: &str, area: Rect) {
    let focused = app.focus.pane() == id;
    let border_style = if focused {
        Style::default()
            .fg(ACID)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER)
    };
    let title_style = if focused {
        Style::default()
            .fg(ACID)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ACID)
    };

    let scroll = app.scroll_of(id);
    let lines = app.lines_for(id);
    let content_h = area.height.saturating_sub(2); // borders
    let max_scroll = lines.len().saturating_sub(content_h as usize) as u16;
    let scroll = scroll.min(max_scroll);

    let text_lines: Vec<Line> = lines
        .iter()
        .map(|l| style_line(id, l))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(format!(" {title} "), title_style))
        .style(Style::default().bg(PANEL_BG));

    let para = Paragraph::new(text_lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    f.render_widget(para, area);

    // scrollbar when focused and overflow
    if focused && max_scroll > 0 {
        let mut state = ScrollbarState::new(max_scroll as usize).position(scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"))
                .style(Style::default().fg(MUTED)),
            area,
            &mut state,
        );
    }
}

fn style_line(id: PaneId, raw: &str) -> Line<'static> {
    let owned = raw.to_string();
    match id {
        PaneId::Announce => Line::from(Span::styled(owned, Style::default().fg(TWEET_FG))),
        PaneId::Signals => {
            let style = if owned.starts_with('○') {
                Style::default().fg(BAD)
            } else if owned.starts_with('◐') {
                Style::default().fg(WARN)
            } else {
                Style::default().fg(ACID)
            };
            Line::from(Span::styled(owned, style))
        }
        _ => {
            // highlight deltas
            if owned.contains('▲') {
                Line::from(Span::styled(owned, Style::default().fg(ACID)))
            } else if owned.contains('▼') {
                Line::from(Span::styled(owned, Style::default().fg(BAD)))
            } else {
                Line::from(Span::styled(
                    owned,
                    Style::default().fg(ratatui::style::Color::Rgb(200, 208, 184)),
                ))
            }
        }
    }
}
