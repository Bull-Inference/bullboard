# bullboard

Surfboard-style terminal dashboard for **Bull.inf** — stalk **$ANSEM** and **@blknoiz06** from your shell.

```
BULLBOARD · $ANSEM · @blknoiz06 · feed · activity
```

Live panes: gate / treasury / stake fees / mcap · announce feed · signals · inference activity · ANSEM market · desk KPIs.

## Install

### pipx (recommended)

```bash
pipx install git+https://github.com/Bull-Inference/bullboard.git
# or from a local clone:
pipx install /path/to/bullboard
```

### pip (venv / editable)

```bash
pip install -e .
```

### npm (thin launcher — runs the Python CLI)

```bash
npm install -g bullboard-cli
# or once published; for local:
npm install -g ./npm-launcher
```

## Run

```bash
bullboard
```

Keys: `q` quit · `r` refresh · `n` cycle X feed (`blknoiz06` / `bullinference` / both)

Smoke (no TUI):

```bash
bullboard --once
```

## Env

| Var | Default | Meaning |
|-----|---------|---------|
| `BULLBOARD_API_BASE` | `https://api.bullinf.fun` | Bull API |
| `BULLBOARD_X_HANDLE` | `blknoiz06` | Primary X announce feed |
| `BULLBOARD_X_HANDLE_ALT` | `bullinference` | Alt feed |
| `BULLBOARD_REFRESH_SEC` | `15` | Data poll interval |
| `BULLBOARD_FEED_REFRESH_SEC` | `60` | Tweet poll interval |

## Data

Public Bull.inf endpoints only (`/api/token-price`, `/api/stats`, `/api/markets/feed`, …) + Nitter RSS for X. No wallet. No auth.

Mint: `9cRCn9rGT8V2imeM2BaKs13yhMEais3ruM3rPvTGpump` · Desk: [bullinf.fun](https://bullinf.fun)

## Inspired by

[Surfboard / MAXPANE](https://x.com/cryptokarlheinz/status/2086826710180286738) — same energy, Bull rails.
