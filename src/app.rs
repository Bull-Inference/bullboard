use crate::config::{Config, FeedMode, REFRESH_DATA_SECS};
use crate::fetch::{fetch_announce, fetch_snapshot, http_client, FeedHealth};
use crate::format::{
    ago, bar, clock_mmdd_hhmm, delta_str, fmt_compact, fmt_int, fmt_usd, short_addr, sparkline,
};
use crate::model::{Snapshot, Tweet};
use crate::ui::{self, Focus, PaneAreas, PaneId, NUM_PANES};
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
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

use crate::config::{ACID, MUTED};

/// On-screen control under each announce post (click / Enter / `o` opens URL).
/// Leading spaces keep the acid chip left-aligned under the post body.
pub const VIEW_TWEET_BTN: &str = "  [ view tweet ]  ";

pub struct App {
    pub cfg: Config,
    pub client: Client,
    pub snap: Snapshot,
    pub feed_mode: FeedMode,
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
    pub loading: bool,
    pub should_quit: bool,
    pub status: String,
    /// Desktop notifications on (`t` toggles; BULLBOARD_NOTIFY sets initial).
    pub notify_enabled: bool,
    /// Circuit-breaker state per feed mirror (auto-skips blocking mirrors).
    pub feed_health: FeedHealth,
    last_data: Instant,
    last_feed: Instant,
    /// Tweet ids already announced; a new id fires a desktop notification.
    seen_tweets: HashSet<String>,
}

