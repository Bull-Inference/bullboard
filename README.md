# bullboard

Surfboard-style **Rust TUI** for **Bull.inf** — stalk **$ANSEM** and **@blknoiz06** from your shell.

Multi-pane dashboard with **scrollable sections** (ratatui). Hero-first KPI cards, colored `POST` badges, dual sparklines, holder distribution, audit signals. Tab-focus a pane, then `j/k` / arrows / mouse wheel to scroll.

**v0.3** visual polish pass — closer to MAXPANE / Surfboard hierarchy.

```
BULLBOARD · $ANSEM · @blknoiz06 · feed · activity
```

## Install

### Cargo (recommended)

```bash
cargo install --path /path/to/bullboard
# or from git once published:
# cargo install --git https://github.com/Bull-Inference/bullboard
```

Binary: `bullboard` on your PATH (`~/.cargo/bin`).

### From this repo

```bash
cd bullboard
cargo build --release
cargo install --path .
```

### Legacy Python (pipx)

The original Textual app lives in `legacy-python/` if you still want it:

```bash
pipx install ./legacy-python
```

## Run

```bash
bullboard
bullboard --once          # JSON smoke, no TUI
bullboard --handle blknoiz06
```

### Keys

| Key | Action |
|-----|--------|
| `q` / `Esc` | quit |
| `r` | force refresh all |
| `n` | refresh announce feed (`@blknoiz06`) |
| `Tab` / `Shift+Tab` | focus next / prev pane |
| `↑↓` / `j k` | scroll focused pane |
| `PgUp` / `PgDn` | scroll faster |
| `Home` | scroll to top |
| `1`–`9` | jump focus to pane |
| mouse wheel | scroll focused pane |

Focused pane gets an **acid border** + scrollbar when content overflows.

## Env

| Var | Default |
|-----|---------|
| `BULLBOARD_API_BASE` | `https://api.bullinf.fun` |
| `BULLBOARD_X_HANDLE` | `blknoiz06` |
| `BULLBOARD_MINT` | ANSEM mint |

## Data (public, no keys)

- Bull.inf: network, price, OHLC, stake, markets feed
- Jupiter / Gecko / Rugcheck: holder count, distribution, top wallets
- Nitter RSS: @blknoiz06 announce feed

Mint: `9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump` · Desk: [bullinf.fun](https://bullinf.fun)

## Inspired by

[Surfboard / MAXPANE](https://x.com/cryptokarlheinz/status/2086826710180286738)
