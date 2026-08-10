use crate::config::{Config, FeedMode, NITTER_BASES};
use crate::model::{
    DexPair, FeedItem, RiskFlag, Snapshot, Token, TopHolder, Tweet, WindowStats,
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
        .user_agent("bullboard/0.2")
        .build()?)
}

async fn get_json(client: &Client, url: &str) -> Result<Value, String> {
    match client.get(url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.json::<Value>()
                .await
                .map_err(|e| format!("json: {e}"))
        }
        Err(e) => Err(format!("{e}")),
    }
}

async fn get_text(client: &Client, url: &str) -> Result<String, String> {
    match client.get(url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status().as_u16()));
            }
            resp.text().await.map_err(|e| format!("text: {e}"))
        }
        Err(e) => Err(format!("{e}")),
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
        let tx = p.get("txns").cloned().unwrap_or(Value::Null);
        let h24 = tx.get("h24").cloned().unwrap_or(Value::Null);
        let quote = p
            .pointer("/quoteToken/symbol")
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .to_string();
        pairs.push(DexPair {
            dex_id: json_str(&p, "dexId").unwrap_or_else(|| "?".into()),
            pair_address: json_str(&p, "pairAddress").unwrap_or_default(),
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
            vol_h1: json_f(&vol, "h1"),
            liq_usd: json_f(&liq, "usd"),
            liq_base: json_f(&liq, "base"),
            liq_quote: json_f(&liq, "quote"),
            quote_symbol: quote,
            buys_h24: json_u(&h24, "buys"),
            sells_h24: json_u(&h24, "sells"),
            fdv: json_f(&p, "fdv"),
            mcap: json_f(&p, "marketCap"),
            pair_created_ms: json_u(&p, "pairCreatedAt"),
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
        mint: mint.to_string(),
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

pub async fn fetch_snapshot(client: &Client, cfg: &Config) -> Snapshot {
    let base = cfg.api_base.trim_end_matches('/');
    let mint = cfg.mint.clone();

    let spawn = |name: &'static str, url: String| {
        let c = client.clone();
        tokio::spawn(async move { (name.to_string(), get_json(&c, &url).await) })
    };

    let tasks = [
        spawn("network", format!("{base}/api/network")),
        spawn("config", format!("{base}/api/config")),
        spawn("price", format!("{base}/api/token-price")),
        spawn(
            "ohlc",
            format!("{base}/api/token-ohlc?interval=1h&limit=48"),
        ),
        spawn("stats", format!("{base}/api/stats")),
        spawn("stake", format!("{base}/api/stake/stats")),
        spawn("feed", format!("{base}/api/markets/feed?limit=30")),
        spawn(
            "jupiter",
            format!("https://lite-api.jup.ag/tokens/v2/search?query={mint}"),
        ),
        spawn(
            "gecko",
            format!("https://api.geckoterminal.com/api/v2/networks/solana/tokens/{mint}/info"),
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

    let mut map = HashMap::new();
    for t in tasks {
        if let Ok((name, res)) = t.await {
            map.insert(name, res);
        }
    }

    let mut errors = HashMap::new();
    let mut snap = Snapshot::default();
    snap.network = take_val(&mut map, "network", &mut errors);
    snap.config = take_val(&mut map, "config", &mut errors);
    snap.price = take_val(&mut map, "price", &mut errors);
    snap.ohlc = take_val(&mut map, "ohlc", &mut errors);
    snap.stats = take_val(&mut map, "stats", &mut errors);
    snap.stake = take_val(&mut map, "stake", &mut errors);

    match map.remove("feed") {
        Some(Ok(v)) => {
            if let Some(arr) = v.get("feed").and_then(|f| f.as_array()) {
                snap.feed = arr
                    .iter()
                    .filter_map(|ev| {
                        Some(FeedItem {
                            model_id: ev.get("model_id")?.as_str()?.to_string(),
                            cost: json_f(ev, "cost"),
                            created_at: ev
                                .get("created_at")
                                .and_then(|c| c.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect();
            }
        }
        Some(Err(e)) => {
            errors.insert("feed".into(), e);
        }
        None => {}
    }

    let jup = map.remove("jupiter").unwrap_or(Err("missing".into()));
    let gecko = map.remove("gecko").unwrap_or(Err("missing".into()));
    let rug = map.remove("rugcheck").unwrap_or(Err("missing".into()));
    let dex = map.remove("dex").unwrap_or(Err("missing".into()));
    snap.token = parse_token(&mint, jup, gecko, rug, dex, &mut errors);
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
    t = t.replace("#m", "");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
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
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    tweets
}

pub async fn fetch_tweets(
    client: &Client,
    handle: &str,
    limit: usize,
) -> (Vec<Tweet>, Option<String>) {
    let mut errors = Vec::new();
    for base in NITTER_BASES {
        let url = format!("{base}/{handle}/rss");
        match get_text(client, &url).await {
            Ok(xml) => {
                let mut tweets = parse_rss(&xml, handle);
                tweets.truncate(limit);
                return (tweets, None);
            }
            Err(e) => errors.push(format!("{base}: {e}")),
        }
    }
    (Vec::new(), Some(errors.join("; ")))
}

pub async fn fetch_announce(
    client: &Client,
    cfg: &Config,
    mode: FeedMode,
    limit: usize,
) -> (Vec<Tweet>, Option<String>) {
    match mode {
        FeedMode::Primary => fetch_tweets(client, &cfg.x_handle, limit).await,
        FeedMode::Alt => fetch_tweets(client, &cfg.x_handle_alt, limit).await,
        FeedMode::Both => {
            let (a, ea) = fetch_tweets(client, &cfg.x_handle, limit).await;
            let (b, eb) = fetch_tweets(client, &cfg.x_handle_alt, limit).await;
            let mut merged = a;
            merged.extend(b);
            merged.sort_by(|x, y| y.created_at.cmp(&x.created_at));
            merged.truncate(limit);
            (merged, ea.or(eb))
        }
    }
}

pub async fn once_json(cfg: &Config) -> Result<String> {
    let client = http_client()?;
    let mut snap = fetch_snapshot(&client, cfg).await;
    let (tweets, terr) = fetch_announce(&client, cfg, FeedMode::Primary, 12).await;
    snap.tweets = tweets;
    snap.tweet_error = terr;
    let t = &snap.token;

    let out = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "fetched_at": snap.fetched_at,
        "price_usd": snap.price_usd(),
        "change_24h": snap.change_24h(),
        "token": {
            "holders": t.holder_count,
            "holder_change_24h": t.stats_24h.holder_change,
            "mcap": t.mcap,
            "liquidity": t.liquidity,
            "vol_24h": t.stats_24h.buy_volume.zip(t.stats_24h.sell_volume).map(|(b,s)| b+s)
                .or_else(|| t.primary_pair.as_ref().and_then(|p| p.vol_h24)),
            "buys_24h": t.stats_24h.buys,
            "sells_24h": t.stats_24h.sells,
            "traders_24h": t.stats_24h.traders,
            "top10_pct": t.top10_pct,
            "mint_auth_disabled": t.mint_auth_disabled,
            "freeze_auth_disabled": t.freeze_auth_disabled,
            "pairs": t.pairs.len(),
            "primary_dex": t.primary_pair.as_ref().map(|p| &p.dex_id),
            "organic_score": t.organic_score,
            "rug_score": t.rug_score,
        },
        "feed_n": snap.feed.len(),
        "tweets_n": snap.tweets.len(),
        "tweet_error": snap.tweet_error,
        "errors": snap.errors,
    });
    Ok(serde_json::to_string_pretty(&out)?)
}
