use crate::app::App;
use crate::config::{
    ACID, BAD, BORDER, CANVAS_BG, FG, MUTED, PANEL_BG, POST_FG, TWEET_FG, WARN,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;

pub const NUM_PANES: usize = 9;

/// Dark gutter between peer panels — visible because canvas ≠ panel bg.
const GUTTER: u16 = 2;
/// Wider split between announce and right rail / bottom columns.
const GUTTER_WIDE: u16 = 3;
/// Outer frame inset (left/right) so the board floats off the terminal edge.
const OUTER_H: u16 = 1;
/// Content padding inside feed/detail panes (left/right).
const PAD_H: u16 = 1;
/// KPI cards: generous horizontal pad so hero text floats.
const KPI_PAD_H: u16 = 2;
/// KPI cards: top/bottom pad inside the box.
/// Keep pad modest so h=9 cards still get content_h ≥ 4 (hero·blank·detail·sub).
const KPI_PAD_V: u16 = 1;

/// How a pane is drawn — matches Surfboard: only KPI + feeds are boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneChrome {
    /// Full bordered card on PANEL_BG (top KPI row).
    Card,
    /// Full bordered feed body on PANEL_BG (announce / activity).
    Feed,
    /// Heading + content on canvas, no box (signals / market / holders).
    Open,
}

fn chrome_of(id: PaneId) -> PaneChrome {
    match id {
        PaneId::Gate | PaneId::Treasury | PaneId::Stake | PaneId::Mcap => PaneChrome::Card,
        PaneId::Announce | PaneId::Activity => PaneChrome::Feed,
        PaneId::Signals | PaneId::Market | PaneId::Holders => PaneChrome::Open,
    }
}

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
            // Top cards: short Surfboard-style labels
            PaneId::Gate => "PRICE",
            PaneId::Treasury => "PRIMARY LP",
            PaneId::Stake => "AUDIT",
            PaneId::Mcap => "SUPPLY",
            PaneId::Announce => "ANNOUNCE FEED",
            PaneId::Signals => "SIGNALS",
            PaneId::Activity => "DEX FLOW",
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

/// Screen regions for every pane — used for mouse hit-testing + independent scroll.
#[derive(Clone, Debug, Default)]
pub struct PaneAreas {
    pub rects: [Rect; NUM_PANES],
}

impl PaneAreas {
    pub fn get(&self, id: PaneId) -> Rect {
        self.rects[id as usize]
    }

    pub fn set(&mut self, id: PaneId, r: Rect) {
        self.rects[id as usize] = r;
    }

    pub fn hit(&self, col: u16, row: u16) -> Option<PaneId> {
        for id in PaneId::all() {
            let r = self.get(id);
            if r.width == 0 || r.height == 0 {
                continue;
            }
            if col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y.saturating_add(r.height)
            {
                return Some(id);
            }
        }
        None
    }
}

/// True for the four compact top KPI cards (extra top pad + vertical centering).
fn is_kpi_card(id: PaneId) -> bool {
    matches!(
        id,
        PaneId::Gate | PaneId::Treasury | PaneId::Stake | PaneId::Mcap
    )
}

/// Borders + padding subtracted from a pane rect to get usable content size.
pub fn pane_inner_size(area: Rect, id: PaneId) -> (u16, u16) {
    match chrome_of(id) {
        PaneChrome::Card => {
            let w = area.width.saturating_sub(2 + KPI_PAD_H * 2);
            let h = area.height.saturating_sub(2 + KPI_PAD_V * 2);
            (w, h)
        }
        PaneChrome::Feed => {
            let w = area.width.saturating_sub(2 + PAD_H * 2);
            // border + title pad top + bottom pad
            let h = area.height.saturating_sub(2 + 1 + 1);
            (w, h)
        }
        PaneChrome::Open => {
            // title row + left pad; no border chrome
            let w = area.width.saturating_sub(PAD_H * 2);
            let h = area.height.saturating_sub(1);
            (w, h)
        }
    }
}

/// Content width used for wrap estimates.
pub fn pane_content_width(area: Rect) -> usize {
    // Feed panes (wrap): borders + horizontal pad.
    area.width.saturating_sub(2 + PAD_H * 2) as usize
}

/// Inset the full frame so the board floats slightly off the terminal edge.
fn content_area(frame_area: Rect) -> Rect {
    frame_area.inner(Margin {
        horizontal: OUTER_H,
        vertical: 1,
    })
}

