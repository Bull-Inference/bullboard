use crate::app::App;
use crate::config::{
    ACID, BAD, BLUE, BORDER, CANVAS_BG, CYAN, FG, MUTED, PANEL_BG, POST_FG, PURPLE, TWEET_FG,
    VIOLET, WARN,
};
use crate::format::is_post_line;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
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
/// KPI cards: no vertical box pad — the renderer centers the block itself,
/// so a 4-line card gets real float on any tall-enough top row.
const KPI_PAD_V: u16 = 0;

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
            // Top cards: same plain style as the side panes so the board
            // reads as one thing, not a jargon shelf + a data rail.
            PaneId::Gate => "PRICE",
            PaneId::Treasury => "LIQUIDITY",
            PaneId::Stake => "SAFETY",
            PaneId::Mcap => "SUPPLY",
            PaneId::Announce => "ANNOUNCE FEED",
            PaneId::Signals => "SIGNALS",
            PaneId::Activity => "ACTIVITY",
            PaneId::Market => "MARKET",
            PaneId::Holders => "HOLDERS",
        }
    }

    /// Quiet per-pane accent hue for idle titles; focused panes flip to acid.
    pub fn accent(self) -> Color {
        match self {
            PaneId::Gate => ACID,
            PaneId::Treasury => CYAN,
            PaneId::Stake => WARN,
            PaneId::Mcap => VIOLET,
            PaneId::Announce => TWEET_FG,
            PaneId::Signals => FG,
            PaneId::Activity => TWEET_FG,
            PaneId::Market => BLUE,
            PaneId::Holders => PURPLE,
        }
    }

    pub fn from_index(i: usize) -> Self {
        Self::all()[i % NUM_PANES]
    }

    /// Announce wraps; everything else is truncated at the pane edge so the
    /// columnar rows (activity pairs, holders, market) never reflow.
    fn wraps(self) -> bool {
        matches!(self, PaneId::Announce)
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

    // KPI cards: content_h = h - 2 (no vertical box pad). Need h≥7 for a
    // 4-line card plus one row of float; 8+ adds real air.
    let top = match usable {
        0..=12 => 5u16,
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
            // Signals: open checklist sized for its 6 content rows (title row
            // + 6) so it never needs scrolling; activity takes the rest.
            .constraints([Constraint::Length(7), Constraint::Min(8)])
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

    // Tiny terminals get an honest notice instead of a clipped, unreadable board.
    if frame_area.width < 52 || frame_area.height < 16 {
        f.render_widget(
            Block::default().style(Style::default().bg(CANVAS_BG)),
            frame_area,
        );
        let msg = "terminal too small — resize to at least 52×16";
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                msg,
                Style::default().fg(WARN),
            )]))
            .alignment(Alignment::Center)
            .style(Style::default().bg(CANVAS_BG)),
            Rect::new(0, frame_area.height / 2, frame_area.width, 1),
        );
        return;
    }

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

    // Footer: keys left, live status right.
    let ftr_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(ftr);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            app.footer_keys(),
            Style::default().fg(MUTED),
        )]))
        .style(Style::default().bg(CANVAS_BG)),
        ftr_cols[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            app.footer_status(),
            Style::default().fg(MUTED),
        )]))
        .alignment(Alignment::Right)
        .style(Style::default().bg(CANVAS_BG)),
        ftr_cols[1],
    );

    if app.show_help {
        render_help(f, frame_area);
    }
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
        // Idle titles carry the pane's quiet accent hue.
        Style::default().fg(id.accent()).add_modifier(Modifier::BOLD)
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
                    chars.div_ceil(content_w).max(1)
                }
            })
            .sum()
    } else {
        lines.len()
    };
    let (content_h, max_scroll) = pane_scroll_bounds_for(area, id, visual_lines);
    let scroll = app.scroll_of(id).min(max_scroll);

    let mut text_lines: Vec<Line> = if is_kpi_card(id) {
        style_kpi_card_lines(id, &lines)
    } else if id == PaneId::Announce {
        style_announce_lines(&lines, app.selected_tweet, app.hover_tweet)
    } else {
        lines.iter().map(|l| style_line(id, l)).collect()
    };

    // Non-wrapping panes clip with an ellipsis at the pane edge instead of
    // reflowing, so columnar rows (pairs, holders, market) keep their shape.
    // KPI cards get the same treatment as a narrow-terminal safety net.
    if !id.wraps() && content_w > 0 {
        text_lines = text_lines
            .into_iter()
            .map(|l| truncate_line(l, content_w))
            .collect();
    }

    // KPI cards: vertically center the short block in the card.
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
    // Long dynamic titles (announce) get clipped with an ellipsis too.
    let title_text = ellipsize(&title_text, area.width.saturating_sub(2) as usize);

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

