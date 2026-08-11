use crate::config::{Config, REFRESH_DATA_SECS};
use crate::fetch::{fetch_announce, fetch_snapshot, http_client, FeedHealth};
use crate::format::{
    ago, bar, clock_mmdd_hhmm, delta_str, fmt_compact, fmt_int, fmt_usd, is_post_line, short_addr,
    sparkline,
};
use crate::model::{Snapshot, Tweet};
use crate::ui::{self, Focus, PaneAreas, PaneId, NUM_PANES};
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Terminal;
use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::io::stdout;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::config::{ACID, BAD, MUTED, WARN};

/// On-screen control under each announce post (click / Enter / `o` opens URL).
/// Leading spaces keep the acid chip left-aligned under the post body.
pub const VIEW_TWEET_BTN: &str = "  [ view tweet ]  ";

/// Result of a background refresh — the app loop drains these every tick so
/// input never blocks on the network.
enum RefreshMsg {
    /// Fresh market/on-chain snapshot (tweets reattached on receipt).
    Data(Box<Snapshot>),
    /// Fresh announce feed plus the circuit-breaker state that poll produced.
    Feed(Vec<Tweet>, Option<String>, FeedHealth),
}

/// The data endpoints `snap.errors` is keyed by — drives the `src n/m` health
/// counter in the header.
const SOURCE_KEYS: [&str; 7] = [
    "price",
    "ohlc",
    "jupiter",
    "gecko",
    "gecko-token",
    "rugcheck",
    "dexscreener",
];

pub struct App {
    pub cfg: Config,
    pub client: Client,
    pub snap: Snapshot,
    pub focus: Focus,
    /// scroll offset (lines) per pane
    pub scroll: HashMap<PaneId, u16>,
    /// last computed pane rects (updated every frame for mouse hit-test)
    pub pane_areas: PaneAreas,
    /// Selected announce post (acid chip + brighter body).
    pub selected_tweet: Option<usize>,
    /// Pane under the mouse cursor (border / title highlight).
    pub hover_pane: Option<PaneId>,
    /// Announce post under the mouse (soft highlight; not the same as selected).
    pub hover_tweet: Option<usize>,
    /// True while a data refresh is in flight (header shows a spinner).
    pub loading: bool,
    pub should_quit: bool,
    /// Last user-facing message ("updated 5s ago", "opened tweet #2", …) —
    /// rendered in the footer's right segment.
    pub status: String,
    /// Desktop notifications on (`t` toggles; BULLBOARD_NOTIFY sets initial).
    pub notify_enabled: bool,
    /// Circuit-breaker state per feed mirror (auto-skips blocking mirrors).
    pub feed_health: FeedHealth,
    /// `?`/`h` help overlay.
    pub show_help: bool,
    /// Background refreshes deliver results here.
    refresh_rx: mpsc::UnboundedReceiver<RefreshMsg>,
    /// Clone handed to spawned refresh tasks.
    refresh_tx: mpsc::UnboundedSender<RefreshMsg>,
    /// One data / feed refresh in flight at a time (no pile-up).
    data_busy: bool,
    feed_busy: bool,
    last_data: Instant,
    last_feed: Instant,
    /// Tweet ids already announced; a new id fires a desktop notification.
    seen_tweets: HashSet<String>,
    /// Cached pane lines; `None` after `RefreshMsg`.
    line_cache: Option<Box<[Vec<String>]>>,
}

impl App {
    pub fn new(cfg: Config, client: Client) -> Self {
        let notify = cfg.notify;
        let (refresh_tx, refresh_rx) = mpsc::unbounded_channel();
        Self {
            cfg,
            client,
            snap: Snapshot::default(),
            focus: Focus::default(),
            scroll: HashMap::new(),
            pane_areas: PaneAreas::default(),
            selected_tweet: None,
            hover_pane: None,
            hover_tweet: None,
            loading: false,
            should_quit: false,
            status: "loading…".into(),
            notify_enabled: notify,
            feed_health: FeedHealth::default(),
            show_help: false,
            refresh_rx,
            refresh_tx,
            data_busy: false,
            feed_busy: false,
            last_data: Instant::now() - Duration::from_secs(999),
            last_feed: Instant::now() - Duration::from_secs(999),
            seen_tweets: HashSet::new(),
            line_cache: None,
        }
    }

    const fn pane_idx(id: PaneId) -> usize {
        match id {
            PaneId::Gate => 0,
            PaneId::Treasury => 1,
            PaneId::Stake => 2,
            PaneId::Mcap => 3,
            PaneId::Announce => 4,
            PaneId::Signals => 5,
            PaneId::Activity => 6,
            PaneId::Market => 7,
            PaneId::Holders => 8,
        }
    }

    const LINE_BUILDERS: [fn(&App) -> Vec<String>; NUM_PANES] = [
        App::lines_price_flow,
        App::lines_primary_lp,
        App::lines_audit,
        App::lines_supply,
        App::lines_announce,
        App::lines_signals,
        App::lines_activity,
        App::lines_market,
        App::lines_holders,
    ];

    fn ensure_line_cache(&mut self) {
        if self.line_cache.is_some() {
            return;
        }
        let cache: Box<[Vec<String>]> = Self::LINE_BUILDERS
            .iter()
            .map(|build| build(self))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.line_cache = Some(cache);
    }

    pub fn scroll_of(&self, id: PaneId) -> u16 {
        self.scroll.get(&id).copied().unwrap_or(0)
    }

    pub fn set_scroll(&mut self, id: PaneId, v: u16) {
        self.scroll.insert(id, v);
    }

    /// Visual line count for a pane given its current width (wrap-aware).
    fn visual_line_count(&self, id: PaneId, area: Rect) -> usize {
        let lines = self.lines_for(id);
        if id != PaneId::Announce {
            return lines.len();
        }
        let content_w = ui::pane_content_width(area);
        if content_w == 0 {
            return lines.len();
        }
        lines
            .iter()
            .map(|l| Self::announce_line_height(l, content_w))
            .sum()
    }

