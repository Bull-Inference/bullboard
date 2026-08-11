use crate::config::Config;
use crate::model::{
    DexPair, GeckoToken, RiskFlag, Snapshot, Token, TopHolder, Tweet, WindowStats,
};
use anyhow::Result;
use chrono::Utc;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

pub fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(format!("bullboard/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Retry once after a short pause on server errors, network hiccups, and
/// rate-limits on non-Gecko hosts. 4xx (other than 429 on non-Gecko) are real
/// responses and are not retried. Gecko 429 is not retried — the IP budget is
/// already spent and a second hit within 800ms doubles free-tier pressure.
const RETRY_DELAY: Duration = Duration::from_millis(800);

async fn get_json(client: &Client, url: &str) -> Result<Value, String> {
    match fetch_json(client, url).await {
        Ok(v) => Ok(v),
        Err(e) if retryable(&e, url) => {
            tokio::time::sleep(RETRY_DELAY).await;
            fetch_json(client, url).await
        }
        Err(e) => Err(e),
    }
}

async fn get_text(client: &Client, url: &str) -> Result<String, String> {
    match fetch_text(client, url).await {
        Ok(v) => Ok(v),
        Err(e) if retryable(&e, url) => {
            tokio::time::sleep(RETRY_DELAY).await;
            fetch_text(client, url).await
        }
        Err(e) => Err(e),
    }
}

fn retryable(err: &str, url: &str) -> bool {
    if let Some(status) = err.strip_prefix("HTTP ") {
        match status.parse::<u16>() {
            Ok(429) => !url.contains("api.geckoterminal.com"),
            Ok(code) => code >= 500,
            Err(_) => false,
        }
    } else {
        // Reqwest transport failures (timeouts, connection resets, DNS) are
        // worth one retry; parse errors ("json: …") are not.
        err.starts_with("reqwest: ")
    }
}

async fn fetch_json(client: &Client, url: &str) -> Result<Value, String> {
    match client.get(url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<Value>()
                .await
                .map_err(|e| format!("json: {e}"))
        }
        Err(e) => Err(format!("reqwest: {e}")),
    }
}

async fn fetch_text(client: &Client, url: &str) -> Result<String, String> {
    match client.get(url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.text().await.map_err(|e| format!("text: {e}"))
        }
        Err(e) => Err(format!("reqwest: {e}")),
    }
}

fn json_f(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_i64().map(|i| i as f64))
            .or_else(|| x.as_u64().map(|u| u as f64))
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

fn json_u(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| {
        x.as_u64()
            .or_else(|| x.as_i64().map(|i| i as u64))
            .or_else(|| x.as_f64().map(|f| f as u64))
    })
}

fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn take_val(
    map: &mut HashMap<String, Result<Value, String>>,
    key: &str,
    errors: &mut HashMap<String, String>,
) -> Value {
    match map.remove(key) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            errors.insert(key.into(), e);
            Value::Null
        }
        None => Value::Null,
    }
}

fn parse_window(v: Option<&Value>) -> WindowStats {
    let Some(s) = v else {
        return WindowStats::default();
    };
    WindowStats {
        price_change: json_f(s, "priceChange"),
        holder_change: json_f(s, "holderChange"),
        liquidity_change: json_f(s, "liquidityChange"),
        volume_change: json_f(s, "volumeChange"),
        buy_volume: json_f(s, "buyVolume"),
        sell_volume: json_f(s, "sellVolume"),
        buys: json_u(s, "numBuys"),
        sells: json_u(s, "numSells"),
        traders: json_u(s, "numTraders"),
        organic_buyers: json_u(s, "numOrganicBuyers"),
        net_buyers: json_u(s, "numNetBuyers"),
    }
}

fn pick_jup(mint: &str, raw: Result<Value, String>, errors: &mut HashMap<String, String>) -> Option<Value> {
    match raw {
        Ok(Value::Array(arr)) => {
            let mut first = None;
            for row in arr {
                if first.is_none() {
                    first = Some(row.clone());
                }
                if row.get("id").and_then(|i| i.as_str()) == Some(mint) {
                    return Some(row);
                }
            }
            first
        }
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => None,
        Err(e) => {
            errors.insert("jupiter".into(), e);
            None
        }
    }
}