/// Vertical split that shrinks gracefully on short terminals.
///
/// Target (tall): header 1 · top ~9–10 (airy KPI cards) · mid flex · bottom ~10 · footer 1
/// Body rows get a GUTTER-row gap between them (Surfboard spacing).
fn body_row_heights(body_h: u16) -> (u16, u16, u16) {
    // Two wide gutters between top / mid / bot (matches shell_layout spacing).
    let usable = body_h.saturating_sub(GUTTER_WIDE * 2);

    // KPI cards: content_h = h - 2 - 2*KPI_PAD_V. With KPI_PAD_V=1 need h≥8 for 4-line heroes.
    // Cap top so mid (activity) isn't starved on mid-size terminals.
    let top = match usable {
        0..=12 => 6u16,
        13..=20 => 7,
        21..=28 => 8,
        29..=40 => 9,
        _ => 10,
    };
    // Keep bottom lean so mid (activity) gets a real box on mid-size terminals.
    let bot = match usable {
        0..=12 => 3u16,
        13..=20 => 4,
        21..=28 => 5,
        29..=36 => 7,
        _ => 8,
    };

    // Mid always gets leftover; if top+bot ate usable, compress them.
    let (top, bot) = if top + bot + 4 > usable {
        let avail = usable.saturating_sub(4);
        let t = (avail / 3).max(5).min(top);
        let b = avail.saturating_sub(t).max(2).min(bot);
        (t, b)
    } else {
        (top, bot)
    };
    let mid = usable.saturating_sub(top + bot).max(4);
    (top, mid, bot)
}

/// Horizontal split for the top KPI row — stacks 2×2 when very narrow.
fn top_row_areas(area: Rect) -> [Rect; 4] {
    if area.width < 72 {
        // 2×2 grid so cards remain readable / scrollable
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(GUTTER)
            .split(area);
        let r0 = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(GUTTER)
            .split(rows[0]);
        let r1 = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(GUTTER)
            .split(rows[1]);
        [r0[0], r0[1], r1[0], r1[1]]
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .spacing(GUTTER)
            .split(area);
        [cols[0], cols[1], cols[2], cols[3]]
    }
}

/// Mid row: announce left, signals+activity right. Stacks on narrow terminals.
fn mid_row_areas(area: Rect) -> (Rect, Rect, Rect) {
    if area.width < 60 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(25),
                Constraint::Percentage(35),
            ])
            .spacing(GUTTER)
            .split(area);
        (rows[0], rows[1], rows[2])
    } else {
        // Surfboard: announce ~54%, wider dark gap, right rail ~46%.
        let mid = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
            .spacing(GUTTER_WIDE)
            .split(area);
        let right = Layout::default()
            .direction(Direction::Vertical)
            // Signals: short open checklist (~8 rows); activity takes the rest.
            // Signals: compact open checklist; activity keeps a real boxed panel.
            .constraints([Constraint::Length(5), Constraint::Min(8)])
            .spacing(GUTTER)
            .split(mid[1]);
        (mid[0], right[0], right[1])
    }
}

/// Bottom: market | holders. Stack when narrow. Same proportions as mid.
fn bot_row_areas(area: Rect) -> (Rect, Rect) {
    if area.width < 50 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(GUTTER)
            .split(area);
        (rows[0], rows[1])
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
            .spacing(GUTTER_WIDE)
            .split(area);
        (cols[0], cols[1])
    }
}

/// header · gap · body(top/mid/bot) · gap · footer for the inset content area.
fn shell_layout(area: Rect) -> (Rect, Rect, Rect, Rect, Rect) {
    // header, gap, body, gap, footer — air above/below the board body.
    let shell = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(GUTTER),
            Constraint::Min(3),
            Constraint::Length(GUTTER),
            Constraint::Length(1),
        ])
        .split(area);

    let body_area = shell[2];
    let body_h = body_area.height;
    let (top_h, _mid_h, bot_h) = body_row_heights(body_h);
    // Wider air under the KPI shelf so cards float as a separate band.
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Min(3),
            Constraint::Length(bot_h),
        ])
        .spacing(GUTTER_WIDE)
        .split(body_area);

    (shell[0], body[0], body[1], body[2], shell[4])
}

