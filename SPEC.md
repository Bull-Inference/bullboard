# bullboard — terminal stalker for Bull.inf / $ANSEM / @blknoiz06

## Outcome (must match)

A **professional multi-pane terminal dashboard** in the spirit of Surfboard (MAXPANE):
dark matrix aesthetic, live-updating panes, keyboard-driven. Installable via **pipx**
(primary) and optionally **npm** (thin launcher).

```
┌─ BULLBOARD · $ANSEM · parity · feed · activity ──────────────────────────────┐
│ BULL GATE          │ TREASURY / MINT        │ STAKE / FEES        │ ANSEM MCAP │
│ min wallet, onchain│ pubkey, mint short     │ fee split, pool     │ price/fdv  │
├────────────────────┴────────────────────────┼─────────────────────┴───────────┤
│ ANNOUNCE FEED · @blknoiz06 · last N ago     │ SIGNALS                         │
│ MM-DD HH:MM POST  tweet text…               │ · SOLANA OK · markets live      │
│ …                                           │ · liquidity · 24h activity      │
│                                             ├─────────────────────────────────┤
│                                             │ INFERENCE ACTIVITY              │
│                                             │ time model cost ANSEM           │
├─────────────────────────────────────────────┴─────────────────────────────────┤
│ ANSEM MARKET                                │ BULL.INF DESK                   │
│ price 24hΔ vol pool spark                   │ reqs sellers models liq         │
└───────────────────────────────────────────────────────────────────────────────┘
 q quit · r refresh · n next feed · tab focus · updated Xs ago · bullboard v0.x
```

## Identity

| Field | Value |
|-------|--------|
| Package | `bullboard` |
| CLI | `bullboard` |
| Tracked token | **$ANSEM** mint `9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump` |
| Tracked X | **@blknoiz06** (override: `BULLBOARD_X_HANDLE`) |
| API base | `https://api.bullinf.fun` (override: `BULLBOARD_API_BASE`) |
| Brand | ink `#0a0b09`, acid `#c8f542`, mono (IBM Plex Mono / system mono) |

## Data sources (public only — no auth)

| Pane | Source |
|------|--------|
| Gate / treasury / mint | `GET /api/network` + `/api/config` |
| ANSEM price / mcap / spark | `GET /api/token-price` + `/api/token-ohlc?interval=1h&limit=48` |
| Stake / fees | `GET /api/stake/stats` |
| Announce feed | Nitter RSS for `@blknoiz06` (+ optional second handle `bullinference`) |
| Signals | Derived health from network + stats + markets |
| Inference activity | `GET /api/markets/feed?limit=20` |
| Desk KPIs | `GET /api/stats` + `/api/stats/daily` |
| Top models | `GET /api/markets` (sorted by liquidity) |

Fallback: DexScreener token endpoint if Bull price is stale.

## UX keys

- `q` / `Ctrl+C` — quit
- `r` — force refresh all
- `n` — cycle announce feed handle (blknoiz06 ↔ bullinference ↔ both)
- `tab` — cycle focused pane (if Textual)
- Auto-refresh every **15s** (price/stats) / **60s** (tweets)

## Success criteria

1. `pipx install .` (or `pip install -e .`) → `bullboard` launches full TUI
2. Live $ANSEM price + 24h change visible within 2s of launch
3. @blknoiz06 tweets (or clear empty/fallback state) in announce pane
4. Inference feed shows recent Bull.inf completions when API has data
5. Resize-safe layout (min 100×30 recommended)
6. No crash if one endpoint fails — pane shows `—` / error badge
7. README with one-liner install matching Surfboard tweet energy

## Stack

- Python 3.10+
- `textual` (multi-pane TUI)
- `httpx` (async HTTP)
- stdlib RSS/XML for Nitter

## Anti-goals

- No wallet connect / signing
- No admin endpoints
- No Surplus branding
- No fake precision KPIs
- No purple SaaS look