fn parse_dex_pairs(raw: Result<Value, String>, errors: &mut HashMap<String, String>) -> Vec<DexPair> {
    let v = match raw {
        Ok(v) => v,
        Err(e) => {
            errors.insert("dexscreener".into(), e);
            return Vec::new();
        }
    };
    let mut pairs = Vec::new();
    let arr = v
        .get("pairs")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    for p in arr {
        let liq = p.get("liquidity").cloned().unwrap_or(Value::Null);
        let vol = p.get("volume").cloned().unwrap_or(Value::Null);
        let ch = p.get("priceChange").cloned().unwrap_or(Value::Null);
        let quote = p
            .pointer("/quoteToken/symbol")
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .to_string();
        pairs.push(DexPair {
            dex_id: json_str(&p, "dexId").unwrap_or_else(|| "?".into()),
            price_usd: json_f(&p, "priceUsd").or_else(|| {
                p.get("priceUsd")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse().ok())
            }),
            change_m5: json_f(&ch, "m5"),
            change_h1: json_f(&ch, "h1"),
            change_h6: json_f(&ch, "h6"),
            change_h24: json_f(&ch, "h24"),
            vol_h24: json_f(&vol, "h24"),
            liq_usd: json_f(&liq, "usd"),
            quote_symbol: quote,
            fdv: json_f(&p, "fdv"),
            mcap: json_f(&p, "marketCap"),
        });
    }
    pairs.sort_by(|a, b| {
        b.liq_usd
            .partial_cmp(&a.liq_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}

fn parse_token(
    mint: &str,
    jup_raw: Result<Value, String>,
    gecko_raw: Result<Value, String>,
    rug_raw: Result<Value, String>,
    dex_raw: Result<Value, String>,
    errors: &mut HashMap<String, String>,
) -> Token {
    let mut t = Token {
        name: "The Black Bull".into(),
        symbol: "ANSEM".into(),
        ..Default::default()
    };

    if let Some(j) = pick_jup(mint, jup_raw, errors) {
        t.name = json_str(&j, "name").unwrap_or(t.name);
        t.symbol = json_str(&j, "symbol").unwrap_or(t.symbol);
        t.price_usd = json_f(&j, "usdPrice");
        t.mcap = json_f(&j, "mcap");
        t.fdv = json_f(&j, "fdv");
        t.liquidity = json_f(&j, "liquidity");
        t.circ_supply = json_f(&j, "circSupply");
        t.total_supply = json_f(&j, "totalSupply");
        t.holder_count = json_u(&j, "holderCount");
        t.decimals = json_u(&j, "decimals").map(|d| d as u32);
        t.launchpad = json_str(&j, "launchpad");
        t.graduated_at = json_str(&j, "graduatedAt");
        t.graduated_pool = json_str(&j, "graduatedPool");
        t.dev = json_str(&j, "dev");
        t.twitter = json_str(&j, "twitter");
        t.website = json_str(&j, "website");
        t.organic_score = json_f(&j, "organicScore");
        t.organic_label = json_str(&j, "organicScoreLabel");
        t.is_verified = j.get("isVerified").and_then(|v| v.as_bool());
        if let Some(tags) = j.get("tags").and_then(|x| x.as_array()) {
            t.tags = tags
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(audit) = j.get("audit") {
            t.mint_auth_disabled = audit.get("mintAuthorityDisabled").and_then(|v| v.as_bool());
            t.freeze_auth_disabled = audit
                .get("freezeAuthorityDisabled")
                .and_then(|v| v.as_bool());
            t.top_holders_pct = json_f(audit, "topHoldersPercentage");
            t.dev_balance_pct = json_f(audit, "devBalancePercentage");
        }
        t.stats_5m = parse_window(j.get("stats5m"));
        t.stats_1h = parse_window(j.get("stats1h"));
        t.stats_6h = parse_window(j.get("stats6h"));
        t.stats_24h = parse_window(j.get("stats24h"));
    }

    match gecko_raw {
        Ok(g) => {
            if let Some(attrs) = g.pointer("/data/attributes") {
                if t.circ_supply.is_none() {
                    t.circ_supply = json_f(attrs, "normalized_total_supply");
                }
                if let Some(holders) = attrs.get("holders") {
                    if t.holder_count.is_none() {
                        t.holder_count = json_u(holders, "count");
                    }
                    if let Some(dist) = holders.get("distribution_percentage") {
                        t.top10_pct = json_f(dist, "top_10");
                        t.top11_20_pct = json_f(dist, "11_20");
                        t.top21_40_pct = json_f(dist, "21_40");
                        t.rest_pct = json_f(dist, "rest");
                    }
                }
                if t.mint_auth_disabled.is_none() {
                    let ma = attrs.get("mint_authority").and_then(|v| v.as_str());
                    t.mint_auth_disabled = ma.map(|s| s == "no" || s == "disabled");
                }
                if t.freeze_auth_disabled.is_none() {
                    let fa = attrs.get("freeze_authority").and_then(|v| v.as_str());
                    t.freeze_auth_disabled = fa.map(|s| s == "no" || s == "disabled");
                }
                if t.dev.is_none() {
                    t.dev = json_str(attrs, "developer_address");
                }
            }
        }
        Err(e) => {
            errors.insert("gecko".into(), e);
        }
    }

    match rug_raw {
        Ok(r) => {
            if t.holder_count.is_none() {
                t.holder_count = json_u(&r, "totalHolders");
            }
            t.rug_score = json_f(&r, "score_normalised");
            t.lp_locked_pct = json_f(&r, "lpLockedPct");
            t.total_market_liq = json_f(&r, "totalMarketLiquidity");
            t.graph_insiders = json_u(&r, "graphInsidersDetected");
            t.rugged = r.get("rugged").and_then(|v| v.as_bool());
            t.creator = json_str(&r, "creator");
            if let Some(m) = r.get("markets").and_then(|a| a.as_array()) {
                t.markets_n = Some(m.len() as u64);
            }
            if let Some(risks) = r.get("risks").and_then(|a| a.as_array()) {
                for risk in risks.iter().take(6) {
                    t.risks.push(RiskFlag {
                        name: json_str(risk, "name").unwrap_or_default(),
                        level: json_str(risk, "level").unwrap_or_default(),
                        value: json_str(risk, "value").unwrap_or_default(),
                    });
                }
            }
            if let Some(arr) = r.get("topHolders").and_then(|a| a.as_array()) {
                for th in arr.iter().take(15) {
                    let owner = th
                        .get("owner")
                        .or_else(|| th.get("address"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    t.top_holders.push(TopHolder {
                        owner,
                        pct: json_f(th, "pct"),
                        ui_amount: json_f(th, "uiAmount"),
                        insider: th.get("insider").and_then(|v| v.as_bool()).unwrap_or(false),
                    });
                }
            }
        }
        Err(e) => {
            errors.insert("rugcheck".into(), e);
        }
    }

    t.pairs = parse_dex_pairs(dex_raw, errors);
    t.primary_pair = t.pairs.first().cloned();

    // fill price/mcap from primary pair if missing
    if let Some(ref p) = t.primary_pair {
        if t.price_usd.is_none() {
            t.price_usd = p.price_usd;
        }
        if t.mcap.is_none() {
            t.mcap = p.mcap;
        }
        if t.fdv.is_none() {
            t.fdv = p.fdv;
        }
        if t.liquidity.is_none() {
            t.liquidity = p.liq_usd;
        }
    }

    // prefer gecko top10 if jupiter audit missing distribution
    if t.top10_pct.is_none() {
        t.top10_pct = t.top_holders_pct;
    }

    t
}

fn parse_gecko_token(
    raw: Result<Value, String>,
    errors: &mut HashMap<String, String>,
) -> GeckoToken {
    let v = match raw {
        Ok(v) => v,
        Err(e) => {
            errors.insert("gecko-token".into(), e);
            return GeckoToken::default();
        }
    };
    let mut g = GeckoToken::default();
    if let Some(attrs) = v.pointer("/data/attributes") {
        g.price_usd = json_f(attrs, "price_usd");
        if let Some(pct) = attrs.get("price_change_percentage") {
            g.change_24h = json_f(pct, "h24");
        }
        g.market_cap = json_f(attrs, "market_cap_usd");
        g.fdv = json_f(attrs, "fdv_usd");
        // volume_usd is nested: { "h24": "…" }
        g.vol_24h = attrs
            .get("volume_usd")
            .and_then(|v| json_f(v, "h24"));
        g.liquidity = json_f(attrs, "total_reserve_in_usd");
    }
    g
}

/// Gecko endpoints polled each snapshot. Caps free-tier burn: `/pools` was
/// dropped — `gecko_token.liquidity` (`total_reserve_in_usd`) already feeds
/// `Snapshot::gecko_liq_usd`, and a third call was 429'ing the shared IP budget.
fn gecko_fetch_specs(mint: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "gecko",
            format!("https://api.geckoterminal.com/api/v2/networks/solana/tokens/{mint}/info"),
        ),
        (
            "gecko-token",
            format!("https://api.geckoterminal.com/api/v2/networks/solana/tokens/{mint}"),
        ),
    ]
}

pub async fn fetch_snapshot(client: &Client, cfg: &Config) -> Snapshot {
    let base = cfg.api_base.trim_end_matches('/');
    let mint = cfg.mint.clone();

    let spawn = |name: &'static str, url: String| {
        let c = client.clone();
        tokio::spawn(async move { (name.to_string(), get_json(&c, &url).await) })
    };

    // $ANSEM token only — no inference / desk / markets feed.
    // Gecko: 2 endpoints × 4 polls/min = 8/min (was 12); free tier ~30/min/IP.
    let mut tasks = vec![
        spawn("price", format!("{base}/api/token-price")),
        spawn(
            "ohlc",
            format!("{base}/api/token-ohlc?interval=1h&limit=48"),
        ),
        spawn(
            "jupiter",
            format!("https://lite-api.jup.ag/tokens/v2/search?query={mint}"),
        ),
        spawn(
            "rugcheck",
            format!("https://api.rugcheck.xyz/v1/tokens/{mint}/report"),
        ),
        spawn(
            "dex",
            format!("https://api.dexscreener.com/latest/dex/tokens/{mint}"),
        ),
    ];
    for (name, url) in gecko_fetch_specs(&mint) {
        tasks.push(spawn(name, url));
    }

    let mut map = HashMap::new();
    for t in tasks {
        if let Ok((name, res)) = t.await {
            map.insert(name, res);
        }
    }

    let mut errors = HashMap::new();
    let mut snap = Snapshot {
        price: take_val(&mut map, "price", &mut errors),
        ohlc: take_val(&mut map, "ohlc", &mut errors),
        ..Default::default()
    };

    let jup = map.remove("jupiter").unwrap_or_else(|| Err("fetch task crashed".into()));
    let gecko = map.remove("gecko").unwrap_or_else(|| Err("fetch task crashed".into()));
    let gecko_token = map
        .remove("gecko-token")
        .unwrap_or_else(|| Err("fetch task crashed".into()));
    let rug = map.remove("rugcheck").unwrap_or_else(|| Err("fetch task crashed".into()));
    let dex = map.remove("dex").unwrap_or_else(|| Err("fetch task crashed".into()));
    snap.token = parse_token(&mint, jup, gecko, rug, dex, &mut errors);
    snap.gecko_token = parse_gecko_token(gecko_token, &mut errors);
    snap.errors = errors;
    snap.fetched_at = Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    snap
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn clean_tweet_text(raw: &str) -> String {
    let mut t = strip_tags(raw);
    if let Some(idx) = t.find("https://nitter.") {
        t.truncate(idx);
    }
    // Nitter links to the post itself carry a `#m` fragment (the anchor text
    // after truncation above). Strip only that trailing artifact — never a
    // legitimate "#m" inside the post body.
    if t.ends_with("#m") {
        t.truncate(t.len() - 2);
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `https://nitter.net/{author}/status/{id}` → the author segment.
fn author_from_link(link: &str) -> Option<String> {
    let st = link.find("/status/")?;
    let proto = link.find("://").map(|i| i + 3).unwrap_or(0);
    let slash = link[proto..].find('/').map(|i| i + proto).unwrap_or(0);
    if st > slash {
        Some(link[slash + 1..st].to_string())
    } else {
        None
    }
}

/// Quote retweets embed `Name (@handle)` inside a <blockquote> — grab the handle.
fn quoted_author(desc: &str) -> Option<String> {
    let bq = desc.find("<blockquote>")?;
    let at = desc[bq..].find("(@")? + bq;
    let rest = &desc[at + 2..];
    let end = rest.find(')')?;
    let h = &rest[..end];
    if h.is_empty() {
        None
    } else {
        Some(h.to_string())
    }
}

fn parse_rss(xml: &str, handle: &str) -> Vec<Tweet> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut tweets = Vec::new();
    let mut buf = Vec::new();

    let mut in_item = false;
    let mut title = String::new();
    let mut desc = String::new();
    let mut link = String::new();
    let mut pub_date = String::new();
    let mut cur_tag = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" {
                    in_item = true;
                    title.clear();
                    desc.clear();
                    link.clear();
                    pub_date.clear();
                }
                cur_tag = name;
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "item" && in_item {
                    in_item = false;
                    let text = clean_tweet_text(if !desc.is_empty() { &desc } else { &title });
                    if text.is_empty() {
                        continue;
                    }
                    let retweet = title.starts_with("RT by ");
                    let retweet_of = if retweet {
                        author_from_link(&link)
                    } else {
                        None
                    };
                    let quote_author = quoted_author(&desc);
                    let id = link
                        .rsplit("/status/")
                        .next()
                        .unwrap_or("unknown")
                        .trim_end_matches("#m")
                        .to_string();
                    let mut url = link.replace("nitter.net", "x.com");
                    if url.is_empty() {
                        url = format!("https://x.com/{handle}/status/{id}");
                    }
                    let created = if pub_date.is_empty() {
                        None
                    } else {
                        chrono::DateTime::parse_from_rfc2822(&pub_date)
                            .ok()
                            .map(|d| {
                                d.with_timezone(&Utc)
                                    .format("%Y-%m-%dT%H:%M:%SZ")
                                    .to_string()
                            })
                    };
                    tweets.push(Tweet {
                        id,
                        text,
                        created_at: created,
                        url,
                        handle: Some(handle.to_string()),
                        retweet,
                        retweet_of,
                        quote_author,
                    });
                }
                cur_tag.clear();
            }
            Ok(Event::Text(t)) => {
                if !in_item {
                    continue;
                }
                let text = t.unescape().unwrap_or_default().to_string();
                match cur_tag.as_str() {
                    "title" => title.push_str(&text),
                    "description" => desc.push_str(&text),
                    "link" => link.push_str(&text),
                    "pubDate" => pub_date.push_str(&text),
                    _ => {}
                }
            }
            // Nitter wraps description in <![CDATA[...]]> — treat it as text.
            Ok(Event::CData(t)) => {
                if !in_item {
                    continue;
                }
                let text = String::from_utf8_lossy(t.as_ref()).to_string();
                if cur_tag.as_str() == "description" { desc.push_str(&text) }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    tweets
}

/// Nitter keys its RSS cache on the `?cursor=` param, and every public mirror
/// runs with `rssMinutes = 10` — without a cursor, every poll gets the same
/// feed served from Redis, so tweets land ~10 minutes late no matter how fast
/// we poll. A unique cursor value misses the cache and forces a live fetch
/// straight from Twitter on every poll.
fn fresh_cursor() -> String {
    format!("bb{:x}{:x}", Utc::now().timestamp_millis(), std::process::id())
}

fn rss_url(base: &str, handle: &str, cursor: Option<&str>) -> String {
    match cursor {
        Some(c) => format!("{base}/{handle}/rss?cursor={c}"),
        None => format!("{base}/{handle}/rss"),
    }
}

/// Nitter serves block / rate-limit pages as HTTP 200 HTML — treat anything
/// that isn't RSS-shaped as a mirror failure.
fn is_rss(xml: &str) -> bool {
    let t = xml.trim_start();
    t.starts_with("<?xml") || t.starts_with("<rss")
}

fn parse_feed(xml: &str, handle: &str, limit: usize) -> Vec<Tweet> {
    let mut tweets = parse_rss(xml, handle);
    // Newest first — created_at is ISO-8601 UTC, so string cmp is chronological.
    // Sort before truncating so the newest `limit` survive regardless of feed order.
    tweets.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    tweets.truncate(limit);
    tweets
}

/// Newest tweet timestamp in a feed (ISO-8601 UTC → lexicographic == chronological).
fn newest_created(ts: &[Tweet]) -> Option<&str> {
    ts.iter().filter_map(|t| t.created_at.as_deref()).max()
}

/// Circuit-breaker cooldowns for the announce feed.
const TRIP_THRESHOLD: u32 = 3; // polls where every attempt failed before tripping
const TRIP_COOLDOWN_SECS: u64 = 300; // 5 min skip after tripping
const FRESH_THRESHOLD: u32 = 3; // fresh-only failures before downgrading
const FRESH_COOLDOWN_SECS: u64 = 1800; // 30 min plain-only after fresh blocks

/// How a mirror should be probed this poll, decided by its circuit breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorPlan {
    /// Try the fresh live-fetch URL, then the plain cached URL.
    Full,
    /// Fresh fetches kept failing — use only the plain cached URL.
    PlainOnly,
    /// Mirror tripped — skip it for the cooldown period.
    Skip,
}

/// Auto-healing for flaky mirrors: a mirror that fails every attempt for
/// `TRIP_THRESHOLD` polls in a row is skipped for `TRIP_COOLDOWN_SECS`
/// instead of being hammered; a mirror that blocks the fresh-cursor bypass
/// (but still serves cached RSS) is downgraded to plain requests for
/// `FRESH_COOLDOWN_SECS`, then probed fresh again.
#[derive(Clone, Debug, Default)]
pub struct MirrorHealth {
    trip_streak: u32,
    fresh_streak: u32,
    trip_until: Option<u64>,
    fresh_until: Option<u64>,
}

impl MirrorHealth {
    fn plan(&self, now: u64) -> MirrorPlan {
        if self.trip_until.is_some_and(|t| now < t) {
            MirrorPlan::Skip
        } else if self.fresh_until.is_some_and(|t| now < t) {
            MirrorPlan::PlainOnly
        } else {
            MirrorPlan::Full
        }
    }

    fn observe(&mut self, fresh_tried: bool, ok: bool, fresh_ok: bool, now: u64) {
        if !ok {
            // Every allowed attempt failed this poll.
            self.trip_streak += 1;
            self.fresh_streak = 0;
            if self.trip_streak >= TRIP_THRESHOLD {
                self.trip_until = Some(now + TRIP_COOLDOWN_SECS);
            }
            return;
        }
        self.trip_streak = 0;
        if !fresh_tried {
            return; // plain-only poll — nothing new to learn about fresh
        }
        if fresh_ok {
            self.fresh_streak = 0;
            self.fresh_until = None;
        } else {
            self.fresh_streak += 1;
            if self.fresh_streak >= FRESH_THRESHOLD {
                self.fresh_until = Some(now + FRESH_COOLDOWN_SECS);
            }
        }
    }
}

/// Breaker state for every mirror in the feed, owned by the app so tripped
/// mirrors stay skipped across polls (and recover once the cooldown passes).
#[derive(Clone, Debug, Default)]
pub struct FeedHealth {
    mirrors: HashMap<String, MirrorHealth>,
}

impl FeedHealth {
    pub fn plan_mirror(&self, mirror: &str) -> MirrorPlan {
        let now = Utc::now().timestamp() as u64;
        self.mirrors
            .get(mirror)
            .map_or(MirrorPlan::Full, |h| h.plan(now))
    }

    pub fn observe_result(&mut self, mirror: &str, fresh_tried: bool, ok: bool, fresh_ok: bool) {
        let now = Utc::now().timestamp() as u64;
        self.mirrors
            .entry(mirror.into())
            .or_default()
            .observe(fresh_tried, ok, fresh_ok, now);
    }

    /// Mirrors not currently tripped — shown in the pane title when degraded.
    pub fn live_mirrors(&self, mirrors: &[String]) -> usize {
        let now = Utc::now().timestamp() as u64;
        mirrors
            .iter()
            .filter(|m| {
                !self
                    .mirrors
                    .get(*m)
                    .is_some_and(|h| h.plan(now) == MirrorPlan::Skip)
            })
            .count()
    }
}

/// Choose the best mirror result: the freshest feed wins; ties go to the
/// earliest mirror. Errors are reported only when every mirror failed.
fn pick_best_feed(results: Vec<(usize, Result<Vec<Tweet>, String>)>) -> (Vec<Tweet>, Option<String>) {
    let mut best: Option<(usize, Vec<Tweet>)> = None;
    let mut errors = Vec::new();
    for (i, res) in results {
        match res {
            Ok(ts) => {
                let take = match &best {
                    None => true,
                    Some((j, bts)) => {
                        let a = newest_created(&ts);
                        let b = newest_created(bts);
                        match (a, b) {
                            (Some(x), Some(y)) => x > y || (x == y && i < *j),
                            (Some(_), None) => true,
                            (None, Some(_)) => false,
                            (None, None) => i < *j,
                        }
                    }
                };
                if take {
                    best = Some((i, ts));
                }
            }
            Err(e) => errors.push(e),
        }
    }
    let failed = best.is_none();
    let tweets = best.map(|(_, ts)| ts).unwrap_or_default();
    // Mirror errors only matter when every mirror failed — a working feed
    // shouldn't show "mirror error" noise in the pane.
    let err = if failed && !errors.is_empty() {
        Some(errors.join("; "))
    } else {
        None
    };
    (tweets, err)
}

pub async fn fetch_tweets(
    client: &Client,
    handle: &str,
    limit: usize,
    fresh: bool,
    mirrors: &[String],
    health: &mut FeedHealth,
) -> (Vec<Tweet>, Option<String>) {
    // Probe every mirror concurrently so one slow mirror can't stall the feed
    // (a sequential fallback could stack three 12s timeouts per refresh).
    // Each mirror: fresh URL first (live fetch), plain URL as a fallback for
    // mirrors running older Nitter that ignores `cursor`. Tripped mirrors are
    // skipped entirely until their circuit-breaker cooldown expires.
    //
    // Cancel-on-first-fresh: when any mirror returns a non-empty fresh feed,
    // abort the rest so wall time ≈ min(success) instead of max(all). Plain
    // (cached) hits do not abort — a peer may still deliver a live fetch.
    let nonce = fresh.then(fresh_cursor);
    let mut set = tokio::task::JoinSet::new();
    let mut probes: Vec<(String, MirrorPlan)> = Vec::new();
    for base in mirrors {
        let plan = health.plan_mirror(base);
        if plan == MirrorPlan::Skip {
            continue;
        }
        let c = client.clone();
        let b = base.clone();
        let h = handle.to_string();
        let n = nonce.clone();
        let fresh_tried = fresh && plan == MirrorPlan::Full;
        let idx = probes.len();
        probes.push((b.clone(), plan));
        set.spawn(async move {
            let mut errs = Vec::new();
            if fresh_tried {
                let fresh_url = rss_url(&b, &h, n.as_deref());
                match get_text(&c, &fresh_url).await {
                    Ok(xml) if is_rss(&xml) => {
                        return (idx, Ok((true, parse_feed(&xml, &h, limit))));
                    }
                    Ok(_) => errs.push(format!("{b}: not an rss feed (fresh)")),
                    Err(e) => errs.push(format!("{b}: {e} (fresh)")),
                }
            }
            // A stale-but-present feed still beats nothing.
            let plain_url = rss_url(&b, &h, None);
            let outcome = match get_text(&c, &plain_url).await {
                Ok(xml) if is_rss(&xml) => Ok((false, parse_feed(&xml, &h, limit))),
                Ok(_) => Err(format!("{b}: not an rss feed")),
                Err(e) => Err(format!("{b}: {e}")),
            };
            let outcome = match outcome {
                Ok(v) => Ok(v),
                Err(e) => {
                    errs.push(e);
                    Err(errs.join("; "))
                }
            };
            (idx, outcome)
        });
    }

    if probes.is_empty() {
        return (
            Vec::new(),
            Some("all mirrors tripped — cooling down".into()),
        );
    }

    let mut results = Vec::with_capacity(probes.len());
    let mut early = false;
    while let Some(joined) = set.join_next().await {
        match record_mirror_join(health, &probes, fresh, joined) {
            Some((idx, Ok((fresh_ok, ts)))) => {
                if feed_early_done(fresh_ok, &ts) {
                    early = true;
                }
                results.push((idx, Ok(ts)));
            }
            Some((idx, Err(e))) => results.push((idx, Err(e))),
            None => {} // cancelled — not a breaker failure
        }
        if early {
            set.abort_all();
            while let Some(joined) = set.join_next().await {
                match record_mirror_join(health, &probes, fresh, joined) {
                    Some((idx, Ok((_fresh_ok, ts)))) => results.push((idx, Ok(ts))),
                    Some((idx, Err(e))) => results.push((idx, Err(e))),
                    None => {}
                }
            }
            break;
        }
    }
    pick_best_feed(results)
}

/// True when a mirror result is good enough to cancel peers still in flight.
fn feed_early_done(fresh_ok: bool, tweets: &[Tweet]) -> bool {
    fresh_ok && !tweets.is_empty()
}

/// Index into `probes` + fresh flag + tweets, or mirror error string.
type MirrorJoinOk = (usize, Result<(bool, Vec<Tweet>), String>);

/// Map a `JoinSet` completion onto breaker updates + a pick_best row.
/// `None` = task was cancelled (abort) — do not punish the mirror.
fn record_mirror_join(
    health: &mut FeedHealth,
    probes: &[(String, MirrorPlan)],
    fresh: bool,
    joined: Result<MirrorJoinOk, tokio::task::JoinError>,
) -> Option<MirrorJoinOk> {
    match joined {
        Err(e) if e.is_cancelled() => None,
        Ok((idx, Ok((fresh_ok, ts)))) => {
            let (mirror, plan) = &probes[idx];
            let fresh_tried = fresh && *plan == MirrorPlan::Full;
            health.observe_result(mirror, fresh_tried, true, fresh_ok);
            Some((idx, Ok((fresh_ok, ts))))
        }
        Ok((idx, Err(e))) => {
            let (mirror, plan) = &probes[idx];
            let fresh_tried = fresh && *plan == MirrorPlan::Full;
            health.observe_result(mirror, fresh_tried, false, false);
            Some((idx, Err(e)))
        }
        Err(e) => {
            // Non-cancel join failure — no idx; skip breaker.
            let _ = e;
            None
        }
    }
}

pub async fn fetch_announce(
    client: &Client,
    cfg: &Config,
    limit: usize,
    health: &mut FeedHealth,
) -> (Vec<Tweet>, Option<String>) {
    fetch_tweets(client, &cfg.x_handle, limit, cfg.fresh_feed, &cfg.mirrors, health).await
}

pub async fn once_json(cfg: &Config) -> Result<String> {
    let client = http_client()?;
    let mut snap = fetch_snapshot(&client, cfg).await;
    let mut health = FeedHealth::default();
    let (tweets, terr) = fetch_announce(&client, cfg, 12, &mut health).await;
    snap.tweets = tweets;
    snap.tweet_error = terr;
    let t = &snap.token;

    let out = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "fetched_at": snap.fetched_at,
        "price_usd": snap.price_usd(),
        "change_24h": snap.change_24h(),
        "token": {
            "symbol": t.symbol,
            "name": t.name,
            "holders": t.holder_count,
            "holder_change_24h": t.stats_24h.holder_change,
            "mcap": t.mcap,
            "fdv": t.fdv,
            "circ_supply": t.circ_supply,
            "total_supply": t.total_supply,
            "liquidity": t.total_liquidity(),
            "vol_24h": t.stats_24h.buy_volume.zip(t.stats_24h.sell_volume).map(|(b,s)| b+s)
                .or_else(|| t.primary_pair.as_ref().and_then(|p| p.vol_h24)),
            "buys_24h": t.stats_24h.buys,
            "sells_24h": t.stats_24h.sells,
            "traders_24h": t.stats_24h.traders,
            "organic_buyers_24h": t.stats_24h.organic_buyers,
            "net_buyers_24h": t.stats_24h.net_buyers,
            "top10_pct": t.top10_pct,
            "top11_20_pct": t.top11_20_pct,
            "top21_40_pct": t.top21_40_pct,
            "rest_pct": t.rest_pct,
            "dev_balance_pct": t.dev_balance_pct,
            "graph_insiders": t.graph_insiders,
            "lp_locked_pct": t.lp_locked_pct,
            "is_verified": t.is_verified,
            "launchpad": t.launchpad,
            "graduated": t.graduated_at.is_some(),
            "mint_auth_disabled": t.mint_auth_disabled,
            "freeze_auth_disabled": t.freeze_auth_disabled,
            "pairs": t.pairs.len(),
            "primary_dex": t.primary_pair.as_ref().map(|p| &p.dex_id),
            "organic_score": t.organic_score,
            "rug_score": t.rug_score,
        },
        "tweets_n": snap.tweets.len(),
        "tweet_error": snap.tweet_error,
        "gecko": {
            "price_usd": snap.gecko_token.price_usd,
            "liq_usd": snap.gecko_liq_usd(),
            "vol_24h": snap.gecko_token.vol_24h,
            "mcap": snap.gecko_token.market_cap,
        },
        "errors": snap.errors,
    });
    Ok(serde_json::to_string_pretty(&out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture mirroring Nitter's structure: plain post, retweet, quote retweet.
    fn feed() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel>
  <item>
    <title>a normal post</title>
    <description><![CDATA[<p>just a post</p>]]></description>
    <link>https://nitter.net/blknoiz06/status/1#m</link>
    <pubDate>Mon, 10 Aug 2026 12:00:00 GMT</pubDate>
  </item>
  <item>
    <title>RT by @blknoiz06: retweeted content</title>
    <description><![CDATA[<p>retweeted content</p>]]></description>
    <link>https://nitter.net/DokuroSOL/status/2#m</link>
    <pubDate>Mon, 10 Aug 2026 12:01:00 GMT</pubDate>
  </item>
  <item>
    <title>a quote</title>
    <description><![CDATA[<p>my comment</p><hr/><blockquote><b>Kaiz 🐂🀄️ (@Kaiz_294)</b><p>quoted</p></blockquote>]]></description>
    <link>https://nitter.net/blknoiz06/status/3#m</link>
    <pubDate>Mon, 10 Aug 2026 12:02:00 GMT</pubDate>
  </item>
</channel></rss>"#
        .to_string()
    }

    #[test]
    fn tags_retweets_and_quotes() {
        let ts = parse_rss(&feed(), "blknoiz06");
        assert_eq!(ts.len(), 3);
        assert!(!ts[0].retweet);
        assert!(ts[0].retweet_of.is_none());
        assert!(ts[0].quote_author.is_none());
        assert!(ts[1].retweet);
        assert_eq!(ts[1].retweet_of.as_deref(), Some("DokuroSOL"));
        assert_eq!(ts[1].text, "retweeted content");
        assert!(!ts[2].retweet);
        assert_eq!(ts[2].quote_author.as_deref(), Some("Kaiz_294"));
    }

    fn tweet(id: &str, created: Option<&str>) -> Tweet {
        Tweet {
            id: id.into(),
            text: "x".into(),
            created_at: created.map(|s| s.into()),
            url: String::new(),
            handle: Some("h".into()),
            retweet: false,
            retweet_of: None,
            quote_author: None,
        }
    }

    #[test]
    fn rss_url_adds_cursor_only_when_fresh() {
        assert_eq!(rss_url("https://n", "blk", None), "https://n/blk/rss");
        assert_eq!(
            rss_url("https://n", "blk", Some("bb1")),
            "https://n/blk/rss?cursor=bb1"
        );
    }

    #[test]
    fn best_feed_prefers_freshest_tweet() {
        let old = tweet("1", Some("2026-08-10T03:20:03Z"));
        let new = tweet("2", Some("2026-08-10T03:30:03Z"));
        let (ts, err) = pick_best_feed(vec![
            (0, Ok(vec![old])), // earlier mirror, older feed
            (1, Ok(vec![new])), // later mirror, fresher feed
        ]);
        assert_eq!(ts[0].id, "2");
        assert!(err.is_none());
    }

    #[test]
    fn best_feed_ties_go_to_earlier_mirror() {
        let a = tweet("1", Some("2026-08-10T03:20:03Z"));
        let (ts, _) = pick_best_feed(vec![(0, Ok(vec![a.clone()])), (1, Ok(vec![a]))]);
        assert_eq!(ts[0].id, "1");
    }

    #[test]
    fn best_feed_all_errors_reports_them() {
        let (ts, err) = pick_best_feed(vec![(0, Err("a".into())), (1, Err("b".into()))]);
        assert!(ts.is_empty());
        assert_eq!(err.as_deref(), Some("a; b"));
    }

    #[test]
    fn best_feed_success_suppresses_mirror_errors() {
        let a = tweet("1", Some("2026-08-10T03:20:03Z"));
        let (ts, err) = pick_best_feed(vec![
            (0, Ok(vec![a])),
            (1, Err("dead mirror".into())),
        ]);
        assert_eq!(ts[0].id, "1");
        assert!(err.is_none());
    }

    #[test]
    fn breaker_trips_after_repeated_failures_and_recovers() {
        let mut h = MirrorHealth::default();
        for _ in 0..TRIP_THRESHOLD {
            h.observe(true, false, false, 1000);
        }
        assert_eq!(h.plan(1001), MirrorPlan::Skip);
        assert_eq!(h.plan(1000 + TRIP_COOLDOWN_SECS + 1), MirrorPlan::Full);
    }

    #[test]
    fn breaker_downgrades_mirror_that_blocks_fresh() {
        let mut h = MirrorHealth::default();
        for _ in 0..FRESH_THRESHOLD {
            h.observe(true, true, false, 1000); // fresh fails, plain serves
        }
        assert_eq!(h.plan(1001), MirrorPlan::PlainOnly);
        h.observe(false, true, false, 1001); // plain-only poll stays healthy
        assert_eq!(h.plan(1002), MirrorPlan::PlainOnly);
        // Fresh is probed again once the cooldown passes.
        assert_eq!(
            h.plan(1000 + FRESH_COOLDOWN_SECS + 1),
            MirrorPlan::Full
        );
    }

    #[test]
    fn breaker_fresh_success_resets_streaks() {
        let mut h = MirrorHealth::default();
        h.observe(true, true, false, 1000);
        h.observe(true, true, false, 1001);
        assert_eq!(h.plan(1002), MirrorPlan::Full); // under threshold
        h.observe(true, true, true, 1002); // fresh works again
        h.observe(true, true, false, 1003); // then flips again — no stale downgrade
        assert_eq!(h.plan(1004), MirrorPlan::Full);
    }

    #[test]
    fn breaker_fresh_only_failures_never_trip_mirror() {
        let mut h = MirrorHealth::default();
        for _ in 0..10 {
            h.observe(true, true, false, 1000); // plain ok every time
        }
        assert_ne!(h.plan(1001), MirrorPlan::Skip);
    }

    #[test]
    fn parses_gecko_token() {
        let raw = Ok(serde_json::json!({"data": {"attributes": {
            "price_usd": "0.2131",
            "price_change_percentage": {"h24": "5.2"},
            "market_cap_usd": "213100000",
            "fdv_usd": "213200000",
            "total_reserve_in_usd": "2813776.01",
            "volume_usd": {"h24": "14899982.77"},
        }}}));
        let mut errors = HashMap::new();
        let g = parse_gecko_token(raw, &mut errors);
        assert_eq!(g.price_usd, Some(0.2131));
        assert_eq!(g.change_24h, Some(5.2));
        assert_eq!(g.market_cap, Some(213100000.0));
        assert_eq!(g.fdv, Some(213200000.0));
        assert_eq!(g.vol_24h, Some(14899982.77));
        assert_eq!(g.liquidity, Some(2813776.01));
        assert!(errors.is_empty());
    }

    #[test]
    fn gecko_errors_are_recorded_not_fatal() {
        let mut errors = HashMap::new();
        let g = parse_gecko_token(Err("HTTP 429".into()), &mut errors);
        assert_eq!(g.price_usd, None);
        assert_eq!(errors.get("gecko-token").map(String::as_str), Some("HTTP 429"));
    }

    #[test]
    fn clean_text_keeps_legit_hashm() {
        // "#m" inside the body survives; only a trailing #m fragment is cut.
        assert_eq!(clean_tweet_text("the #m move is on"), "the #m move is on");
        assert_eq!(clean_tweet_text("buy the dip #m"), "buy the dip");
        assert_eq!(
            clean_tweet_text("text https://nitter.net/u/status/1#m"),
            "text"
        );
    }

    #[test]
    fn feed_early_done_requires_fresh_nonempty() {
        let t = Tweet {
            id: "1".into(),
            text: "hi".into(),
            ..Default::default()
        };
        assert!(feed_early_done(true, std::slice::from_ref(&t)));
        assert!(!feed_early_done(false, std::slice::from_ref(&t)));
        assert!(!feed_early_done(true, &[]));
        assert!(!feed_early_done(false, &[]));
    }

    #[test]
    fn cancelled_join_does_not_trip_breaker() {
        let probes = vec![
            ("https://nitter.net".into(), MirrorPlan::Full),
            ("https://nitter.poast.org".into(), MirrorPlan::Full),
        ];
        let mut health = FeedHealth::default();
        let win = Ok((
            0usize,
            Ok((
                true,
                vec![Tweet {
                    id: "1".into(),
                    text: "x".into(),
                    ..Default::default()
                }],
            )),
        ));
        let got = record_mirror_join(&mut health, &probes, true, win);
        assert!(matches!(got, Some((0, Ok((true, ref ts)))) if feed_early_done(true, ts)));

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancelled = rt.block_on(async {
            let mut set = tokio::task::JoinSet::new();
            set.spawn(async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                (
                    1usize,
                    Ok((false, Vec::<Tweet>::new())) as Result<(bool, Vec<Tweet>), String>,
                )
            });
            set.abort_all();
            set.join_next().await.unwrap()
        });
        assert!(cancelled.as_ref().err().is_some_and(|e| e.is_cancelled()));
        let recorded = record_mirror_join(&mut health, &probes, true, cancelled);
        assert!(recorded.is_none());
        assert_eq!(
            health.plan_mirror("https://nitter.poast.org"),
            MirrorPlan::Full
        );
        assert_eq!(health.plan_mirror("https://nitter.net"), MirrorPlan::Full);
    }

    #[test]
    fn retryable_matches_flaky_failures_only() {
        let gecko = "https://api.geckoterminal.com/api/v2/networks/solana/tokens/x";
        let jup = "https://lite-api.jup.ag/tokens/v2/search?query=x";
        // Gecko 429 is not retried — doubles free-tier pressure with near-zero upside.
        assert!(!retryable("HTTP 429", gecko));
        // Non-Gecko 429 still gets one backoff retry.
        assert!(retryable("HTTP 429", jup));
        assert!(retryable("HTTP 500", jup));
        assert!(retryable("HTTP 503", gecko));
        assert!(retryable(
            "reqwest: operation timed out",
            "https://api.rugcheck.xyz/v1/tokens/x/report"
        ));
        assert!(retryable(
            "reqwest: error sending request for url (x): connection reset by peer",
            jup
        ));
        assert!(!retryable("HTTP 404", jup));
        assert!(!retryable("HTTP 401", jup));
        assert!(!retryable("json: unexpected end of input", jup));
        assert!(!retryable("text: stream error", jup));
    }

    #[test]
    fn gecko_fetch_specs_is_two_and_omits_pools() {
        let specs = gecko_fetch_specs("9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump");
        assert_eq!(specs.len(), 2);
        let names: Vec<_> = specs.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["gecko", "gecko-token"]);
        for (_, url) in &specs {
            assert!(url.contains("api.geckoterminal.com"));
            assert!(!url.contains("/pools"));
        }
    }
}