/// Compute every pane rect for the current terminal size. Pure / no side effects.
pub fn layout_panes(frame_area: Rect) -> PaneAreas {
    let area = content_area(frame_area);
    let (_hdr, top_area, mid_area, bot_area, _ftr) = shell_layout(area);

    let top = top_row_areas(top_area);
    let (announce, signals, activity) = mid_row_areas(mid_area);
    let (market, holders) = bot_row_areas(bot_area);

    let mut areas = PaneAreas::default();
    areas.set(PaneId::Gate, top[0]);
    areas.set(PaneId::Treasury, top[1]);
    areas.set(PaneId::Stake, top[2]);
    areas.set(PaneId::Mcap, top[3]);
    areas.set(PaneId::Announce, announce);
    areas.set(PaneId::Signals, signals);
    areas.set(PaneId::Activity, activity);
    areas.set(PaneId::Market, market);
    areas.set(PaneId::Holders, holders);
    areas
}

/// Scroll bounds that honor per-pane padding (KPI top pad).
pub fn pane_scroll_bounds_for(area: Rect, id: PaneId, line_count: usize) -> (u16, u16) {
    let (_w, content_h) = pane_inner_size(area, id);
    let max_scroll = line_count.saturating_sub(content_h as usize) as u16;
    (content_h, max_scroll)
}

pub fn draw(f: &mut Frame, app: &App) {
    let frame_area = f.area();
    // Canvas is darker than panel cards — gutters become real dark air.
    f.render_widget(
        Block::default().style(Style::default().bg(CANVAS_BG)),
        frame_area,
    );

    let area = content_area(frame_area);
    let (hdr, _top, _mid, _bot, ftr) = shell_layout(area);

    // Header: muted chrome, acid only on price / Δ (Surfboard ticker).
    f.render_widget(
        Paragraph::new(app.header_line())
            .alignment(Alignment::Center)
            .style(Style::default().bg(CANVAS_BG)),
        hdr,
    );

    let areas = layout_panes(frame_area);

    render_pane(f, app, PaneId::Gate, areas.get(PaneId::Gate));
    render_pane(f, app, PaneId::Treasury, areas.get(PaneId::Treasury));
    render_pane(f, app, PaneId::Stake, areas.get(PaneId::Stake));
    render_pane(f, app, PaneId::Mcap, areas.get(PaneId::Mcap));

    render_pane_titled(
        f,
        app,
        PaneId::Announce,
        &app.announce_title(),
        areas.get(PaneId::Announce),
    );
    render_pane(f, app, PaneId::Signals, areas.get(PaneId::Signals));
    render_pane(f, app, PaneId::Activity, areas.get(PaneId::Activity));

    render_pane(f, app, PaneId::Market, areas.get(PaneId::Market));
    render_pane(f, app, PaneId::Holders, areas.get(PaneId::Holders));

    // Footer on canvas.
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            app.footer_text(),
            Style::default().fg(MUTED),
        )]))
        .style(Style::default().bg(CANVAS_BG)),
        ftr,
    );
}

fn render_pane(f: &mut Frame, app: &App, id: PaneId, area: Rect) {
    render_pane_titled(f, app, id, id.title(), area);
}