    fn max_scroll_for(&self, id: PaneId) -> u16 {
        let area = self.pane_areas.get(id);
        let n = self.visual_line_count(id, area);
        ui::pane_scroll_bounds_for(area, id, n).1
    }

    /// Scroll any pane independently; clamps to content.
    pub fn scroll_pane(&mut self, id: PaneId, delta: i32) {
        let max = self.max_scroll_for(id) as i32;
        let cur = self.scroll_of(id) as i32;
        let next = (cur + delta).clamp(0, max) as u16;
        self.set_scroll(id, next);
    }

    pub fn scroll_focused(&mut self, delta: i32) {
        self.scroll_pane(self.focus.pane(), delta);
    }

    /// Re-clamp every pane after resize / content change.
    pub fn clamp_all_scrolls(&mut self) {
        for id in PaneId::all() {
            let max = self.max_scroll_for(id);
            let cur = self.scroll_of(id);
            if cur > max {
                self.set_scroll(id, max);
            }
        }
    }

    pub fn focus_at(&mut self, col: u16, row: u16) -> Option<PaneId> {
        let hit = self.pane_areas.hit(col, row)?;
        // set Focus by index
        if let Some(idx) = PaneId::all().iter().position(|&p| p == hit) {
            self.focus = Focus::from_index(idx);
        }
        Some(hit)
    }

    /// Update hover targets from mouse move. Cheap; called every Moved event.
    pub fn set_hover(&mut self, col: u16, row: u16) {
        let pane = self.pane_areas.hit(col, row);
        self.hover_pane = pane;
        if pane == Some(PaneId::Announce) {
            self.hover_tweet = self.tweet_under_cursor(col, row);
        } else {
            self.hover_tweet = None;
        }
    }

    /// Which announce tweet (if any) is under the cursor — body or button row.
    fn tweet_under_cursor(&self, col: u16, row: u16) -> Option<usize> {
        let area = self.pane_areas.get(PaneId::Announce);
        if area.width < 4 || area.height < 2 {
            return None;
        }
        let inner_x = area.x.saturating_add(1 + 1);
        let inner_y = area.y.saturating_add(1 + 1);
        let inner_w = area.width.saturating_sub(2 + 2);
        let inner_h = area.height.saturating_sub(2 + 1 + 1);
        if col < inner_x
            || col >= inner_x.saturating_add(inner_w)
            || row < inner_y
            || row >= inner_y.saturating_add(inner_h)
        {
            return None;
        }
        let content_row = row
            .saturating_sub(inner_y)
            .saturating_add(self.scroll_of(PaneId::Announce));
        self.announce_hit(content_row, inner_w as usize).0
    }

    /// Kick off a background data refresh; results arrive via `refresh_rx`.
    /// Input never blocks on the network — the loop drains the channel.
    pub fn trigger_data(&mut self) {
        if self.data_busy {
            return;
        }
        self.data_busy = true;
        self.loading = true;
        let cfg = self.cfg.clone();
        let client = self.client.clone();
        let tx = self.refresh_tx.clone();
        tokio::spawn(async move {
            let snap = fetch_snapshot(&client, &cfg).await;
            let _ = tx.send(RefreshMsg::Data(Box::new(snap)));
        });
    }

    /// Kick off a background announce-feed refresh. The circuit-breaker state
    /// lives in the app; the task gets a clone and hands the updated state
    /// back inside the message so tripped mirrors stay skipped across polls.
    pub fn trigger_feed(&mut self) {
        if self.feed_busy {
            return;
        }
        self.feed_busy = true;
        let cfg = self.cfg.clone();
        let client = self.client.clone();
        let tx = self.refresh_tx.clone();
        let mut health = self.feed_health.clone();
        tokio::spawn(async move {
            let (tweets, err) = fetch_announce(&client, &cfg, 40, &mut health).await;
            let _ = tx.send(RefreshMsg::Feed(tweets, err, health));
        });
    }

    /// Apply one background refresh result (called every tick while queued).
    fn handle_msg(&mut self, msg: RefreshMsg) {
        self.line_cache = None;
        match msg {
            RefreshMsg::Data(boxed) => {
                let old = std::mem::take(&mut self.snap);
                let merged = Snapshot::merge_stale(*boxed, &old);
                let mut merged = merged;
                // fetch_snapshot doesn't know about the feed — reattach it.
                merged.tweets = old.tweets;
                merged.tweet_error = old.tweet_error;
                self.snap = merged;
                self.last_data = Instant::now();
                self.data_busy = false;
                self.loading = false;
                self.status = format!("updated {}", ago(self.snap.fetched_at.as_deref()));
            }
            RefreshMsg::Feed(tweets, err, health) => {
                // Nitter mirrors are flaky: if a poll comes back empty with an
                // error, keep the last good feed so the pane never blanks.
                if tweets.is_empty() && err.is_some() && !self.snap.tweets.is_empty() {
                    self.snap.tweet_error = err;
                } else {
                    self.notify_new_tweets(&tweets);
                    self.snap.tweets = tweets;
                    self.snap.tweet_error = err;
                }
                self.feed_health = health;
                self.last_feed = Instant::now();
                self.feed_busy = false;
            }
        }
    }

    /// Fire desktop notifications for announce tweets seen for the first time
    /// (only while the `t` toggle is on). The first fetch just seeds the
    /// seen-set, so a fresh run doesn't replay the whole feed as a wall of
    /// notifications. The seen-set always tracks the feed, so toggling off and
    /// back on never replays posts that arrived in between.
    fn notify_new_tweets(&mut self, tweets: &[Tweet]) {
        let seeded = !self.seen_tweets.is_empty();
        let fresh = mark_new_tweets(&mut self.seen_tweets, tweets, seeded);
        if !self.notify_enabled {
            return;
        }
        for i in fresh {
            let t = &tweets[i];
            let mut body = t.text.replace('\n', " ").trim().to_string();
            if body.chars().count() > 140 {
                body = body.chars().take(137).collect::<String>() + "…";
            }
            if let Some(url) = tweet_view_url(t) {
                body.push_str(&format!(" — {url}"));
            }
            let title = format!("⬢ @{} posted", self.cfg.x_handle);
            tokio::task::spawn_blocking(move || {
                let _ = notify_desktop(&title, &body);
            });
        }
    }

