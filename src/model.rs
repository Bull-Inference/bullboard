use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Tweet {
    pub id: String,
    pub text: String,
    pub created_at: Option<String>,
    pub url: String,
    pub handle: Option<String>,
    /// True when this item is a retweet (Nitter titles them "RT by @…").
    pub retweet: bool,
    /// Original author of a retweet, parsed from the status URL.
    pub retweet_of: Option<String>,
    /// Quoted author of a quote retweet, parsed from the embedded blockquote.
    pub quote_author: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TopHolder {
    pub owner: String,
    pub pct: Option<f64>,
    pub ui_amount: Option<f64>,
    pub insider: bool,
}

#[derive(Clone, Debug, Default)]
pub struct WindowStats {
    pub price_change: Option<f64>,
    pub holder_change: Option<f64>,
    pub liquidity_change: Option<f64>,
    pub volume_change: Option<f64>,
    pub buy_volume: Option<f64>,
    pub sell_volume: Option<f64>,
    pub buys: Option<u64>,
    pub sells: Option<u64>,
    pub traders: Option<u64>,
    pub organic_buyers: Option<u64>,
    pub net_buyers: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct DexPair {
    pub dex_id: String,
    pub pair_address: String,
    pub price_usd: Option<f64>,
    pub change_m5: Option<f64>,
    pub change_h1: Option<f64>,
    pub change_h6: Option<f64>,
    pub change_h24: Option<f64>,
    pub vol_h24: Option<f64>,
    pub vol_h1: Option<f64>,
    pub liq_usd: Option<f64>,
    pub liq_base: Option<f64>,
    pub liq_quote: Option<f64>,
    pub quote_symbol: String,
    pub buys_h24: Option<u64>,
    pub sells_h24: Option<u64>,
    pub fdv: Option<f64>,
    pub mcap: Option<f64>,
    pub pair_created_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct RiskFlag {
    pub name: String,
    pub level: String,
    pub value: String,
}

/// Aggregated $ANSEM on-chain / market intelligence.
#[derive(Clone, Debug, Default)]
pub struct Token {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub price_usd: Option<f64>,
    pub mcap: Option<f64>,
    pub fdv: Option<f64>,
    pub liquidity: Option<f64>,
    pub circ_supply: Option<f64>,
    pub total_supply: Option<f64>,
    pub holder_count: Option<u64>,
    pub decimals: Option<u32>,

    pub launchpad: Option<String>,
    pub graduated_at: Option<String>,
    pub graduated_pool: Option<String>,
    pub dev: Option<String>,
    pub creator: Option<String>,
    pub twitter: Option<String>,
    pub website: Option<String>,

    pub organic_score: Option<f64>,
    pub organic_label: Option<String>,
    pub is_verified: Option<bool>,
    pub tags: Vec<String>,

    // audit (jupiter + gecko + rug)
    pub mint_auth_disabled: Option<bool>,
    pub freeze_auth_disabled: Option<bool>,
    pub top_holders_pct: Option<f64>,
    pub dev_balance_pct: Option<f64>,
    pub top10_pct: Option<f64>,
    pub top11_20_pct: Option<f64>,
    pub top21_40_pct: Option<f64>,
    pub rest_pct: Option<f64>,
    pub rug_score: Option<f64>,
    pub lp_locked_pct: Option<f64>,
    pub total_market_liq: Option<f64>,
    pub graph_insiders: Option<u64>,
    pub rugged: Option<bool>,
    pub markets_n: Option<u64>,
    pub risks: Vec<RiskFlag>,
    pub top_holders: Vec<TopHolder>,

    pub stats_5m: WindowStats,
    pub stats_1h: WindowStats,
    pub stats_6h: WindowStats,
    pub stats_24h: WindowStats,

    pub pairs: Vec<DexPair>,
    pub primary_pair: Option<DexPair>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub price: Value,
    pub ohlc: Value,
    pub token: Token,
    pub tweets: Vec<Tweet>,
    pub tweet_error: Option<String>,
    pub errors: HashMap<String, String>,
    pub fetched_at: Option<String>,
}

impl Snapshot {
    pub fn price_usd(&self) -> Option<f64> {
        self.token
            .price_usd
            .or_else(|| self.price.get("usd_price").and_then(|v| v.as_f64()))
            .or_else(|| self.ohlc.pointer("/stats/price_usd").and_then(|v| v.as_f64()))
            .or_else(|| {
                self.token
                    .primary_pair
                    .as_ref()
                    .and_then(|p| p.price_usd)
            })
    }

    pub fn change_24h(&self) -> Option<f64> {
        self.token
            .stats_24h
            .price_change
            .or_else(|| {
                self.token
                    .primary_pair
                    .as_ref()
                    .and_then(|p| p.change_h24)
            })
            .or_else(|| self.ohlc.pointer("/stats/change_24h").and_then(|v| v.as_f64()))
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

    pub fn volumes(&self) -> Vec<f64> {
        self.ohlc
            .get("candles")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("volume").and_then(|v| v.as_f64()))
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

}
