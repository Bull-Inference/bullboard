use crate::config::{Config, FeedMode, REFRESH_DATA_SECS, REFRESH_FEED_SECS};
use crate::fetch::{fetch_announce, fetch_snapshot, http_client};
use crate::format::{
    ago, clock_mmdd_hhmm, delta_str, fmt_ansem, fmt_compact, fmt_int, fmt_usd, rate_pct, short_addr,
    sparkline,
};
use crate::model::Snapshot;
use crate::ui::{self, Focus, PaneId, NUM_PANES};
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use reqwest::Client;
use std::collections::HashMap;
use std::io::stdout;
use std::time::{Duration, Instant};

pub struct App {
    pub cfg: Config,
    pub client: Client,
    pub snap: Snapshot,
    pub feed_mode: FeedMode,
    pub focus: Focus,
    /// scroll offset (lines) per pane
    pub scroll: HashMap<PaneId, u16>,
    pub loading: bool,
    pub should_quit: bool,
    pub status: String,
    last_data: Instant,
    last_feed: Instant,
}

impl App {
    pub fn new(cfg: Config, client: Client) -> Self {
        Self {
            cfg,
            client,
            snap: Snapshot::default(),
            feed_mode: FeedMode::Primary,
            focus: Focus::default(),
            scroll: HashMap::new(),
            loading: false,
            should_quit: false,
            status: "loading…".into(),
            last_data: Instant::now() - Duration::from_secs(999),
            last_feed: Instant::now() - Duration::from_secs(999),
        }
    }

    pub fn scroll_of(&self, id: PaneId) -> u16 {
        self.scroll.get(&id).copied().unwrap_or(0)
    }

    pub fn set_scroll(&mut self, id: PaneId, v: u16) {
        self.scroll.insert(id, v);
    }

    pub fn scroll_focused(&mut self, delta: i32) {
        let id = self.focus.pane();
        let cur = self.scroll_of(id) as i32;
        let next = (cur + delta).max(0) as u16;
        self.set_scroll(id, next);
    }

    pub async fn refresh_data(&mut self) {
        self.loading = true;
        let tweets = self.snap.tweets.clone();
        let terr = self.snap.tweet_error.clone();
        self.snap = fetch_snapshot(&self.client, &self.cfg).await;
        self.snap.tweets = tweets;
        self.snap.tweet_error = terr;
        self.last_data = Instant::now();
        self.loading = false;
        self.status = format!("updated {}", ago(self.snap.fetched_at.as_deref()));
    }

    pub async fn refresh_feed(&mut self) {
        let (tweets, err) =
            fetch_announce(&self.client, &self.cfg, self.feed_mode, 40).await;
        self.snap.tweets = tweets;
        self.snap.tweet_error = err;
        self.last_feed = Instant::now();
        // reset announce scroll on feed cycle only if empty scroll keep
    }

    pub async fn refresh_all(&mut self) {
        self.refresh_data().await;
        self.refresh_feed().await;
    }

    pub fn lines_for(&self, id: PaneId) -> Vec<String> {
        match id {
            PaneId::Gate => self.lines_gate(),
            PaneId::Treasury => self.lines_treasury(),
            PaneId::Stake => self.lines_stake(),
            PaneId::Mcap => self.lines_mcap(),
            PaneId::Announce => self.lines_announce(),
            PaneId::Signals => self.lines_signals(),
            PaneId::Activity => self.lines_activity(),
            PaneId::Market => self.lines_market(),
            PaneId::Holders => self.lines_holders(),
        }
    }