    pub fn lines_for(&self, id: PaneId) -> Vec<String> {
        if let Some(cache) = &self.line_cache {
            return cache[Self::pane_idx(id)].clone();
        }
        Self::LINE_BUILDERS[Self::pane_idx(id)](self)
    }

    fn announce_line_height(line: &str, content_w: usize) -> usize {
        if line.is_empty() || line == VIEW_TWEET_BTN {
            1
        } else {
            line.chars().count().max(1).div_ceil(content_w).max(1)
        }
    }

    /// Best available 24h volume across sources (Jupiter → pair → Bull API → Gecko).
    fn vol24(&self) -> Option<f64> {
        let t = &self.snap.token;
        let p = t.primary_pair.as_ref();
        t.stats_24h
            .buy_volume
            .zip(t.stats_24h.sell_volume)
            .map(|(b, s)| b + s)
            .or_else(|| p.and_then(|x| x.vol_h24))
            .or_else(|| self.snap.ohlc_stat_f("volume_24h"))
            .or(self.snap.gecko_token.vol_24h)
    }

    /// Best available liquidity across sources.
    fn liq_usd(&self) -> Option<f64> {
        let t = &self.snap.token;
        t.total_liquidity()
            .or_else(|| self.snap.ohlc_stat_f("liquidity_usd"))
            .or_else(|| self.snap.gecko_liq_usd())
    }

    /// (healthy, total) data sources — drives the `src n/m` header counter.
    fn sources_health(&self) -> (usize, usize) {
        let bad = SOURCE_KEYS
            .iter()
            .filter(|k| self.snap.errors.contains_key(**k))
            .count();
        (SOURCE_KEYS.len() - bad, SOURCE_KEYS.len())
    }

    /// The token's display symbol, with the ANSEM default for custom mints.
    fn symbol(&self) -> &str {
        let s = self.snap.token.symbol.trim();
        if s.is_empty() {
            "ANSEM"
        } else {
            s
        }
    }

    /// KPI card: hero · 24h · vol · gecko cross-check (amber `(!)` when the
    /// two price sources disagree by more than 1%).
    fn lines_price_flow(&self) -> Vec<String> {
        let mut gk = format!("gecko {}", fmt_usd(self.snap.gecko_token.price_usd));
        if let (Some(px), Some(g)) = (self.snap.price_usd(), self.snap.gecko_token.price_usd) {
            if px != 0.0 && (g - px).abs() / px.abs() > 0.01 {
                gk.push_str(" (!)");
            }
        }
        vec![
            fmt_usd(self.snap.price_usd()),
            format!("24h {}", delta_str(self.snap.change_24h())),
            format!("vol {}", fmt_usd(self.vol24())),
            gk,
        ]
    }

    /// Total LP across every live pool (DexScreener sum), cross-checked with
    /// RugCheck's market-wide total and GeckoTerminal's per-pool reserves.
    /// Every source gets its own named line so a number is never orphaned.
    fn lines_primary_lp(&self) -> Vec<String> {
        let t = &self.snap.token;
        let total = t.total_liquidity();
        let pools = t.pairs.len();
        let Some(p) = t.primary_pair.as_ref() else {
            let mut lines = vec![
                fmt_usd(total),
                "no pair".into(),
                if pools > 0 {
                    format!("{pools} pools")
                } else {
                    "—".into()
                },
            ];
            if let Some(g) = self.snap.gecko_liq_usd() {
                lines.push(format!("gecko {}", fmt_usd(Some(g))));
            }
            return lines;
        };
        let pools_s = if pools == 1 {
            "1 pool".into()
        } else {
            format!("{pools} pools")
        };
        // Line 2: DEX + the pair it quotes, e.g. "PUMPSWAP ANSEM/SOL".
        let quote = p.quote_symbol.trim();
        let pair_line = if quote.is_empty() || quote == "?" {
            p.dex_id.to_uppercase()
        } else {
            format!("{} {}/{}", p.dex_id.to_uppercase(), self.symbol(), quote)
        };
        // Second-opinion liquidity with the same (!) disagreement flag the
        // Activity pane uses, so the card and the rail agree.
        let mut gk = String::new();
        if let Some(g) = self.snap.gecko_liq_usd() {
            let flag = match total {
                Some(h) if h > 0.0 && (g - h).abs() / h > 0.25 => " (!)",
                _ => "",
            };
            gk = format!("gecko {}{flag}", fmt_usd(Some(g)));
        }
        let mut lines = vec![
            fmt_usd(total),
            pair_line,
            pools_s,
        ];
        // RugCheck.xyz independently scans the same markets — its total is
        // the second opinion for the headline number.
        if let Some(r) = t.total_market_liq {
            if r > 0.0 {
                lines.push(format!("rugcheck {}", fmt_usd(Some(r))));
            }
        } else if let Some(pl) = p.liq_usd {
            lines.push(format!("primary {}", fmt_usd(Some(pl))));
        }
        if !gk.is_empty() {
            lines.push(gk);
        }
        lines
    }

