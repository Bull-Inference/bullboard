use crate::config::{Config, FeedMode, NITTER_BASES};
use crate::model::{FeedItem, Holders, Snapshot, TopHolder, Tweet};
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

fn parse_holders(
    mint: &str,
    jup_raw: Result<Value, String>,
    gecko_raw: Result<Value, String>,
    rug_raw: Result<Value, String>,
    errors: &mut HashMap<String, String>,
) -> Holders {
    let mut h = Holders::default();

    let jup_obj = match jup_raw {
        Ok(Value::Array(arr)) => {
            let mut found = None;
            let mut first = None;
            for row in arr {
                if first.is_none() {
                    first = Some(row.clone());
                }
                if row.get("id").and_then(|i| i.as_str()) == Some(mint) {
                    found = Some(row);
                    break;
                }
            }
            found.or(first)
        }
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => None,
        Err(e) => {
            errors.insert("jupiter".into(), e);
            None
        }
    };

    if let Some(ref j) = jup_obj {
        h.holder_count = json_u(j, "holderCount");
        h.circ_supply = json_f(j, "circSupply");
        if let Some(s) = j.get("stats1h") {
            h.holder_change_1h = json_f(s, "holderChange");
        }
        if let Some(s) = j.get("stats6h") {
            h.holder_change_6h = json_f(s, "holderChange");
        }
        if let Some(s) = j.get("stats24h") {
            h.holder_change_24h = json_f(s, "holderChange");
            h.traders_24h = json_u(s, "numTraders");
        }
    }

    match gecko_raw {
        Ok(g) => {
            if let Some(attrs) = g.pointer("/data/attributes") {
                if h.circ_supply.is_none() {
                    h.circ_supply = json_f(attrs, "normalized_total_supply");
                }
                if let Some(holders) = attrs.get("holders") {
                    if h.holder_count.is_none() {
                        h.holder_count = json_u(holders, "count");
                    }
                    if let Some(dist) = holders.get("distribution_percentage") {
                        h.top10_pct = json_f(dist, "top_10");
                        h.rest_pct = json_f(dist, "rest");
                    }
                }
            }
        }
        Err(e) => {
            errors.insert("gecko".into(), e);
        }
    }

    match rug_raw {
        Ok(r) => {
            if h.holder_count.is_none() {
                h.holder_count = json_u(&r, "totalHolders");
            }
            if let Some(arr) = r.get("topHolders").and_then(|a| a.as_array()) {
                for th in arr.iter().take(12) {
                    let owner = th
                        .get("owner")
                        .or_else(|| th.get("address"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    h.top_holders.push(TopHolder {
                        owner,
                        pct: json_f(th, "pct"),
                    });
                }
            }
        }
        Err(e) => {
            errors.insert("rugcheck".into(), e);
        }
    }

    h
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
    snap.holders = parse_holders(&mint, jup, gecko, rug, &mut errors);
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

    let out = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "fetched_at": snap.fetched_at,
        "price_usd": snap.price_usd(),
        "change_24h": snap.change_24h(),
        "holders": {
            "holder_count": snap.holders.holder_count,
            "holder_change_24h": snap.holders.holder_change_24h,
            "top10_pct": snap.holders.top10_pct,
            "traders_24h": snap.holders.traders_24h,
        },
        "feed_n": snap.feed.len(),
        "tweets_n": snap.tweets.len(),
        "tweet_error": snap.tweet_error,
        "errors": snap.errors,
    });
    Ok(serde_json::to_string_pretty(&out)?)
}