    fn lines_gate(&self) -> Vec<String> {
        let s = &self.snap;
        let min_w = s
            .net_f("min_wallet_ansem")
            .or_else(|| s.cfg_f("min_wallet_ansem"));
        let onchain = s
            .net_bool("per_call_onchain")
            .or_else(|| s.config.get("per_call_onchain").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let ready = s.net_bool("solana_ready").unwrap_or(false);
        vec![
            format!(
                "min wallet   {} $ANSEM",
                min_w.map(|v| format!("{v}")).unwrap_or_else(|| "—".into())
            ),
            format!("per-call     {}", if onchain { "ON" } else { "off" }),
            format!("solana       {}", if ready { "READY" } else { "DOWN" }),
            format!("cluster      {}", s.net_str("cluster").unwrap_or("—")),
            format!(
                "mode         {}",
                s.net_str("transfer_mode").unwrap_or("—")
            ),
        ]
    }

    fn lines_treasury(&self) -> Vec<String> {
        let s = &self.snap;
        let treasury = s
            .net_str("treasury")
            .or_else(|| s.net_str("settlement_authority"))
            .unwrap_or("");
        let mint = s.net_str("mint").unwrap_or("—");
        vec![
            format!("treasury  {}", short_addr(treasury, 6)),
            format!("mint      {}", short_addr(mint, 6)),
            format!(
                "token     {}",
                s.net_str("token_name").unwrap_or("The Black Bull")
            ),
            format!(
                "symbol    {}",
                s.net_str("token_symbol").unwrap_or("$ANSEM")
            ),
            format!(
                "decimals  {}",
                s.net_f("decimals")
                    .map(|d| format!("{d:.0}"))
                    .unwrap_or_else(|| "—".into())
            ),
        ]
    }

    fn lines_stake(&self) -> Vec<String> {
        let s = &self.snap;
        let fee = s.stake_f("platform_fee").or_else(|| s.cfg_f("platform_fee"));
        let staker = s
            .stake_f("staker_fee_rate")
            .or_else(|| s.cfg_f("staker_fee_rate"));
        let buyback = s
            .stake_f("buyback_fee_rate")
            .or_else(|| s.cfg_f("buyback_fee_rate"));
        vec![
            format!("platform   {}", rate_pct(fee)),
            format!("staker     {}", rate_pct(staker)),
            format!("buyback    {}", rate_pct(buyback)),
            format!(
                "routed 24h {}",
                fmt_ansem(s.stake_f("fees_routed_24h_ansem"))
            ),
            format!("pool       {}", fmt_ansem(s.stake_f("pool_pending_ansem"))),
        ]
    }

    fn lines_mcap(&self) -> Vec<String> {
        let s = &self.snap;
        let src = s
            .price
            .get("source")
            .and_then(|v| v.as_str())
            .or_else(|| s.ohlc.pointer("/stats/source").and_then(|v| v.as_str()))
            .unwrap_or("—");
        vec![
            format!("price   {}", fmt_usd(s.price_usd())),
            format!("24h     {}", delta_str(s.change_24h())),
            format!(
                "mcap    {}",
                fmt_usd(
                    s.ohlc_stat_f("market_cap")
                        .or_else(|| s.ohlc_stat_f("fdv"))
                )
            ),
            format!("fdv     {}", fmt_usd(s.ohlc_stat_f("fdv"))),
            format!("src     {src}"),
        ]
    }

    fn lines_announce(&self) -> Vec<String> {
        let s = &self.snap;
        if s.tweets.is_empty() {
            let mut lines = vec!["no posts in window".into()];
            if let Some(e) = &s.tweet_error {
                lines.push(e.chars().take(100).collect());
            }
            return lines;
        }
        s.tweets
            .iter()
            .map(|t| {
                let when = clock_mmdd_hhmm(t.created_at.as_deref());
                let prefix = if self.feed_mode == FeedMode::Both {
                    t.handle
                        .as_ref()
                        .map(|h| format!("@{h} "))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let mut text = t.text.replace('\n', " ");
                if text.chars().count() > 140 {
                    text = text.chars().take(137).collect::<String>() + "…";
                }
                format!("{when} POST  {prefix}{text}")
            })
            .collect()
    }

    fn lines_signals(&self) -> Vec<String> {
        let s = &self.snap;
        let mut lines = Vec::new();

        let ready = s.net_bool("solana_ready").unwrap_or(false);
        lines.push(format!(
            "{} SOLANA       {}",
            if ready { "●" } else { "○" },
            if ready {
                "ready · on-chain settle"
            } else {
                "not ready"
            }
        ));

        let transfer = s.net_str("transfer_mode").unwrap_or("unknown");
        lines.push(format!(
            "{} TRANSFER     {transfer}",
            if transfer == "solana" { "●" } else { "◐" }
        ));

        if s.price_usd().is_some() {
            let src = s
                .price
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("live");
            lines.push(format!("● PRICE        {src} feed"));
        } else {
            lines.push("○ PRICE        no quote".into());
        }

        let models = s.stats_u("live_models").unwrap_or(0);
        let liq = s.stats_f("liquidity_remaining");
        if models > 0 {
            lines.push(format!(
                "● MARKETS      {models} models · liq {}",
                fmt_compact(liq)
            ));
        } else {
            lines.push("◐ MARKETS      empty board".into());
        }

        let act = s
            .stats
            .pointer("/activity_24h/requests")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)));
        match act {
            Some(n) if n > 0 => lines.push(format!("● ACTIVITY 24h {n} requests")),
            Some(_) => lines.push("◐ ACTIVITY 24h quiet".into()),
            None => lines.push("◐ ACTIVITY 24h no data".into()),
        }