    fn lines_audit(&self) -> Vec<String> {
        let t = &self.snap.token;
        let mint_ok = t.mint_auth_disabled.unwrap_or(false);
        let freeze_ok = t.freeze_auth_disabled.unwrap_or(false);
        let safe = mint_ok && freeze_ok && t.rugged != Some(true);
        let hero = if safe {
            "CLEAN".to_string()
        } else if t.rugged == Some(true) {
            "FLAGGED".to_string()
        } else {
            "REVIEW".to_string()
        };
        let lp = t
            .lp_locked_pct
            .map(|p| format!("{p:.0}%"))
            .unwrap_or_else(|| "—".into());
        let rug = t
            .rug_score
            .map(|s| format!("rug {s:.0}"))
            .unwrap_or_else(|| "rug —".into());
        let insiders = t
            .graph_insiders
            .map(|n| format!("insiders {n}"))
            .unwrap_or_else(|| "insiders —".into());
        // Dev holdings ≥ 10% is a real rug-watch signal — badge it.
        let mut dev_badge = String::new();
        if let Some(p) = t.dev_balance_pct {
            if p >= 10.0 {
                dev_badge = format!(" [dev {p:.0}%]");
            }
        }
        vec![
            hero,
            format!(
                "mint {} · freeze {}",
                if mint_ok { "off" } else { "on" },
                if freeze_ok { "off" } else { "on" }
            ),
            format!("{rug} · {insiders}"),
            format!("lp {lp} locked{dev_badge}"),
        ]
    }

