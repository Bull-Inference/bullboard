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

impl WindowStats {
    /// True when no window stats were parsed — the source failed this poll.
    pub fn is_empty(&self) -> bool {
        self.price_change.is_none()
            && self.holder_change.is_none()
            && self.liquidity_change.is_none()
            && self.volume_change.is_none()
            && self.buy_volume.is_none()
            && self.sell_volume.is_none()
            && self.buys.is_none()
            && self.sells.is_none()
            && self.traders.is_none()
            && self.organic_buyers.is_none()
            && self.net_buyers.is_none()
    }
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
    pub liq_usd: Option<f64>,
    /// Quote token symbol (e.g. "USDC") — shown on the LIQUIDITY card.
    pub quote_symbol: String,
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

/// GeckoTerminal token endpoint — a second independent price / volume /
/// market-cap source to cross-check against Bull.inf and Jupiter.
#[derive(Clone, Debug, Default)]
pub struct GeckoToken {
    pub price_usd: Option<f64>,
    pub change_24h: Option<f64>,
    pub market_cap: Option<f64>,
    pub fdv: Option<f64>,
    pub vol_24h: Option<f64>,
    /// `total_reserve_in_usd` — Gecko's authoritative token-level liquidity.
    pub liquidity: Option<f64>,
}

/// One GeckoTerminal pool — per-pool on-chain reserves are usually closer to
/// real liquidity than DexScreener's summed figure.
#[derive(Clone, Debug, Default)]
pub struct GeckoPool {
    pub liq_usd: Option<f64>,
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

impl Token {
    /// Estimated total liquidity across ALL markets.
    ///
    /// Best source: sum of every DexScreener pair's USD liquidity (each pool is
    /// real, current, per-DEX). Cross-check: Rugcheck's `totalMarketLiquidity`
    /// aggregates the same markets. Fallback: Jupiter's `liquidity` (top pool).
    pub fn total_liquidity(&self) -> Option<f64> {
        let sum: f64 = self.pairs.iter().filter_map(|p| p.liq_usd).sum();
        if sum > 0.0 {
            Some(sum)
        } else {
            self.total_market_liq.or(self.liquidity)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub price: Value,
    pub ohlc: Value,
    pub token: Token,
    pub gecko_token: GeckoToken,
    pub gecko_pools: Vec<GeckoPool>,
    pub tweets: Vec<Tweet>,
    pub tweet_error: Option<String>,
    pub errors: HashMap<String, String>,
    pub fetched_at: Option<String>,
}

impl Snapshot {
    /// Fill every gap in a fresh snapshot from the previous one, so a flaky
    /// source keeps showing its last good values instead of flickering "—"
    /// every poll. Fields the new snapshot actually resolved are kept;
    /// anything it failed to resolve (None / empty) falls back to the old
    /// value. `errors` and `fetched_at` always come from the new snapshot.
    pub fn merge_stale(mut new: Snapshot, old: &Snapshot) -> Snapshot {
        macro_rules! keep_opt {
            ($dst:expr, $src:expr) => {
                if $dst.is_none() {
                    $dst = $src.clone();
                }
            };
        }

        if new.price.is_null() {
            new.price = old.price.clone();
        }
        if new.ohlc.is_null() {
            new.ohlc = old.ohlc.clone();
        }

        let nt = &mut new.token;
        let ot = &old.token;
        if nt.name.is_empty() {
            nt.name = ot.name.clone();
        }
        if nt.symbol.is_empty() {
            nt.symbol = ot.symbol.clone();
        }
        keep_opt!(nt.price_usd, ot.price_usd);
        keep_opt!(nt.mcap, ot.mcap);
        keep_opt!(nt.fdv, ot.fdv);
        keep_opt!(nt.liquidity, ot.liquidity);
        keep_opt!(nt.circ_supply, ot.circ_supply);
        keep_opt!(nt.total_supply, ot.total_supply);
        keep_opt!(nt.holder_count, ot.holder_count);
        keep_opt!(nt.decimals, ot.decimals);
        keep_opt!(nt.launchpad, ot.launchpad);
        keep_opt!(nt.graduated_at, ot.graduated_at);
        keep_opt!(nt.graduated_pool, ot.graduated_pool);
        keep_opt!(nt.dev, ot.dev);
        keep_opt!(nt.creator, ot.creator);
        keep_opt!(nt.twitter, ot.twitter);
        keep_opt!(nt.website, ot.website);
        keep_opt!(nt.organic_score, ot.organic_score);
        keep_opt!(nt.organic_label, ot.organic_label);
        keep_opt!(nt.is_verified, ot.is_verified);
        keep_opt!(nt.mint_auth_disabled, ot.mint_auth_disabled);
        keep_opt!(nt.freeze_auth_disabled, ot.freeze_auth_disabled);
        keep_opt!(nt.top_holders_pct, ot.top_holders_pct);
        keep_opt!(nt.dev_balance_pct, ot.dev_balance_pct);
        keep_opt!(nt.top10_pct, ot.top10_pct);
        keep_opt!(nt.top11_20_pct, ot.top11_20_pct);
        keep_opt!(nt.top21_40_pct, ot.top21_40_pct);
        keep_opt!(nt.rest_pct, ot.rest_pct);
        keep_opt!(nt.rug_score, ot.rug_score);
        keep_opt!(nt.lp_locked_pct, ot.lp_locked_pct);
        keep_opt!(nt.total_market_liq, ot.total_market_liq);
        keep_opt!(nt.graph_insiders, ot.graph_insiders);
        keep_opt!(nt.rugged, ot.rugged);
        keep_opt!(nt.markets_n, ot.markets_n);
        if nt.tags.is_empty() {
            nt.tags = ot.tags.clone();
        }
        if nt.risks.is_empty() {
            nt.risks = ot.risks.clone();
        }
        if nt.top_holders.is_empty() {
            nt.top_holders = ot.top_holders.clone();
        }
        if nt.pairs.is_empty() {
            nt.pairs = ot.pairs.clone();
        }
        if nt.primary_pair.is_none() {
            nt.primary_pair = ot.primary_pair.clone();
        }
        for (nw, ow) in [
            (&mut nt.stats_5m, &ot.stats_5m),
            (&mut nt.stats_1h, &ot.stats_1h),
            (&mut nt.stats_6h, &ot.stats_6h),
            (&mut nt.stats_24h, &ot.stats_24h),
        ] {
            if nw.is_empty() {
                *nw = ow.clone();
            }
        }

        let ng = &mut new.gecko_token;
        let og = &old.gecko_token;
        keep_opt!(ng.price_usd, og.price_usd);
        keep_opt!(ng.change_24h, og.change_24h);
        keep_opt!(ng.market_cap, og.market_cap);
        keep_opt!(ng.fdv, og.fdv);
        keep_opt!(ng.vol_24h, og.vol_24h);
        keep_opt!(ng.liquidity, og.liquidity);
        if new.gecko_pools.is_empty() {
            new.gecko_pools = old.gecko_pools.clone();
        }

        new
    }

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
            .or(self.gecko_token.price_usd)
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
            .or(self.gecko_token.change_24h)
    }

    /// Gecko's view of liquidity: the token endpoint's `total_reserve_in_usd`
    /// when available, else the sum of per-pool reserves.
    pub fn gecko_liq_usd(&self) -> Option<f64> {
        if let Some(v) = self.gecko_token.liquidity {
            return Some(v);
        }
        let mut sum = 0.0;
        let mut any = false;
        for p in &self.gecko_pools {
            if let Some(v) = p.liq_usd {
                sum += v;
                any = true;
            }
        }
        any.then_some(sum)
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

    /// 24h high/low from the last 24 hourly candles.
    pub fn range_24h(&self) -> (Option<f64>, Option<f64>) {
        let mut hi = None;
        let mut lo = None;
        if let Some(arr) = self.ohlc.get("candles").and_then(|c| c.as_array()) {
            for c in arr.iter().rev().take(24) {
                if let Some(h) = c.get("high").and_then(|v| v.as_f64()) {
                    hi = Some(hi.map_or(h, |x: f64| x.max(h)));
                }
                if let Some(l) = c.get("low").and_then(|v| v.as_f64()) {
                    lo = Some(lo.map_or(l, |x: f64| x.min(l)));
                }
            }
        }
        (hi, lo)
    }

    pub fn ohlc_stat_f(&self, key: &str) -> Option<f64> {
        self.ohlc.pointer(&format!("/stats/{key}")).and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_u64().map(|u| u as f64))
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_stale_keeps_last_good_when_new_missing() {
        let mut old = Snapshot::default();
        old.token.price_usd = Some(0.0042);
        old.token.rug_score = Some(87.0);
        old.token.holder_count = Some(1234);
        old.token.top10_pct = Some(42.0);
        old.gecko_token.price_usd = Some(0.0041);

        // New snapshot: everything failed except a fresh rug score.
        let mut new = Snapshot::default();
        new.token.price_usd = None;
        new.token.rug_score = Some(90.0);
        new.token.pairs = Vec::new();
        new.errors.insert("jupiter".into(), "HTTP 429".into());
        new.fetched_at = Some("2026-08-11T00:00:00Z".into());

        let merged = Snapshot::merge_stale(new, &old);
        assert_eq!(merged.token.price_usd, Some(0.0042)); // kept from old
        assert_eq!(merged.token.rug_score, Some(90.0)); // new wins when present
        assert_eq!(merged.token.holder_count, Some(1234));
        assert_eq!(merged.token.top10_pct, Some(42.0));
        assert_eq!(merged.gecko_token.price_usd, Some(0.0041));
        assert_eq!(merged.errors.get("jupiter").map(String::as_str), Some("HTTP 429"));
        assert_eq!(merged.fetched_at.as_deref(), Some("2026-08-11T00:00:00Z"));
    }

    #[test]
    fn merge_stale_prefers_fresh_values() {
        let mut old = Snapshot::default();
        old.token.price_usd = Some(0.0010);
        let mut new = Snapshot::default();
        new.token.price_usd = Some(0.0020);
        new.token.holder_count = None; // source hiccuped on holders only
        old.token.holder_count = Some(500);

        let merged = Snapshot::merge_stale(new, &old);
        assert_eq!(merged.token.price_usd, Some(0.0020)); // fresh price wins
        assert_eq!(merged.token.holder_count, Some(500)); // stale holders kept
    }

    #[test]
    fn merge_stale_keeps_raw_json_sparkline_data() {
        let old = Snapshot {
            ohlc: serde_json::json!({"candles": [{"close": 1.0}]}),
            ..Default::default()
        };
        let new = Snapshot::default(); // ohlc fetch failed → Value::Null
        let merged = Snapshot::merge_stale(new, &old);
        assert_eq!(merged.closes(), vec![1.0]);
    }
}