/// KPI card lines: hero (bold acid) → detail (pane accent) → whisper (muted),
/// with deltas / flags / badges re-colored by `rich_spans`.
fn style_kpi_card_lines(id: PaneId, lines: &[String]) -> Vec<Line<'static>> {
    let accent = id.accent();
    let mut out = Vec::with_capacity(lines.len());
    let mut non_empty_idx = 0usize;
    for raw in lines {
        if raw.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        let base = match non_empty_idx {
            0 => Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            1 => Style::default().fg(accent),
            _ => Style::default().fg(MUTED),
        };
        out.push(Line::from(rich_spans(raw, base)));
        non_empty_idx += 1;
    }
    out
}

/// Color the meaningful tokens in machine-built lines: ▲/▼ deltas by
/// direction, `(!)` source disagreements, `[badge]` chips (verified /
/// graduated acid, dev amber, launchpad neutral, `[####--]` bars muted), and
/// trailing insider `*` marks. Everything else keeps `base`.
fn rich_spans(raw: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let flush = |out: &mut Vec<Span<'static>>, plain: &mut String| {
        if !plain.is_empty() {
            out.push(Span::styled(std::mem::take(plain), base));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '▲' || c == '▼' {
            flush(&mut out, &mut plain);
            // Arrow + its value ("▲ 5.2%") color as one token.
            let mut tok = String::from(c);
            i += 1;
            if i < chars.len() && chars[i].is_whitespace() {
                tok.push(chars[i]);
                i += 1;
                while i < chars.len() && !chars[i].is_whitespace() {
                    tok.push(chars[i]);
                    i += 1;
                }
            }
            let color = if c == '▲' { ACID } else { BAD };
            out.push(Span::styled(
                tok,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        if c == '(' && chars.get(i + 1) == Some(&'!') && chars.get(i + 2) == Some(&')') {
            flush(&mut out, &mut plain);
            out.push(Span::styled(
                "(!)".to_string(),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ));
            i += 3;
            continue;
        }
        if c == '[' {
            if let Some(rel) = chars[i + 1..].iter().position(|&x| x == ']') {
                let j = i + 1 + rel;
                let inner: String = chars[i + 1..j].iter().collect();
                flush(&mut out, &mut plain);
                let style = if inner.contains("verified") || inner.contains("graduated") {
                    Style::default().fg(ACID).add_modifier(Modifier::BOLD)
                } else if inner.contains("dev") {
                    Style::default().fg(WARN).add_modifier(Modifier::BOLD)
                } else if inner.chars().any(|c| c.is_alphabetic()) {
                    Style::default().fg(FG) // launchpad tags, etc.
                } else {
                    Style::default().fg(MUTED) // [####----] bars
                };
                out.push(Span::styled(format!("[{inner}]"), style));
                i = j + 1;
                continue;
            }
        }
        if c == '*' && (i == 0 || chars[i - 1].is_whitespace()) {
            flush(&mut out, &mut plain);
            out.push(Span::styled(
                "*".to_string(),
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ));
            i += 1;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush(&mut out, &mut plain);
    out
}

/// Clip a styled line to `max_chars` with a trailing ellipsis, preserving
/// each span's style through the cut.
fn truncate_line(line: Line<'static>, max_chars: usize) -> Line<'static> {
    if max_chars == 0 {
        return Line::default();
    }
    let total: usize = line
        .spans
        .iter()
        .map(|s| s.content.chars().count())
        .sum();
    if total <= max_chars {
        return line;
    }
    let keep = max_chars.saturating_sub(1); // room for the ellipsis
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in line.spans {
        let n = span.content.chars().count();
        if used + n <= keep {
            out.push(span);
            used += n;
        } else {
            let room = keep.saturating_sub(used);
            if room > 0 {
                let s: String = span.content.chars().take(room).collect();
                out.push(Span::styled(s, span.style));
            }
            out.push(Span::styled("…".to_string(), span.style));
            break;
        }
    }
    Line::from(out)
}

/// Clip a plain string to `max_chars` with an ellipsis (titles).
fn ellipsize(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max <= 2 {
        return s.chars().take(max).collect();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Centered help overlay — the complete key + mouse reference. Any key or
/// click closes it (handled in the app's event loop).
fn render_help(f: &mut Frame, area: Rect) {
    const HELP: &[(&str, &str)] = &[
        ("q / esc", "quit"),
        ("? / h", "toggle this help"),
        ("r", "refresh all data now"),
        ("n", "refresh tweet feed now"),
        ("t", "toggle desktop notifications"),
        ("tab / shift-tab", "next / previous pane"),
        ("1-9", "jump straight to a pane"),
        ("j / k / arrows", "scroll focused pane"),
        ("space / f · b", "page down / page up"),
        ("g / G", "top / bottom of pane"),
        ("enter / o", "open tweet in browser"),
        ("mouse", "hover highlight · click focus · wheel scroll"),
    ];
    let width = 46u16.min(area.width.saturating_sub(4));
    let height = (HELP.len() as u16 + 3).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:<16}"), Style::default().fg(ACID)),
                Span::styled(*v, Style::default().fg(FG)),
            ])
        })
        .collect();
    f.render_widget(Clear, Rect::new(x, y, width, height));
    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACID))
                    .title(Span::styled(
                        " HELP — press any key to close ",
                        Style::default().fg(ACID).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(PANEL_BG)),
            )
            .style(Style::default().bg(PANEL_BG)),
        Rect::new(x, y, width, height),
    );
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
        if is_post_line(raw) {
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
    // "MM-DD HH:MM POST text…" — matched structurally so bodies containing
    // the literal " POST " can't be mis-tagged.
    if is_post_line(raw) {
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
            return Line::from(vec![
                Span::styled(
                    when,
                    if hovered {
                        Style::default().fg(FG)
                    } else {
                        Style::default().fg(MUTED)
                    },
                ),
                Span::styled(
                    " POST ",
                    Style::default().fg(POST_FG).add_modifier(Modifier::BOLD),
                ),
                Span::styled(rest, body_style),
            ]);
        }
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
    let detail_style = if mark.starts_with('○') {
        Style::default().fg(BAD)
    } else if mark.starts_with('◐') {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(ACID)
    };
    let mut spans = vec![
        Span::styled(mark.to_string(), mark_style),
        Span::styled(
            format!("{label:<12}"),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(rich_spans(&detail, detail_style));
    Line::from(spans)
}

fn style_activity(raw: &str) -> Line<'static> {
    if raw.is_empty() {
        return Line::from("");
    }
    // Window rows are the flow readout — whole-row acid tint.
    if raw.starts_with("5m")
        || raw.starts_with("1h")
        || raw.starts_with("6h")
        || raw.starts_with("24h")
        || raw.starts_with("TOTAL LP")
    {
        return Line::from(rich_spans(raw, Style::default().fg(ACID)));
    }
    Line::from(rich_spans(raw, Style::default().fg(FG)))
}

fn style_kpi(raw: &str, hero_first: bool) -> Line<'static> {
    // Hero lines: start with ANSEM / big price / HOLDERS count
    if hero_first
        && (raw.starts_with("ANSEM ")
            || raw.starts_with("$")
            || raw.starts_with("holders ")
            || raw.starts_with("HOLDERS"))
    {
        return Line::from(rich_spans(
            raw,
            Style::default().fg(ACID).add_modifier(Modifier::BOLD),
        ));
    }
    if raw.starts_with("──") {
        return Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(MUTED),
        ));
    }
    // label/value split on 2+ spaces → whisper label, lit value
    if let Some(idx) = raw.find("  ") {
        let label = raw[..idx].to_string();
        let value = raw[idx..].to_string();
        let mut spans = vec![Span::styled(label, Style::default().fg(MUTED))];
        spans.extend(rich_spans(&value, Style::default().fg(FG)));
        return Line::from(spans);
    }
    Line::from(rich_spans(raw, Style::default().fg(FG)))
}