    fn lines_supply(&self) -> Vec<String> {
        let t = &self.snap.token;
        let holders = fmt_int(t.holder_count);
        let mcap = fmt_usd(
            t.mcap
                .or_else(|| self.snap.ohlc_stat_f("market_cap"))
                .or(self.snap.gecko_token.market_cap),
        );
        let fdv = fmt_usd(
            t.fdv
                .or(self.snap.gecko_token.fdv)
                .or_else(|| t.primary_pair.as_ref().and_then(|p| p.fdv)),
        );
        // Badges live on the MARKET head (wide pane); cards stay terse.
        let circ = t
            .circ_supply
            .map(|c| fmt_compact(Some(c)))
            .unwrap_or_else(|| "—".into());
        // circ == total (no burn) reads as "circ 999.82M / 999.82M" — show
        // just the one number when they're effectively the same.
        let circ_line = match (t.circ_supply, t.total_supply) {
            (Some(c), Some(total)) if total > 0.0 && ((total - c) / total).abs() < 0.01 => {
                format!("circ {circ}")
            }
            (Some(_), Some(total)) => format!("circ {circ} / {}", fmt_compact(Some(total))),
            (Some(_), None) => format!("circ {circ}"),
            (None, Some(total)) => format!("circ — / {}", fmt_compact(Some(total))),
            (None, None) => "circ —".into(),
        };
        vec![
            holders,
            format!("mcap {mcap}"),
            circ_line,
            format!("fdv {fdv}"),
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
        let mut out = Vec::new();
        for t in &s.tweets {
            let when = clock_mmdd_hhmm(t.created_at.as_deref());
            let mut text = t.text.replace('\n', " ");
            if text.chars().count() > 160 {
                text = text.chars().take(157).collect::<String>() + "…";
            }
            // Tag retweets / quote retweets so they don't read like original posts.
            let tag = if let Some(a) = &t.quote_author {
                format!("QT @{a} ")
            } else if let Some(a) = &t.retweet_of {
                format!("RT @{a} ")
            } else if t.retweet {
                "RT ".into()
            } else {
                String::new()
            };
            // POST body → blank → button → blank (between posts)
            out.push(format!("{} POST {}{}", when, tag, text));
            out.push(String::new());
            if tweet_view_url(t).is_some() {
                out.push(VIEW_TWEET_BTN.to_string());
            }
            out.push(String::new());
        }
        // A failed poll keeps the last good feed — surface it so a stale
        // pane doesn't read as frozen.
        if let Some(e) = &s.tweet_error {
            out.push(format!(
                "— mirror error, showing last good ({})",
                e.chars().take(50).collect::<String>()
            ));
        }
        out
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

        let liq = t.total_liquidity();
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

        // Dev wallet concentration — a fat dev balance is a rug-watch flag.
        match t.dev_balance_pct {
            Some(p) if p >= 10.0 => lines.push(format!("○ DEV CONC    {p:.1}% high")),
            Some(p) if p >= 5.0 => lines.push(format!("◐ DEV CONC    {p:.1}%")),
            Some(p) => lines.push(format!("● DEV CONC    {p:.1}%")),
            None => {}
        }

        // Cap at 6 high-signal rows; rug/price/organic/risks only if room.
        if lines.len() < 6 {
            match t.rugged {
                Some(false) => lines.push("● RUG FLAG     clean".into()),
                Some(true) => lines.push("○ RUG FLAG     FLAGGED".into()),
                None => {}
            }
        }
        if lines.len() < 6 {
            if let Some(ch) = self.snap.change_24h() {
                let mark = if ch >= 0.0 { "●" } else { "◐" };
                lines.push(format!("{mark} PRICE 24h    {}", delta_str(Some(ch))));
            }
        }
        if lines.len() < 6 {
            if let Some(score) = t.organic_score {
                lines.push(format!("● ORGANIC      {score:.0}"));
            }
        }
        for r in t.risks.iter() {
            if lines.len() >= 6 {
                break;
            }
            let mark = if r.level == "danger" { "○" } else { "◐" };
            lines.push(format!("{mark} RISK         {} {}", r.name, r.value));
        }
        lines.truncate(6);
        lines
    }

    fn lines_activity(&self) -> Vec<String> {
        let s = &self.snap;
        let t = &s.token;
        let mut lines = Vec::new();
        // One row per window (not two). Right-aligned columns so the legs
        // line up across windows instead of trailing off ragged.
        for (label, w) in [
            ("5m", &t.stats_5m),
            ("1h", &t.stats_1h),
            ("6h", &t.stats_6h),
            ("24h", &t.stats_24h),
        ] {
            lines.push(format!(
                "{label:<3} B {:>8}  S {:>8}  {:>6}B/{:<6}S",
                fmt_usd(w.buy_volume),
                fmt_usd(w.sell_volume),
                fmt_int(w.buys),
                fmt_int(w.sells),
            ));
        }
        lines.push(String::new());
        // 24h buyer-quality summary: organic vs net split is the strongest
        // "is this real demand" signal; trader count completes the row.
        let s24 = &t.stats_24h;
        if s24.organic_buyers.is_some() || s24.net_buyers.is_some() || s24.traders.is_some() {
            let mut sum = "24h ".to_string();
            if let (Some(o), Some(n)) = (s24.organic_buyers, s24.net_buyers) {
                sum.push_str(&format!("org {o} · net {n}"));
            }
            if let Some(tr) = s24.traders {
                if sum != "24h " {
                    sum.push_str(" · ");
                }
                sum.push_str(&format!("tr {}", fmt_int(Some(tr))));
            }
            lines.push(sum);
        }
        // Aggregate liquidity across all pools — per DEX, biggest first.
        if let Some(tot) = t.total_liquidity() {
            let pools = t.pairs.len();
            lines.push(format!(
                "TOTAL LP  {} · {pools} pool{}",
                fmt_usd(Some(tot)),
                if pools == 1 { "" } else { "s" }
            ));
        }
        let mut dexs: Vec<(String, f64, f64, usize)> = Vec::new();
        for p in &t.pairs {
            let name = p.dex_id.clone();
            match dexs.iter_mut().find(|(n, _, _, _)| *n == name) {
                Some(e) => {
                    e.1 += p.liq_usd.unwrap_or(0.0);
                    e.2 += p.vol_h24.unwrap_or(0.0);
                    e.3 += 1;
                }
                None => dexs.push((name, p.liq_usd.unwrap_or(0.0), p.vol_h24.unwrap_or(0.0), 1)),
            }
        }
        dexs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (dex, liq, vol, n) in dexs.iter().take(4) {
            let note = if *n > 1 { format!(" ×{n}") } else { String::new() };
            lines.push(format!(
                "{:<9} liq {} vol {}{}",
                dex,
                fmt_usd(Some(*liq)),
                fmt_usd(Some(*vol)),
                note
            ));
        }
        if t.pairs.is_empty() {
            lines.push("no pair data".into());
        }
        // Second-opinion liquidity from Gecko token reserves; flag it
        // when it disagrees with DexScreener by more than 25%.
        if let Some(g_liq) = s.gecko_liq_usd() {
            let dex_liq: f64 = t.pairs.iter().filter_map(|x| x.liq_usd).sum();
            let flag = if dex_liq > 0.0 && (g_liq - dex_liq).abs() / dex_liq > 0.25 {
                " (!)"
            } else {
                ""
            };
            lines.push(format!(
                "gecko  liq {} vol {}{flag}",
                fmt_usd(Some(g_liq)),
                fmt_usd(s.gecko_token.vol_24h),
            ));
        }
        lines
    }

    fn lines_market(&self) -> Vec<String> {
        let s = &self.snap;
        let t = &s.token;
        let price_spark = sparkline(&s.closes(), 28);
        let vol_spark = sparkline(&s.volumes(), 28);
        let p = t.primary_pair.as_ref();
        let vol24 = self.vol24();
        let liq = self.liq_usd();
        let symbol = self.symbol();
        // Second-opinion price: surface it when sources diverge > 1%.
        let mut head = format!(
            "{symbol} {}   24h {}",
            fmt_usd(s.price_usd()),
            delta_str(s.change_24h())
        );
        if let (Some(px), Some(gk)) = (s.price_usd(), s.gecko_token.price_usd) {
            if px != 0.0 && ((gk - px).abs() / px.abs()) > 0.01 {
                head.push_str(&format!(" · gecko {}", fmt_usd(Some(gk))));
            }
        }
        // Badge chips — verified / launchpad / graduated read as trust marks.
        if t.is_verified == Some(true) {
            head.push_str(" [verified]");
        }
        if let Some(lp) = &t.launchpad {
            head.push_str(&format!(" [{lp}]"));
        }
        if t.graduated_at.is_some() {
            head.push_str(" [graduated]");
        }
        // ≤6 lines: price+24h, windows, 24h range, vol·liq, price spark, vol spark
        let (hi, lo) = s.range_24h();
        vec![
            head,
            format!(
                "5m {}  1h {}  6h {}",
                delta_str(t.stats_5m.price_change.or_else(|| p.and_then(|x| x.change_m5))),
                delta_str(t.stats_1h.price_change.or_else(|| p.and_then(|x| x.change_h1))),
                delta_str(t.stats_6h.price_change.or_else(|| p.and_then(|x| x.change_h6)))
            ),
            format!("24h {} / {}", fmt_usd(lo), fmt_usd(hi)),
            format!("vol {} · liq {}", fmt_usd(vol24), fmt_usd(liq)),
            format!("price {price_spark}"),
            format!("vol   {vol_spark}"),
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
        // count + deltas, top10 bar, distribution bands, blank, top holders
        let mut lines = vec![
            format!(
                "{}  1h {}  6h {}  24h {}",
                fmt_int(t.holder_count),
                delta_str(t.stats_1h.holder_change),
                delta_str(t.stats_6h.holder_change),
                delta_str(t.stats_24h.holder_change)
            ),
            format!(
                "top10 {:>5} {}",
                top10.map(|p| format!("{p:.1}%")).unwrap_or_else(|| "—".into()),
                bar(top10, 14)
            ),
            String::new(),
        ];
        // Gecko's holder-distribution bands — how much of the supply sits in
        // the top 40 wallets vs the rest (concentration = rug risk).
        if let (Some(a), Some(b), Some(c)) = (t.top11_20_pct, t.top21_40_pct, t.rest_pct) {
            lines.push(format!(
                "dist  11-20 {a:.1}% · 21-40 {b:.1}% · rest {c:.1}%"
            ));
        }
        for (i, th) in t.top_holders.iter().take(4).enumerate() {
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
        lines
    }

    /// Surfboard ticker: muted chrome, acid only on price + change. Richer in
    /// v0.4: symbol, 24h delta colored by direction, 24h volume, and a live
    /// `src n/m` health counter (amber when a source is down).
    pub fn header_line(&self) -> Line<'static> {
        let symbol = self.symbol();
        let price = fmt_usd(self.snap.price_usd());
        let ch = delta_str(self.snap.change_24h());
        let delta_style = if ch.starts_with('▲') {
            Style::default().fg(ACID).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(BAD).add_modifier(Modifier::BOLD)
        };
        let (src_ok, src_total) = self.sources_health();
        let src_style = if src_ok < src_total {
            Style::default().fg(WARN)
        } else {
            Style::default().fg(MUTED)
        };
        let mut spans = vec![
            Span::styled(
                format!(" BULLBOARD · {symbol} "),
                Style::default().fg(MUTED),
            ),
            Span::styled(
                price,
                Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · 24h ".to_string(), Style::default().fg(MUTED)),
            Span::styled(ch, delta_style),
            Span::styled(
                format!(" · vol {} ", fmt_usd(self.vol24())),
                Style::default().fg(MUTED),
            ),
            Span::styled(
                format!("· src {src_ok}/{src_total} "),
                src_style,
            ),
        ];
        if self.loading {
            spans.push(Span::styled("⟳ ".to_string(), Style::default().fg(MUTED)));
        }
        Line::from(spans)
    }

    /// Footer left segment: the key shortcuts (full reference in the `?` overlay).
    pub fn footer_keys(&self) -> &'static str {
        " q quit · r refresh · n feed · t alerts · ? help · o open · tab · j/k "
    }

    /// Footer right segment: live status — last action, focused pane, alert
    /// toggle, and version.
    pub fn footer_status(&self) -> String {
        let pane = self.focus.pane().title();
        let alerts = if self.notify_enabled { "on" } else { "off" };
        format!(
            "{} · {} · alerts:{alerts} · v{}",
            self.status,
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
        // Degraded mirror health shows up in the title instead of hiding.
        let live = self.feed_health.live_mirrors(&self.cfg.mirrors);
        let total = self.cfg.mirrors.len();
        let mirrors = if live < total {
            format!(" · mirrors {live}/{total}")
        } else {
            String::new()
        };
        format!(
            "ANNOUNCE · @{} · {}s · last {last}{mirrors}",
            self.cfg.x_handle,
            self.last_feed.elapsed().as_secs(),
        )
    }

    /// Hit-test announce content. Body click selects; button click selects + opens.
    pub fn click_announce(&mut self, col: u16, row: u16) -> bool {
        let area = self.pane_areas.get(PaneId::Announce);
        if area.width < 4 || area.height < 2 {
            return false;
        }
        // Feed chrome: border + PAD_H + top/bottom pad 1 (matches ui Feed Padding).
        let inner_x = area.x.saturating_add(1 + 1); // border + pad
        let inner_y = area.y.saturating_add(1 + 1); // border + top pad
        let inner_w = area.width.saturating_sub(2 + 2);
        let inner_h = area.height.saturating_sub(2 + 1 + 1); // border + top + bottom pad
        if col < inner_x
            || col >= inner_x.saturating_add(inner_w)
            || row < inner_y
            || row >= inner_y.saturating_add(inner_h)
        {
            return false;
        }
        let content_row = row
            .saturating_sub(inner_y)
            .saturating_add(self.scroll_of(PaneId::Announce));
        let content_w = inner_w as usize;
        let (owner, is_btn) = self.announce_hit(content_row, content_w);
        if let Some(idx) = owner {
            self.selected_tweet = Some(idx);
        }
        if is_btn {
            if let Some(idx) = owner {
                return self.open_tweet_at(idx);
            }
        }
        false
    }

    /// Which tweet owns a content row, and whether that row is the view button.
    fn announce_hit(&self, content_row: u16, content_w: usize) -> (Option<usize>, bool) {
        if content_w == 0 {
            return (None, false);
        }
        let lines = self.lines_for(PaneId::Announce);
        let mut visual_at: usize = 0;
        let mut tweet_i: Option<usize> = None;
        let target = content_row as usize;
        for line in &lines {
            if line.is_empty() {
                if visual_at == target {
                    return (None, false);
                }
                visual_at += 1;
                continue;
            }
            if is_post_line(line) {
                tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
            }
            let h = Self::announce_line_height(line, content_w);
            if target >= visual_at && target < visual_at + h {
                return (tweet_i, line == VIEW_TWEET_BTN);
            }
            visual_at += h;
        }
        (None, false)
    }

    /// Open tweet by index in the browser. Returns true if a URL was launched.
    pub fn open_tweet_at(&mut self, idx: usize) -> bool {
        let Some(t) = self.snap.tweets.get(idx) else {
            return false;
        };
        let Some(url) = tweet_view_url(t) else {
            self.status = "no tweet url".into();
            return false;
        };
        self.selected_tweet = Some(idx);
        match open_url_in_browser(&url) {
            Ok(()) => {
                self.status = format!("opened tweet #{}", idx + 1);
                true
            }
            Err(e) => {
                self.status = format!("open failed: {e}");
                false
            }
        }
    }

    /// Open selected tweet, else first visible button (Enter / `o` on announce).
    pub fn open_focused_tweet(&mut self) -> bool {
        if self.focus.pane() != PaneId::Announce {
            return false;
        }
        if let Some(idx) = self.selected_tweet {
            if idx < self.snap.tweets.len() {
                return self.open_tweet_at(idx);
            }
        }
        let area = self.pane_areas.get(PaneId::Announce);
        let content_w = ui::pane_content_width(area).max(1);
        let scroll = self.scroll_of(PaneId::Announce) as usize;
        let lines = self.lines_for(PaneId::Announce);
        let mut visual_at: usize = 0;
        let mut tweet_i: Option<usize> = None;
        for line in &lines {
            if line.is_empty() {
                visual_at += 1;
                continue;
            }
            if is_post_line(line) {
                tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
            }
            let h = Self::announce_line_height(line, content_w);
            let line_end = visual_at + h;
            if line_end > scroll && line == VIEW_TWEET_BTN {
                if let Some(idx) = tweet_i {
                    return self.open_tweet_at(idx);
                }
            }
            visual_at = line_end;
        }
        if !self.snap.tweets.is_empty() {
            return self.open_tweet_at(0);
        }
        false
    }
}

/// Launch a URL in the system browser (macOS `open`, else `xdg-open`).
fn open_url_in_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Command::new("xdg-open").arg(url).spawn()?;
        Ok(())
    }
}

/// Record tweet ids we've already announced; return indices of genuinely new
/// tweets. Until the seen-set is seeded (first fetch) nothing counts as new,
/// so a fresh run doesn't replay the whole feed as notifications.
fn mark_new_tweets(seen: &mut HashSet<String>, tweets: &[Tweet], seeded: bool) -> Vec<usize> {
    let mut fresh = Vec::new();
    for (i, t) in tweets.iter().enumerate() {
        if t.id.is_empty() || t.id == "unknown" {
            continue;
        }
        if seen.insert(t.id.clone()) && seeded {
            fresh.push(i);
        }
    }
    fresh
}

/// Fire a desktop notification: macOS Notification Center via `osascript`,
/// Linux via `notify-send`. Same platform-branch style as `open_url_in_browser`.
fn notify_desktop(title: &str, body: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // AppleScript string literals: escape backslash first, then quotes.
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\" sound name \"Glass\"",
            esc(body),
            esc(title)
        );
        Command::new("osascript").arg("-e").arg(script).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("notify-send").arg(title).arg(body).spawn()?;
        return Ok(());
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Windows: PowerShell toast — TODO when the app gains Windows support.
        let _ = (title, body);
        return Ok(());
    }
}