impl App {
    pub fn new(cfg: Config, client: Client) -> Self {
        let notify = cfg.notify;
        Self {
            cfg,
            client,
            snap: Snapshot::default(),
            feed_mode: FeedMode::Primary,
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
            last_data: Instant::now() - Duration::from_secs(999),
            last_feed: Instant::now() - Duration::from_secs(999),
            seen_tweets: HashSet::new(),
        }
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
        if !matches!(id, PaneId::Announce | PaneId::Activity) {
            return lines.len();
        }
        let content_w = ui::pane_content_width(area);
        if content_w == 0 {
            return lines.len();
        }
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
            fetch_announce(&self.client, &self.cfg, self.feed_mode, 40, &mut self.feed_health)
                .await;
        // Nitter mirrors are flaky: if a poll comes back empty with an error,
        // keep the last good feed so the pane never blanks on a hiccup.
        if tweets.is_empty() && err.is_some() && !self.snap.tweets.is_empty() {
            self.snap.tweet_error = err;
        } else {
            self.notify_new_tweets(&tweets);
            self.snap.tweets = tweets;
            self.snap.tweet_error = err;
        }
        self.last_feed = Instant::now();
        // reset announce scroll on feed cycle only if empty scroll keep
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

    /// Surfboard-style KPI: 3 lines (hero · detail · whisper) so content_h≥4 yields float.
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
            fmt_usd(self.snap.price_usd()),
            format!("24h {}", delta_str(self.snap.change_24h())),
            format!("vol {}", fmt_usd(vol24)),
            // Second-opinion price, always visible (not just on divergence).
            format!("gecko {}", fmt_usd(self.snap.gecko_token.price_usd)),
        ]
    }

    /// Total LP across every live pool (DexScreener sum), cross-checked with
    /// Rugcheck's market-wide total. Primary pool shown as detail/whisper.
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
        let whisper = match t.total_market_liq {
            Some(r) if r > 0.0 => format!("{pools_s} · rug {}", fmt_usd(Some(r))),
            _ => format!("{pools_s} · primary {}", fmt_usd(p.liq_usd)),
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
            format!(
                "{} · {}",
                p.dex_id.to_uppercase(),
                short_addr(&p.pair_address, 5)
            ),
            whisper,
        ];
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
        vec![
            hero,
            format!(
                "mint {} · freeze {}",
                if mint_ok { "off" } else { "ON" },
                if freeze_ok { "off" } else { "ON" }
            ),
            t.rug_score
                .map(|s| format!("rug {s:.0}"))
                .unwrap_or_else(|| "rug —".into()),
            format!("lp {lp} locked"),
        ]
    }

    fn lines_supply(&self) -> Vec<String> {
        let t = &self.snap.token;
        let holders = fmt_int(t.holder_count);
        let mcap = fmt_usd(
            t.mcap
                .or_else(|| self.snap.ohlc_stat_f("market_cap"))
                .or_else(|| self.snap.gecko_token.market_cap),
        );
        let fdv = fmt_usd(
            t.fdv
                .or_else(|| self.snap.gecko_token.fdv)
                .or_else(|| t.primary_pair.as_ref().and_then(|p| p.fdv)),
        );
        vec![
            holders,
            format!("mcap {mcap}"),
            format!("circ {}", fmt_compact(t.circ_supply)),
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
        // One row per window (not two).
        for (label, w) in [
            ("5m", &t.stats_5m),
            ("1h", &t.stats_1h),
            ("6h", &t.stats_6h),
            ("24h", &t.stats_24h),
        ] {
            lines.push(format!(
                "{label:<3} B {}  S {}  {}B/{}S  tr {}",
                fmt_usd(w.buy_volume),
                fmt_usd(w.sell_volume),
                fmt_int(w.buys),
                fmt_int(w.sells),
                fmt_int(w.traders)
            ));
        }
        lines.push(String::new());
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
        // Second-opinion liquidity from Gecko's per-pool reserves; flag it
        // with (!) when it disagrees with DexScreener by more than 25% —
        // Gecko's per-pool reserves are usually the real number.
        if let Some(g_liq) = s.gecko_liq_usd() {
            let dex_liq: f64 = t.pairs.iter().filter_map(|x| x.liq_usd).sum();
            let flag = if dex_liq > 0.0 && (g_liq - dex_liq).abs() / dex_liq > 0.25 {
                " (!)"
            } else {
                ""
            };
            lines.push(format!(
                "gecko  {} pools · liq {} vol {}{flag}",
                s.gecko_pools.len(),
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
        let vol24 = t
            .stats_24h
            .buy_volume
            .zip(t.stats_24h.sell_volume)
            .map(|(b, se)| b + se)
            .or_else(|| p.and_then(|x| x.vol_h24))
            .or_else(|| s.ohlc_stat_f("volume_24h"))
            .or_else(|| s.gecko_token.vol_24h);
        let liq = t
            .total_liquidity()
            .or_else(|| s.ohlc_stat_f("liquidity_usd"))
            .or_else(|| s.gecko_liq_usd());
        // Second-opinion price: surface it when sources diverge > 1%.
        let mut head = format!(
            "ANSEM {}   24h {}",
            fmt_usd(s.price_usd()),
            delta_str(s.change_24h())
        );
        if let (Some(px), Some(gk)) = (s.price_usd(), s.gecko_token.price_usd) {
            if px != 0.0 && ((gk - px).abs() / px.abs()) > 0.01 {
                head.push_str(&format!(" · gecko {}", fmt_usd(Some(gk))));
            }
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
        // count + deltas, top10 bar, blank, top 3 holders only
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

    pub fn header_text(&self) -> String {
        let price = fmt_usd(self.snap.price_usd());
        let ch = delta_str(self.snap.change_24h());
        format!(" BULLBOARD · ANSEM {price} · {ch} ")
    }

    /// Surfboard ticker: muted chrome, acid only on price + change.
    pub fn header_line(&self) -> Line<'static> {
        let price = fmt_usd(self.snap.price_usd());
        let ch = delta_str(self.snap.change_24h());
        Line::from(vec![
            Span::styled(
                " BULLBOARD · ANSEM ".to_string(),
                Style::default().fg(MUTED),
            ),
            Span::styled(
                price,
                Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ".to_string(), Style::default().fg(MUTED)),
            Span::styled(
                ch,
                Style::default().fg(ACID).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ".to_string(), Style::default().fg(MUTED)),
        ])
    }

    pub fn footer_text(&self) -> String {
        let updated = ago(self.snap.fetched_at.as_deref());
        let pane = self.focus.pane().title();
        let alerts = if self.notify_enabled { "on" } else { "off" };
        format!(
            " q quit · r refresh · n feed · t alerts · o open · tab next · j/k scroll · {} · {} · alerts:{alerts} · v{} ",
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
        // Degraded mirror health shows up in the title instead of hiding.
        let live = self.feed_health.live_mirrors(&self.cfg.mirrors);
        let total = self.cfg.mirrors.len();
        let mirrors = if live < total {
            format!(" · mirrors {live}/{total}")
        } else {
            String::new()
        };
        format!(
            "ANNOUNCE FEED · @{} · updated {}s · last {last}{mirrors}",
            self.feed_mode.label(&self.cfg),
            self.last_feed.elapsed().as_secs(),
        )
    }

    /// Map a content-row (already scroll-adjusted) inside the announce pane.
    /// Returns `Some(tweet_idx)` only when the visual line is `VIEW_TWEET_BTN`.
    /// Body rows and empty spacers return `None` (focus-only, no open).
    fn tweet_index_at_content_row(&self, content_row: u16, content_w: usize) -> Option<usize> {
        if content_w == 0 || self.snap.tweets.is_empty() {
            return None;
        }
        let lines = self.lines_for(PaneId::Announce);
        let mut visual_at: usize = 0;
        let mut tweet_i: Option<usize> = None;
        let target = content_row as usize;

        for line in &lines {
            if line.is_empty() {
                // spacer — dead zone, never open
                if visual_at == target {
                    return None;
                }
                visual_at += 1;
                continue;
            }
            if line == VIEW_TWEET_BTN {
                // button belongs to the current tweet — only open path
                let h = 1;
                if target >= visual_at && target < visual_at + h {
                    return tweet_i;
                }
                visual_at += h;
                continue;
            }
            // post body — track which tweet owns following button; body click = no open
            if line.contains(" POST ") {
                tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
            }
            let chars = line.chars().count().max(1);
            let h = ((chars + content_w - 1) / content_w).max(1);
            if target >= visual_at && target < visual_at + h {
                return None;
            }
            visual_at += h;
        }
        None
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
            if line.contains(" POST ") {
                tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
            }
            let h = if line == VIEW_TWEET_BTN {
                1
            } else {
                let chars = line.chars().count().max(1);
                ((chars + content_w - 1) / content_w).max(1)
            };
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
            if line.contains(" POST ") {
                tweet_i = Some(tweet_i.map(|i| i + 1).unwrap_or(0));
            }
            let h = if line == VIEW_TWEET_BTN {
                1
            } else {
                let chars = line.chars().count().max(1);
                ((chars + content_w - 1) / content_w).max(1)
            };
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
        return Ok(());
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
        return Ok(());
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

    let raw = if !t.url.is_empty() {
        t.url.clone()
    } else if !t.id.is_empty() && t.id != "unknown" {
        format!("https://x.com/{handle}/status/{}", t.id)
    } else {
        return None;
    };

    // Rewrite nitter mirrors → x.com so the link opens on the real site.
    let mut url = raw;
    for host in [
        "nitter.net",
        "nitter.privacydev.net",
        "nitter.poast.org",
        "nitter.1d4.us",
        "nitter.cz",
    ] {
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
        // Layout first so hit-tests + scroll clamps match what we paint.
        let size = terminal.size()?;
        app.pane_areas = ui::layout_panes(Rect::new(0, 0, size.width, size.height));
        app.clamp_all_scrolls();

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
                        // refresh @blknoiz06 announce only (no inference alt handle)
                        app.set_scroll(PaneId::Announce, 0);
                        app.refresh_feed().await;
                    }
                    KeyCode::Char('t') => {
                        // Toggle desktop notifications (env sets initial state).
                        app.notify_enabled = !app.notify_enabled;
                        app.status = format!(
                            "notifications {}",
                            if app.notify_enabled { "on" } else { "off" }
                        );
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
                },
                Event::Mouse(m) => {
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

        if app.last_data.elapsed() >= Duration::from_secs(REFRESH_DATA_SECS) {
            app.refresh_data().await;
            app.clamp_all_scrolls();
        }
        if app.last_feed.elapsed() >= Duration::from_secs(app.cfg.feed_secs) {
            app.refresh_feed().await;
            app.clamp_all_scrolls();
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
}



