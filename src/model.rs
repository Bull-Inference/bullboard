use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Tweet {
    #[allow(dead_code)]
    pub id: String,
    pub text: String,
    pub created_at: Option<String>,
    #[allow(dead_code)]
    pub url: String,
    pub handle: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FeedItem {
    pub model_id: String,
    pub cost: Option<f64>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TopHolder {
    pub owner: String,
    pub pct: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub network: Value,
    pub config: Value,
    pub price: Value,
    pub ohlc: Value,
    pub stats: Value,
    pub stake: Value,
    pub feed: Vec<FeedItem>,
    pub holders: Holders,
    pub tweets: Vec<Tweet>,
    pub tweet_error: Option<String>,
    pub errors: HashMap<String, String>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Holders {
    pub holder_count: Option<u64>,
    pub circ_supply: Option<f64>,
    pub holder_change_1h: Option<f64>,
    pub holder_change_6h: Option<f64>,
    pub holder_change_24h: Option<f64>,
    pub traders_24h: Option<u64>,
    pub top10_pct: Option<f64>,
    pub rest_pct: Option<f64>,
    pub top_holders: Vec<TopHolder>,
}

impl Snapshot {
    pub fn price_usd(&self) -> Option<f64> {
        self.price
            .get("usd_price")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                self.ohlc
                    .pointer("/stats/price_usd")
                    .and_then(|v| v.as_f64())
            })
            .or_else(|| self.network.get("usd_price").and_then(|v| v.as_f64()))
    }

    pub fn change_24h(&self) -> Option<f64> {
        self.ohlc
            .pointer("/stats/change_24h")
            .and_then(|v| v.as_f64())
    }

    pub fn closes(&self) -> Vec<f64> {
        self.ohlc
            .get("candles")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("close").and_then(|v| v.as_f64()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn ohlc_stat_f(&self, key: &str) -> Option<f64> {
        self.ohlc.pointer(&format!("/stats/{key}")).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

    pub fn net_str(&self, key: &str) -> Option<&str> {
        self.network.get(key).and_then(|v| v.as_str())
    }

    pub fn net_f(&self, key: &str) -> Option<f64> {
        self.network.get(key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

    pub fn net_bool(&self, key: &str) -> Option<bool> {
        self.network.get(key).and_then(|v| v.as_bool())
    }

    pub fn cfg_f(&self, key: &str) -> Option<f64> {
        self.config.get(key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

    pub fn stake_f(&self, key: &str) -> Option<f64> {
        self.stake.get(key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

    pub fn stats_f(&self, key: &str) -> Option<f64> {
        self.stats.get(key).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

    pub fn stats_u(&self, key: &str) -> Option<u64> {
        self.stats.get(key).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().map(|i| i as u64))
                .or_else(|| v.as_f64().map(|f| f as u64))
        })
    }
}