fn render_pane_titled(f: &mut Frame, app: &App, id: PaneId, title: &str, area: Rect) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let chrome = chrome_of(id);
    let focused = app.focus.pane() == id;
    let hovered = app.hover_pane == Some(id);
    // Focus > hover > idle. Hover lifts border so panels feel interactive.
    let border_style = if focused {
        Style::default().fg(ACID).add_modifier(Modifier::BOLD)
    } else if hovered {
        Style::default().fg(ACID)
    } else {
        Style::default().fg(BORDER)
    };
    let title_style = if focused {
        Style::default().fg(ACID).add_modifier(Modifier::BOLD)
    } else if hovered {
        Style::default().fg(FG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD)
    };

    let lines = app.lines_for(id);
    let content_w = match chrome {
        PaneChrome::Open => area.width.saturating_sub(PAD_H * 2) as usize,
        _ => pane_content_width(area),
    };
    let visual_lines = if id.wraps() && content_w > 0 {
        lines
            .iter()
            .map(|l| {
                let chars = l.chars().count();
                if chars == 0 {
                    1
                } else {
                    ((chars + content_w - 1) / content_w).max(1)
                }
            })
            .sum()
    } else {
        lines.len()
    };
    let (content_h, max_scroll) = pane_scroll_bounds_for(area, id, visual_lines);
    let scroll = app.scroll_of(id).min(max_scroll);

    let mut text_lines: Vec<Line> = if is_kpi_card(id) {
        style_kpi_card_lines(&lines)
    } else if id == PaneId::Announce {
        style_announce_lines(&lines, app.selected_tweet, app.hover_tweet)
    } else {
        lines.iter().map(|l| style_line(id, l)).collect()
    };

    // KPI cards: vertically center the short 3-line block in the card.
    if is_kpi_card(id) && max_scroll == 0 && content_h as usize > text_lines.len() {
        let spare = content_h as usize - text_lines.len();
        let top_blank = spare / 2;
        if top_blank > 0 {
            let mut padded = vec![Line::from(""); top_blank];
            padded.append(&mut text_lines);
            text_lines = padded;
        }
    }

    let overflow = max_scroll > 0;
    let title_text = if overflow {
        format!(" {title} · ↕ ")
    } else {
        format!(" {title} ")
    };

    match chrome {
        PaneChrome::Card | PaneChrome::Feed => {
            let pad = if chrome == PaneChrome::Card {
                Padding::new(KPI_PAD_H, KPI_PAD_H, KPI_PAD_V, KPI_PAD_V)
            } else {
                Padding::new(PAD_H, PAD_H, 1, 1)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(Span::styled(title_text, title_style))
                .padding(pad)
                .style(Style::default().bg(PANEL_BG));

            let mut para = Paragraph::new(text_lines).block(block).scroll((scroll, 0));
            if is_kpi_card(id) {
                para = para.alignment(Alignment::Center);
            }
            if id.wraps() {
                para = para.wrap(Wrap { trim: false });
            }
            f.render_widget(para, area);

            if overflow {
                render_scrollbar(f, area, focused, max_scroll, scroll, /*boxed*/ true);
            }
        }
        PaneChrome::Open => {
            // Surfboard open section: heading row, then content on canvas (no box).
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1)])
                .split(area);

            f.render_widget(
                Paragraph::new(Line::from(Span::styled(title_text, title_style)))
                    .style(Style::default().bg(CANVAS_BG)),
                chunks[0],
            );

            let body = chunks[1].inner(Margin {
                horizontal: PAD_H,
                vertical: 0,
            });
            let mut para = Paragraph::new(text_lines)
                .style(Style::default().bg(CANVAS_BG))
                .scroll((scroll, 0));
            if id.wraps() {
                para = para.wrap(Wrap { trim: false });
            }
            // Open focus: title already flips to acid; no edge │ (avoids layout jump).
            f.render_widget(para, body);

            if overflow {
                render_scrollbar(f, area, focused, max_scroll, scroll, /*boxed*/ false);
            }
        }
    }
}

fn render_scrollbar(
    f: &mut Frame,
    area: Rect,
    focused: bool,
    max_scroll: u16,
    scroll: u16,
    boxed: bool,
) {
    let mut state = ScrollbarState::new(max_scroll as usize).position(scroll as usize);
    let bar_style = if focused {
        Style::default().fg(ACID)
    } else {
        Style::default().fg(MUTED)
    };
    let bar_area = if boxed {
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        })
    } else {
        // Open section: bar sits under the title row.
        Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        }
    };
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .thumb_symbol("█")
            .style(bar_style),
        bar_area,
        &mut state,
    );
}

/// Surfboard KPI: hero (bold acid) → blank → detail (muted) → sub (quieter).
fn style_kpi_card_lines(lines: &[String]) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut non_empty_idx = 0usize;
    for raw in lines {
        if raw.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        let style = match non_empty_idx {
            0 => Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            1 => Style::default().fg(FG),   // detail
            _ => Style::default().fg(MUTED), // whisper
        };
        // Color positive/negative deltas on any line.
        if raw.contains('▲') {
            out.push(Line::from(Span::styled(
                raw.clone(),
                Style::default().fg(ACID).add_modifier(if non_empty_idx == 0 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            )));
        } else if raw.contains('▼') {
            out.push(Line::from(Span::styled(
                raw.clone(),
                Style::default().fg(BAD).add_modifier(if non_empty_idx == 0 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            )));
        } else {
            out.push(Line::from(Span::styled(raw.clone(), style)));
        }
        non_empty_idx += 1;
    }
    out
}

fn style_line(id: PaneId, raw: &str) -> Line<'static> {
    if raw.is_empty() {
        return Line::from("");
    }
    match id {
        PaneId::Announce => style_announce(raw, false, false),
        PaneId::Signals => style_signal(raw),
        PaneId::Activity => style_activity(raw),
        PaneId::Gate | PaneId::Mcap | PaneId::Market => style_kpi(raw, true),
        _ => style_kpi(raw, false),
    }
}