        if let Some(hc) = s.holders.holder_count {
            let mut detail = fmt_int(Some(hc));
            if let Some(ch) = s.holders.holder_change_24h {
                detail.push_str(&format!(" · 24h {ch:+.2}%"));
            }
            if let Some(t10) = s.holders.top10_pct {
                if t10 >= 50.0 {
                    detail.push_str(&format!(" · top10 {t10:.1}%"));
                    lines.push(format!("◐ HOLDERS      {detail}"));
                } else {
                    lines.push(format!("● HOLDERS      {detail}"));
                }
            } else {
                lines.push(format!("● HOLDERS      {detail}"));
            }
        }

        for (k, e) in s.errors.iter().take(4) {
            lines.push(format!("○ {}  {}", k.to_uppercase(), &e.chars().take(40).collect::<String>()));
        }
        lines
    }

    fn lines_activity(&self) -> Vec<String> {
        let s = &self.snap;
        if s.feed.is_empty() {
            return vec!["no inference yet".into()];
        }
        s.feed
            .iter()
            .map(|ev| {
                let t = clock_mmdd_hhmm(ev.created_at.as_deref());
                let model: String = ev.model_id.chars().take(22).collect();
                format!("{t}  {model:<22} {}", fmt_ansem(ev.cost))
            })
            .collect()
    }

    fn lines_market(&self) -> Vec<String> {
        let s = &self.snap;
        let spark = sparkline(&s.closes(), 28);
        let pair = s
            .ohlc
            .pointer("/stats/pair_address")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        vec![
            format!(
                "price  {}   24h {}",
                fmt_usd(s.price_usd()),
                delta_str(s.change_24h())
            ),
            format!(
                "vol    {}   liq {}",
                fmt_usd(s.ohlc_stat_f("volume_24h")),
                fmt_usd(s.ohlc_stat_f("liquidity_usd"))
            ),
            format!(
                "hi/lo  {} / {}",
                fmt_usd(s.ohlc_stat_f("high_24h")),
                fmt_usd(s.ohlc_stat_f("low_24h"))
            ),
            spark,
            format!("pair   {}", short_addr(pair, 6)),
        ]
    }

    fn lines_holders(&self) -> Vec<String> {
        let h = &self.snap.holders;
        if h.holder_count.is_none() && h.top_holders.is_empty() {
            let err = self
                .snap
                .errors
                .get("jupiter")
                .or_else(|| self.snap.errors.get("gecko"))
                .cloned()
                .unwrap_or_else(|| "—".into());
            return vec!["holders unavailable".into(), err];
        }

        let mut lines = vec![
            format!(
                "holders  {}   24h {}",
                fmt_int(h.holder_count),
                delta_str(h.holder_change_24h)
            ),
            format!(
                "1h {}   6h {}",
                delta_str(h.holder_change_1h),
                delta_str(h.holder_change_6h)
            ),
            format!(
                "top10    {}   rest {}",
                h.top10_pct
                    .map(|p| format!("{p:.2}%"))
                    .unwrap_or_else(|| "—".into()),
                h.rest_pct
                    .map(|p| format!("{p:.2}%"))
                    .unwrap_or_else(|| "—".into())
            ),
            format!(
                "circ     {}   traders 24h {}",
                fmt_compact(h.circ_supply),
                fmt_compact(h.traders_24h.map(|u| u as f64))
            ),
        ];
        for (i, th) in h.top_holders.iter().enumerate() {
            let pct = th
                .pct
                .map(|p| format!("{p:>5.1}%"))
                .unwrap_or_else(|| "  —  ".into());
            lines.push(format!(
                "#{}  {pct}  {}",
                i + 1,
                short_addr(&th.owner, 4)
            ));
        }
        lines
    }

    pub fn header_text(&self) -> String {
        let ch = delta_str(self.snap.change_24h());
        let handle = self.feed_mode.label(&self.cfg);
        let act = self
            .snap
            .stats
            .pointer("/activity_24h/requests")
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
            .unwrap_or(0);
        format!(" BULLBOARD · $ANSEM · @{handle} · {ch} · feed · {act} req/24h ")
    }

    pub fn footer_text(&self) -> String {
        let updated = ago(self.snap.fetched_at.as_deref());
        let pane = self.focus.pane().title();
        format!(
            " q quit · r refresh · n feed({}) · tab focus · ↑↓/jk/pg scroll · mouse wheel · focus:{pane} · updated {updated} · bullboard v{} ",
            self.feed_mode.as_str(),
            env!("CARGO_PKG_VERSION")
        )
    }

    pub fn announce_title(&self) -> String {
        let last = self
            .snap
            .tweets
            .first()
            .and_then(|t| t.created_at.as_deref())
            .map(|s| ago(Some(s)))
            .unwrap_or_else(|| "—".into());
        format!(
            "ANNOUNCE FEED · @{} · last {last}",
            self.feed_mode.label(&self.cfg)
        )
    }
}