/// Prefer stored tweet URL; fall back to `x.com/{handle}/status/{id}`.
/// Nitter hosts are rewritten to x.com for a usable browser link.
fn tweet_view_url(t: &Tweet) -> Option<String> {
    let handle = t
        .handle
        .as_deref()
        .filter(|h| !h.is_empty())
        .unwrap_or("i");

    let mut url = if !t.url.is_empty() {
        t.url.clone()
    } else if !t.id.is_empty() && t.id != "unknown" {
        format!("https://x.com/{handle}/status/{}", t.id)
    } else {
        return None;
    };

    for host in ["nitter.net", "nitter.privacydev.net", "nitter.poast.org"] {
        if url.contains(host) {
            url = url.replace(host, "x.com");
            break;
        }
    }
    // Strip nitter hash fragments like #m
    if let Some(idx) = url.find('#') {
        url.truncate(idx);
    }
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

pub async fn run_tui(cfg: Config) -> Result<()> {
    // A panic must never leave the user's terminal in raw mode / alternate
    // screen — restore it first, then let the normal hook print the panic.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut out = stdout();
        let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture);
        prev_hook(info);
    }));

    let client = http_client()?;
    let mut app = App::new(cfg, client);

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Kick off the first fetches in the background — the first frame draws
    // instantly ("loading…") instead of blocking on up to 8 API calls.
    app.trigger_data();
    app.trigger_feed();

    let tick = Duration::from_millis(100);
    let result = loop {
        // Apply any finished background refreshes, then lay out + paint.
        while let Ok(msg) = app.refresh_rx.try_recv() {
            app.handle_msg(msg);
        }
        app.ensure_line_cache();
        // Layout first so hit-tests + scroll clamps match what we paint.
        let size = terminal.size()?;
        app.pane_areas = ui::layout_panes(Rect::new(0, 0, size.width, size.height));
        app.clamp_all_scrolls();

        terminal.draw(|f| ui::draw(f, &app))?;

        let timeout = tick;
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Help overlay swallows the key that closes it.
                    if app.show_help {
                        app.show_help = false;
                    } else {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                app.should_quit = true;
                            }
                            // Raw mode disables ISIG, so Ctrl+C arrives here.
                            KeyCode::Char('c')
                                if key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                app.should_quit = true;
                            }
                            KeyCode::Char('r') => {
                                app.trigger_data();
                                app.trigger_feed();
                            }
                            KeyCode::Char('n') => {
                                app.set_scroll(PaneId::Announce, 0);
                                app.trigger_feed();
                            }
                            KeyCode::Char('t') => {
                                // Toggle desktop notifications (env sets initial state).
                                app.notify_enabled = !app.notify_enabled;
                                app.status = format!(
                                    "notifications {}",
                                    if app.notify_enabled { "on" } else { "off" }
                                );
                            }
                            KeyCode::Char('?') | KeyCode::Char('h') => {
                                app.show_help = true;
                            }
                            KeyCode::Tab => {
                                app.focus = app.focus.next();
                            }
                            KeyCode::BackTab => {
                                app.focus = app.focus.prev();
                            }
                            // Scroll focused pane (mouse wheel targets pane under cursor instead).
                            KeyCode::Down
                            | KeyCode::Char('j')
                            | KeyCode::Char('J') => app.scroll_focused(1),
                            KeyCode::Up
                            | KeyCode::Char('k')
                            | KeyCode::Char('K') => app.scroll_focused(-1),
                            KeyCode::PageDown | KeyCode::Char(' ') | KeyCode::Char('f') => {
                                app.scroll_focused(5)
                            }
                            KeyCode::PageUp | KeyCode::Char('b') => app.scroll_focused(-5),
                            KeyCode::Home | KeyCode::Char('g') => {
                                app.set_scroll(app.focus.pane(), 0);
                            }
                            KeyCode::End | KeyCode::Char('G') => {
                                let id = app.focus.pane();
                                let max = app.max_scroll_for(id);
                                app.set_scroll(id, max);
                            }
                            // Open tweet under cursor scroll position when announce is focused.
                            KeyCode::Enter | KeyCode::Char('o') | KeyCode::Char('O') => {
                                let _ = app.open_focused_tweet();
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                let n = c.to_digit(10).unwrap_or(0) as usize;
                                if (1..=NUM_PANES).contains(&n) {
                                    app.focus = Focus::from_index(n - 1);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Mouse(m) => {
                    if app.show_help {
                        app.show_help = false;
                    }
                    // Scroll / click / hover the pane under the cursor.
                    let under = app.pane_areas.hit(m.column, m.row);
                    match m.kind {
                        MouseEventKind::Moved => {
                            app.set_hover(m.column, m.row);
                        }
                        MouseEventKind::ScrollDown => {
                            app.set_hover(m.column, m.row);
                            if let Some(id) = under {
                                app.scroll_pane(id, 1);
                            } else {
                                app.scroll_focused(1);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            app.set_hover(m.column, m.row);
                            if let Some(id) = under {
                                app.scroll_pane(id, -1);
                            } else {
                                app.scroll_focused(-1);
                            }
                        }
                        MouseEventKind::Down(_) => {
                            app.set_hover(m.column, m.row);
                            let _ = app.focus_at(m.column, m.row);
                            // Open browser only when click lands on VIEW_TWEET_BTN row.
                            if under == Some(PaneId::Announce) {
                                let _ = app.click_announce(m.column, m.row);
                            }
                        }
                        MouseEventKind::Drag(_) => {
                            app.set_hover(m.column, m.row);
                        }
                        _ => {}
                    }
                }
                Event::Resize(_, _) => {
                    let size = terminal.size()?;
                    app.pane_areas =
                        ui::layout_panes(Rect::new(0, 0, size.width, size.height));
                    app.clamp_all_scrolls();
                }
                _ => {}
            }
        }

        if app.should_quit {
            break Ok(());
        }

        // Periodic refreshes — skip the trigger while one is still in flight
        // so slow endpoints can't stack requests.
        if !app.data_busy && app.last_data.elapsed() >= Duration::from_secs(REFRESH_DATA_SECS) {
            app.trigger_data();
        }
        if !app.feed_busy && app.last_feed.elapsed() >= Duration::from_secs(app.cfg.feed_secs) {
            app.trigger_feed();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tweet(id: &str) -> Tweet {
        Tweet {
            id: id.into(),
            text: "test".into(),
            created_at: None,
            url: String::new(),
            handle: Some("blknoiz06".into()),
            retweet: false,
            retweet_of: None,
            quote_author: None,
        }
    }

    #[test]
    fn first_fetch_seeds_without_notifying() {
        let mut seen = HashSet::new();
        let tweets = vec![tweet("1"), tweet("2")];
        let fresh = mark_new_tweets(&mut seen, &tweets, false);
        assert!(fresh.is_empty());
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn new_tweet_after_seed_is_reported() {
        let mut seen = HashSet::new();
        let tweets = vec![tweet("1")];
        mark_new_tweets(&mut seen, &tweets, false);
        let tweets = vec![tweet("2"), tweet("1")];
        let fresh = mark_new_tweets(&mut seen, &tweets, true);
        assert_eq!(fresh, vec![0]);
    }

    #[test]
    fn unknown_ids_never_notify() {
        let mut seen = HashSet::new();
        let tweets = vec![tweet(""), tweet("unknown"), tweet("real")];
        let fresh = mark_new_tweets(&mut seen, &tweets, true);
        assert_eq!(fresh, vec![2]);
    }

    #[test]
    fn refresh_invalidates_line_cache() {
        let mut app = App::new(Config::from_env(), http_client().unwrap());
        app.ensure_line_cache();
        assert!(app.line_cache.is_some());
        app.handle_msg(RefreshMsg::Data(Box::new(Snapshot::default())));
        assert!(app.line_cache.is_none());
    }
}




#[cfg(test)]
mod screenshot_tool {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use std::io::Write;

    /// Manual screenshot tool — renders the live board (real data, real feed)
    /// to a PPM image for the README. Run with:
    ///
    /// ```sh
    /// cargo test --release render_readme_screenshot -- --ignored
    /// sips -s format png /tmp/bullboard.ppm --out docs/bullboard.png
    /// ```
    #[test]
    #[ignore = "manual screenshot tool"]
    fn render_readme_screenshot() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut app = App::new(Config::from_env(), http_client().unwrap());
        rt.block_on(async {
            app.trigger_data();
            app.trigger_feed();
            let deadline = Instant::now() + Duration::from_secs(25);
            while (app.snap.fetched_at.is_none() || app.snap.tweets.is_empty())
                && Instant::now() < deadline
            {
                while let Ok(msg) = app.refresh_rx.try_recv() {
                    app.handle_msg(msg);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        app.ensure_line_cache();
        let mut terminal = Terminal::new(TestBackend::new(140, 46)).unwrap();
        terminal
            .draw(|f| crate::ui::draw(f, &app))
            .expect("draw board");
        let buf = terminal.backend().buffer();

        let mut out = std::fs::File::create("/tmp/bullboard.ppm").unwrap();
        writeln!(out, "P3\n{} {}\n255", buf.area.width, buf.area.height).unwrap();
        for cell in &buf.content {
            let (r, g, b) = match (cell.fg, cell.bg) {
                (Color::Rgb(r, g, b), _) => (r, g, b),
                (_, Color::Rgb(r, g, b)) => (r, g, b),
                _ => (5, 6, 4), // canvas
            };
            writeln!(out, "{r} {g} {b}").unwrap();
        }
        eprintln!(
            "wrote /tmp/bullboard.ppm ({}x{}) — price {:?}",
            buf.area.width,
            buf.area.height,
            app.snap.price_usd()
        );
    }
}