/// Announce lines: selected = solid acid; hover = soft lift; idle = default.
fn style_announce_lines(
    lines: &[String],
    selected: Option<usize>,
    hovered: Option<usize>,
) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut tweet_i: Option<usize> = None;
    for raw in lines {
        if raw.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        if raw.contains(" POST ") {
            tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
        }
        let is_sel = matches!((tweet_i, selected), (Some(a), Some(b)) if a == b);
        let is_hov = matches!((tweet_i, hovered), (Some(a), Some(b)) if a == b);
        out.push(style_announce(raw, is_sel, is_hov));
    }
    out
}

fn style_announce(raw: &str, selected: bool, hovered: bool) -> Line<'static> {
    // Button: muted → hover outline → selected inverse.
    if raw == crate::app::VIEW_TWEET_BTN {
        let trimmed = raw.trim_start();
        let lead = raw.len().saturating_sub(trimmed.len());
        let lead_s = " ".repeat(lead);
        let chip = if selected {
            Style::default()
                .fg(CANVAS_BG)
                .bg(ACID)
                .add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default()
                .fg(ACID)
                .bg(PANEL_BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD)
        };
        return Line::from(vec![
            Span::styled(lead_s, Style::default()),
            Span::styled(trimmed.to_string(), chip),
        ]);
    }
    // "MM-DD HH:MM POST text…"
    if let Some(idx) = raw.find(" POST ") {
        let when = raw[..idx].to_string();
        let rest = raw[idx + 6..].to_string();
        let body_style = if selected {
            Style::default().fg(ACID)
        } else if hovered {
            Style::default().fg(FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TWEET_FG)
        };
        let post_style = if hovered || selected {
            Style::default().fg(POST_FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(POST_FG).add_modifier(Modifier::BOLD)
        };
        return Line::from(vec![
            Span::styled(
                when,
                if hovered {
                    Style::default().fg(FG)
                } else {
                    Style::default().fg(MUTED)
                },
            ),
            Span::styled(" POST ", post_style),
            Span::styled(rest, body_style),
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
    if raw.contains(" B ")
        || raw.contains("B $")
        || raw.starts_with("5m")
        || raw.starts_with("1h")
        || raw.starts_with("6h")
        || raw.starts_with("24h")
    {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(ACID),
        ));
    }
    if raw.contains("ANSEM") {
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


#[cfg(test)]
mod layout_geom_tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn kpi_cards_have_gutters_and_outer_inset() {
        let area = Rect::new(0, 0, 120, 40);
        let panes = layout_panes(area);
        let g = panes.get(PaneId::Gate);
        let t = panes.get(PaneId::Treasury);
        // outer inset (horizontal + vertical content_area margin)
        assert!(g.x >= 1, "outer pad missing: x={}", g.x);
        assert!(g.y >= 2, "outer vertical inset missing: y={}", g.y);
        // horizontal gutter between first two KPI cards
        let gap = t.x as i32 - (g.x as i32 + g.width as i32);
        assert!(gap >= 2, "kpi gutter missing: gap={}", gap);
        // cards shorter than a data dump (height allows air); tall enough for heroes
        assert!(g.height <= 12, "kpi too tall: {}", g.height);
        assert!(g.height >= 7, "kpi too short for hero centering: {}", g.height);
        // content_h = h - borders - pad; need ≥4 for hero·blank·detail·sub without scroll
        let (cw, ch) = pane_inner_size(g, PaneId::Gate);
        assert!(cw > 0);
        assert!(
            ch >= 4,
            "kpi content_h too small for centering: h={} content_h={}",
            g.height,
            ch
        );
        // announce left of signals; mid canyon ≥ GUTTER_WIDE
        let a = panes.get(PaneId::Announce);
        let s = panes.get(PaneId::Signals);
        let act = panes.get(PaneId::Activity);
        assert!(a.x < s.x);
        assert!(a.width > s.width, "announce should be wider");
        let mid_gap = s.x as i32 - (a.x as i32 + a.width as i32);
        assert!(mid_gap >= 3, "mid canyon too narrow: gap={}", mid_gap);
        // signals capped short (open checklist); activity is a real box
        assert!(s.height <= 9, "signals too tall: {}", s.height);
        assert!(act.height >= 6, "activity starved: {}", act.height);
        eprintln!(
            "OK gate={:?} treasury={:?} announce={:?} signals={:?} activity={:?} content_h={}",
            g, t, a, s, act, ch
        );
    }
}