pub async fn run_tui(cfg: Config) -> Result<()> {
    let client = http_client()?;
    let mut app = App::new(cfg, client);

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // initial fetch
    app.refresh_all().await;

    let tick = Duration::from_millis(100);
    let result = loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        let timeout = tick;
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.should_quit = true;
                    }
                    KeyCode::Char('r') => {
                        app.refresh_all().await;
                    }
                    KeyCode::Char('n') => {
                        app.feed_mode = app.feed_mode.next();
                        app.set_scroll(PaneId::Announce, 0);
                        app.refresh_feed().await;
                    }
                    KeyCode::Tab => {
                        app.focus = app.focus.next();
                    }
                    KeyCode::BackTab => {
                        app.focus = app.focus.prev();
                    }
                    KeyCode::Down | KeyCode::Char('j') => app.scroll_focused(1),
                    KeyCode::Up | KeyCode::Char('k') => app.scroll_focused(-1),
                    KeyCode::PageDown => app.scroll_focused(5),
                    KeyCode::PageUp => app.scroll_focused(-5),
                    KeyCode::Home => app.set_scroll(app.focus.pane(), 0),
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let n = c.to_digit(10).unwrap_or(0) as usize;
                        if (1..=NUM_PANES).contains(&n) {
                            app.focus = Focus::from_index(n - 1);
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollDown => app.scroll_focused(1),
                    MouseEventKind::ScrollUp => app.scroll_focused(-1),
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if app.should_quit {
            break Ok(());
        }

        if app.last_data.elapsed() >= Duration::from_secs(REFRESH_DATA_SECS) {
            app.refresh_data().await;
        }
        if app.last_feed.elapsed() >= Duration::from_secs(REFRESH_FEED_SECS) {
            app.refresh_feed().await;
        }
    };

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}


