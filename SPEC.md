# bullboard — terminal dashboard for $ANSEM / @blknoiz06

## Outcome (must match)

A **professional multi-pane terminal dashboard** in the spirit of Surfboard
(MAXPANE): dark matrix aesthetic, live-updating panes, keyboard + mouse
driven. A single static Rust binary, installable via `cargo install`; a thin
npm launcher is optional.

```
   BULLBOARD · ANSEM $0.2090 · 24h ▲ +19.22% · vol $20.25M · src 8/8
 ┌ PRICE ────────┐ ┌ LIQUIDITY ─────────┐ ┌ SAFETY ───────────┐ ┌ SUPPLY ──────────┐
 │    $0.2090    │ │      $4.57M        │ │      CLEAN        │ │     142,175      │
 │ 24h ▲ +19.22% │ │ PUMPSWAP ANSEM/SOL │ │ mint off·freezeoff│ │ mcap $208.99M    │
 │ vol $20.24M   │ │      30 pools      │ │ rug 43·insiders 12│ │ circ 999.82M     │
 │  gecko $0.2091│ │  rugcheck $4.58M   │ │   lp 82% locked   │ │   fdv $208.99M   │
 │               │ │   gecko $2.80M (!) │ │                   │ │                  │
 ┌ ANNOUNCE · @blknoiz06 · 9s · last 3h · ↕ ─────────────┐  SIGNALS · ↕
 │ 08-11 06:27 POST RT @Saint10Fourteen no disrespect…    │  ● MINT AUTH  disabled·safe
 │   [ view tweet ]                                       │  ● FREEZE AUTH disabled·safe
 │ 08-11 06:08 POST RT @Jxjethro this szn and forever…    │  ◐ LIQUIDITY  $2.13M deep
 │                                                        │  ┌ ACTIVITY · ↕ ────────────┐
 └────────────────────────────────────────────────────────┘  │ 5m B $12.35K S $10.27K … │
  MARKET · ↕                                  HOLDERS · ↕    │ 24h org 495 · net 3409   │
  ANSEM $0.2090 24h ▲ +19.22% [verified]     142,175 1h …    │ meteora 6e7V… 44d liq …  │
  5m ▲ 1h ▼ 6h ▲  24h low/high              top10 62.7% ▓    │ gecko 6 pools (!)       │
  price ▂▂▃▃▄▅▇█  vol ▁▁▂▃▄▆              dist 2.8/3.5/31   └─────────────────────────┘
 q quit · r refresh · n feed · t alerts · ? help · o open · tab · j/k   updated 2s · PRICE · alerts:off · v0.4.0
```

## Identity

| Field | Value |
|-------|--------|
| Package | `bullboard` (Rust, edition 2021) |
| CLI | `bullboard` (flags: `--api-base`, `--handle`, `--once`) |
| Tracked token | **$ANSEM** mint `9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump` |
| Tracked X | **@blknoiz06** (override: `BULLBOARD_X_HANDLE`) |
| API base | `https://api.bullinf.fun` (override: `BULLBOARD_API_BASE`) |
| Brand | canvas `#050604`, panel `#161812`, acid `#c8f542`, per-pane accents |

## Data sources (public only — no auth)

| Pane / data | Source |
|-------------|--------|
| Price, 24h change, OHLC (sparklines, 24h high/low) | Bull.inf `GET /api/token-price` + `GET /api/token-ohlc?interval=1h&limit=48` |
| Token details, window stats, audit | Jupiter `GET https://lite-api.jup.ag/tokens/v2/search?query={mint}` |
| Second-opinion price / liquidity / volume / mcap | GeckoTerminal `…/tokens/{mint}` + `…/tokens/{mint}/info` (holder distribution). `/pools` not polled — token `total_reserve_in_usd` is the liquidity signal. |
| Rug check, LP lock, insiders, top holders | RugCheck `GET https://api.rugcheck.xyz/v1/tokens/{mint}/report` |
| DEX pairs (liquidity, volume, pool age, quote) | DexScreener `GET https://api.dexscreener.com/latest/dex/tokens/{mint}` |
| Announce feed | Nitter mirrors (public RSS, fresh-cursor bypass, circuit breaker) |

Resilience: one retry on 5xx/timeout (and on 429 for non-Gecko hosts;
GeckoTerminal 429 is not retried — free-tier IP budget). A failed poll
keeps the last good values (`Snapshot::merge_stale`) so the board never
flickers `—`; the header shows a live `src n/m` health counter.

## UX keys

- `q` / `Esc` / `Ctrl+C` — quit (panic hook restores the terminal on crash)
- `?` / `h` — help overlay (full key + mouse reference)
- `r` — force refresh all (background — input never blocks)
- `n` — refresh announce feed, reset scroll
- `t` — toggle desktop notifications
- `tab` / `shift-tab`, `1`–`9` — pane focus
- `j`/`k`/arrows, `space`/`f`/`b`, `g`/`G` — scroll focused pane
- `enter` / `o` — open tweet in browser
- mouse — hover highlight, click focus, wheel scroll, click `[ view tweet ]`

Auto-refresh: data every **15s**, feed every **30s** (`BULLBOARD_FEED_SECS`,
min 5).

## Architecture

- `src/config.rs` — env config, palette constants
- `src/model.rs` — data model, `Snapshot::merge_stale` keep-last-good
- `src/fetch.rs` — HTTP (retry-once), JSON/RSS parsing, feed circuit breaker
- `src/app.rs` — app state, background refresh channel, pane line builders,
  notifications, event loop
- `src/ui.rs` — ratatui layout + rendering (accents, delta coloring, badges,
  truncation, help overlay)
- `src/format.rs` — number/date/sparkline/bar formatters

## Success criteria

1. `cargo install --git …` → `bullboard` launches the full TUI
2. First frame draws instantly; live price visible as soon as the first fetch lands
3. @blknoiz06 tweets (or clear empty/fallback state) in the announce pane
4. Cross-checks: Gecko vs primary price (`(!)` >1%), Gecko vs Dex liquidity (`(!)` >25%)
5. Resize-safe layout with stacking breakpoints; graceful at ≥ 52×16
6. No crash if one endpoint fails — pane shows `—`, last good values persist
7. Zero clippy warnings, unit tests green

## Stack

- Rust 2021: ratatui 0.29, crossterm, tokio, reqwest (rustls), quick-xml,
  serde_json, chrono, clap, anyhow

## Anti-goals

- No wallet connect / signing
- No accounts, passwords, or API keys
- No deploy / server component — reads public data only
- No fake precision KPIs