#[cfg(test)]
mod style_tests {
    use super::*;

    fn join(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn rich_spans_colors_each_delta_individually() {
        // Mixed ▲/▼ on one line — each arrow gets its own direction color
        // instead of the whole line going green.
        let spans = rich_spans("1h ▲ 2.1%  6h ▼ 1.3%", Style::default().fg(FG));
        assert_eq!(join(&spans), "1h ▲ 2.1%  6h ▼ 1.3%");
        let up = spans.iter().find(|s| s.content == "▲ 2.1%").unwrap();
        assert_eq!(up.style.fg, Some(ACID));
        let down = spans.iter().find(|s| s.content == "▼ 1.3%").unwrap();
        assert_eq!(down.style.fg, Some(BAD));
    }

    #[test]
    fn rich_spans_styles_disagreement_flag() {
        let spans = rich_spans("gecko $1.2K (!)", Style::default().fg(MUTED));
        let flag = spans.iter().find(|s| s.content == "(!)").unwrap();
        assert_eq!(flag.style.fg, Some(WARN));
        assert!(flag.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn rich_spans_styles_badges_and_bars() {
        let spans = rich_spans(
            "mcap $1.2M [verified] [dev 12%] [####----]",
            Style::default().fg(FG),
        );
        let verified = spans.iter().find(|s| s.content == "[verified]").unwrap();
        assert_eq!(verified.style.fg, Some(ACID));
        let dev = spans.iter().find(|s| s.content == "[dev 12%]").unwrap();
        assert_eq!(dev.style.fg, Some(WARN));
        let bar = spans.iter().find(|s| s.content == "[####----]").unwrap();
        assert_eq!(bar.style.fg, Some(MUTED));
    }

    #[test]
    fn rich_spans_styles_insider_mark() {
        let spans = rich_spans("0x12…34 *", Style::default().fg(FG));
        let star = spans.iter().find(|s| s.content == "*").unwrap();
        assert_eq!(star.style.fg, Some(WARN));
    }

    #[test]
    fn truncate_line_clips_with_ellipsis_keeping_style() {
        let line = Line::from(vec![
            Span::styled("abcd", Style::default().fg(ACID)),
            Span::styled("ef", Style::default().fg(MUTED)),
        ]);
        // "abcd" fits the 4-char budget; the ellipsis takes the cut point
        // with the style of the span it replaced.
        let cut = truncate_line(line, 5);
        assert_eq!(cut.spans[0].content, "abcd");
        assert_eq!(cut.spans[0].style.fg, Some(ACID));
        assert_eq!(cut.spans[1].content, "…");
        assert_eq!(cut.spans[1].style.fg, Some(MUTED));

        // Mid-span cut keeps the partial prefix styled like the rest.
        let line = Line::from(vec![
            Span::styled("abcdef", Style::default().fg(ACID)),
            Span::styled("gh", Style::default().fg(MUTED)),
        ]);
        let cut = truncate_line(line, 4);
        assert_eq!(cut.spans[0].content, "abc");
        assert_eq!(cut.spans[0].style.fg, Some(ACID));
        assert_eq!(cut.spans[1].content, "…");

        let short = Line::from("hi");
        assert_eq!(truncate_line(short, 5).spans.len(), 1);
        assert_eq!(truncate_line(Line::from("x"), 0).spans.len(), 0);
    }

    #[test]
    fn ellipsize_clips_long_strings() {
        assert_eq!(ellipsize("hello", 10), "hello");
        assert_eq!(ellipsize("hello", 3), "he…");
        assert_eq!(ellipsize("hello", 2), "he");
        assert_eq!(ellipsize("hello", 0), "");
    }
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

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::app::App;
    use crate::config::Config;
    use crate::fetch::http_client;
    use crate::model::{DexPair, Snapshot};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A board with enough real-ish data to exercise every styling path:
    /// accents, mixed deltas, disagreement flags, and badges.
    fn test_app() -> App {
        let mut app = App::new(Config::from_env(), http_client().unwrap());
        app.snap = Snapshot {
            token: crate::model::Token {
                symbol: "ANSEM".into(),
                name: "The Black Bull".into(),
                price_usd: Some(0.21),
                holder_count: Some(1234),
                is_verified: Some(true),
                launchpad: Some("pump.fun".into()),
                primary_pair: Some(DexPair {
                    dex_id: "raydium".into(),
                    quote_symbol: "USDC".into(),
                    liq_usd: Some(1_000_000.0),
                    ..Default::default()
                }),
                stats_24h: crate::model::WindowStats {
                    price_change: Some(3.2),
                    buy_volume: Some(500_000.0),
                    sell_volume: Some(300_000.0),
                    organic_buyers: Some(120),
                    net_buyers: Some(45),
                    traders: Some(900),
                    ..Default::default()
                },
                stats_1h: crate::model::WindowStats {
                    price_change: Some(-1.5), // ▼ mixed with ▲ elsewhere
                    ..Default::default()
                },
                ..Default::default()
            },
            gecko_token: crate::model::GeckoToken {
                // >1% divergence → amber (!) on the PRICE card
                price_usd: Some(0.25),
                ..Default::default()
            },
            ..Default::default()
        };
        app
    }

    #[test]
    fn renders_pane_accents_deltas_flags_and_badges() {
        let app = test_app();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|f| crate::ui::draw(f, &app))
            .expect("draw");
        let buf = terminal.backend().buffer();

        let find = |pred: &dyn Fn(&ratatui::buffer::Cell) -> bool| {
            buf.content.iter().any(pred)
        };
        // LIQUIDITY title carries its cyan accent (idle title, not focused).
        assert!(
            find(&|c| c.symbol() == "L" && c.fg == CYAN),
            "treasury accent missing"
        );
        // The PRICE card hero is bold acid.
        assert!(find(&|c| c.symbol() == "$" && c.fg == ACID && c.modifier.contains(Modifier::BOLD)));
        // Gecko price divergence flagged amber.
        assert!(
            find(&|c| c.symbol() == "!" && c.fg == WARN),
            "(!) disagreement flag missing"
        );
        // Mixed deltas color each arrow: ▼ is BAD even when ▲ exists nearby.
        assert!(
            find(&|c| c.symbol() == "▼" && c.fg == BAD),
            "down delta should be BAD"
        );
        assert!(find(&|c| c.symbol() == "▲" && c.fg == ACID));
        // Verified badge chip renders acid.
        assert!(
            find(&|c| c.symbol() == "v" && c.fg == ACID),
            "verified badge missing"
        );
        // Market head uses the real symbol.
        assert!(find(&|c| c.symbol() == "A" && c.fg == ACID && c.modifier.contains(Modifier::BOLD)));
    }
}
