use crate::config::{Config, FeedMode, REFRESH_DATA_SECS, REFRESH_FEED_SECS};
use crate::fetch::{fetch_announce, fetch_snapshot, http_client};
use crate::format::{
    age_from_ms, ago, bar, clock_mmdd_hhmm, delta_str, fmt_ansem, fmt_compact, fmt_int, fmt_usd,
    short_addr, sparkline,
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
            PaneId::Gate => self.lines_price_flow(),
            PaneId::Treasury => self.lines_primary_lp(),
            PaneId::Stake => self.lines_audit(),
            PaneId::Mcap => self.lines_supply(),
            PaneId::Announce => self.lines_announce(),
            PaneId::Signals => self.lines_signals(),
            PaneId::Activity => self.lines_activity(),
            PaneId::Market => self.lines_market(),
            PaneId::Holders => self.lines_holders(),
        }
    }

    /// Hero-first KPI: price, 24h, windows, flow — fits ~6 rows.
    fn lines_price_flow(&self) -> Vec<String> {
        let t = &self.snap.token;
        let s24 = &t.stats_24h;
        let pair = t.primary_pair.as_ref();
        let vol24 = s24
            .buy_volume
            .zip(s24.sell_volume)
            .map(|(b, s)| b + s)
            .or_else(|| pair.and_then(|p| p.vol_h24));
        vec![
            format!("ANSEM  {}", fmt_usd(self.snap.price_usd())),
            format!("24h    {}", delta_str(self.snap.change_24h())),
            format!(
                "5m {}  1h {}",
                delta_str(t.stats_5m.price_change.or_else(|| pair.and_then(|p| p.change_m5))),
                delta_str(t.stats_1h.price_change.or_else(|| pair.and_then(|p| p.change_h1)))
            ),
            format!(
                "6h {}",
                delta_str(t.stats_6h.price_change.or_else(|| pair.and_then(|p| p.change_h6)))
            ),
            format!("vol    {}", fmt_usd(vol24)),
            format!(
                "traders {}  net {}",
                fmt_int(s24.traders),
                fmt_int(s24.net_buyers)
            ),
            format!(
                "tx     {}B / {}S",
                fmt_int(s24.buys),
                fmt_int(s24.sells)
            ),
        ]
    }

    fn lines_primary_lp(&self) -> Vec<String> {
        let t = &self.snap.token;
        let Some(p) = t.primary_pair.as_ref() else {
            return vec![
                "no pair data".into(),
                format!("mint  {}", short_addr(&t.mint, 6)),
            ];
        };
        let total_liq: f64 = t.pairs.iter().filter_map(|x| x.liq_usd).sum();
        let all = if total_liq > 0.0 {
            Some(total_liq)
        } else {
            t.liquidity
        };
        vec![
            format!("{}  {}", p.dex_id.to_uppercase(), short_addr(&p.pair_address, 5)),
            format!("liq     {}", fmt_usd(p.liq_usd)),
            format!("all     {}  ({} pools)", fmt_usd(all), t.pairs.len()),
            format!("base    {} ANSEM", fmt_compact(p.liq_base)),
            format!("quote   {} {}", fmt_compact(p.liq_quote), p.quote_symbol),
            format!("age     {}   vol {}", age_from_ms(p.pair_created_ms), fmt_usd(p.vol_h24)),
            format!(
                "tx 24h  {}B / {}S",
                fmt_int(p.buys_h24),
                fmt_int(p.sells_h24)
            ),
        ]
    }

    fn lines_audit(&self) -> Vec<String> {
        let t = &self.snap.token;
        let mint_ok = t.mint_auth_disabled.unwrap_or(false);
        let freeze_ok = t.freeze_auth_disabled.unwrap_or(false);
        let mut lines = vec![
            format!(
                "mint    {}",
                if mint_ok { "DISABLED ✓" } else { "ENABLED ⚠" }
            ),
            format!(
                "freeze  {}",
                if freeze_ok { "DISABLED ✓" } else { "ENABLED ⚠" }
            ),
            format!(
                "rug     {}",
                t.rug_score
                    .map(|s| format!("{s:.0}/100"))
                    .unwrap_or_else(|| "—".into())
            ),
            format!(
                "organic {}",
                t.organic_score
                    .map(|s| {
                        let lab = t.organic_label.as_deref().unwrap_or("");
                        format!("{s:.0} {lab}").trim().to_string()
                    })
                    .unwrap_or_else(|| "—".into())
            ),
            format!(
                "lp lock {}",
                t.lp_locked_pct
                    .map(|p| format!("{p:.1}%"))
                    .unwrap_or_else(|| "n/a".into())
            ),
            format!(
                "insider {}  mkts {}",
                fmt_int(t.graph_insiders),
                fmt_int(t.markets_n)
            ),
        ];
        for r in t.risks.iter().take(3) {
            let val = if r.value.is_empty() {
                String::new()
            } else {
                format!(" {}", r.value)
            };
            lines.push(format!("· {}{}", r.name, val));
        }
        if let Some(dev) = &t.dev {
            lines.push(format!("dev     {}", short_addr(dev, 4)));
        }
        lines
    }

    fn lines_supply(&self) -> Vec<String> {
        let t = &self.snap.token;
        let circ = t.circ_supply;
        vec![
            format!("holders {}", fmt_int(t.holder_count)),
            format!("24h     {}", delta_str(t.stats_24h.holder_change)),
            format!("circ    {}", fmt_compact(circ)),
            format!("total   {}", fmt_compact(t.total_supply.or(circ))),
            format!(
                "mcap    {}",
                fmt_usd(t.mcap.or_else(|| self.snap.ohlc_stat_f("market_cap")))
            ),
            format!(
                "fdv     {}",
                fmt_usd(t.fdv.or_else(|| self.snap.ohlc_stat_f("fdv")))
            ),
            format!(
                "launch  {} {}",
                t.launchpad.as_deref().unwrap_or("—"),
                t.graduated_at
                    .as_deref()
                    .map(|s| s.get(..10).unwrap_or(s))
                    .unwrap_or("")
            ),
        ]
    }

    fn lines_announce(&self) -> Vec<String> {
        let s = &self.snap;
        if s.tweets.is_empty() {
            let mut lines = vec!["no posts in window".into()];
            if let Some(e) = &s.tweet_error {
                lines.push(e.chars().take(90).collect());
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
                // keep full-ish text; pane wraps
                let mut text = t.text.replace('\n', " ");
                if text.chars().count() > 220 {
                    text = text.chars().take(217).collect::<String>() + "…";
                }
                format!("{} POST {}{}", when, prefix, text)
            })
            .collect()
    }

    fn lines_signals(&self) -> Vec<String> {
        let t = &self.snap.token;
        let mut lines = Vec::new();

        let mint_ok = t.mint_auth_disabled.unwrap_or(false);
        lines.push(format!(
            "{} MINT AUTH    {}",
            if mint_ok { "●" } else { "○" },
            if mint_ok { "disabled · safe" } else { "still enabled" }
        ));
        let freeze_ok = t.freeze_auth_disabled.unwrap_or(false);
        lines.push(format!(
            "{} FREEZE AUTH  {}",
            if freeze_ok { "●" } else { "○" },
            if freeze_ok { "disabled · safe" } else { "still enabled" }
        ));

        let liq = t.liquidity.or_else(|| t.primary_pair.as_ref().and_then(|p| p.liq_usd));
        match liq {
            Some(v) if v >= 500_000.0 => {
                lines.push(format!("● LIQUIDITY    {} deep", fmt_usd(Some(v))))
            }
            Some(v) if v >= 50_000.0 => {
                lines.push(format!("◐ LIQUIDITY    {} thin", fmt_usd(Some(v))))
            }
            Some(v) => lines.push(format!("○ LIQUIDITY    {} low", fmt_usd(Some(v)))),
            None => lines.push("○ LIQUIDITY    no data".into()),
        }

        match t.stats_24h.holder_change {
            Some(c) if c > 0.0 => {
                lines.push(format!("● HOLDERS 24h  {} growing", delta_str(Some(c))))
            }
            Some(c) if c < 0.0 => {
                lines.push(format!("◐ HOLDERS 24h  {} shrinking", delta_str(Some(c))))
            }
            Some(_) => lines.push("◐ HOLDERS 24h  flat".into()),
            None => lines.push("◐ HOLDERS 24h  no data".into()),
        }

        match t.top10_pct.or(t.top_holders_pct) {
            Some(p) if p >= 50.0 => {
                lines.push(format!("◐ TOP10 CONC   {p:.1}% concentrated"))
            }
            Some(p) => lines.push(format!("● TOP10 CONC   {p:.1}% ok")),
            None => lines.push("◐ TOP10 CONC   —".into()),
        }

        match t.lp_locked_pct {
            Some(p) if p >= 50.0 => lines.push(format!("● LP LOCKED    {p:.1}%")),
            Some(p) => lines.push(format!("◐ LP LOCKED    {p:.1}%")),
            None => lines.push("◐ LP LOCKED    n/a".into()),
        }

        match t.rugged {
            Some(false) => lines.push("● RUG FLAG     clean".into()),
            Some(true) => lines.push("○ RUG FLAG     FLAGGED".into()),
            None => {}
        }

        if let Some(ch) = self.snap.change_24h() {
            let mark = if ch >= 0.0 { "●" } else { "◐" };
            lines.push(format!("{mark} PRICE 24h    {}", delta_str(Some(ch))));
        }

        let ready = self.snap.net_bool("solana_ready").unwrap_or(false);
        lines.push(format!(
            "{} SOLANA       {}",
            if ready { "●" } else { "○" },
            if ready { "rpc ready" } else { "down" }
        ));

        if let Some(score) = t.organic_score {
            lines.push(format!("● ORGANIC      {score:.0}"));
        }

        for r in t.risks.iter().take(2) {
            let mark = if r.level == "danger" { "○" } else { "◐" };
            lines.push(format!("{mark} RISK         {} {}", r.name, r.value));
        }
        lines
    }

    fn lines_activity(&self) -> Vec<String> {
        let t = &self.snap.token;
        let mut lines = Vec::new();
        lines.push("── dex flow ──".into());
        for (label, w) in [
            ("5m", &t.stats_5m),
            ("1h", &t.stats_1h),
            ("6h", &t.stats_6h),
            ("24h", &t.stats_24h),
        ] {
            lines.push(format!(
                "{label:<3} B {}  S {}  tr {}",
                fmt_usd(w.buy_volume),
                fmt_usd(w.sell_volume),
                fmt_int(w.traders)
            ));
            lines.push(format!(
                "    {}B/{}S  h {}",
                fmt_int(w.buys),
                fmt_int(w.sells),
                delta_str(w.holder_change)
            ));
        }
        lines.push("── top pairs ──".into());
        for p in t.pairs.iter().take(5) {
            lines.push(format!(
                "{:<8} liq {} vol {}",
                p.dex_id,
                fmt_usd(p.liq_usd),
                fmt_usd(p.vol_h24)
            ));
            lines.push(format!(
                "  {} {}  {}",
                short_addr(&p.pair_address, 4),
                p.quote_symbol,
                delta_str(p.change_h24)
            ));
        }
        if !self.snap.feed.is_empty() {
            lines.push("── inference ──".into());
            for ev in self.snap.feed.iter().take(6) {
                let tm = clock_mmdd_hhmm(ev.created_at.as_deref());
                let model: String = ev.model_id.chars().take(16).collect();
                lines.push(format!("{tm} {model:<16} {}", fmt_ansem(ev.cost)));
            }
        }
        lines
    }

    fn lines_market(&self) -> Vec<String> {
        let s = &self.snap;
        let t = &s.token;
        let price_spark = sparkline(&s.closes(), 28);
        let vol_spark = sparkline(&s.volumes(), 28);
        let p = t.primary_pair.as_ref();
        let vol24 = t
            .stats_24h
            .buy_volume
            .zip(t.stats_24h.sell_volume)
            .map(|(b, se)| b + se)
            .or_else(|| p.and_then(|x| x.vol_h24))
            .or_else(|| s.ohlc_stat_f("volume_24h"));
        let liq = t
            .liquidity
            .or_else(|| p.and_then(|x| x.liq_usd))
            .or_else(|| s.ohlc_stat_f("liquidity_usd"));
        let buy_v = t.stats_24h.buy_volume;
        let sell_v = t.stats_24h.sell_volume;
        let bias = match (buy_v, sell_v) {
            (Some(b), Some(se)) if b + se > 0.0 => {
                format!("buy bias {:.0}%", (b / (b + se)) * 100.0)
            }
            _ => "buy bias —".into(),
        };
        vec![
            format!(
                "ANSEM {}   24h {}",
                fmt_usd(s.price_usd()),
                delta_str(s.change_24h())
            ),
            format!(
                "5m {}  1h {}  6h {}",
                delta_str(t.stats_5m.price_change.or_else(|| p.and_then(|x| x.change_m5))),
                delta_str(t.stats_1h.price_change.or_else(|| p.and_then(|x| x.change_h1))),
                delta_str(t.stats_6h.price_change.or_else(|| p.and_then(|x| x.change_h6)))
            ),
            format!("vol  {}   liq {}", fmt_usd(vol24), fmt_usd(liq)),
            format!("buy  {}   sell {}", fmt_usd(buy_v), fmt_usd(sell_v)),
            format!(
                "tx   {}B / {}S   tr {}",
                fmt_int(t.stats_24h.buys.or_else(|| p.and_then(|x| x.buys_h24))),
                fmt_int(t.stats_24h.sells.or_else(|| p.and_then(|x| x.sells_h24))),
                fmt_int(t.stats_24h.traders)
            ),
            format!(
                "hi   {}  lo {}",
                fmt_usd(s.ohlc_stat_f("high_24h")),
                fmt_usd(s.ohlc_stat_f("low_24h"))
            ),
            format!("price {price_spark}"),
            format!("vol   {vol_spark}"),
            format!("mcap {}  fdv {}", fmt_usd(t.mcap), fmt_usd(t.fdv)),
            format!(
                "{} · {}",
                p.map(|x| format!("{} {}", x.dex_id, short_addr(&x.pair_address, 5)))
                    .unwrap_or_else(|| "pair —".into()),
                bias
            ),
        ]
    }

    fn lines_holders(&self) -> Vec<String> {
        let t = &self.snap.token;
        if t.holder_count.is_none() && t.top_holders.is_empty() {
            return vec![
                "holders unavailable".into(),
                self.snap
                    .errors
                    .get("jupiter")
                    .cloned()
                    .unwrap_or_else(|| "—".into()),
            ];
        }
        let top10 = t.top10_pct.or(t.top_holders_pct);
        let mut lines = vec![
            format!(
                "{} holders · tr 24h {}",
                fmt_int(t.holder_count),
                fmt_int(t.stats_24h.traders)
            ),
            format!(
                "1h {}  6h {}  24h {}",
                delta_str(t.stats_1h.holder_change),
                delta_str(t.stats_6h.holder_change),
                delta_str(t.stats_24h.holder_change)
            ),
            format!(
                "top10 {:>5} {}",
                top10.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "—".into()),
                bar(top10, 14)
            ),
            format!(
                "11-20 {:>5} {}",
                t.top11_20_pct
                    .map(|p| format!("{p:.1}%"))
                    .unwrap_or_else(|| "—".into()),
                bar(t.top11_20_pct, 14)
            ),
            format!(
                "rest  {:>5} {}",
                t.rest_pct
                    .map(|p| format!("{p:.1}%"))
                    .unwrap_or_else(|| "—".into()),
                bar(t.rest_pct, 14)
            ),
            format!(
                "dev {} · organic 24h {}",
                t.dev_balance_pct
                    .map(|p| {
                        if p < 0.01 {
                            format!("{p:.4}%")
                        } else {
                            format!("{p:.2}%")
                        }
                    })
                    .unwrap_or_else(|| "—".into()),
                fmt_int(t.stats_24h.organic_buyers)
            ),
            format!(
                "circ {} · net {}",
                fmt_compact(t.circ_supply),
                fmt_int(t.stats_24h.net_buyers)
            ),
            "── top holders ──".into(),
        ];
        for (i, th) in t.top_holders.iter().take(8).enumerate() {
            let pct = th
                .pct
                .map(|p| format!("{p:>5.2}%"))
                .unwrap_or_else(|| "   —  ".into());
            let amt = th
                .ui_amount
                .map(|a| fmt_compact(Some(a)))
                .unwrap_or_else(|| "—".into());
            let flag = if th.insider { " *" } else { "" };
            lines.push(format!(
                "#{:<2} {pct} {:>9}  {}{flag}",
                i + 1,
                amt,
                short_addr(&th.owner, 4)
            ));
        }
        if let Some(c) = &t.creator {
            lines.push(format!("creator {}", short_addr(c, 5)));
        }
        if let Some(d) = &t.dev {
            lines.push(format!("dev     {}", short_addr(d, 5)));
        }
        lines
    }

    pub fn header_text(&self) -> String {
        let price = fmt_usd(self.snap.price_usd());
        let ch = delta_str(self.snap.change_24h());
        let handle = self.feed_mode.label(&self.cfg);
        let holders = fmt_int(self.snap.token.holder_count);
        let liq = fmt_usd(
            self.snap.token.liquidity.or_else(|| {
                self.snap
                    .token
                    .primary_pair
                    .as_ref()
                    .and_then(|p| p.liq_usd)
            }),
        );
        format!(
            " BULLBOARD · ANSEM {price} · {ch} · holders {holders} · liq {liq} · @{handle} "
        )
    }

    pub fn footer_text(&self) -> String {
        let updated = ago(self.snap.fetched_at.as_deref());
        let pane = self.focus.pane().title();
        format!(
            " q quit · r refresh · n feed · tab · ↑↓ scroll · {}s · updated {} · focus:{} · bullboard v{} · matrix ",
            REFRESH_DATA_SECS,
            updated,
            pane,
            env!("CARGO_PKG_VERSION"),
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


