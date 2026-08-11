<div align="center">

# ⬢ bullboard

**Surfboard-style Rust TUI for Bull.inf** — stalk `$ANSEM` and `@blknoiz06` from your shell.

![version](https://img.shields.io/badge/version-0.3.3-0a0b09?style=for-the-badge&labelColor=1b1f16)
![license](https://img.shields.io/badge/license-MIT-0a0b09?style=for-the-badge&labelColor=1b1f16)
![rust](https://img.shields.io/badge/rust-2021_edition-0a0b09?style=for-the-badge&labelColor=1b1f16&logo=rust&logoColor=c8f542)
![tui](https://img.shields.io/badge/built_with-ratatui-0a0b09?style=for-the-badge&labelColor=1b1f16)
![solana](https://img.shields.io/badge/solana-%24ANSEM-0a0b09?style=for-the-badge&labelColor=1b1f16&logo=solana&logoColor=c8f542)
![x](https://img.shields.io/badge/x-%40blknoiz06-0a0b09?style=for-the-badge&labelColor=1b1f16&logo=x&logoColor=c8f542)

<img src="docs/bullboard.png" alt="bullboard — Surfboard-style TUI for $ANSEM / @blknoiz06" width="100%" />

</div>

---

## ✨ Features

- **Multi-pane dashboard** — Surfboard / MAXPANE-style layout built on `ratatui`
- **Hero KPI cards** — `$ANSEM` price, 24h change, volume, liquidity, holders at a glance
- **Announce feed** — `POST`-badged tweets from `@blknoiz06`, live within ~30s of posting (bypasses Nitter's 10-minute RSS cache), each with a `[ view tweet ]` button
- **Live market data** — dual sparklines (price + volume), holder distribution bars, top wallets
- **Signals & activity** — health signals, DEX flow, and the Bull.inf inference feed
- **Mouse-first interaction** — hover highlighting, click-to-focus, wheel-to-scroll
- **Keyboard-driven** — tab-focus panes, jump with `1`–`9`, scroll with `j/k`, open tweets with `o`
- **Auto-refresh** — data every 15s, feed every 30s; a failing endpoint shows `—`, never crashes
- **Desktop alerts** — opt-in notification when `@blknoiz06` posts (macOS Notification Center / Linux `notify-send`); `t` toggles it live

## 🚀 Install

### Cargo (recommended)

```bash
cargo install --git https://github.com/Bull-Inference/bullboard
```

Binary: `bullboard` on your PATH (`~/.cargo/bin`).

### From this repo

```bash
git clone https://github.com/Bull-Inference/bullboard
cd bullboard
cargo build --release
cargo install --path .
```

### Legacy Python (pipx)

The original Textual app lives in `legacy-python/` if you still want it:

```bash
pipx install ./legacy-python
```

## ▶️ Run

```bash
bullboard                    # launch the TUI
bullboard --once             # fetch snapshot JSON and exit (no TUI)
bullboard --handle blknoiz06 # override the announce feed handle
```

## ⌨️ Keys

| Key | Action |
|-----|--------|
| `q` / `Esc` | quit |
| `r` | force refresh all |
| `n` | refresh announce feed (`@blknoiz06`) |
| `t` | toggle desktop notifications (initial state from `BULLBOARD_NOTIFY`) |
| `Tab` / `Shift+Tab` | focus next / prev pane |
| `↑↓` / `j k` | scroll focused pane |
| `PgUp` / `PgDn` · `f` / `b` · `Space` | scroll faster |
| `Home` / `g` | scroll to top |
| `End` / `G` | scroll to bottom |
| `Enter` / `o` | open tweet under cursor |
| `1`–`9` | jump focus to pane |
| mouse | hover highlight · click focuses · wheel scrolls pane under cursor |

## ⚙️ Env

| Var | Default |
|-----|---------|
| `BULLBOARD_API_BASE` | `https://api.bullinf.fun` |
| `BULLBOARD_X_HANDLE` | `blknoiz06` |
| `BULLBOARD_MINT` | ANSEM mint |
| `BULLBOARD_NOTIFY` | `1` — desktop notifications on at launch (default off; toggle anytime with `t`) |
| `BULLBOARD_FRESH_FEED` | `1` — bypass the mirrors' 10-minute RSS cache: every poll triggers a live fetch, so tweets land within ~30s of posting (set `0` for polite cached mode) |
| `BULLBOARD_FEED_SECS` | `30` — announce feed poll interval in seconds (min 5) |
| `BULLBOARD_MIRRORS` | `https://nitter.net,...` — comma-separated Nitter mirrors; mirrors that keep failing are auto-skipped for 5 min, so swapping in a fresh instance is just an env change |

## 📡 Data (public, no keys)

| Pane | Source |
|------|--------|
| Network / gate / treasury | Bull.inf `GET /api/network` + `/api/config` |
| Price / mcap / spark | `GET /api/token-price` + `/api/token-ohlc` |
| Stake / fees | `GET /api/stake/stats` |
| Announce feed | Nitter RSS for `@blknoiz06` |
| Signals | Derived health from network + stats + markets |
| DEX flow / inference | `GET /api/markets/feed` + `GET /api/markets` |
| Holders / distribution | Jupiter / Gecko / Rugcheck |

Mint: `9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump` · Desk: [bullinf.fun](https://bullinf.fun)

## 🔗 Inspired by

[Surfboard / MAXPANE](https://x.com/cryptokarlheinz/status/2086826710180286738)

## 📄 License

MIT — see [LICENSE](LICENSE)
